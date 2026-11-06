//! Modo Texturizado: pintar materiales PBR por capas sobre un asset 3D con el motor de
//! TextureCraft (`maker_textura`). Sustituye la distribución del editor mientras está abierto:
//! estante de materiales a la izquierda, visor 3D/UV en el centro, texture sets, capas y
//! propiedades a la derecha. Ver `docs/TEXTURIZADO.md`.

mod capas;
mod es;
mod estante;
pub mod gpu;
mod propiedades;
mod ventanas;
mod vista;

use crate::editor::Editor;
use crate::theme;
use egui::{RichText, Ui};
use maker_engine::components::{EntityId, MeshRenderer, MeshSource};
use maker_engine::log;
use maker_textura::sesion::{self, Analisis, Opciones, Textura};
use maker_textura::tc;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Texto de la interfaz de TextureCraft en español (el original si no está traducido).
pub fn tr(s: &str) -> &str {
    static T: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    T.get_or_init(|| es::TABLE.iter().copied().collect()).get(s).copied().unwrap_or(s)
}

/// Asset elegido para texturizar, a la espera de las opciones.
pub struct Pedido {
    pub ruta: PathBuf,
    pub analisis: Result<Analisis, String>,
    pub op: Opciones,
    pub entidad: Option<EntityId>,
}

pub struct ModoTextura {
    pub t: Textura,
    /// Entidad cuya malla pasa al modelo texturizado al aplicar.
    pub entidad: Option<EntityId>,
    enlace: Option<gpu::Enlace>,
    pub vista_uv: bool,
    uv_zoom: f32,
    uv_pan: [f32; 2],
    aux: Option<gpu::Aux>,
    estante: estante::Estante,
    /// `layer.list` del texture set activo, por revisión del documento.
    capas: (u64, usize, Value),
    renombrar: Option<(u64, String)>,
    ventanas: ventanas::Ventanas,
    ultimo_punto: Option<tc::geom::glam::Vec2>,
    confirmar_salida: bool,
}

impl ModoTextura {
    /// Ejecuta un comando de TextureCraft; los errores van a la consola.
    pub fn run(&mut self, id: &str, p: Value) -> Option<Value> {
        match tc::execute(&mut self.t.s, id, &p) {
            Ok(v) => Some(v),
            Err(e) => {
                log::error("texturizado", format!("{id}: {e}"));
                None
            }
        }
    }

    /// Sustituye la sesión por otra del mismo asset (cambiada fuera del editor).
    pub fn recargar(&mut self, t: Textura) {
        let entidad = self.entidad;
        self.t = t;
        self.entidad = entidad;
        self.capas = (u64::MAX, 0, Value::Null);
        self.renombrar = None;
    }

    /// Pide confirmar la salida (cambios sin guardar).
    pub fn pedir_salida(&mut self) {
        self.confirmar_salida = true;
    }

    fn capas(&mut self) -> &Value {
        let ts = self.t.s.ts();
        if self.capas.0 != self.t.s.revision || self.capas.1 != ts || self.capas.2.is_null() {
            let v = tc::execute(&mut self.t.s, "layer.list", &json!({ "ts": ts })).unwrap_or(Value::Null);
            self.capas = (self.t.s.revision, ts, v);
        }
        &self.capas.2
    }
}

/// Elige un asset para texturizar: un `.tcraft` se abre directamente; un modelo pide opciones.
pub fn pedir(ed: &mut Editor, ruta: PathBuf, entidad: Option<EntityId>, rs: &egui_wgpu::RenderState) {
    if ed.is_playing() {
        return;
    }
    if ruta.extension().is_some_and(|e| e == "tcraft") {
        abrir(ed, &ruta, Opciones::default(), entidad, rs);
        return;
    }
    // Ya texturizado: se retoma su proyecto.
    let previo = sesion::carpeta_de(&ed.project.root, &sesion::nombre_de(&ruta)).join(format!("{}.tcraft", sesion::nombre_de(&ruta)));
    if ruta.starts_with(ed.project.root.join(sesion::CARPETA)) && previo.is_file() {
        abrir(ed, &previo, Opciones::default(), entidad, rs);
        return;
    }
    let analisis = sesion::analizar(&ruta);
    let mut op = Opciones::default();
    if let Ok(a) = &analisis {
        op.uv_unicos = a.solapado() && !a.con_piel;
        op.desde_original = !op.uv_unicos;
        if a.triangulos > 200_000 {
            op.resolucion = 1024;
        }
    }
    ed.pedido_textura = Some(Pedido { ruta, analisis, op, entidad });
}

/// La entidad seleccionada con un modelo: su archivo.
pub fn modelo_de(ed: &Editor, id: EntityId) -> Option<PathBuf> {
    match ed.scene.get::<MeshRenderer>(id)?.mesh {
        MeshSource::Model(m) => Some(maker_engine::biblioteca::resolve(&m, Some(&ed.project.root))),
        MeshSource::Primitive(_) => None,
    }
}

fn abrir(ed: &mut Editor, ruta: &Path, op: Opciones, entidad: Option<EntityId>, rs: &egui_wgpu::RenderState) {
    match sesion::abrir(ruta, &ed.project.root, op) {
        Ok(mut t) => {
            for l in t.s.log.drain(..) {
                log::info("texturizado", l);
            }
            tc::execute(&mut t.s, "viewport.frame", &json!({})).ok();
            log::info("texturizado", format!("abierto {}", ruta.display()));
            ed.textura = Some(Box::new(ModoTextura {
                t,
                entidad,
                enlace: Some(gpu::iniciar(rs)),
                vista_uv: false,
                uv_zoom: 1.0,
                uv_pan: [0.0, 0.0],
                aux: None,
                estante: Default::default(),
                capas: (u64::MAX, 0, Value::Null),
                renombrar: None,
                ventanas: Default::default(),
                ultimo_punto: None,
                confirmar_salida: false,
            }));
        }
        Err(e) => log::error("texturizado", format!("{}: {e}", ruta.display())),
    }
}

/// Opciones antes de abrir un modelo.
pub fn ventana_pedido(ed: &mut Editor, ctx: &egui::Context, rs: &egui_wgpu::RenderState) {
    let Some(p) = &mut ed.pedido_textura else { return };
    let mut decision = None;
    egui::Modal::new(egui::Id::unique("pedido-textura")).show(ctx, |ui| {
        ui.set_width(380.0);
        let nombre = p.ruta.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        ui.label(RichText::new(format!("Texturizar {nombre}")).strong());
        ui.add_space(6.0);
        match &p.analisis {
            Err(e) => {
                ui.colored_label(theme::RED, e);
            }
            Ok(a) => {
                ui.label(RichText::new(format!("{} triángulos · {} materiales", a.triangulos, a.materiales)).color(theme::TEXT3));
                if a.solapado() {
                    ui.colored_label(theme::ORANGE, format!("El {:.0} % de los UV se solapa: lo pintado en un sitio aparece en otros.", a.solape * 100.0));
                }
                ui.add_space(6.0);
                egui::Grid::new("pedido-grid").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                    ui.label("Resolución");
                    egui::ComboBox::from_id_salt("pedido-res").selected_text(format!("{} px", p.op.resolucion)).show_ui(ui, |ui| {
                        for r in [512, 1024, 2048, 4096] {
                            ui.selectable_value(&mut p.op.resolucion, r, format!("{r} px"));
                        }
                    });
                    ui.end_row();
                    ui.label("UV únicos");
                    ui.add_enabled_ui(!a.con_piel, |ui| {
                        if ui.checkbox(&mut p.op.uv_unicos, "").on_disabled_hover_text("Personaje con esqueleto: perdería la piel").changed() && p.op.uv_unicos {
                            p.op.desde_original = false;
                        }
                    });
                    ui.end_row();
                    ui.label("Partir de sus texturas");
                    ui.add_enabled(!p.op.uv_unicos, egui::Checkbox::without_text(&mut p.op.desde_original)).on_disabled_hover_text("Con UV nuevos las texturas originales no encajan");
                    ui.end_row();
                });
            }
        }
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if ui.add_enabled(p.analisis.is_ok(), egui::Button::new(RichText::new("Abrir").color(egui::Color32::WHITE)).fill(theme::ACCENT)).clicked() {
                decision = Some(true);
            }
            if ui.button("Cancelar").clicked() {
                decision = Some(false);
            }
        });
    });
    match decision {
        Some(true) => {
            if let Some(p) = ed.pedido_textura.take() {
                abrir(ed, &p.ruta, p.op, p.entidad, rs);
            }
        }
        Some(false) => ed.pedido_textura = None,
        None => {}
    }
}

fn guardar(ed: &mut Editor) {
    let Some(m) = &mut ed.textura else { return };
    match sesion::guardar(&mut m.t) {
        Ok(p) => ed.status = format!("Guardado {}", ed.project.relative(&p)),
        Err(e) => log::error("texturizado", e),
    }
}

/// Escribe texturas y modelo, y pone el modelo nuevo en la entidad de origen.
fn aplicar(ed: &mut Editor) {
    let Some(m) = &mut ed.textura else { return };
    let glb = match sesion::aplicar(&mut m.t) {
        Ok(g) => g,
        Err(e) => {
            log::error("texturizado", e);
            return;
        }
    };
    let entidad = m.entidad;
    let rel = ed.project.relative(&glb);
    ed.renderer.clear_model_cache();
    if let Some(id) = entidad
        && let Some(mut mr) = ed.scene.get::<MeshRenderer>(id)
        && ed.scene.contains(id)
    {
        mr.mesh = MeshSource::Model(rel.clone());
        ed.scene.insert(id, mr);
        ed.commit("Aplicar texturizado");
    }
    ed.status = format!("Aplicado: {rel}");
    log::info("texturizado", format!("aplicado en {rel}"));
}

fn salir(ed: &mut Editor, rs: &egui_wgpu::RenderState) {
    ed.textura = None;
    gpu::cerrar(rs);
}

/// Atajos del modo (los del editor no se procesan mientras está abierto).
pub fn atajos(ed: &mut Editor, ctx: &egui::Context) {
    use egui::{Key as K, KeyboardShortcut as S, Modifiers as M};
    if ctx.egui_wants_keyboard_input() {
        return;
    }
    let mut accion = None;
    ctx.input_mut(|i| {
        if i.consume_shortcut(&S::new(M::COMMAND | M::SHIFT, K::Z)) || i.consume_shortcut(&S::new(M::COMMAND, K::Y)) {
            accion = Some("edit.redo");
        } else if i.consume_shortcut(&S::new(M::COMMAND, K::Z)) {
            accion = Some("edit.undo");
        } else if i.consume_shortcut(&S::new(M::COMMAND, K::S)) {
            accion = Some("guardar");
        } else if i.consume_key(M::NONE, K::F) {
            accion = Some("viewport.frame");
        } else if i.consume_key(M::NONE, K::B) {
            accion = Some("tool.paint");
        } else if i.consume_key(M::NONE, K::E) {
            accion = Some("tool.eraser");
        } else if i.consume_key(M::NONE, K::P) {
            accion = Some("tool.polygonFill");
        } else if i.consume_key(M::NONE, K::I) {
            accion = Some("tool.colorPicker");
        } else if i.consume_key(M::NONE, K::Tab) {
            accion = Some("vista");
        } else if i.consume_key(M::NONE, K::OpenBracket) {
            accion = Some("menor");
        } else if i.consume_key(M::NONE, K::CloseBracket) {
            accion = Some("mayor");
        }
    });
    let Some(a) = accion else { return };
    if a == "guardar" {
        guardar(ed);
        return;
    }
    let Some(m) = &mut ed.textura else { return };
    match a {
        "vista" => m.vista_uv = !m.vista_uv,
        "menor" | "mayor" => {
            let k = if a == "mayor" { 1.25 } else { 0.8 };
            let b = &mut m.t.s.tool.brush;
            b.size = (b.size * k).clamp(0.0005, 0.25);
        }
        id => {
            m.run(id, json!({}));
        }
    }
}

/// Barra superior del modo (sustituye a la de Play).
pub fn barra(ed: &mut Editor, ui: &mut Ui, rs: &egui_wgpu::RenderState) {
    let mut accion = None;
    {
        let Some(m) = &mut ed.textura else { return };
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("Texturizado · {}", m.t.nombre)).color(theme::TEXT2).strong());
            if m.t.s.modified {
                ui.label(RichText::new("●").color(theme::ORANGE));
            }
            ui.separator();
            let puede_deshacer = m.t.s.can_undo();
            if ui.add_enabled(puede_deshacer, egui::Button::new("↶")).on_hover_text(m.t.s.undo_label().map(|l| format!("Deshacer: {}  Ctrl+Z", tr(l))).unwrap_or_default()).clicked() {
                m.run("edit.undo", json!({}));
            }
            if ui.add_enabled(m.t.s.can_redo(), egui::Button::new("↷")).on_hover_text(m.t.s.redo_label().map(|l| format!("Rehacer: {}  Ctrl+Y", tr(l))).unwrap_or_default()).clicked() {
                m.run("edit.redo", json!({}));
            }
            ui.separator();
            if ui.selectable_label(!m.vista_uv, "3D").clicked() {
                m.vista_uv = false;
            }
            if ui.selectable_label(m.vista_uv, "UV").on_hover_text("Tab").clicked() {
                m.vista_uv = true;
            }
            vista::modo_de_vista(m, ui);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Salir").clicked() {
                    accion = Some(0);
                }
                if ui.add(egui::Button::new(RichText::new("Aplicar al asset").color(egui::Color32::WHITE)).fill(theme::ACCENT)).on_hover_text("Guarda, exporta las texturas y pone el modelo texturizado en la escena").clicked() {
                    accion = Some(1);
                }
                if ui.button("Guardar").on_hover_text("Ctrl+S").clicked() {
                    accion = Some(2);
                }
                ui.separator();
                if ui.button("Exportar…").clicked() {
                    m.ventanas.exportar = true;
                }
                if ui.button("Hornear…").clicked() {
                    m.ventanas.hornear = true;
                }
            });
        });
    }
    match accion {
        Some(0) => {
            if ed.textura.as_ref().is_some_and(|m| m.t.s.modified) {
                if let Some(m) = &mut ed.textura {
                    m.confirmar_salida = true;
                }
            } else {
                salir(ed, rs);
            }
        }
        Some(1) => aplicar(ed),
        Some(2) => guardar(ed),
        _ => {}
    }
}

/// Distribución completa del modo.
pub fn mostrar(ed: &mut Editor, ui: &mut Ui, rs: &egui_wgpu::RenderState) {
    let ctx = ui.ctx().clone();
    {
        let Some(m) = &mut ed.textura else { return };
        egui::Panel::left("textura-estante").frame(theme::sidebar_frame()).resizable(true).default_size(250.0).min_size(190.0).show(ui, |ui| estante::mostrar(m, ui));
        egui::Panel::right("textura-derecha").frame(theme::sidebar_frame()).resizable(true).default_size(340.0).min_size(260.0).show(ui, |ui| {
            capas::texture_sets(m, ui);
            ui.separator();
            egui::Panel::bottom("textura-props").resizable(true).default_size(ui.available_height() * 0.5).frame(egui::Frame::new()).show(ui, |ui| {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| propiedades::mostrar(m, ui));
            });
            capas::mostrar(m, ui);
        });
        egui::CentralPanel::no_frame().show(ui, |ui| {
            egui::Frame::new().fill(theme::CHROME).inner_margin(egui::Margin::symmetric(6, 4)).show(ui, |ui| {
                vista::herramientas(m, ui);
                vista::mostrar(m, ui);
            });
        });
        ventanas::mostrar(m, &ctx);
        for l in m.t.s.log.drain(..) {
            log::info("texturizado", l);
        }
        if m.t.s.stroke_active() || m.t.s.caches.iter().any(|c| !c.upload.is_empty()) || m.t.s.pending_tiles() > 0 {
            ctx.request_repaint();
        }
    }
    // Salir con cambios.
    let pedir = ed.textura.as_ref().is_some_and(|m| m.confirmar_salida);
    if pedir {
        let mut d = None;
        egui::Modal::new(egui::Id::unique("textura-salir")).show(&ctx, |ui| {
            ui.set_width(320.0);
            ui.label(RichText::new("¿Guardar el texturizado?").strong());
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.add(egui::Button::new(RichText::new("Guardar").color(egui::Color32::WHITE)).fill(theme::ACCENT)).clicked() {
                    d = Some(1);
                }
                if ui.button("Descartar").clicked() {
                    d = Some(2);
                }
                if ui.button("Cancelar").clicked() {
                    d = Some(0);
                }
            });
        });
        match d {
            Some(1) => {
                guardar(ed);
                salir(ed, rs);
            }
            Some(2) => salir(ed, rs),
            Some(_) => {
                if let Some(m) = &mut ed.textura {
                    m.confirmar_salida = false;
                }
            }
            None => {}
        }
    }
}

/// Atiende la petición de texturizar de este fotograma (Contenido, Inspector, menú).
pub fn procesar(ed: &mut Editor, rs: &egui_wgpu::RenderState) {
    if let Some((ruta, entidad)) = ed.texturizar.take()
        && ed.textura.is_none()
    {
        pedir(ed, ruta, entidad, rs);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn textos_en_espanol_y_mallas() {
        assert_eq!(super::tr("Base color"), "Color base");
        assert_eq!(super::tr("sin traducir"), "sin traducir");
        assert!(maker_textura::sesion::es_malla(std::path::Path::new("a/b.GLB")));
        assert!(!maker_textura::sesion::es_malla(std::path::Path::new("a/b.png")));
        assert_eq!(crate::contenido_ui::Tipo::de(std::path::Path::new("x.tcraft")), crate::contenido_ui::Tipo::Textura);
    }
}
