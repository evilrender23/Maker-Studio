//! Pestaña Contenido: los archivos del proyecto como la Biblioteca. Árbol de carpetas a la
//! izquierda, filtro por tipo, búsqueda y una cuadrícula con miniaturas: los modelos se
//! renderizan con el propio motor (una por fotograma, en caché fuera del proyecto) y el resto
//! lleva su icono pixel. Arrastrar un modelo al viewport o a la jerarquía lo añade; doble clic
//! abre escenas y scripts, entra en carpetas o añade modelos.

use crate::editor::Editor;
use crate::icons::{self, Icon};
use crate::theme;
use crate::viewport::AssetDrag;
use egui::{Color32, RichText, Ui};
use maker_render::SceneRenderer;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const CELDA: f32 = 104.0;
const MINI: f32 = 80.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tipo {
    Carpeta,
    Escena,
    Modelo,
    Script,
    Otro,
}

impl Tipo {
    const FILTROS: [Tipo; 4] = [Tipo::Escena, Tipo::Modelo, Tipo::Script, Tipo::Otro];

    pub fn de(p: &Path) -> Tipo {
        if p.is_dir() {
            return Tipo::Carpeta;
        }
        match p.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
            Some("mscene") => Tipo::Escena,
            Some("gltf" | "glb") => Tipo::Modelo,
            Some("rs") => Tipo::Script,
            _ => Tipo::Otro,
        }
    }

    fn nombre(self) -> &'static str {
        match self {
            Tipo::Carpeta => "Carpetas",
            Tipo::Escena => "Escenas",
            Tipo::Modelo => "Modelos",
            Tipo::Script => "Scripts",
            Tipo::Otro => "Otros",
        }
    }

    fn icono(self) -> (Icon, Color32) {
        match self {
            Tipo::Carpeta => (Icon::Carpeta, theme::YELLOW),
            Tipo::Escena => (Icon::Escena, theme::GREEN),
            Tipo::Modelo => (Icon::Modelo, theme::ACCENT),
            Tipo::Script => (Icon::Script, theme::ORANGE),
            Tipo::Otro => (Icon::Archivo, theme::TEXT3),
        }
    }
}

#[derive(Default)]
pub struct ContenidoUi {
    /// Carpeta abierta, relativa a la raíz del proyecto ("" = raíz).
    carpeta: String,
    tipo: Option<Tipo>,
    buscar: String,
    seleccion: Option<PathBuf>,
    /// Miniaturas de modelos por ruta: `None` = falló (se muestra el icono).
    minis: HashMap<PathBuf, Option<egui::TextureHandle>>,
    renderer: Option<SceneRenderer>,
}

/// Carpetas del proyecto (relativas), sin ocultas, `target` ni `export`, en orden.
fn carpetas(raiz: &Path) -> Vec<String> {
    fn rec(raiz: &Path, d: &Path, out: &mut Vec<String>) {
        let Ok(rd) = std::fs::read_dir(d) else { return };
        let mut v: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.is_dir() && visible(p)).collect();
        v.sort();
        for p in v {
            out.push(relativa(raiz, &p));
            rec(raiz, &p, out);
        }
    }
    let mut out = Vec::new();
    rec(raiz, raiz, &mut out);
    out
}

fn visible(p: &Path) -> bool {
    let n = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    !(n.starts_with('.') || n == "target" || n == "export")
}

fn relativa(raiz: &Path, p: &Path) -> String {
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
            if relativa(raiz, p).to_lowercase().contains(&q) {
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
    /// Miniatura de un modelo: de la caché o, como mucho una por fotograma, renderizada.
    fn mini(&mut self, ed_renderer: &SceneRenderer, raiz: &Path, ctx: &egui::Context, p: &Path, puede_render: &mut bool) -> Option<egui::TextureHandle> {
        if let Some(t) = self.minis.get(p) {
            return t.clone();
        }
        let archivo = clave_cache(p);
        let img = match archivo.as_ref().and_then(|f| image::open(f).ok()) {
            Some(i) => Some(i.to_rgba8()),
            None if *puede_render => {
                *puede_render = false;
                let r = self.renderer.get_or_insert_with(|| {
                    let (d, q) = ed_renderer.gpu();
                    let mut r = SceneRenderer::new(d, q);
                    r.project_root = Some(raiz.to_path_buf());
                    r
                });
                let src = maker_engine::components::MeshSource::Model(relativa(raiz, p));
                let img = maker_render::miniatura::miniatura(r, &src, None, 96);
                if let (Some(i), Some(f)) = (&img, &archivo) {
                    let _ = i.save(f);
                }
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

enum Accion {
    Entrar(String),
    AbrirEscena(String),
    AbrirScript(PathBuf),
    AnadirModelo(String),
}

pub fn show(ed: &mut Editor, ui: &mut Ui) {
    let raiz = ed.project.root.clone();
    let ctx = ui.ctx().clone();
    barra(ed, ui);
    ui.add_space(4.0);
    let c = &mut ed.contenido;
    if !c.carpeta.is_empty() && !raiz.join(&c.carpeta).is_dir() {
        c.carpeta.clear();
    }
    let mut accion = None;
    egui::Panel::left("contenido-carpetas").resizable(true).default_size(190.0).min_size(140.0).frame(egui::Frame::new().inner_margin(egui::Margin { right: 6, ..Default::default() })).show(ui, |ui| {
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let nombre = ed.project.name();
            if ui.add(egui::Button::selectable(c.carpeta.is_empty(), nombre).min_size(egui::vec2(ui.available_width(), 0.0))).clicked() {
                c.carpeta.clear();
            }
            for d in carpetas(&raiz) {
                let nivel = d.matches('/').count() + 1;
                let nombre = d.rsplit('/').next().unwrap_or(&d).to_string();
                ui.horizontal(|ui| {
                    ui.add_space(nivel as f32 * 12.0);
                    if ui.add(egui::Button::selectable(c.carpeta == d, nombre).min_size(egui::vec2(ui.available_width(), 0.0))).clicked() {
                        c.carpeta = d.clone();
                    }
                });
            }
        });
    });
    ui.horizontal(|ui| {
        if !c.carpeta.is_empty() && c.buscar.trim().is_empty() && ui.small_button("↑").on_hover_text("Carpeta superior").clicked() {
            c.carpeta = c.carpeta.rsplit_once('/').map(|(a, _)| a.to_string()).unwrap_or_default();
        }
        egui::ComboBox::from_id_salt("contenido-tipo").selected_text(c.tipo.map(Tipo::nombre).unwrap_or("Todos los tipos")).width(150.0).show_ui(ui, |ui| {
            ui.selectable_value(&mut c.tipo, None, "Todos los tipos");
            for t in Tipo::FILTROS {
                ui.selectable_value(&mut c.tipo, Some(t), t.nombre());
            }
        });
        ui.add(egui::TextEdit::singleline(&mut c.buscar).hint_text("Buscar…").desired_width(220.0));
    });
    ui.add_space(4.0);
    let items = elementos(&raiz, &c.carpeta, &c.buscar, c.tipo);
    let escena_actual = ed.scene_path.clone();
    let cols = ((ui.available_width() / CELDA).floor() as usize).max(1);
    let filas = items.len().div_ceil(cols);
    let mut puede_render = true;
    let renderer = &ed.renderer;
    egui::ScrollArea::vertical().auto_shrink([false, false]).show_rows(ui, CELDA + 4.0, filas, |ui, rango| {
        for fila in rango {
            ui.horizontal(|ui| {
                for p in items.iter().skip(fila * cols).take(cols) {
                    let tipo = Tipo::de(p);
                    let rel = relativa(&raiz, p);
                    let seleccionado = c.seleccion.as_deref() == Some(p.as_path()) || (tipo == Tipo::Escena && *p == escena_actual);
                    let tex = if tipo == Tipo::Modelo { c.mini(renderer, &raiz, &ctx, p, &mut puede_render) } else { None };
                    let sentido = if tipo == Tipo::Modelo { egui::Sense::click_and_drag() } else { egui::Sense::click() };
                    let (rect, resp) = ui.allocate_exact_size(egui::vec2(CELDA - 4.0, CELDA), sentido);
                    let pt = ui.painter_at(rect);
                    let fondo = if seleccionado {
                        theme::ACCENT_SOFT
                    } else if resp.hovered() {
                        theme::ROW_HOVER
                    } else {
                        Color32::TRANSPARENT
                    };
                    pt.rect_filled(rect, 4.0, fondo);
                    if seleccionado {
                        pt.rect_stroke(rect, 4.0, egui::Stroke::new(1.0, theme::ACCENT), egui::StrokeKind::Inside);
                    }
                    let img_rect = egui::Rect::from_center_size(egui::pos2(rect.center().x, rect.top() + 4.0 + MINI * 0.5), egui::vec2(MINI, MINI));
                    if let Some(t) = &tex {
                        pt.image(t.id(), img_rect, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
                    } else {
                        // Icono pixel a escala entera (×3) para que se vea nítido.
                        let (icono, color) = tipo.icono();
                        let lado = icons::side(&ctx) * 3.0;
                        icons::image(&ctx, icono, color).paint_at(ui, egui::Rect::from_center_size(img_rect.center(), egui::vec2(lado, lado)));
                    }
                    let nombre = elidir(ui, &p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), CELDA - 10.0);
                    pt.text(egui::pos2(rect.center().x, rect.bottom() - 4.0), egui::Align2::CENTER_BOTTOM, nombre, egui::FontId::proportional(11.0), theme::TEXT1);
                    if resp.drag_started() {
                        egui::DragAndDrop::set_payload(&resp.ctx, AssetDrag(rel.clone()));
                    }
                    if resp.clicked() {
                        c.seleccion = Some(p.clone());
                    }
                    if resp.double_clicked() {
                        accion = Some(match tipo {
                            Tipo::Carpeta => Accion::Entrar(rel.clone()),
                            Tipo::Escena => Accion::AbrirEscena(rel.clone()),
                            Tipo::Modelo => Accion::AnadirModelo(rel.clone()),
                            Tipo::Script | Tipo::Otro => Accion::AbrirScript(p.clone()),
                        });
                    }
                    resp.on_hover_text(rel);
                }
            });
        }
    });
    if !puede_render {
        // Quedan miniaturas por renderizar: otra en el siguiente fotograma.
        ctx.request_repaint();
    }
    match accion {
        Some(Accion::Entrar(d)) => {
            ed.contenido.carpeta = d;
            ed.contenido.buscar.clear();
        }
        Some(Accion::AbrirEscena(s)) => {
            if ed.project.relative(&ed.scene_path) != s {
                if ed.dirty {
                    ed.save_scene();
                }
                ed.open_scene(&s);
            }
        }
        Some(Accion::AbrirScript(p)) => {
            if Tipo::de(&p) == Tipo::Script || es_texto(&p) {
                ed.open_script(p);
            }
        }
        Some(Accion::AnadirModelo(m)) => {
            ed.add_model(&m);
        }
        None => {}
    }
}

/// Archivos de texto que el editor de código puede abrir.
fn es_texto(p: &Path) -> bool {
    matches!(p.extension().and_then(|e| e.to_str()), Some("toml" | "json" | "md" | "txt" | "maker" | "wgsl" | "ron" | "yaml" | "yml"))
}

/// Botones de crear e importar (los de antes de la pestaña).
fn barra(ed: &mut Editor, ui: &mut Ui) {
    ui.horizontal(|ui| {
        ui.menu_button("Nueva escena", |ui| {
            ui.label("Nombre:");
            let r = ui.text_edit_singleline(&mut ed.new_script_name);
            if (ui.button("Crear").clicked() || (r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))) && !ed.new_script_name.trim().is_empty() {
                let n = std::mem::take(&mut ed.new_script_name);
                ed.new_scene(&n);
                ui.close();
            }
        });
        ui.menu_button("Nuevo script", |ui| {
            ui.label("Nombre del tipo (PascalCase):");
            let r = ui.text_edit_singleline(&mut ed.new_script_name);
            if (ui.button("Crear").clicked() || (r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))) && !ed.new_script_name.trim().is_empty() {
                let n = std::mem::take(&mut ed.new_script_name);
                ed.create_script(&n);
                ui.close();
            }
        });
        ui.menu_button("Nuevo efecto", |ui| {
            ui.label("Nombre del archivo:");
            let r = ui.text_edit_singleline(&mut ed.new_script_name);
            if (ui.button("Crear").clicked() || (r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))) && !ed.new_script_name.trim().is_empty() {
                let n = std::mem::take(&mut ed.new_script_name);
                ed.create_post_effect(&n);
                ui.close();
            }
        })
        .response
        .on_hover_text(format!("Efecto de posprocesado WGSL en {}", maker_engine::postproceso::CUSTOM_DIR));
        ui.menu_button("Importar modelo", |ui| {
            ui.label("Ruta de un .gltf / .glb:");
            ui.add(egui::TextEdit::singleline(&mut ed.import_path).desired_width(320.0));
            if ui.button("Importar").clicked() {
                crate::panels::import_model(ed);
                ui.close();
            }
        });
        if ui.button("Abrir carpeta").clicked() {
            let d = ed.project.root.join(&ed.contenido.carpeta);
            let _ = std::process::Command::new(if cfg!(windows) { "explorer" } else if cfg!(target_os = "macos") { "open" } else { "xdg-open" }).arg(d).spawn();
        }
        if let Some(l) = &ed.lib {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new(format!("Scripts: {}", l.names().join(", "))).small().color(theme::GREEN));
            });
        }
    });
}

fn elidir(ui: &Ui, texto: &str, ancho: f32) -> String {
    let font = egui::FontId::proportional(11.0);
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

    #[test]
    fn lista_carpeta_y_busca() {
        let r = std::env::temp_dir().join(format!("maker-cont-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&r);
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
        assert_eq!(carpetas(&r), ["assets", "assets/models", "scenes"]);
        let _ = std::fs::remove_dir_all(&r);
    }
}
