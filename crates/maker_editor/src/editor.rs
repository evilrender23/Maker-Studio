//! Estado del editor y operaciones (sin UI): escena, selección, deshacer,
//! modo Play, compilación y recarga de scripts.

use crate::camera::EditorCamera;
use crate::gizmo::Gizmo;
use glam::Vec3;
use maker_engine::components::*;
use maker_engine::scene::SceneFile;
use maker_engine::{Input, Project, Runtime, Scene, ScriptLib, log};
use maker_render::SceneRenderer;
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
}

/// Historial de deshacer basado en instantáneas de la escena (simple y
/// exacto: cada paso es la escena completa serializada).
#[derive(Default)]
pub struct Undo {
    undo: Vec<(String, SceneFile)>,
    redo: Vec<(String, SceneFile)>,
    pub committed: Option<SceneFile>,
}

impl Undo {
    pub const LIMIT: usize = 200;
    pub fn reset(&mut self, scene: &Scene) {
        self.undo.clear();
        self.redo.clear();
        self.committed = Some(scene.to_file());
    }
    /// Si la escena cambió respecto a la última confirmada, apila el estado anterior.
    pub fn commit(&mut self, scene: &Scene, label: &str) -> bool {
        let now = scene.to_file();
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
    pub fn undo(&mut self) -> Option<(String, SceneFile)> {
        let (label, prev) = self.undo.pop()?;
        if let Some(cur) = self.committed.replace(prev.clone()) {
            self.redo.push((label.clone(), cur));
        }
        Some((label, prev))
    }
    pub fn redo(&mut self) -> Option<(String, SceneFile)> {
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
    pub import_path: String,
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
            import_path: String::new(),
            pending_reload_after_play: false,
            dirty: false,
            pending_edit: None,
            pending_save_script: false,
            biblioteca: Default::default(),
            contenido: Default::default(),
            viewport_rect: egui::Rect::NOTHING,
            animation: Default::default(),
        };
        ed.undo.reset(&ed.scene);
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
            Ok(s) => {
                self.scene = s;
                self.scene_path = path;
                self.selection = None;
                self.undo.reset(&self.scene);
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
        if self.undo.commit(&self.scene, label) {
            self.dirty = true;
        }
    }

    pub fn undo(&mut self) {
        if self.is_playing() {
            return;
        }
        if let Some((label, f)) = self.undo.undo() {
            self.scene = Scene::from_file(&f);
            self.after_history();
            self.status = format!("Deshecho: {label}");
        }
    }
    pub fn redo(&mut self) {
        if self.is_playing() {
            return;
        }
        if let Some((label, f)) = self.undo.redo() {
            self.scene = Scene::from_file(&f);
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
        let pos = (self.cam.pivot * 2.0).round() * 0.5;
        let name = what.label();
        let id = self.scene.spawn(name);
        let mut t = Transform::from_translation(Vec3::new(pos.x, pos.y.max(0.0), pos.z));
        match what {
            Create::Empty => {}
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
        self.play = Some(PlaySession { runtime, snapshot, selection: self.selection });
        self.game_focus = true;
        self.input.release_all();
        self.gizmo.cancel();
    }

    pub fn stop(&mut self) {
        let Some(p) = self.play.take() else { return };
        drop(p.runtime);
        self.scene = Scene::from_file(&p.snapshot);
        self.selection = p.selection.filter(|s| self.scene.contains(*s));
        self.game_focus = false;
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
            Some(p) => p.runtime.tick(&mut self.scene, &self.input, dt),
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
        for dir in ["scenes", "assets"] {
            copy_dir(&self.project.root.join(dir), &out.join(dir))?;
        }
        // Solo lo que usan las escenas de la biblioteca integrada, junto al ejecutable.
        let scenes: Vec<_> = self.project.scenes().iter().filter_map(|s| Scene::load(&self.project.root.join(s)).ok()).map(|s| s.to_file()).collect();
        let copied = maker_engine::biblioteca::export_used(&scenes, &out)?;
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

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Create {
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
        u.reset(&s);
        let a = s.spawn("A");
        assert!(u.commit(&s, "crear A"));
        assert!(!u.commit(&s, "sin cambios"));
        s.set_name(a, "B");
        assert!(u.commit(&s, "renombrar"));
        let (l, f) = u.undo().unwrap();
        assert_eq!(l, "renombrar");
        let s2 = Scene::from_file(&f);
        assert_eq!(s2.name(a), "A");
        let (_, f) = u.undo().unwrap();
        assert!(!Scene::from_file(&f).contains(a));
        let (_, f) = u.redo().unwrap();
        assert!(Scene::from_file(&f).contains(a));
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
