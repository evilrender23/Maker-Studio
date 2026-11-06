//! Ventana «Orden de escenas» (como *Scenes In Build* de Unity): la lista
//! ordenada de `project.maker`. Arrastrar filas las reordena; soltar una escena
//! de Contenido la añade; la primera activa (con el icono de inicio) es la de
//! arranque. Cada cambio se guarda en el acto.

use crate::editor::Editor;
use crate::icons::{self, Icon};
use crate::theme;
use egui::{RichText, Ui};
use maker_engine::escenas;

/// Escena arrastrada desde Contenido (ruta relativa).
#[derive(Clone, Debug)]
pub struct EscenaDrag(pub String);

/// Fila de la lista que se está arrastrando.
#[derive(Clone, Copy, Debug)]
struct FilaDrag(usize);

const ALTO: f32 = 24.0;

enum Cambio {
    Mover(usize, usize),
    Insertar(String, usize),
    Activa(usize, bool),
    Quitar(usize),
}

pub fn ventana(ed: &mut Editor, ctx: &egui::Context) {
    let mut abierta = ed.ventana_escenas;
    let mut cambio = None;
    let mut jugar = false;
    egui::Window::new("Orden de escenas").open(&mut abierta).collapsible(false).default_width(420.0).min_width(320.0).show(ctx, |ui| {
        cambio = lista(ed, ui);
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let abierta = ed.project.relative(&ed.scene_path);
            let ya = ed.project.file.escenas.iter().any(|e| e.ruta == abierta);
            if ui.add_enabled(!ya, egui::Button::new("Añadir escena abierta")).clicked() {
                cambio = Some(Cambio::Insertar(abierta, usize::MAX));
            }
            let fuera: Vec<String> = ed.project.scenes().into_iter().filter(|s| !ed.project.file.escenas.iter().any(|e| e.ruta == *s)).collect();
            ui.add_enabled_ui(!fuera.is_empty(), |ui| {
                ui.menu_button("Añadir…", |ui| {
                    for s in fuera {
                        if ui.button(&s).clicked() {
                            cambio = Some(Cambio::Insertar(s, usize::MAX));
                        }
                    }
                });
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let b = egui::Button::image(icons::image(ui.ctx(), Icon::Inicio, theme::ACCENT));
                if ui.add_enabled(!ed.is_playing(), b).on_hover_text("Play desde la primera escena (Stop vuelve a la abierta)").clicked() {
                    jugar = true;
                }
            });
        });
    });
    ed.ventana_escenas = abierta;
    if let Some(c) = cambio {
        let l = &mut ed.project.file.escenas;
        match c {
            Cambio::Mover(de, antes) => escenas::mover_antes(l, de, antes),
            Cambio::Insertar(r, pos) => escenas::insertar(l, &r, pos),
            Cambio::Activa(i, v) => l[i].activa = v,
            Cambio::Quitar(i) => {
                l.remove(i);
            }
        }
        ed.guardar_orden_escenas();
    }
    if jugar {
        ed.play_desde_inicio();
    }
}

/// Filas de la lista. Devuelve el cambio pedido (como mucho uno por fotograma).
fn lista(ed: &Editor, ui: &mut Ui) -> Option<Cambio> {
    let mut cambio = None;
    let raiz = &ed.project.root;
    let l = &ed.project.file.escenas;
    let inicio = l.iter().position(|e| e.activa);
    let arrastrando = egui::DragAndDrop::has_payload_of_type::<FilaDrag>(ui.ctx()) || egui::DragAndDrop::has_payload_of_type::<EscenaDrag>(ui.ctx());
    // Destino de lo que se suelta: índice de la fila delante de la que cae.
    let mut destino: Option<(usize, f32)> = None;
    let mut indice = 0;
    let ancho = ui.available_width();
    for (i, e) in l.iter().enumerate() {
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(ancho, ALTO), egui::Sense::drag());
        if resp.drag_started() {
            egui::DragAndDrop::set_payload(ui.ctx(), FilaDrag(i));
        }
        if resp.hovered() && !arrastrando {
            ui.painter().rect_filled(rect, 3.0, theme::ROW_HOVER);
        }
        if arrastrando && let Some(p) = ui.ctx().pointer_hover_pos().filter(|p| rect.contains(*p)) {
            destino = Some(if p.y < rect.center().y { (i, rect.top()) } else { (i + 1, rect.bottom()) });
        }
        let mut fila = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink2(egui::vec2(4.0, 0.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
        let falta = escenas::falta(raiz, &e.ruta);
        let num = if e.activa {
            indice += 1;
            (indice - 1).to_string()
        } else {
            "–".into()
        };
        fila.add_sized([22.0, ALTO], egui::Label::new(RichText::new(num).monospace().color(theme::TEXT3)));
        let mut activa = e.activa;
        if fila.checkbox(&mut activa, "").on_hover_text("Activa: se exporta y cuenta para los scripts").changed() {
            cambio = Some(Cambio::Activa(i, activa));
        }
        let lado = icons::side(fila.ctx());
        if Some(i) == inicio {
            fila.add(icons::image(fila.ctx(), Icon::Inicio, theme::ACCENT)).on_hover_text("Escena de arranque");
        } else {
            fila.add_space(lado + fila.spacing().item_spacing.x);
        }
        let color = if falta {
            theme::RED
        } else if e.activa {
            theme::TEXT1
        } else {
            theme::TEXT4
        };
        let r = fila.add(egui::Label::new(RichText::new(&e.ruta).color(color)).selectable(false).truncate());
        if falta {
            r.on_hover_text("No existe el archivo: no se exportará");
        }
        fila.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.small_button("✕").on_hover_text("Quitar del orden (el archivo no se borra)").clicked() {
                cambio = Some(Cambio::Quitar(i));
            }
        });
    }
    // Zona bajo la lista: soltar aquí añade al final (y con la lista vacía es la única).
    let (resto, _) = ui.allocate_exact_size(egui::vec2(ancho, if l.is_empty() { 48.0 } else { 12.0 }), egui::Sense::hover());
    if l.is_empty() {
        ui.painter().text(resto.center(), egui::Align2::CENTER_CENTER, "Arrastra escenas desde Contenido", egui::FontId::proportional(12.0), theme::TEXT3);
    }
    if arrastrando && ui.ctx().pointer_hover_pos().is_some_and(|p| resto.contains(p)) {
        destino = Some((l.len(), resto.top()));
    }
    if let Some((antes, y)) = destino {
        ui.painter().hline(ui.min_rect().x_range(), y, egui::Stroke::new(2.0, theme::ACCENT));
        if ui.input(|i| i.pointer.any_released()) {
            if let Some(f) = egui::DragAndDrop::take_payload::<FilaDrag>(ui.ctx()) {
                cambio = Some(Cambio::Mover(f.0, antes));
            } else if let Some(s) = egui::DragAndDrop::take_payload::<EscenaDrag>(ui.ctx()) {
                cambio = Some(Cambio::Insertar(s.0.clone(), antes));
            }
        }
    }
    cambio
}

/// Para el menú de Archivo: pone la escena abierta la primera (la de arranque).
pub fn hacer_inicial(ed: &mut Editor) {
    let r = ed.project.relative(&ed.scene_path);
    escenas::insertar(&mut ed.project.file.escenas, &r, 0);
    ed.guardar_orden_escenas();
}
