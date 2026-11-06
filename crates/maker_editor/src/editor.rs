//! Estado del editor y operaciones (sin UI): escena, selección, deshacer,
//! modo Play, compilación y recarga de scripts.

use crate::camera::EditorCamera;
use crate::gizmo::Gizmo;
use glam::Vec3;
use maker_engine::components::*;
use maker_engine::scene::SceneFile;
use maker_engine::ui::plantilla::PlantillaUi;
use maker_engine::ui::{self, SistemaUi};
use maker_engine::{Input, Project, Runtime, Scene, ScriptLib, log};
use maker_render::{SceneRenderer, UiRenderer};
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant, SystemTime};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BottomTab {
    Content,
    Library,
    Console,
    Script,
    Estilos,
}

/// Estado guardado en el historial: la escena y los documentos de estilos y
/// temas de interfaz (ruta → JSON), que también se editan desde el editor.
#[derive(Clone, Debug, PartialEq)]
pub struct Instantanea {
    pub escena: SceneFile,
    pub ui: BTreeMap<String, String>,
}

/// Historial de deshacer basado en instantáneas (simple y exacto: cada paso
/// es la escena completa serializada más los estilos de interfaz).
#[derive(Default)]
pub struct Undo {
    undo: Vec<(String, Instantanea)>,
    redo: Vec<(String, Instantanea)>,
    pub committed: Option<Instantanea>,
}

impl Undo {
    pub const LIMIT: usize = 200;
    pub fn reset_con(&mut self, scene: &Scene, ui: BTreeMap<String, String>) {
        self.undo.clear();
        self.redo.clear();
        self.committed = Some(Instantanea { escena: scene.to_file(), ui });
    }
    /// Si la escena o los documentos de interfaz cambiaron respecto a lo último
    /// confirmado, apila el estado anterior.
    pub fn commit_con(&mut self, scene: &Scene, ui: BTreeMap<String, String>, label: &str) -> bool {
        let now = Instantanea { escena: scene.to_file(), ui };
        match &self.committed {
            Some(prev) if *prev == now => false,
            Some(prev) => {
                self.undo.push((label.to_string(), prev.clone()));
                if self.undo.len() > Self::LIMIT {
                    self.undo.remove(0);
                }
                self.redo.clear();
                self.committed = Some(now);
                true
            }
            None => {
                self.committed = Some(now);
                false
            }
        }
    }
    pub fn undo(&mut self) -> Option<(String, Instantanea)> {
        let (label, prev) = self.undo.pop()?;
        if let Some(cur) = self.committed.replace(prev.clone()) {
            self.redo.push((label.clone(), cur));
        }
        Some((label, prev))
    }
    pub fn redo(&mut self) -> Option<(String, Instantanea)> {
        let (label, next) = self.redo.pop()?;
        if let Some(cur) = self.committed.replace(next.clone()) {
            self.undo.push((label.clone(), cur));
        }
        Some((label, next))
    }
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
    pub fn undo_label(&self) -> Option<&str> {
        self.undo.last().map(|u| u.0.as_str())
    }
    pub fn redo_label(&self) -> Option<&str> {
        self.redo.last().map(|u| u.0.as_str())
    }
}

pub struct PlaySession {
    pub runtime: Runtime,
    pub snapshot: SceneFile,
    pub selection: Option<EntityId>,
}

pub struct BuildJob {
    pub rx: Receiver<BuildMsg>,
    pub started: Instant,
}

pub enum BuildMsg {
    Line(String),
    Done(bool),
}

/// Un script abierto en el editor de código.
pub struct OpenScript {
    pub path: PathBuf,
    pub text: String,
    pub saved: String,
}

pub struct Editor {
    pub project: Project,
    pub scene: Scene,
    pub scene_path: PathBuf,
    pub selection: Option<EntityId>,
    pub undo: Undo,
    pub play: Option<PlaySession>,
    pub lib: Option<Arc<ScriptLib>>,
    pub lib_mtime: Option<SystemTime>,
    pub lib_error: Option<String>,
    pub build: Option<BuildJob>,
    pub last_build_ok: Option<bool>,
    pub renderer: SceneRenderer,
    pub tex_id: Option<egui::TextureId>,
    pub tex_gen: u64,
    pub cam: EditorCamera,
    pub gizmo: Gizmo,
    pub input: Input,
    pub game_focus: bool,
    /// Captura del ratón durante Play.
    pub cursor: maker_engine::CursorCapture,
    pub show_game_view: bool,
    pub show_grid: bool,
    pub shadows: bool,
    /// Posprocesado en el visor (Play incluido).
    pub post_fx: bool,
    pub bottom_tab: BottomTab,
    pub script: Option<OpenScript>,
    pub hierarchy_filter: String,
    pub rename: Option<(EntityId, String)>,
    pub euler_cache: Option<(EntityId, glam::Quat, Vec3)>,
    pub console_seen: usize,
    pub console_filter: [bool; 3],
    pub status: String,
    pub frame_times: Vec<f32>,
    pub last_lib_check: Instant,
    /// Compilar solos los scripts cuando cambian sus fuentes (Preferencias).
    pub auto_build: bool,
    /// Huella de `scripts/src` + `Cargo.toml` de la última compilación (o de la apertura).
    src_huella: Option<u64>,
    /// Comprobar las fuentes en el siguiente fotograma (p. ej. al guardar un script).
    comprobar_fuentes: bool,
    pub new_script_name: String,
    pub importador: crate::importar::Importador,
    pub pending_reload_after_play: bool,
    /// Escena modificada desde el último guardado.
    pub dirty: bool,
    /// Edición en curso del inspector; se confirma al soltar el ratón.
    pub pending_edit: Option<String>,
    pub pending_save_script: bool,
    /// Navegador de la biblioteca integrada.
    pub biblioteca: crate::biblioteca_ui::BibliotecaUi,
    pub contenido: crate::contenido_ui::ContenidoUi,
    /// Dónde está el visor 3D en la ventana (para capturarlo).
    pub viewport_rect: egui::Rect,
    /// Animación en modo edición (vista previa del Animator seleccionado).
    pub animation: maker_engine::animation::Animation,
    /// Interfaz en modo edición (estilos y temas del proyecto, layout para dibujarla y editarla).
    pub ui: SistemaUi,
    pub ui_render: Option<UiRenderer>,
    /// Modo de edición 2D de la interfaz en el viewport.
    pub modo_ui: bool,
    /// Dibujar la interfaz sobre el viewport.
    pub mostrar_ui: bool,
    pub ui_arrastre: Option<crate::ui_editor::Arrastre>,
    /// Editor de estilos y temas.
    pub estilos: crate::estilos_ui::EstilosUi,
    /// Plantillas leídas (para calcular los retoques de cada instancia).
    plantillas: BTreeMap<String, Option<PlantillaUi>>,
}

impl Editor {
    pub fn open(project: Project, rs: &egui_wgpu::RenderState) -> Editor {
        let scene_path = project.main_scene_path();
        let scene = match Scene::load(&scene_path) {
            Ok(s) => s,
            Err(e) => {
                log::error("editor", format!("no se pudo abrir la escena principal: {e}"));
                Scene::starter()
            }
        };
        let mut renderer = SceneRenderer::new(&rs.device, &rs.queue);
        renderer.project_root = Some(project.root.clone());
        let ui = SistemaUi::del_proyecto(&project.root, &project.file.tema_ui);
        let mut scene = scene;
        if ui::plantilla::sincronizar_todas(&mut scene, &project.root) > 0 {
            log::info("interfaz", "instancias de plantillas puestas al día");
        }
        let mut ed = Editor {
            project,
            scene,
            scene_path,
            selection: None,
            undo: Undo::default(),
            play: None,
            lib: None,
            lib_mtime: None,
            lib_error: None,
            build: None,
            last_build_ok: None,
            renderer,
            tex_id: None,
            tex_gen: 0,
            cam: EditorCamera::default(),
            gizmo: Gizmo::default(),
            input: Input::default(),
            game_focus: false,
            cursor: Default::default(),
            show_game_view: true,
            show_grid: true,
            shadows: true,
            post_fx: true,
            bottom_tab: BottomTab::Content,
            script: None,
            hierarchy_filter: String::new(),
            rename: None,
            euler_cache: None,
            console_seen: 0,
            console_filter: [true; 3],
            status: String::new(),
            frame_times: Vec::new(),
            last_lib_check: Instant::now(),
            auto_build: true,
            src_huella: None,
            comprobar_fuentes: true,
            new_script_name: String::new(),
            importador: Default::default(),
            pending_reload_after_play: false,
            dirty: false,
            pending_edit: None,
            pending_save_script: false,
            biblioteca: Default::default(),
            contenido: Default::default(),
            viewport_rect: egui::Rect::NOTHING,
            animation: Default::default(),
            ui,
            ui_render: None,
            modo_ui: false,
            mostrar_ui: true,
            ui_arrastre: None,
            estilos: Default::default(),
            plantillas: BTreeMap::new(),
        };
        let docs = ed.docs_ui();
        ed.undo.reset_con(&ed.scene, docs);
        ed.reload_scripts(false);
        log::info("editor", format!("proyecto abierto: {} ({})", ed.project.name(), ed.project.root.display()));
        ed
    }

    pub fn is_playing(&self) -> bool {
        self.play.is_some()
    }

    pub fn scene_name(&self) -> String {
        self.scene_path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "sin título".into())
    }

    // ───────────────────────────── escena ─────────────────────────────

    pub fn save_scene(&mut self) {
        if self.is_playing() {
            self.status = "No se guarda durante Play (los cambios de Play se descartan al detener)".into();
            return;
        }
        match self.scene.save(&self.scene_path) {
            Ok(()) => {
                self.dirty = false;
                self.status = format!("Escena guardada: {}", self.project.relative(&self.scene_path));
                log::info("editor", self.status.clone());
            }
            Err(e) => log::error("editor", format!("al guardar: {e}")),
        }
    }

    pub fn open_scene(&mut self, rel: &str) {
        if self.is_playing() {
            self.stop();
        }
        let path = self.project.root.join(rel);
        match Scene::load(&path) {
            Ok(mut s) => {
                ui::plantilla::sincronizar_todas(&mut s, &self.project.root);
                self.scene = s;
                self.scene_path = path;
                self.selection = None;
                let docs = self.docs_ui();
                self.undo.reset_con(&self.scene, docs);
                self.dirty = false;
                log::info("editor", format!("escena abierta: {rel}"));
            }
            Err(e) => log::error("editor", e),
        }
    }

    pub fn new_scene(&mut self, name: &str) {
        let rel = format!("scenes/{}.mscene", name.trim().replace(['/', '\\'], "_"));
        let path = self.project.root.join(&rel);
        if path.exists() {
            log::warn("editor", format!("{rel} ya existe"));
            return;
        }
        let s = Scene::starter();
        if let Err(e) = s.save(&path) {
            log::error("editor", e);
            return;
        }
        self.open_scene(&rel);
    }

    /// Confirma un cambio en el historial (llamar cuando termina una edición).
    pub fn commit(&mut self, label: &str) {
        if self.is_playing() {
            return;
        }
        self.actualizar_retoques();
        let docs = self.docs_ui();
        // Los estilos y temas se guardan al confirmar (son assets, no parte de la escena).
        let antes = self.undo.committed.as_ref().map(|c| c.ui.clone()).unwrap_or_default();
        if antes != docs {
            for (ruta, txt) in &docs {
                if antes.get(ruta) != Some(txt) {
                    let p = self.project.root.join(ruta);
                    if let Some(d) = p.parent() {
                        let _ = std::fs::create_dir_all(d);
                    }
                    if let Err(e) = std::fs::write(&p, txt) {
                        log::error("interfaz", format!("{ruta}: {e}"));
                    }
                }
            }
            for ruta in antes.keys().filter(|r| !docs.contains_key(*r)) {
                let _ = std::fs::remove_file(self.project.root.join(ruta));
            }
        }
        let escena_antes = self.undo.committed.as_ref().map(|c| c.escena.clone());
        if self.undo.commit_con(&self.scene, docs, label) && escena_antes.as_ref() != Some(&self.scene.to_file()) {
            self.dirty = true;
        }
    }

    /// Estilos y temas de interfaz como texto (para el historial).
    pub fn docs_ui(&self) -> BTreeMap<String, String> {
        let r = &self.ui.recursos;
        let mut out = BTreeMap::new();
        for (ruta, h) in &r.hojas {
            out.insert(ruta.clone(), serde_json::to_string_pretty(h).unwrap_or_default());
        }
        for (n, t) in &r.temas {
            let ruta = r.rutas_temas.get(n).cloned().unwrap_or_else(|| format!("ui/temas/{n}.mtema"));
            out.insert(ruta, serde_json::to_string_pretty(t).unwrap_or_default());
        }
        out
    }

    /// Escribe a disco los documentos de interfaz que cambiaron (y borra los
    /// que ya no están) y los recarga en memoria.
    pub fn aplicar_docs_ui(&mut self, docs: &BTreeMap<String, String>) {
        let actuales = self.docs_ui();
        for (ruta, txt) in docs {
            if actuales.get(ruta) != Some(txt) {
                let p = self.project.root.join(ruta);
                if let Some(d) = p.parent() {
                    let _ = std::fs::create_dir_all(d);
                }
                if let Err(e) = std::fs::write(&p, txt) {
                    log::error("interfaz", format!("{ruta}: {e}"));
                }
            }
        }
        for ruta in actuales.keys().filter(|r| !docs.contains_key(*r)) {
            let _ = std::fs::remove_file(self.project.root.join(ruta));
        }
        let r = &mut self.ui.recursos;
        r.hojas.clear();
        r.temas.clear();
        r.rutas_temas.clear();
        for (ruta, txt) in docs {
            if ruta.ends_with(&format!(".{}", ui::EXT_ESTILO)) {
                if let Ok(h) = serde_json::from_str(txt) {
                    r.hojas.insert(ruta.clone(), h);
                }
            } else if let Ok(t) = serde_json::from_str(txt) {
                let n = std::path::Path::new(ruta).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                r.rutas_temas.insert(n.clone(), ruta.clone());
                r.temas.insert(n, t);
            }
        }
        r.reindexar();
    }

    /// Plantilla por ruta (cacheada).
    pub fn plantilla(&mut self, ruta: &str) -> Option<PlantillaUi> {
        let raiz = self.project.root.clone();
        self.plantillas
            .entry(ruta.to_string())
            .or_insert_with(|| match PlantillaUi::cargar(&raiz.join(ruta)) {
                Ok(p) => Some(p),
                Err(e) => {
                    log::warn("interfaz", e);
                    None
                }
            })
            .clone()
    }

    /// Recalcula los retoques de las instancias de plantillas tras una edición.
    fn actualizar_retoques(&mut self) {
        let raices: Vec<(EntityId, String)> = self.scene.world.query::<(&EntityId, &ui::InstanciaUi)>().iter().map(|(id, i)| (*id, i.plantilla.clone())).collect();
        for (id, ruta) in raices {
            if let Some(p) = self.plantilla(&ruta) {
                ui::plantilla::actualizar_overrides(&mut self.scene, id, &p);
            }
        }
    }

    /// Guarda el subárbol de `id` como plantilla `ui/plantillas/<nombre>.mui` y lo
    /// convierte en una instancia suya.
    pub fn guardar_como_plantilla(&mut self, id: EntityId) {
        let Some(p) = PlantillaUi::desde_escena(&self.scene, id) else { return };
        let nombre: String = self.scene.name(id).chars().map(|c| if c.is_alphanumeric() || c == '_' || c == '-' { c } else { '_' }).collect();
        let mut rel = format!("{}/plantillas/{nombre}.{}", ui::DIR_UI, ui::EXT_PLANTILLA);
        let mut k = 1;
        while self.project.root.join(&rel).exists() {
            k += 1;
            rel = format!("{}/plantillas/{nombre}_{k}.{}", ui::DIR_UI, ui::EXT_PLANTILLA);
        }
        if let Err(e) = p.guardar(&self.project.root.join(&rel)) {
            log::error("interfaz", e);
            return;
        }
        self.vincular(id, &rel, &p);
        self.plantillas.insert(rel.clone(), Some(p));
        self.commit("Guardar como plantilla");
        log::info("interfaz", format!("plantilla guardada: {rel}"));
    }

    /// Marca un subárbol como instancia de una plantilla recién guardada desde él.
    fn vincular(&mut self, raiz: EntityId, rel: &str, p: &PlantillaUi) {
        let h = self.scene.hierarchy();
        let mut orden = vec![raiz];
        let mut i = 0;
        while i < orden.len() {
            orden.extend_from_slice(h.children_of(orden[i]));
            i += 1;
        }
        for (k, id) in orden.iter().enumerate() {
            if let Some(r) = p.entidades.get(k) {
                self.scene.insert(*id, ui::OrigenPlantilla(r.id.0));
            }
        }
        self.scene.insert(raiz, ui::InstanciaUi { plantilla: rel.to_string(), overrides: Default::default() });
    }

    /// Escribe la instancia `raiz` como su plantilla y pone al día las demás instancias.
    pub fn aplicar_a_plantilla(&mut self, raiz: EntityId) {
        let Some(inst) = self.scene.get::<ui::InstanciaUi>(raiz) else { return };
        let Some(p) = PlantillaUi::desde_escena(&self.scene, raiz) else { return };
        if let Err(e) = p.guardar(&self.project.root.join(&inst.plantilla)) {
            log::error("interfaz", e);
            return;
        }
        // Los ids locales pasan a ser los del orden actual.
        self.vincular(raiz, &inst.plantilla, &p);
        self.plantillas.insert(inst.plantilla.clone(), Some(p.clone()));
        let otras: Vec<EntityId> = self.scene.world.query::<(&EntityId, &ui::InstanciaUi)>().iter().filter(|(id, i)| **id != raiz && i.plantilla == inst.plantilla).map(|(id, _)| *id).collect();
        for o in otras {
            ui::plantilla::sincronizar(&mut self.scene, o, &p);
        }
        self.commit("Aplicar a la plantilla");
    }

    /// Vuelve a los valores de la plantilla (todos o un retoque).
    pub fn revertir_plantilla(&mut self, raiz: EntityId, retoque: Option<&str>) {
        let Some(inst) = self.scene.get::<ui::InstanciaUi>(raiz) else { return };
        let Some(p) = self.plantilla(&inst.plantilla) else { return };
        if let Some(mut i) = self.scene.get_mut::<ui::InstanciaUi>(raiz) {
            match retoque {
                Some(r) => {
                    i.overrides.remove(r);
                }
                None => i.overrides.clear(),
            }
        }
        ui::plantilla::sincronizar(&mut self.scene, raiz, &p);
        self.commit("Revertir a la plantilla");
    }

    /// Instancia una plantilla `.mui` bajo `padre` (o en el lienzo de la selección).
    pub fn instanciar_plantilla(&mut self, rel: &str, padre: Option<EntityId>) -> Option<EntityId> {
        self.plantillas.remove(rel);
        let p = self.plantilla(rel)?;
        let padre = padre.or_else(|| self.lienzo_destino());
        let id = p.instanciar(&mut self.scene, rel, padre)?;
        self.selection = Some(id);
        self.commit("Instanciar plantilla");
        Some(id)
    }

    /// Dónde crear elementos de interfaz: la selección si es de interfaz, su
    /// lienzo, o el primer lienzo de la escena (se crea uno si no hay).
    pub fn lienzo_destino(&mut self) -> Option<EntityId> {
        if let Some(s) = self.selection.filter(|s| self.scene.contains(*s))
            && (self.scene.has::<ui::RectUi>(s) || self.scene.has::<ui::Lienzo>(s))
        {
            return Some(s);
        }
        let primero = self.scene.world.query::<(&EntityId, &ui::Lienzo)>().iter().map(|(i, _)| *i).min_by_key(|i| i.0);
        if primero.is_some() {
            return primero;
        }
        let l = self.scene.spawn("Lienzo");
        self.scene.insert(l, ui::Lienzo::default());
        Some(l)
    }

    /// Crea un elemento de interfaz.
    pub fn crear_ui(&mut self, que: CrearUi) -> EntityId {
        if que == CrearUi::Lienzo {
            let id = self.scene.spawn("Lienzo");
            self.scene.insert(id, ui::Lienzo::default());
            self.selection = Some(id);
            self.commit("Crear Lienzo");
            return id;
        }
        let padre = self.lienzo_destino();
        let id = self.scene.spawn(que.label());
        let (rect, widget, cont) = que.componentes();
        self.scene.insert(id, rect);
        if let Some(w) = widget {
            self.scene.insert(id, w);
        }
        if let Some(c) = cont {
            self.scene.insert(id, c);
        }
        if let Some(p) = padre {
            self.scene.set_parent(id, Some(p), false);
        }
        self.selection = Some(id);
        self.commit(&format!("Crear {}", que.label()));
        id
    }

    pub fn undo(&mut self) {
        if self.is_playing() {
            return;
        }
        if let Some((label, f)) = self.undo.undo() {
            self.scene = Scene::from_file(&f.escena);
            self.aplicar_docs_ui(&f.ui);
            self.after_history();
            self.status = format!("Deshecho: {label}");
        }
    }
    pub fn redo(&mut self) {
        if self.is_playing() {
            return;
        }
        if let Some((label, f)) = self.undo.redo() {
            self.scene = Scene::from_file(&f.escena);
            self.aplicar_docs_ui(&f.ui);
            self.after_history();
            self.status = format!("Rehecho: {label}");
        }
    }
    fn after_history(&mut self) {
        if self.selection.is_some_and(|s| !self.scene.contains(s)) {
            self.selection = None;
        }
        self.euler_cache = None;
        self.dirty = true;
    }

    /// Crea una entidad delante de la cámara (o hija de la selección con `as_child`).
    pub fn create(&mut self, what: Create) -> EntityId {
        if let Create::Ui(u) = what {
            return self.crear_ui(u);
        }
        let pos = (self.cam.pivot * 2.0).round() * 0.5;
        let name = what.label();
        let id = self.scene.spawn(name);
        let mut t = Transform::from_translation(Vec3::new(pos.x, pos.y.max(0.0), pos.z));
        match what {
            Create::Ui(_) | Create::Empty => {}
            Create::Shape(s) => {
                if s != PrimitiveShape::Plane {
                    t.translation.y = t.translation.y.max(0.5);
                }
                self.scene.insert(id, MeshRenderer::primitive(s));
            }
            Create::DirectionalLight => {
                t.translation.y = 8.0;
                t.set_euler_degrees(Vec3::new(-50.0, -30.0, 0.0));
                self.scene.insert(id, Light::directional());
            }
            Create::PointLight => {
                t.translation.y = 2.0;
                self.scene.insert(id, Light::point());
            }
            Create::Camera => {
                t.translation += Vec3::new(0.0, 2.0, 6.0);
                self.scene.insert(id, Camera { primary: self.scene.primary_camera().is_none(), ..Default::default() });
            }
            Create::PhysicsShape(s) => {
                t.translation.y = t.translation.y.max(3.0);
                self.scene.insert(id, MeshRenderer::primitive(s));
                self.scene.insert(id, RigidBody::dynamic());
                self.scene.insert(id, Collider::new(ColliderShape::fit(s)));
            }
        }
        self.scene.set_transform(id, t);
        self.scene.update_transforms();
        self.selection = Some(id);
        self.commit(&format!("Crear {name}"));
        id
    }

    pub fn add_model(&mut self, rel: &str) -> EntityId {
        let name = std::path::Path::new(rel).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Modelo".into());
        let id = self.scene.spawn(&name);
        let pos = self.cam.pivot;
        self.scene.set_transform(id, Transform::from_translation(Vec3::new(pos.x, 0.0, pos.z)));
        self.scene.insert(id, MeshRenderer { mesh: MeshSource::Model(rel.to_string()), material: Material::default(), cast_shadows: true });
        crate::biblioteca_ui::decorate_new_model(self, id, rel);
        self.scene.update_transforms();
        self.selection = Some(id);
        self.commit(&format!("Añadir modelo {name}"));
        id
    }

    pub fn delete_selected(&mut self) {
        if let Some(id) = self.selection.take() {
            let name = self.scene.name(id);
            self.scene.despawn_recursive(id);
            self.commit(&format!("Eliminar {name}"));
        }
    }

    pub fn duplicate_selected(&mut self) {
        if let Some(id) = self.selection
            && let Some(d) = self.scene.duplicate(id)
        {
            self.selection = Some(d);
            self.commit("Duplicar");
        }
    }

    /// Encuadra la selección (tecla F).
    pub fn focus_selected(&mut self) {
        let Some(id) = self.selection else { return };
        let m = self.scene.global(id);
        let mut radius = 1.0;
        if let Some(mr) = self.scene.get::<MeshRenderer>(id)
            && let Some(b) = self.renderer.mesh_aabb(&mr.mesh)
        {
            let wb = b.transformed(&m);
            radius = wb.size().length() * 0.5;
            self.cam.focus(wb.center(), radius);
            return;
        }
        self.cam.focus(m.w_axis.truncate(), radius);
    }

    // ───────────────────────────── Play ─────────────────────────────

    pub fn play(&mut self) {
        if self.is_playing() {
            return;
        }
        self.commit("Antes de Play");
        let snapshot = self.scene.to_file();
        let mut runtime = Runtime::start_en(&mut self.scene, self.lib.clone(), Some(&self.project.root));
        runtime.project_root = Some(self.project.root.clone());
        runtime.ui.recursos = self.ui.recursos.clone();
        self.modo_ui = false;
        self.play = Some(PlaySession { runtime, snapshot, selection: self.selection });
        self.game_focus = true;
        self.cursor.iniciar(self.project.file.capturar_raton, true);
        self.input.release_all();
        self.gizmo.cancel();
    }

    pub fn stop(&mut self) {
        let Some(p) = self.play.take() else { return };
        drop(p.runtime);
        self.scene = Scene::from_file(&p.snapshot);
        self.selection = p.selection.filter(|s| self.scene.contains(*s));
        self.game_focus = false;
        self.cursor.parar();
        self.input.release_all();
        self.euler_cache = None;
        log::info("motor", "■ Stop: escena restaurada");
        if self.pending_reload_after_play {
            self.pending_reload_after_play = false;
            self.reload_scripts(true);
        }
    }

    pub fn toggle_pause(&mut self) {
        if let Some(p) = &mut self.play {
            p.runtime.paused = !p.runtime.paused;
        }
    }

    /// Avanza un fotograma mientras está en pausa.
    pub fn step_frame(&mut self) {
        if let Some(p) = &mut self.play {
            p.runtime.paused = false;
            p.runtime.tick(&mut self.scene, &self.input, 1.0 / 60.0);
            p.runtime.paused = true;
        }
    }

    pub fn tick(&mut self, dt: f32) {
        match &mut self.play {
            Some(p) => {
                p.runtime.tick(&mut self.scene, &self.input, dt);
                self.cursor.modo_script(p.runtime.cursor_mode, self.game_focus);
            }
            // Fuera de Play solo avanza el Animator seleccionado (vista previa).
            None => {
                let sel = self.selection;
                self.animation.update(&mut self.scene, Some(&self.project.root), dt, |id| Some(id) == sel);
            }
        }
        self.input.end_frame();
        self.poll_build();
        if self.comprobar_fuentes || self.last_lib_check.elapsed() > Duration::from_secs(1) {
            self.comprobar_fuentes = false;
            self.last_lib_check = Instant::now();
            self.auto_build_check();
            self.check_lib_changed();
        }
    }

    /// Compilación automática: si las fuentes de los scripts cambiaron desde la última compilación
    /// (al guardar, o por el agente o un editor externo), compila y recarga en caliente. Al abrir el
    /// proyecto también compila si las fuentes son más nuevas que la biblioteca (o no existe).
    fn auto_build_check(&mut self) {
        if !self.auto_build || self.build.is_some() {
            return;
        }
        let dir = self.project.scripts_dir();
        if !dir.join("Cargo.toml").exists() {
            return;
        }
        let (huella, mas_nueva) = huella_fuentes(&dir);
        match self.src_huella {
            None => {
                self.src_huella = Some(huella);
                let lib = self.lib_file_mtime();
                if lib.is_none() || mas_nueva.zip(lib).is_some_and(|(f, l)| f > l) {
                    log::info("scripts", "las fuentes son más nuevas que la biblioteca: compilando");
                    self.start_build(false);
                }
            }
            Some(h) if h != huella => self.start_build(false),
            _ => {}
        }
    }

    // ───────────────────────────── scripts ─────────────────────────────

    pub fn script_names(&self) -> Vec<String> {
        self.lib.as_ref().map(|l| l.names()).unwrap_or_default()
    }

    fn lib_file_mtime(&self) -> Option<SystemTime> {
        std::fs::metadata(self.project.script_lib_path()).and_then(|m| m.modified()).ok()
    }

    fn check_lib_changed(&mut self) {
        if self.build.is_some() {
            return;
        }
        let m = self.lib_file_mtime();
        if m.is_some() && m != self.lib_mtime {
            if self.is_playing() {
                self.pending_reload_after_play = true;
            } else {
                self.reload_scripts(true);
            }
        }
    }

    pub fn reload_scripts(&mut self, verbose: bool) {
        let path = self.project.script_lib_path();
        self.lib_mtime = self.lib_file_mtime();
        if !path.exists() {
            self.lib = None;
            self.lib_error = Some("Scripts sin compilar (F7)".into());
            if verbose {
                log::warn("scripts", format!("no existe {} — compila con F7", path.display()));
            }
            return;
        }
        match ScriptLib::load(&path) {
            Ok(l) => {
                let names = l.names();
                log::info("scripts", format!("biblioteca cargada en caliente: {} script(s): {}", names.len(), names.join(", ")));
                self.lib = Some(Arc::new(l));
                self.lib_error = None;
            }
            Err(e) => {
                log::error("scripts", e.clone());
                self.lib_error = Some(e);
            }
        }
    }

    /// F7: guarda el script abierto y compila.
    pub fn build_scripts(&mut self) {
        self.start_build(true);
    }

    fn start_build(&mut self, guardar: bool) {
        if self.build.is_some() {
            return;
        }
        if guardar
            && let Some(s) = &self.script
            && s.text != s.saved
        {
            self.save_script();
        }
        // Lo que cambie a partir de aquí provoca otra compilación cuando acabe esta.
        self.src_huella = Some(huella_fuentes(&self.project.scripts_dir()).0);
        let child = match self.project.spawn_script_build() {
            Ok(c) => c,
            Err(e) => {
                log::error("scripts", e);
                return;
            }
        };
        log::info("scripts", "compilando scripts (cargo build --release)…");
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let mut child = child;
            let tx2 = tx.clone();
            let out = child.stdout.take();
            let t_out = std::thread::spawn(move || {
                if let Some(o) = out {
                    for l in BufReader::new(o).lines().map_while(Result::ok) {
                        let _ = tx2.send(BuildMsg::Line(l));
                    }
                }
            });
            if let Some(e) = child.stderr.take() {
                for l in BufReader::new(e).lines().map_while(Result::ok) {
                    let _ = tx.send(BuildMsg::Line(l));
                }
            }
            let _ = t_out.join();
            let ok = child.wait().map(|s| s.success()).unwrap_or(false);
            let _ = tx.send(BuildMsg::Done(ok));
        });
        self.build = Some(BuildJob { rx, started: Instant::now() });
    }

    fn poll_build(&mut self) {
        let Some(job) = &self.build else { return };
        let mut done = None;
        while let Ok(m) = job.rx.try_recv() {
            match m {
                BuildMsg::Line(l) => {
                    let lt = l.trim_start();
                    if lt.starts_with("error") {
                        log::error("cargo", l);
                    } else if lt.starts_with("warning") {
                        log::warn("cargo", l);
                    } else if !l.trim().is_empty() {
                        log::info("cargo", l);
                    }
                }
                BuildMsg::Done(ok) => done = Some(ok),
            }
        }
        if let Some(ok) = done {
            let secs = job.started.elapsed().as_secs_f32();
            self.build = None;
            self.last_build_ok = Some(ok);
            if ok {
                log::info("scripts", format!("compilación correcta en {secs:.1} s"));
                if self.is_playing() {
                    self.pending_reload_after_play = true;
                    log::info("scripts", "los scripts nuevos se cargarán al detener Play");
                } else {
                    self.reload_scripts(true);
                }
            } else {
                log::error("scripts", format!("la compilación falló ({secs:.1} s) — revisa los errores de arriba"));
                self.bottom_tab = BottomTab::Console;
            }
        }
    }

    pub fn open_script(&mut self, path: PathBuf) {
        match std::fs::read_to_string(&path) {
            Ok(t) => {
                self.script = Some(OpenScript { path, saved: t.clone(), text: t });
                self.bottom_tab = BottomTab::Script;
            }
            Err(e) => log::error("editor", format!("{}: {e}", path.display())),
        }
    }

    pub fn save_script(&mut self) {
        if let Some(s) = &mut self.script {
            match std::fs::write(&s.path, &s.text) {
                Ok(()) => {
                    s.saved = s.text.clone();
                    self.comprobar_fuentes = true;
                    self.status = format!("Guardado {}", s.path.file_name().unwrap_or_default().to_string_lossy());
                }
                Err(e) => log::error("editor", format!("{}: {e}", s.path.display())),
            }
        }
    }

    pub fn create_script(&mut self, name: &str) {
        let lib_rs = self.project.scripts_dir().join("src/lib.rs");
        let src = std::fs::read_to_string(&lib_rs).unwrap_or_default();
        match crate::code::add_script_to_lib(&src, name.trim()) {
            Ok(new) => {
                if let Err(e) = std::fs::write(&lib_rs, &new) {
                    log::error("editor", e.to_string());
                    return;
                }
                log::info("scripts", format!("script {} creado en scripts/src/lib.rs — compílalo con F7", name.trim()));
                self.open_script(lib_rs);
                if let Some(s) = &self.script {
                    let _ = s;
                }
            }
            Err(e) => log::error("scripts", e),
        }
    }

    /// Exporta un juego autónomo: reproductor + datos + biblioteca de scripts.
    pub fn export_game(&mut self) -> Result<PathBuf, String> {
        self.save_scene();
        let player = std::env::current_exe().map_err(|e| e.to_string())?.with_file_name(if cfg!(windows) { "maker-player.exe" } else { "maker-player" });
        if !player.exists() {
            return Err(format!("no se encuentra {} (compila el workspace con cargo build --release)", player.display()));
        }
        let name = maker_engine::project::sanitize_crate_name(self.project.name()).trim_end_matches("_scripts").to_string();
        let out = self.project.root.join("export").join(&name);
        let _ = std::fs::remove_dir_all(&out);
        std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
        let exe = out.join(if cfg!(windows) { format!("{name}.exe") } else { name.clone() });
        std::fs::copy(&player, &exe).map_err(|e| format!("copiando el reproductor: {e}"))?;
        std::fs::copy(self.project.root.join(maker_engine::project::PROJECT_FILE), out.join(maker_engine::project::PROJECT_FILE)).map_err(|e| e.to_string())?;
        for dir in ["scenes", "assets", ui::DIR_UI] {
            copy_dir(&self.project.root.join(dir), &out.join(dir))?;
        }
        // Solo lo que usan las escenas de la biblioteca integrada, junto al ejecutable.
        let scenes: Vec<_> = self.project.scenes().iter().filter_map(|s| Scene::load(&self.project.root.join(s)).ok()).map(|s| s.to_file()).collect();
        // Imágenes y fuentes de la interfaz: las de la biblioteca van con las demás;
        // las del proyecto fuera de scenes/, assets/ y ui/ se copian una a una.
        let usados = ui::recursos_usados(&scenes, &self.ui.recursos);
        let (extra, propios): (std::collections::BTreeSet<String>, Vec<String>) = {
            let (b, p): (Vec<String>, Vec<String>) = usados.into_iter().partition(|r| maker_engine::biblioteca::is_library_path(r));
            (b.into_iter().collect(), p)
        };
        for r in propios {
            let src = self.project.root.join(&r);
            let dst = out.join(&r);
            if src.is_file() && !dst.exists() {
                if let Some(d) = dst.parent() {
                    std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
                }
                std::fs::copy(&src, &dst).map_err(|e| format!("{r}: {e}"))?;
            }
        }
        let copied = maker_engine::biblioteca::export_used(&scenes, &extra, &out)?;
        if !copied.is_empty() {
            log::info("editor", format!("biblioteca: {} archivos copiados al juego", copied.len()));
        }
        // Efectos de posprocesado propios y LUT que usan las escenas.
        for rel in maker_engine::postproceso::post_files_used(scenes.iter().map(|s| &s.environment)) {
            let (src, dst) = (self.project.root.join(&rel), out.join(&rel));
            if dst.exists() {
                continue;
            }
            if !src.is_file() {
                log::warn("editor", format!("posprocesado: falta {rel}"));
                continue;
            }
            std::fs::create_dir_all(dst.parent().unwrap()).map_err(|e| e.to_string())?;
            std::fs::copy(&src, &dst).map_err(|e| format!("{rel}: {e}"))?;
        }
        let lib = self.project.script_lib_path();
        if lib.exists() {
            let rel = lib.strip_prefix(&self.project.root).unwrap_or(&lib).to_path_buf();
            let dst = out.join(rel);
            std::fs::create_dir_all(dst.parent().unwrap()).map_err(|e| e.to_string())?;
            std::fs::copy(&lib, &dst).map_err(|e| e.to_string())?;
            // El nombre del crate (y por tanto del archivo) se lee de aquí.
            std::fs::copy(self.project.scripts_manifest(), out.join("scripts/Cargo.toml")).map_err(|e| e.to_string())?;
        }
        log::info("editor", format!("juego exportado en {}", out.display()));
        Ok(out)
    }

    /// Lanza el juego en una ventana aparte (como "Standalone Game" de Unreal).
    pub fn run_standalone(&mut self) {
        self.save_scene();
        let player = std::env::current_exe().map(|e| e.with_file_name(if cfg!(windows) { "maker-player.exe" } else { "maker-player" }));
        match player {
            Ok(p) if p.exists() => match std::process::Command::new(&p).arg(&self.project.root).arg("--scene").arg(&self.scene_path).spawn() {
                Ok(_) => log::info("editor", "juego lanzado en ventana aparte"),
                Err(e) => log::error("editor", format!("no se pudo lanzar {}: {e}", p.display())),
            },
            _ => log::error("editor", "no se encuentra maker-player junto al editor"),
        }
    }
}

fn copy_dir(src: &std::path::Path, dst: &std::path::Path) -> Result<(), String> {
    if !src.exists() {
        return Ok(());
    }
    std::fs::create_dir_all(dst).map_err(|e| e.to_string())?;
    for e in std::fs::read_dir(src).map_err(|e| e.to_string())?.flatten() {
        let p = e.path();
        let to = dst.join(e.file_name());
        if p.is_dir() {
            copy_dir(&p, &to)?;
        } else {
            std::fs::copy(&p, &to).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// Elementos de interfaz del menú Crear → Interfaz.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CrearUi {
    Lienzo,
    Widget(ui::TipoWidget),
    Mascara,
    Contenedor(ui::TipoContenedor),
}

impl CrearUi {
    pub const MENU: [CrearUi; 14] = [
        CrearUi::Lienzo,
        CrearUi::Widget(ui::TipoWidget::Panel),
        CrearUi::Widget(ui::TipoWidget::Imagen),
        CrearUi::Widget(ui::TipoWidget::Texto),
        CrearUi::Widget(ui::TipoWidget::Boton),
        CrearUi::Widget(ui::TipoWidget::Barra),
        CrearUi::Widget(ui::TipoWidget::Deslizador),
        CrearUi::Widget(ui::TipoWidget::Casilla),
        CrearUi::Widget(ui::TipoWidget::CampoTexto),
        CrearUi::Mascara,
        CrearUi::Contenedor(ui::TipoContenedor::Horizontal),
        CrearUi::Contenedor(ui::TipoContenedor::Vertical),
        CrearUi::Contenedor(ui::TipoContenedor::Rejilla),
        CrearUi::Widget(ui::TipoWidget::Personalizado),
    ];
    pub fn label(self) -> &'static str {
        match self {
            CrearUi::Lienzo => "Lienzo",
            CrearUi::Widget(t) => t.nombre(),
            CrearUi::Mascara => "Máscara",
            CrearUi::Contenedor(ui::TipoContenedor::Horizontal) => "Caja horizontal",
            CrearUi::Contenedor(ui::TipoContenedor::Vertical) => "Caja vertical",
            CrearUi::Contenedor(ui::TipoContenedor::Rejilla) => "Rejilla",
        }
    }
    pub fn componentes(self) -> (ui::RectUi, Option<ui::WidgetUi>, Option<ui::Contenedor>) {
        match self {
            CrearUi::Lienzo => (ui::RectUi::estirado(), None, None),
            CrearUi::Widget(t) => {
                let [w, h] = t.tam_inicial();
                (ui::RectUi::centrado(w, h), Some(ui::WidgetUi::nuevo(t)), None)
            }
            CrearUi::Mascara => (ui::RectUi { recortar: true, ..ui::RectUi::centrado(400.0, 260.0) }, Some(ui::WidgetUi::nuevo(ui::TipoWidget::Panel)), None),
            CrearUi::Contenedor(tipo) => {
                let c = ui::Contenedor { tipo, ajustar: tipo != ui::TipoContenedor::Rejilla, ..Default::default() };
                let [w, h] = if tipo == ui::TipoContenedor::Horizontal { [400.0, 80.0] } else { [280.0, 300.0] };
                (ui::RectUi::centrado(w, h), None, Some(c))
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Create {
    Ui(CrearUi),
    Empty,
    Shape(PrimitiveShape),
    PhysicsShape(PrimitiveShape),
    DirectionalLight,
    PointLight,
    Camera,
}

impl Create {
    pub fn label(self) -> &'static str {
        match self {
            Create::Ui(u) => u.label(),
            Create::Empty => "Entidad vacía",
            Create::Shape(s) | Create::PhysicsShape(s) => s.label(),
            Create::DirectionalLight => "Luz direccional",
            Create::PointLight => "Luz puntual",
            Create::Camera => "Cámara",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undo_redo_roundtrip() {
        let mut s = Scene::starter();
        let mut u = Undo::default();
        u.reset_con(&s, BTreeMap::new());
        let a = s.spawn("A");
        assert!(u.commit_con(&s, BTreeMap::new(), "crear A"));
        assert!(!u.commit_con(&s, BTreeMap::new(), "sin cambios"));
        s.set_name(a, "B");
        assert!(u.commit_con(&s, BTreeMap::new(), "renombrar"));
        let (l, f) = u.undo().unwrap();
        assert_eq!(l, "renombrar");
        let s2 = Scene::from_file(&f.escena);
        assert_eq!(s2.name(a), "A");
        let (_, f) = u.undo().unwrap();
        assert!(!Scene::from_file(&f.escena).contains(a));
        let (_, f) = u.redo().unwrap();
        assert!(Scene::from_file(&f.escena).contains(a));
        // Los estilos de interfaz entran en el historial aunque la escena no cambie.
        let mut docs = BTreeMap::new();
        docs.insert("ui/estilos/a.mestilo".to_string(), "{}".to_string());
        assert!(u.commit_con(&Scene::from_file(&f.escena), docs.clone(), "estilo"));
        let (l, f) = u.undo().unwrap();
        assert_eq!(l, "estilo");
        assert!(f.ui.is_empty());
        assert!(u.can_redo());
    }
}

/// Huella (rutas, tamaños y fechas) de `scripts/Cargo.toml` y de todo `scripts/src`, y la fecha
/// del archivo más nuevo. Sin leer contenidos: cuesta microsegundos con decenas de archivos.
pub fn huella_fuentes(scripts: &std::path::Path) -> (u64, Option<SystemTime>) {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    let mut mas_nueva: Option<SystemTime> = None;
    let mut pendientes = vec![scripts.join("src")];
    let mut archivos = vec![scripts.join("Cargo.toml")];
    while let Some(d) = pendientes.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                pendientes.push(p);
            } else {
                archivos.push(p);
            }
        }
    }
    archivos.sort();
    for p in archivos {
        if let Ok(m) = std::fs::metadata(&p) {
            let t = m.modified().ok();
            p.hash(&mut h);
            m.len().hash(&mut h);
            t.hash(&mut h);
            mas_nueva = mas_nueva.max(t);
        }
    }
    (h.finish(), mas_nueva)
}

#[cfg(test)]
mod tests_auto_build {
    use super::huella_fuentes;

    #[test]
    fn la_huella_cambia_al_editar_un_script() {
        let d = std::env::temp_dir().join(format!("maker-huella-{}", std::process::id()));
        std::fs::create_dir_all(d.join("src/sub")).unwrap();
        std::fs::write(d.join("Cargo.toml"), "[package]").unwrap();
        std::fs::write(d.join("src/lib.rs"), "fn a() {}").unwrap();
        let (a, _) = huella_fuentes(&d);
        assert_eq!(a, huella_fuentes(&d).0, "estable sin cambios");
        std::fs::write(d.join("src/sub/otro.rs"), "x").unwrap();
        let (b, nueva) = huella_fuentes(&d);
        assert_ne!(a, b, "un archivo nuevo en una subcarpeta cuenta");
        assert!(nueva.is_some());
        std::fs::write(d.join("src/lib.rs"), "fn a() { 1; }").unwrap();
        assert_ne!(b, huella_fuentes(&d).0, "editar cuenta");
        let _ = std::fs::remove_dir_all(d);
    }
}
