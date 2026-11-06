//! Contenido: explorador de archivos del proyecto al estilo del Project de Unity / Content
//! Browser de Unreal. Árbol de carpetas plegable, ruta con atrás/adelante, «Añadir» con
//! nombre en línea, cuadrícula (tamaño ajustable) o lista, selección múltiple, menús
//! contextuales, renombrar (F2), duplicar (Ctrl+D), eliminar (Supr, a la papelera del
//! proyecto), mover arrastrando sobre carpetas y barra de estado. Al renombrar o mover se
//! actualizan las referencias en escenas, plantillas y `project.maker`.

use crate::editor::Editor;
use crate::icons::{self, Icon};
use crate::theme;
use crate::viewport::AssetDrag;
use egui::{Color32, Key, RichText, Stroke, Ui};
use maker_render::SceneRenderer;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

const FILA: f32 = 22.0;
/// Carpeta (oculta) del proyecto donde va lo eliminado.
pub const PAPELERA: &str = ".maker/papelera";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tipo {
    Carpeta,
    Escena,
    Modelo,
    Imagen,
    Script,
    Estilo,
    Tema,
    Plantilla,
    Fuente,
    EstiloVisual,
    Otro,
}

impl Tipo {
    const FILTROS: [Tipo; 10] = [Tipo::Escena, Tipo::Modelo, Tipo::Imagen, Tipo::Script, Tipo::Estilo, Tipo::Tema, Tipo::Plantilla, Tipo::Fuente, Tipo::EstiloVisual, Tipo::Otro];

    pub fn de(p: &Path) -> Tipo {
        if p.is_dir() {
            return Tipo::Carpeta;
        }
        match p.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
            Some("mscene") => Tipo::Escena,
            Some("gltf" | "glb") => Tipo::Modelo,
            Some("png" | "jpg" | "jpeg" | "webp" | "bmp" | "tga" | "gif") => Tipo::Imagen,
            Some("rs") => Tipo::Script,
            Some("mestilo") => Tipo::Estilo,
            Some("mtema") => Tipo::Tema,
            Some("mui") => Tipo::Plantilla,
            Some("ttf" | "otf") => Tipo::Fuente,
            Some(maker_engine::estilo_visual::EXT) => Tipo::EstiloVisual,
            _ => Tipo::Otro,
        }
    }

    fn nombre(self) -> &'static str {
        match self {
            Tipo::Carpeta => "Carpetas",
            Tipo::Escena => "Escenas",
            Tipo::Modelo => "Modelos",
            Tipo::Imagen => "Imágenes",
            Tipo::Script => "Scripts",
            Tipo::Estilo => "Estilos",
            Tipo::Tema => "Temas",
            Tipo::Plantilla => "Plantillas de interfaz",
            Tipo::Fuente => "Fuentes",
            Tipo::EstiloVisual => "Estilos visuales",
            Tipo::Otro => "Otros",
        }
    }

    fn singular(self) -> &'static str {
        match self {
            Tipo::Carpeta => "Carpeta",
            Tipo::Escena => "Escena",
            Tipo::Modelo => "Modelo",
            Tipo::Imagen => "Imagen",
            Tipo::Script => "Script",
            Tipo::Estilo => "Estilo",
            Tipo::Tema => "Tema",
            Tipo::Plantilla => "Plantilla",
            Tipo::Fuente => "Fuente",
            Tipo::EstiloVisual => "Estilo visual",
            Tipo::Otro => "Archivo",
        }
    }

    fn icono(self) -> (Icon, Color32) {
        match self {
            Tipo::Carpeta => (Icon::Carpeta, theme::YELLOW),
            Tipo::Escena => (Icon::Escena, theme::GREEN),
            Tipo::Modelo => (Icon::Modelo, theme::ACCENT),
            Tipo::Imagen => (Icon::Imagen, theme::CYAN),
            Tipo::Script => (Icon::Script, theme::ORANGE),
            Tipo::Estilo => (Icon::Estilo, theme::PURPLE),
            Tipo::Tema => (Icon::Tema, theme::PURPLE),
            Tipo::Plantilla => (Icon::Plantilla, theme::GREEN),
            Tipo::Fuente => (Icon::Fuente, theme::CYAN),
            Tipo::EstiloVisual => (Icon::EstiloVisual, theme::WARM),
            Tipo::Otro => (Icon::Archivo, theme::TEXT3),
        }
    }
}

/// Lo que se puede crear desde «Añadir» (con el nombre escrito en línea).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Creable {
    Carpeta,
    Escena,
    Script,
    Efecto,
}

impl Creable {
    fn tipo(self) -> Tipo {
        match self {
            Creable::Carpeta => Tipo::Carpeta,
            Creable::Escena => Tipo::Escena,
            Creable::Script => Tipo::Script,
            Creable::Efecto => Tipo::Otro,
        }
    }
    fn nombre_inicial(self) -> &'static str {
        match self {
            Creable::Carpeta => "Nueva carpeta",
            Creable::Escena => "NuevaEscena",
            Creable::Script => "NuevoScript",
            Creable::Efecto => "nuevo_efecto",
        }
    }
}

pub struct ContenidoUi {
    /// Carpeta abierta, relativa a la raíz del proyecto ("" = raíz).
    carpeta: String,
    tipo: Option<Tipo>,
    buscar: String,
    /// Selección (rutas absolutas) y ancla para Mayús+clic.
    seleccion: Vec<PathBuf>,
    ancla: Option<PathBuf>,
    /// Miniaturas por ruta: `None` = falló (se muestra el icono).
    minis: HashMap<PathBuf, Option<egui::TextureHandle>>,
    renderer: Option<SceneRenderer>,
    lista: bool,
    /// Lado de las miniaturas en la cuadrícula (px).
    tam: f32,
    atras: Vec<String>,
    adelante: Vec<String>,
    /// Carpetas desplegadas en el árbol.
    abiertas: HashSet<String>,
    /// Renombrando: (ruta, texto, ¿pedir foco?).
    renombrar: Option<(PathBuf, String, bool)>,
    /// Creando: (qué, nombre, ¿pedir foco?).
    creando: Option<(Creable, String, bool)>,
    /// Pendiente de confirmar eliminación.
    borrar: Option<Vec<PathBuf>>,
    /// Elementos que se arrastran (para soltarlos en una carpeta).
    arrastrando: Vec<PathBuf>,
}

impl Default for ContenidoUi {
    fn default() -> Self {
        Self {
            carpeta: String::new(),
            tipo: None,
            buscar: String::new(),
            seleccion: Vec::new(),
            ancla: None,
            minis: HashMap::new(),
            renderer: None,
            lista: false,
            tam: 72.0,
            atras: Vec::new(),
            adelante: Vec::new(),
            abiertas: HashSet::new(),
            renombrar: None,
            creando: None,
            borrar: None,
            arrastrando: Vec::new(),
        }
    }
}

impl ContenidoUi {
    /// Carpeta abierta, relativa a la raíz.
    pub fn carpeta(&self) -> &str {
        &self.carpeta
    }

    /// Abre la carpeta de `p` y lo selecciona (tras importarlo).
    pub fn mostrar(&mut self, raiz: &Path, p: &Path) {
        if let Some(d) = p.parent() {
            self.ir(relativa(raiz, d));
        }
        self.buscar.clear();
        self.tipo = None;
        self.seleccion = vec![p.to_path_buf()];
        self.ancla = Some(p.to_path_buf());
    }

    /// Navega a una carpeta guardando el historial.
    fn ir(&mut self, d: String) {
        if d != self.carpeta {
            self.atras.push(std::mem::replace(&mut self.carpeta, d));
            self.adelante.clear();
            self.seleccion.clear();
            self.renombrar = None;
            self.creando = None;
        }
        // Desplegar el árbol hasta ella.
        let mut acc = String::new();
        for parte in self.carpeta.split('/').filter(|s| !s.is_empty()) {
            if !acc.is_empty() {
                acc.push('/');
            }
            acc.push_str(parte);
            self.abiertas.insert(acc.clone());
        }
    }

    fn seleccionado(&self, p: &Path) -> bool {
        self.seleccion.iter().any(|s| s == p)
    }

    /// Clic sobre un elemento con Ctrl (alternar) / Mayús (rango) como en cualquier explorador.
    fn clic(&mut self, p: &Path, items: &[PathBuf], mods: egui::Modifiers) {
        if mods.shift
            && let Some(a) = &self.ancla
            && let (Some(i), Some(j)) = (items.iter().position(|x| x == a), items.iter().position(|x| x == p))
        {
            let (i, j) = (i.min(j), i.max(j));
            self.seleccion = items[i..=j].to_vec();
            return;
        }
        if mods.command {
            if let Some(i) = self.seleccion.iter().position(|s| s == p) {
                self.seleccion.remove(i);
            } else {
                self.seleccion.push(p.to_path_buf());
            }
        } else {
            self.seleccion = vec![p.to_path_buf()];
        }
        self.ancla = Some(p.to_path_buf());
    }
}

/// Subcarpetas visibles de `d`, ordenadas.
fn subcarpetas(d: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(d) else { return Vec::new() };
    let mut v: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.is_dir() && visible(p)).collect();
    v.sort_by_key(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()));
    v
}

fn visible(p: &Path) -> bool {
    let n = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    !(n.starts_with('.') || n == "target" || n == "export")
}

pub(crate) fn relativa(raiz: &Path, p: &Path) -> String {
    p.strip_prefix(raiz).map(|r| r.to_string_lossy().replace('\\', "/")).unwrap_or_default()
}

/// Elementos a mostrar: con búsqueda, todo el proyecto; si no, la carpeta abierta (carpetas primero).
pub fn elementos(raiz: &Path, carpeta: &str, buscar: &str, tipo: Option<Tipo>) -> Vec<PathBuf> {
    let q = buscar.trim().to_lowercase();
    let mut v: Vec<PathBuf> = Vec::new();
    if q.is_empty() {
        let d = raiz.join(carpeta);
        if let Ok(rd) = std::fs::read_dir(&d) {
            v = rd.flatten().map(|e| e.path()).filter(|p| visible(p)).collect();
        }
    } else {
        maker_engine::project::walk(raiz, &mut |p| {
            if p.file_name().is_some_and(|n| n.to_string_lossy().to_lowercase().contains(&q)) {
                v.push(p.to_path_buf());
            }
        });
    }
    v.retain(|p| match tipo {
        None => true,
        Some(t) => Tipo::de(p) == t,
    });
    v.sort_by_key(|p| (!p.is_dir(), p.file_name().map(|n| n.to_string_lossy().to_lowercase())));
    v
}

// ---------------------------------------------------------------------------------------------
// Operaciones de archivos

/// Archivos de texto del proyecto que pueden citar rutas de assets.
fn con_referencias(p: &Path) -> bool {
    let ext = p.extension().and_then(|e| e.to_str()).unwrap_or_default();
    matches!(ext, "mscene" | "mui" | "mestilo" | "mtema" | "maker") || ext == maker_engine::estilo_visual::EXT
}

/// Cambia `"viejo"` y `"viejo/…"` por la ruta nueva dentro de un texto JSON.
pub fn reemplazar_ruta(txt: &str, viejo: &str, nuevo: &str) -> String {
    txt.replace(&format!("\"{viejo}\""), &format!("\"{nuevo}\"")).replace(&format!("\"{viejo}/"), &format!("\"{nuevo}/"))
}

/// Actualiza las referencias a `viejo` en los archivos del proyecto. Devuelve cuántos cambiaron.
pub fn actualizar_referencias(raiz: &Path, viejo: &str, nuevo: &str) -> usize {
    let mut n = 0;
    let mut archivos = Vec::new();
    maker_engine::project::walk(raiz, &mut |p| {
        if con_referencias(p) {
            archivos.push(p.to_path_buf());
        }
    });
    let proyecto = raiz.join(maker_engine::project::PROJECT_FILE);
    if !archivos.contains(&proyecto) {
        archivos.push(proyecto);
    }
    for f in archivos {
        let Ok(t) = std::fs::read_to_string(&f) else { continue };
        let r = reemplazar_ruta(&t, viejo, nuevo);
        if r != t && std::fs::write(&f, r).is_ok() {
            n += 1;
        }
    }
    n
}

/// Copia un archivo o carpeta entera.
fn copiar(de: &Path, a: &Path) -> std::io::Result<()> {
    if de.is_dir() {
        std::fs::create_dir_all(a)?;
        for e in std::fs::read_dir(de)?.flatten() {
            copiar(&e.path(), &a.join(e.file_name()))?;
        }
        Ok(())
    } else {
        std::fs::copy(de, a).map(|_| ())
    }
}

/// Nombre válido para un archivo/carpeta (sin separadores ni vacío).
fn nombre_valido(s: &str) -> Option<String> {
    let s = s.trim();
    (!s.is_empty() && !s.contains(['/', '\\']) && s != "." && s != "..").then(|| s.to_string())
}

// ---------------------------------------------------------------------------------------------
// Miniaturas

/// Carpeta de caché de miniaturas (fuera del proyecto).
fn cache() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("LOCALAPPDATA").map(PathBuf::from))
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))?;
    let d = base.join("maker-studio").join("miniaturas");
    std::fs::create_dir_all(&d).ok()?;
    Some(d)
}

/// Archivo de caché de la miniatura: depende de la ruta, el tamaño y la fecha del modelo.
fn clave_cache(p: &Path) -> Option<PathBuf> {
    use std::hash::{Hash, Hasher};
    let m = std::fs::metadata(p).ok()?;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    p.hash(&mut h);
    m.len().hash(&mut h);
    m.modified().ok().hash(&mut h);
    Some(cache()?.join(format!("{:016x}.png", h.finish())))
}

impl ContenidoUi {
    /// Olvida la miniatura de un modelo reescrito (la siguiente se genera de nuevo).
    pub fn invalidar_miniatura(&mut self, raiz: &Path, rel: &str) {
        self.minis.remove(&raiz.join(rel));
        if let Some(r) = &mut self.renderer {
            r.reload_model(&maker_engine::components::MeshSource::Model(rel.to_string()));
        }
    }

    /// Miniatura de un modelo o imagen: de la caché o, como mucho una por fotograma, generada.
    fn mini(&mut self, ed_renderer: &SceneRenderer, raiz: &Path, ctx: &egui::Context, p: &Path, puede: &mut bool) -> Option<egui::TextureHandle> {
        if let Some(t) = self.minis.get(p) {
            return t.clone();
        }
        let tipo = Tipo::de(p);
        let archivo = if tipo == Tipo::Modelo { clave_cache(p) } else { None };
        let img = match archivo.as_ref().and_then(|f| image::open(f).ok()) {
            Some(i) => Some(i.to_rgba8()),
            None if *puede => {
                *puede = false;
                let img = if tipo == Tipo::Imagen {
                    image::open(p).ok().map(|i| i.thumbnail(160, 160).to_rgba8())
                } else {
                    let r = self.renderer.get_or_insert_with(|| {
                        let (d, q) = ed_renderer.gpu();
                        let mut r = SceneRenderer::new(d, q);
                        r.project_root = Some(raiz.to_path_buf());
                        r
                    });
                    let src = maker_engine::components::MeshSource::Model(relativa(raiz, p));
                    let img = maker_render::miniatura::miniatura(r, &src, None, 128);
                    if let (Some(i), Some(f)) = (&img, &archivo) {
                        let _ = i.save(f);
                    }
                    img
                };
                if img.is_none() {
                    self.minis.insert(p.to_path_buf(), None);
                }
                img
            }
            None => return None,
        }?;
        let ci = egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw());
        let t = ctx.load_texture(format!("contenido-{}", p.display()), ci, egui::TextureOptions::LINEAR);
        self.minis.insert(p.to_path_buf(), Some(t.clone()));
        Some(t)
    }
}

// ---------------------------------------------------------------------------------------------
// Acciones (se aplican al final, con acceso a todo el editor)

enum Accion {
    Abrir(PathBuf),
    Crear(Creable, String),
    Renombrar(PathBuf, String),
    Mover(Vec<PathBuf>, PathBuf),
    Duplicar(Vec<PathBuf>),
    Borrar(Vec<PathBuf>),
    Explorador(PathBuf),
    OrdenEscenas,
}

pub fn show(ed: &mut Editor, ui: &mut Ui) {
    let raiz = ed.project.root.clone();
    ed.importador.rect_contenido = ui.max_rect();
    let ctx = ui.ctx().clone();
    if !ed.contenido.carpeta.is_empty() && !raiz.join(&ed.contenido.carpeta).is_dir() {
        ed.contenido.carpeta.clear();
    }
    let mut acciones: Vec<Accion> = Vec::new();
    barra(ed, ui, &raiz, &mut acciones);
    ui.add_space(2.0);
    let items = {
        let c = &ed.contenido;
        elementos(&raiz, &c.carpeta, &c.buscar, c.tipo)
    };
    let n_sel = ed.contenido.seleccion.len();
    egui::Panel::bottom("contenido-estado").exact_size(20.0).frame(egui::Frame::new()).show(ui, |ui| {
        ui.horizontal_centered(|ui| {
            let t = if n_sel > 0 { format!("{} elementos · {} seleccionados", items.len(), n_sel) } else { format!("{} elementos", items.len()) };
            ui.label(RichText::new(t).small().color(theme::TEXT3));
            if !ed.contenido.lista {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add(egui::Slider::new(&mut ed.contenido.tam, 40.0..=160.0).show_value(false)).on_hover_text("Tamaño de las miniaturas");
                });
            }
        });
    });
    egui::Panel::left("contenido-carpetas").resizable(true).default_size(200.0).min_size(130.0).frame(egui::Frame::new().inner_margin(egui::Margin { right: 6, ..Default::default() })).show(ui, |ui| {
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let nombre = ed.project.name();
            arbol(&mut ed.contenido, ui, &raiz, &raiz, &nombre, 0, &mut acciones);
        });
    });
    elementos_ui(ed, ui, &raiz, &items, &mut acciones);
    teclado(ed, ui, &items, &mut acciones);
    confirmar_borrado(ed, &ctx, &raiz, &mut acciones);
    // Arrastre interno: al soltar fuera de una carpeta se olvida.
    if !ed.contenido.arrastrando.is_empty() && ctx.input(|i| i.pointer.any_released()) {
        ed.contenido.arrastrando.clear();
    }
    for a in acciones {
        aplicar(ed, &raiz, a);
    }
}

/// Fila del árbol de carpetas (recursiva).
fn arbol(c: &mut ContenidoUi, ui: &mut Ui, raiz: &Path, d: &Path, nombre: &str, nivel: usize, acciones: &mut Vec<Accion>) {
    let rel = relativa(raiz, d);
    let hijos = subcarpetas(d);
    let abierta = nivel == 0 || c.abiertas.contains(&rel);
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), FILA), egui::Sense::click());
    let p = ui.painter_at(rect);
    let seleccionada = c.carpeta == rel && c.buscar.trim().is_empty();
    let destino = !c.arrastrando.is_empty() && ui.rect_contains_pointer(rect) && !c.arrastrando.iter().any(|a| d.starts_with(a));
    if destino {
        p.rect(rect, 3.0, theme::ACCENT_SOFT, Stroke::new(1.0, theme::ACCENT), egui::StrokeKind::Inside);
        if ui.input(|i| i.pointer.any_released()) {
            acciones.push(Accion::Mover(std::mem::take(&mut c.arrastrando), d.to_path_buf()));
        }
    } else if seleccionada {
        p.rect_filled(rect, 3.0, theme::ACCENT_SOFT);
    } else if resp.hovered() {
        p.rect_filled(rect, 3.0, theme::ROW_HOVER);
    }
    let x = rect.left() + 4.0 + nivel as f32 * 14.0;
    let flecha = egui::Rect::from_center_size(egui::pos2(x + 6.0, rect.center().y), egui::vec2(14.0, FILA));
    if !hijos.is_empty() && nivel > 0 {
        let (cx, cy) = (flecha.center().x, rect.center().y);
        let pts = if abierta {
            vec![egui::pos2(cx - 3.5, cy - 2.0), egui::pos2(cx + 3.5, cy - 2.0), egui::pos2(cx, cy + 2.5)]
        } else {
            vec![egui::pos2(cx - 2.0, cy - 3.5), egui::pos2(cx - 2.0, cy + 3.5), egui::pos2(cx + 2.5, cy)]
        };
        p.add(egui::Shape::convex_polygon(pts, theme::TEXT3, Stroke::NONE));
    }
    let lado = icons::side(ui.ctx());
    let icono = if nivel == 0 { Icon::Biblioteca } else { Icon::Carpeta };
    icons::image(ui.ctx(), icono, if nivel == 0 { theme::ACCENT } else { theme::YELLOW }).paint_at(ui, egui::Rect::from_min_size(egui::pos2(x + 14.0, rect.center().y - lado / 2.0), egui::vec2(lado, lado)));
    p.text(egui::pos2(x + 18.0 + lado, rect.center().y), egui::Align2::LEFT_CENTER, nombre, egui::FontId::proportional(12.5), theme::TEXT1);
    if resp.clicked() {
        let en_flecha = resp.interact_pointer_pos().is_some_and(|q| flecha.contains(q));
        if en_flecha && nivel > 0 {
            if !c.abiertas.remove(&rel) {
                c.abiertas.insert(rel.clone());
            }
        } else {
            c.buscar.clear();
            c.ir(rel.clone());
        }
    }
    if resp.double_clicked() && nivel > 0 && !c.abiertas.remove(&rel) {
        c.abiertas.insert(rel.clone());
    }
    resp.context_menu(|ui| menu_fondo(ui, c, d, acciones));
    if abierta {
        for h in hijos {
            let n = h.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            arbol(c, ui, raiz, &h, &n, nivel + 1, acciones);
        }
    }
}

/// Barra superior: Añadir, Importar, atrás/adelante, ruta, y a la derecha vista, filtro y búsqueda.
fn barra(ed: &mut Editor, ui: &mut Ui, raiz: &Path, acciones: &mut Vec<Accion>) {
    ui.horizontal(|ui| {
        ui.menu_button(RichText::new("＋ Añadir").strong(), |ui| {
            menu_crear(ui, &mut ed.contenido);
        });
        let destino = raiz.join(&ed.contenido.carpeta);
        crate::importar::botones(ed, ui, destino);
        ui.separator();
        let nombre = ed.project.name();
        let c = &mut ed.contenido;
        if ui.add_enabled(!c.atras.is_empty(), egui::Button::new("◀").small()).on_hover_text("Atrás").clicked()
            && let Some(d) = c.atras.pop()
        {
            c.adelante.push(std::mem::replace(&mut c.carpeta, d));
            c.seleccion.clear();
        }
        if ui.add_enabled(!c.adelante.is_empty(), egui::Button::new("▶").small()).on_hover_text("Adelante").clicked()
            && let Some(d) = c.adelante.pop()
        {
            c.atras.push(std::mem::replace(&mut c.carpeta, d));
            c.seleccion.clear();
        }
        // Ruta: Proyecto › assets › models (cada parte lleva a su carpeta y acepta lo arrastrado).
        let mut partes = vec![(String::new(), nombre.to_string())];
        let mut acc = String::new();
        for s in c.carpeta.split('/').filter(|s| !s.is_empty()) {
            if !acc.is_empty() {
                acc.push('/');
            }
            acc.push_str(s);
            partes.push((acc.clone(), s.to_string()));
        }
        ui.spacing_mut().item_spacing.x = 2.0;
        let n = partes.len();
        for (i, (rel, txt)) in partes.into_iter().enumerate() {
            if i > 0 {
                ui.label(RichText::new("›").color(theme::TEXT3));
            }
            let r = ui.add(egui::Button::new(RichText::new(&txt).color(if i + 1 == n { theme::TEXT1 } else { theme::TEXT2 })).frame(false));
            if !c.arrastrando.is_empty() && r.contains_pointer() {
                ui.painter().rect_stroke(r.rect, 3.0, Stroke::new(1.0, theme::ACCENT), egui::StrokeKind::Inside);
                if ui.input(|i| i.pointer.any_released()) {
                    acciones.push(Accion::Mover(std::mem::take(&mut c.arrastrando), raiz.join(&rel)));
                }
            }
            if r.clicked() {
                c.buscar.clear();
                c.ir(rel);
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            if ui.add(egui::Button::selectable(c.lista, "☰")).on_hover_text("Lista").clicked() {
                c.lista = true;
            }
            if ui.add(egui::Button::selectable(!c.lista, "▦")).on_hover_text("Cuadrícula").clicked() {
                c.lista = false;
            }
            egui::ComboBox::from_id_salt("contenido-tipo").selected_text(c.tipo.map(Tipo::nombre).unwrap_or("Todo")).width(120.0).show_ui(ui, |ui| {
                ui.selectable_value(&mut c.tipo, None, "Todo");
                for t in Tipo::FILTROS {
                    ui.selectable_value(&mut c.tipo, Some(t), t.nombre());
                }
            });
            ui.add(egui::TextEdit::singleline(&mut c.buscar).hint_text("Buscar en el proyecto…").desired_width(200.0));
        });
    });
}

/// Submenú de crear (en «Añadir» y en el clic derecho del fondo).
fn menu_crear(ui: &mut Ui, c: &mut ContenidoUi) {
    for (que, txt) in [(Creable::Carpeta, "Carpeta"), (Creable::Escena, "Escena"), (Creable::Script, "Script"), (Creable::Efecto, "Efecto de posprocesado")] {
        if ui.button(txt).clicked() {
            c.buscar.clear();
            c.renombrar = None;
            c.creando = Some((que, que.nombre_inicial().to_string(), true));
            ui.close();
        }
    }
}

/// Menú del fondo (zona vacía o carpeta del árbol).
fn menu_fondo(ui: &mut Ui, c: &mut ContenidoUi, dir: &Path, acciones: &mut Vec<Accion>) {
    ui.menu_button("Crear", |ui| menu_crear(ui, c));
    ui.separator();
    if ui.button("Mostrar en el explorador").clicked() {
        acciones.push(Accion::Explorador(dir.to_path_buf()));
        ui.close();
    }
    if ui.button("Orden de escenas…").clicked() {
        acciones.push(Accion::OrdenEscenas);
        ui.close();
    }
}

/// Cuadrícula o lista de la carpeta abierta (o de la búsqueda).
fn elementos_ui(ed: &mut Editor, ui: &mut Ui, raiz: &Path, items: &[PathBuf], acciones: &mut Vec<Accion>) {
    let ctx = ui.ctx().clone();
    let escena_actual = ed.scene_path.clone();
    let renderer = &ed.renderer;
    let c = &mut ed.contenido;
    let mut puede = true;
    // Entrada nueva (creando) primero, luego los elementos.
    let mut entradas: Vec<Option<&PathBuf>> = Vec::with_capacity(items.len() + 1);
    if c.creando.is_some() {
        entradas.push(None);
    }
    entradas.extend(items.iter().map(Some));
    let area = ui.available_rect_before_wrap();
    let fondo = ui.interact(area, ui.scope_id().with("contenido-fondo"), egui::Sense::click());
    if fondo.clicked() {
        c.seleccion.clear();
    }
    let dir = raiz.join(&c.carpeta);
    fondo.context_menu(|ui| menu_fondo(ui, c, &dir, acciones));

    let lista = c.lista;
    let (celda_w, celda_h) = if lista { (ui.available_width(), FILA) } else { (c.tam + 24.0, c.tam + 30.0) };
    let cols = if lista { 1 } else { ((ui.available_width() / (celda_w + 4.0)).floor() as usize).max(1) };
    let filas = entradas.len().div_ceil(cols);
    egui::ScrollArea::vertical().auto_shrink([false, false]).show_rows(ui, celda_h + if lista { 0.0 } else { 4.0 }, filas, |ui, rango| {
        if lista {
            ui.spacing_mut().item_spacing.y = 0.0;
        }
        for fila in rango {
            ui.horizontal(|ui| {
                for e in entradas.iter().skip(fila * cols).take(cols) {
                    let (rect, resp) = ui.allocate_exact_size(egui::vec2(celda_w, celda_h), egui::Sense::click_and_drag());
                    let Some(p) = e else {
                        // Elemento nuevo: icono + nombre editable.
                        let Some((que, mut texto, foco)) = c.creando.clone() else { continue };
                        celda(ui, c, rect, que.tipo(), None, false, false);
                        let r = editar_nombre(ui, rect, lista, &mut texto, foco);
                        if ui.input(|i| i.key_pressed(Key::Escape)) {
                            c.creando = None;
                        } else if r.lost_focus() {
                            c.creando = None;
                            if let Some(n) = nombre_valido(&texto) {
                                acciones.push(Accion::Crear(que, n));
                            }
                        } else {
                            c.creando = Some((que, texto, false));
                        }
                        continue;
                    };
                    let p: &PathBuf = p;
                    let tipo = Tipo::de(p);
                    let sel = c.seleccionado(p) || (tipo == Tipo::Escena && *p == escena_actual && c.seleccion.is_empty());
                    let tex = if matches!(tipo, Tipo::Modelo | Tipo::Imagen) { c.mini(renderer, raiz, &ctx, p, &mut puede) } else { None };
                    // Soltar lo arrastrado sobre una carpeta.
                    let destino = tipo == Tipo::Carpeta && !c.arrastrando.is_empty() && ui.rect_contains_pointer(rect) && !c.arrastrando.iter().any(|a| p.starts_with(a));
                    if destino && ui.input(|i| i.pointer.any_released()) {
                        acciones.push(Accion::Mover(std::mem::take(&mut c.arrastrando), p.clone()));
                    }
                    let renombrando = c.renombrar.as_ref().is_some_and(|(r, _, _)| r == p);
                    celda(ui, c, rect, tipo, tex.as_ref(), sel || destino, resp.hovered());
                    let nombre = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    if renombrando {
                        let Some((_, mut texto, foco)) = c.renombrar.clone() else { continue };
                        let r = editar_nombre(ui, rect, lista, &mut texto, foco);
                        if ui.input(|i| i.key_pressed(Key::Escape)) {
                            c.renombrar = None;
                        } else if r.lost_focus() {
                            c.renombrar = None;
                            if texto != nombre
                                && let Some(n) = nombre_valido(&texto)
                            {
                                acciones.push(Accion::Renombrar(p.clone(), n));
                            }
                        } else {
                            c.renombrar = Some((p.clone(), texto, false));
                        }
                    } else {
                        texto_celda(ui, rect, lista, &nombre, tipo, p);
                    }
                    if resp.drag_started() {
                        if !c.seleccionado(p) {
                            c.seleccion = vec![p.clone()];
                        }
                        c.arrastrando = c.seleccion.clone();
                        let rel = relativa(raiz, p);
                        match tipo {
                            Tipo::Escena => egui::DragAndDrop::set_payload(&ctx, crate::escenas_ui::EscenaDrag(rel)),
                            Tipo::Modelo | Tipo::Plantilla => egui::DragAndDrop::set_payload(&ctx, AssetDrag(rel)),
                            _ => {}
                        }
                    }
                    if resp.clicked() {
                        let mods = ui.input(|i| i.modifiers);
                        c.clic(p, items, mods);
                    }
                    if resp.secondary_clicked() && !c.seleccionado(p) {
                        c.seleccion = vec![p.clone()];
                    }
                    if resp.double_clicked() {
                        acciones.push(Accion::Abrir(p.clone()));
                    }
                    let rel = relativa(raiz, p);
                    let resp = if c.arrastrando.is_empty() { resp.on_hover_text(&rel) } else { resp };
                    resp.context_menu(|ui| {
                        if ui.button("Abrir").clicked() {
                            acciones.push(Accion::Abrir(p.clone()));
                            ui.close();
                        }
                        if ui.button("Mostrar en el explorador").clicked() {
                            acciones.push(Accion::Explorador(p.parent().unwrap_or(raiz).to_path_buf()));
                            ui.close();
                        }
                        ui.separator();
                        if ui.add(egui::Button::new("Renombrar").shortcut_text("F2")).clicked() {
                            c.renombrar = Some((p.clone(), nombre.clone(), true));
                            ui.close();
                        }
                        if ui.add(egui::Button::new("Duplicar").shortcut_text("Ctrl+D")).clicked() {
                            acciones.push(Accion::Duplicar(c.seleccion.clone()));
                            ui.close();
                        }
                        if ui.add(egui::Button::new("Eliminar").shortcut_text("Supr")).clicked() {
                            c.borrar = Some(c.seleccion.clone());
                            ui.close();
                        }
                        ui.separator();
                        if ui.button("Copiar ruta").clicked() {
                            ui.ctx().copy_text(rel.clone());
                            ui.close();
                        }
                    });
                }
            });
        }
    });
    // Etiqueta junto al puntero mientras se arrastra.
    if !c.arrastrando.is_empty()
        && let Some(pos) = ctx.pointer_latest_pos()
    {
        let n = c.arrastrando.len();
        let t = if n > 1 { format!("{n} elementos") } else { c.arrastrando[0].file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default() };
        let pintor = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique("contenido-arrastre")));
        let galley = pintor.layout_no_wrap(t, egui::FontId::proportional(12.0), theme::TEXT1);
        let r = egui::Rect::from_min_size(pos + egui::vec2(14.0, 10.0), galley.size() + egui::vec2(12.0, 6.0));
        pintor.rect(r, 4.0, theme::CARD, Stroke::new(1.0, theme::STROKE2), egui::StrokeKind::Inside);
        pintor.galley(r.min + egui::vec2(6.0, 3.0), galley, theme::TEXT1);
    }
    if !puede {
        // Quedan miniaturas: otra en el siguiente fotograma.
        ctx.request_repaint();
    }
}

/// Fondo e icono/miniatura de una celda.
fn celda(ui: &Ui, c: &ContenidoUi, rect: egui::Rect, tipo: Tipo, tex: Option<&egui::TextureHandle>, sel: bool, hover: bool) {
    let p = ui.painter_at(rect);
    if sel {
        p.rect(rect, 4.0, theme::ACCENT_SOFT, Stroke::new(1.0, theme::ACCENT), egui::StrokeKind::Inside);
    } else if hover {
        p.rect_filled(rect, 4.0, theme::ROW_HOVER);
    }
    let ctx = ui.ctx();
    let (icono, color) = tipo.icono();
    let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
    if c.lista {
        let lado = icons::side(ctx);
        let r = egui::Rect::from_min_size(egui::pos2(rect.left() + 6.0, rect.center().y - lado / 2.0), egui::vec2(lado, lado));
        match tex {
            Some(t) => {
                p.image(t.id(), r, uv, Color32::WHITE);
            }
            None => icons::image(ctx, icono, color).paint_at(ui, r),
        }
        return;
    }
    let img = egui::Rect::from_min_size(egui::pos2(rect.center().x - c.tam / 2.0, rect.top() + 4.0), egui::vec2(c.tam, c.tam));
    match tex {
        Some(t) => {
            let [w, h] = t.size();
            let k = c.tam / w.max(h) as f32;
            p.image(t.id(), egui::Rect::from_center_size(img.center(), egui::vec2(w as f32 * k, h as f32 * k)), uv, Color32::WHITE);
        }
        None => {
            // Icono pixel a escala entera para que se vea nítido.
            let base = icons::side(ctx);
            let lado = base * (c.tam * 0.6 / base).floor().max(1.0);
            icons::image(ctx, icono, color).paint_at(ui, egui::Rect::from_center_size(img.center(), egui::vec2(lado, lado)));
        }
    }
    // Franja con el color del tipo bajo la miniatura (como Unreal).
    if tipo != Tipo::Carpeta {
        p.rect_filled(egui::Rect::from_min_size(egui::pos2(img.left(), img.bottom() + 1.0), egui::vec2(img.width(), 2.0)), 1.0, color.gamma_multiply(0.8));
    }
}

/// Nombre (y en la lista, tipo y tamaño) de una celda.
fn texto_celda(ui: &Ui, rect: egui::Rect, lista: bool, nombre: &str, tipo: Tipo, p: &Path) {
    let pt = ui.painter_at(rect);
    let font = egui::FontId::proportional(if lista { 12.5 } else { 11.5 });
    if lista {
        let x = rect.left() + 12.0 + icons::side(ui.ctx());
        let (ancho_tipo, ancho_tam) = (110.0, 80.0);
        let ancho_nombre = (rect.width() - (x - rect.left()) - ancho_tipo - ancho_tam - 8.0).max(40.0);
        pt.text(egui::pos2(x, rect.center().y), egui::Align2::LEFT_CENTER, elidir(ui, nombre, ancho_nombre, &font), font.clone(), theme::TEXT1);
        pt.text(egui::pos2(rect.right() - ancho_tam - ancho_tipo, rect.center().y), egui::Align2::LEFT_CENTER, tipo.singular(), font.clone(), theme::TEXT3);
        if tipo != Tipo::Carpeta
            && let Ok(m) = std::fs::metadata(p)
        {
            pt.text(egui::pos2(rect.right() - 8.0, rect.center().y), egui::Align2::RIGHT_CENTER, tamano(m.len()), font, theme::TEXT3);
        }
    } else {
        let texto = elidir(ui, nombre, rect.width() - 6.0, &font);
        pt.text(egui::pos2(rect.center().x, rect.bottom() - 4.0), egui::Align2::CENTER_BOTTOM, texto, font, theme::TEXT1);
    }
}

/// Cuadro de texto para renombrar/crear en el sitio del nombre.
fn editar_nombre(ui: &mut Ui, rect: egui::Rect, lista: bool, texto: &mut String, foco: bool) -> egui::Response {
    let r = if lista {
        let x = rect.left() + 10.0 + icons::side(ui.ctx());
        egui::Rect::from_min_max(egui::pos2(x, rect.top() + 1.0), egui::pos2((x + 260.0).min(rect.right()), rect.bottom() - 1.0))
    } else {
        egui::Rect::from_min_max(egui::pos2(rect.left() + 1.0, rect.bottom() - 20.0), egui::pos2(rect.right() - 1.0, rect.bottom()))
    };
    let id = egui::Id::unique("contenido-nombre");
    let resp = ui.put(r, egui::TextEdit::singleline(texto).id(id).font(egui::FontId::proportional(11.5)).margin(egui::Margin::symmetric(3, 1)));
    if foco {
        resp.request_focus();
        // Seleccionar el nombre sin la extensión, como en los exploradores.
        let fin = texto.rfind('.').filter(|&i| i > 0).unwrap_or(texto.len());
        let fin = texto[..fin].chars().count();
        let mut st = egui::TextEdit::load_state(ui.ctx(), id).unwrap_or_default();
        st.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(fin))));
        st.store(ui.ctx(), id);
    }
    resp
}

fn tamano(b: u64) -> String {
    match b {
        0..1024 => format!("{b} B"),
        1024..1_048_576 => format!("{:.0} KB", b as f64 / 1024.0),
        _ => format!("{:.1} MB", b as f64 / 1_048_576.0),
    }
}

/// Atajos con el ratón sobre Contenido y sin escribir en ningún campo.
fn teclado(ed: &mut Editor, ui: &Ui, items: &[PathBuf], acciones: &mut Vec<Accion>) {
    let ctx = ui.ctx().clone();
    if ctx.egui_wants_keyboard_input() || !ui.rect_contains_pointer(ui.max_rect()) {
        return;
    }
    let c = &mut ed.contenido;
    ctx.input_mut(|i| {
        if i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, Key::A)) {
            c.seleccion = items.to_vec();
        }
        if i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, Key::D)) && !c.seleccion.is_empty() {
            acciones.push(Accion::Duplicar(c.seleccion.clone()));
        }
        if i.consume_key(egui::Modifiers::NONE, Key::F2)
            && let [p] = c.seleccion.as_slice()
        {
            let n = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            c.renombrar = Some((p.clone(), n, true));
        }
        if i.consume_key(egui::Modifiers::NONE, Key::Delete) && !c.seleccion.is_empty() {
            c.borrar = Some(c.seleccion.clone());
        }
        if i.consume_key(egui::Modifiers::NONE, Key::Enter)
            && let [p] = c.seleccion.as_slice()
        {
            acciones.push(Accion::Abrir(p.clone()));
        }
        if i.consume_key(egui::Modifiers::NONE, Key::Backspace) && !c.carpeta.is_empty() {
            let arriba = c.carpeta.rsplit_once('/').map(|(a, _)| a.to_string()).unwrap_or_default();
            c.ir(arriba);
        }
    });
}

/// Diálogo de confirmación para eliminar (va a la papelera del proyecto).
fn confirmar_borrado(ed: &mut Editor, ctx: &egui::Context, raiz: &Path, acciones: &mut Vec<Accion>) {
    let Some(lista) = ed.contenido.borrar.clone() else { return };
    let mut cerrar = false;
    egui::Modal::new(egui::Id::unique("contenido-borrar")).show(ctx, |ui| {
        ui.set_width(340.0);
        let t = match lista.as_slice() {
            [p] => format!("¿Eliminar «{}»?", relativa(raiz, p)),
            v => format!("¿Eliminar {} elementos?", v.len()),
        };
        ui.label(RichText::new(t).strong());
        ui.label(RichText::new(format!("Se mueven a {PAPELERA}.")).small().color(theme::TEXT3));
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button("Eliminar").clicked() || ui.input(|i| i.key_pressed(Key::Enter)) {
                acciones.push(Accion::Borrar(lista.clone()));
                cerrar = true;
            }
            if ui.button("Cancelar").clicked() || ui.input(|i| i.key_pressed(Key::Escape)) {
                cerrar = true;
            }
        });
    });
    if cerrar {
        ed.contenido.borrar = None;
    }
}

// ---------------------------------------------------------------------------------------------

/// Tras renombrar o mover `viejo` → `nuevo` (absolutas): referencias en disco, escena abierta,
/// proyecto y estado del explorador.
fn tras_mover(ed: &mut Editor, raiz: &Path, viejo: &Path, nuevo: &Path) {
    let (rv, rn) = (relativa(raiz, viejo), relativa(raiz, nuevo));
    let n = actualizar_referencias(raiz, &rv, &rn);
    // Escena abierta (puede tener cambios sin guardar): en memoria.
    let json = ed.scene.to_json();
    let cambiado = reemplazar_ruta(&json, &rv, &rn);
    if cambiado != json
        && let Ok(s) = maker_engine::scene::Scene::from_json(&cambiado)
    {
        ed.scene = s;
    }
    if let Ok(j) = serde_json::to_string(&ed.project.file) {
        let r = reemplazar_ruta(&j, &rv, &rn);
        if r != j
            && let Ok(f) = serde_json::from_str(&r)
        {
            ed.project.file = f;
        }
    }
    if let Ok(resto) = ed.scene_path.strip_prefix(viejo) {
        ed.scene_path = if resto.as_os_str().is_empty() { nuevo.to_path_buf() } else { nuevo.join(resto) };
    }
    if let Some(s) = &mut ed.script
        && let Ok(resto) = s.path.strip_prefix(viejo)
    {
        s.path = if resto.as_os_str().is_empty() { nuevo.to_path_buf() } else { nuevo.join(resto) };
    }
    let c = &mut ed.contenido;
    if let Ok(resto) = Path::new(&c.carpeta).strip_prefix(&rv) {
        c.carpeta = relativa(raiz, &nuevo.join(resto));
    }
    c.minis.retain(|p, _| !p.starts_with(viejo));
    if n > 0 {
        maker_engine::log::info("contenido", format!("{rv} → {rn} ({n} archivos con referencias actualizados)"));
    } else {
        maker_engine::log::info("contenido", format!("{rv} → {rn}"));
    }
}

fn abrir_explorador(p: &Path) {
    let _ = std::process::Command::new(if cfg!(windows) { "explorer" } else if cfg!(target_os = "macos") { "open" } else { "xdg-open" }).arg(p).spawn();
}

fn aplicar(ed: &mut Editor, raiz: &Path, a: Accion) {
    use maker_engine::log;
    match a {
        Accion::Abrir(p) => abrir(ed, raiz, &p),
        Accion::Crear(que, nombre) => {
            let dir = raiz.join(&ed.contenido.carpeta);
            match que {
                Creable::Carpeta => {
                    let d = crate::importar::libre(&dir, &nombre, &[]);
                    match std::fs::create_dir_all(&d) {
                        Ok(()) => ed.contenido.seleccion = vec![d],
                        Err(e) => log::error("contenido", format!("{}: {e}", d.display())),
                    }
                }
                Creable::Escena => {
                    let n = nombre.trim_end_matches(".mscene");
                    let p = crate::importar::libre(&dir, &format!("{n}.mscene"), &[]);
                    match maker_engine::scene::Scene::starter().save(&p) {
                        Ok(()) => {
                            if ed.dirty {
                                ed.save_scene();
                            }
                            ed.open_scene(&relativa(raiz, &p));
                            ed.contenido.seleccion = vec![p];
                        }
                        Err(e) => log::error("contenido", e),
                    }
                }
                Creable::Script => ed.create_script(&nombre),
                Creable::Efecto => {
                    if let Some(rel) = ed.create_post_effect(&nombre) {
                        ed.open_script(raiz.join(rel));
                    }
                }
            }
        }
        Accion::Renombrar(p, nombre) => {
            let destino = p.with_file_name(&nombre);
            if destino.exists() {
                log::warn("contenido", format!("ya existe {}", relativa(raiz, &destino)));
                return;
            }
            match std::fs::rename(&p, &destino) {
                Ok(()) => {
                    tras_mover(ed, raiz, &p, &destino);
                    ed.contenido.seleccion = vec![destino];
                }
                Err(e) => log::error("contenido", format!("{}: {e}", p.display())),
            }
        }
        Accion::Mover(lista, dir) => {
            let mut nuevos = Vec::new();
            for p in lista {
                if p.parent() == Some(dir.as_path()) || dir.starts_with(&p) {
                    continue;
                }
                let nombre = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let destino = crate::importar::libre(&dir, &nombre, &[]);
                match std::fs::rename(&p, &destino) {
                    Ok(()) => {
                        tras_mover(ed, raiz, &p, &destino);
                        nuevos.push(destino);
                    }
                    Err(e) => log::error("contenido", format!("{}: {e}", p.display())),
                }
            }
            if !nuevos.is_empty() {
                ed.contenido.seleccion = nuevos;
            }
        }
        Accion::Duplicar(lista) => {
            let mut nuevos = Vec::new();
            for p in lista {
                let (Some(dir), Some(nombre)) = (p.parent(), p.file_name()) else { continue };
                let destino = crate::importar::libre(dir, &nombre.to_string_lossy(), &[]);
                match copiar(&p, &destino) {
                    Ok(()) => nuevos.push(destino),
                    Err(e) => log::error("contenido", format!("{}: {e}", p.display())),
                }
            }
            if !nuevos.is_empty() {
                ed.contenido.seleccion = nuevos;
            }
        }
        Accion::Borrar(lista) => {
            let s = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
            let papelera = raiz.join(PAPELERA).join(s.to_string());
            for p in lista {
                if ed.scene_path.starts_with(&p) {
                    log::warn("contenido", format!("{} está abierta: no se elimina", relativa(raiz, &p)));
                    continue;
                }
                let destino = papelera.join(relativa(raiz, &p));
                let r = destino.parent().map(std::fs::create_dir_all).unwrap_or(Ok(())).and_then(|_| std::fs::rename(&p, &destino));
                match r {
                    Ok(()) => log::info("contenido", format!("eliminado {} (en {PAPELERA})", relativa(raiz, &p))),
                    Err(e) => log::error("contenido", format!("{}: {e}", p.display())),
                }
                ed.contenido.minis.remove(&p);
            }
            ed.contenido.seleccion.clear();
        }
        Accion::Explorador(d) => abrir_explorador(&d),
        Accion::OrdenEscenas => ed.ventana_escenas = true,
    }
}

/// Doble clic / Intro: entra en carpetas, abre escenas, scripts y estilos, añade modelos…
fn abrir(ed: &mut Editor, raiz: &Path, p: &Path) {
    let rel = relativa(raiz, p);
    match Tipo::de(p) {
        Tipo::Carpeta => {
            ed.contenido.buscar.clear();
            ed.contenido.ir(rel);
        }
        Tipo::Escena => {
            if ed.project.relative(&ed.scene_path) != rel {
                if ed.dirty {
                    ed.save_scene();
                }
                ed.open_scene(&rel);
            }
        }
        Tipo::Modelo => {
            ed.add_model(&rel);
        }
        Tipo::Estilo => {
            ed.estilos.abrir_hoja(&ed.ui.recursos, &rel);
            ed.bottom_tab = crate::editor::BottomTab::Estilos;
        }
        Tipo::Tema => {
            ed.estilos.abrir_tema(&p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default());
            ed.bottom_tab = crate::editor::BottomTab::Estilos;
        }
        Tipo::Plantilla => {
            ed.instanciar_plantilla(&rel, None);
        }
        Tipo::EstiloVisual => crate::estilo_visual_ui::aplicar_archivo(ed, p),
        Tipo::Script => ed.open_script(p.to_path_buf()),
        Tipo::Imagen | Tipo::Fuente | Tipo::Otro => {
            if es_texto(p) {
                ed.open_script(p.to_path_buf());
            } else {
                abrir_explorador(p);
            }
        }
    }
}

/// Archivos de texto que el editor de código puede abrir.
fn es_texto(p: &Path) -> bool {
    matches!(p.extension().and_then(|e| e.to_str()), Some("toml" | "json" | "md" | "txt" | "maker" | "wgsl" | "ron" | "yaml" | "yml"))
}

fn elidir(ui: &Ui, texto: &str, ancho: f32, font: &egui::FontId) -> String {
    let cabe = |s: &str| ui.ctx().fonts_mut(|f| f.layout_no_wrap(s.to_string(), font.clone(), Color32::BLACK).size().x) <= ancho;
    if cabe(texto) {
        return texto.to_string();
    }
    let mut c: Vec<char> = texto.chars().collect();
    while !c.is_empty() {
        c.pop();
        let s: String = c.iter().collect::<String>() + "…";
        if cabe(&s) {
            return s;
        }
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proyecto(n: &str) -> PathBuf {
        let r = std::env::temp_dir().join(format!("maker-cont-{n}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&r);
        r
    }

    #[test]
    fn lista_carpeta_y_busca() {
        let r = proyecto("lista");
        for d in ["scenes", "assets/models", "target", ".maker"] {
            std::fs::create_dir_all(r.join(d)).unwrap();
        }
        for f in ["scenes/main.mscene", "assets/models/casa.glb", "project.maker", "target/x.rs"] {
            std::fs::write(r.join(f), "x").unwrap();
        }
        let raiz: Vec<String> = elementos(&r, "", "", None).iter().map(|p| relativa(&r, p)).collect();
        assert_eq!(raiz, ["assets", "scenes", "project.maker"], "carpetas primero, sin ocultas ni target");
        let modelos: Vec<String> = elementos(&r, "", "cas", None).iter().map(|p| relativa(&r, p)).collect();
        assert_eq!(modelos, ["assets/models/casa.glb"]);
        assert_eq!(elementos(&r, "", "", Some(Tipo::Escena)).len(), 0, "el filtro sin búsqueda mira la carpeta abierta");
        assert_eq!(subcarpetas(&r).iter().map(|p| relativa(&r, p)).collect::<Vec<_>>(), ["assets", "scenes"]);
        let _ = std::fs::remove_dir_all(&r);
    }

    #[test]
    fn renombrar_actualiza_referencias() {
        let r = proyecto("refs");
        std::fs::create_dir_all(r.join("scenes")).unwrap();
        std::fs::write(r.join("project.maker"), r#"{"name":"P","main_scene":"scenes/a.mscene"}"#).unwrap();
        std::fs::write(r.join("scenes/a.mscene"), r#"{"m":"assets/casa.glb","t":"assets/casa.glb.png","d":"assets/sub/x.glb"}"#).unwrap();
        assert_eq!(actualizar_referencias(&r, "assets/casa.glb", "modelos/casa.glb"), 1);
        assert_eq!(actualizar_referencias(&r, "assets/sub", "otros/sub"), 1);
        let t = std::fs::read_to_string(r.join("scenes/a.mscene")).unwrap();
        assert_eq!(t, r#"{"m":"modelos/casa.glb","t":"assets/casa.glb.png","d":"otros/sub/x.glb"}"#, "solo rutas exactas o prefijos de carpeta");
        assert_eq!(actualizar_referencias(&r, "scenes/a.mscene", "scenes/b.mscene"), 1);
        assert!(std::fs::read_to_string(r.join("project.maker")).unwrap().contains("scenes/b.mscene"));
        let _ = std::fs::remove_dir_all(&r);
    }

    #[test]
    fn seleccion_con_ctrl_y_mayus() {
        let items: Vec<PathBuf> = ["a", "b", "c", "d"].iter().map(PathBuf::from).collect();
        let mut c = ContenidoUi::default();
        c.clic(&items[1], &items, egui::Modifiers::NONE);
        c.clic(&items[3], &items, egui::Modifiers::SHIFT);
        assert_eq!(c.seleccion, items[1..=3].to_vec());
        c.clic(&items[2], &items, egui::Modifiers::COMMAND);
        assert_eq!(c.seleccion, vec![items[1].clone(), items[3].clone()]);
        c.clic(&items[0], &items, egui::Modifiers::NONE);
        assert_eq!(c.seleccion, vec![items[0].clone()]);
        assert_eq!(nombre_valido(" a/b "), None);
        assert_eq!(nombre_valido(" Nivel 2 ").as_deref(), Some("Nivel 2"));
    }
}
