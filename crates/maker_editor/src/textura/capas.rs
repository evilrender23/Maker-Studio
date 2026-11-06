//! Texture sets y pila de capas (de arriba abajo), con máscaras, arrastrar para reordenar y
//! soltar materiales del estante.

use super::estante::Arrastre;
use super::{ModoTextura, tr};
use crate::theme;
use egui::{RichText, Ui};
use maker_textura::tc::StackTarget;
use maker_textura::tc::core::LayerId;
use serde_json::{Value, json};

/// Capa que se arrastra dentro de la pila.
#[derive(Clone, Copy, Debug)]
struct CapaArrastre(u64);

pub fn texture_sets(m: &mut ModoTextura, ui: &mut Ui) {
    theme::section_label(ui, "Texture sets");
    let activo = m.t.s.ts();
    let sets: Vec<(String, bool, u32)> = m.t.s.doc.project.texture_sets.iter().map(|t| (t.name.clone(), t.visible, t.resolution)).collect();
    let mut accion: Option<(&str, Value)> = None;
    for (i, (nombre, visible, res)) in sets.iter().enumerate() {
        ui.horizontal(|ui| {
            if ui.small_button(if *visible { "👁" } else { "–" }).on_hover_text("Visibilidad").clicked() {
                accion = Some(("textureSet.visible", json!({ "ts": i, "visible": !visible })));
            }
            if ui.add(egui::Button::selectable(i == activo, nombre).min_size(egui::vec2(ui.available_width() - 80.0, 0.0))).clicked() && i != activo {
                accion = Some(("textureSet.select", json!({ "ts": i })));
            }
            egui::ComboBox::from_id_salt(("ts-res", i)).selected_text(format!("{res}")).width(64.0).show_ui(ui, |ui| {
                for r in [256u32, 512, 1024, 2048, 4096, 8192] {
                    if ui.selectable_label(*res == r, format!("{r}")).clicked() && *res != r {
                        accion = Some(("textureSet.resolution", json!({ "ts": i, "resolution": r })));
                    }
                }
            });
        });
    }
    if let Some((id, p)) = accion {
        m.run(id, p);
    }
}

pub fn mostrar(m: &mut ModoTextura, ui: &mut Ui) {
    theme::section_label(ui, "Capas");
    let sel = m.t.s.selected_layer().map(|l| l.0);
    let mut acciones: Vec<(&'static str, Value)> = Vec::new();
    ui.horizontal_wrapped(|ui| {
        if ui.small_button("+ Pintura").clicked() {
            acciones.push(("layer.add.paint", json!({})));
        }
        if ui.small_button("+ Relleno").clicked() {
            acciones.push(("layer.add.fill", json!({})));
        }
        if ui.small_button("+ Carpeta").clicked() {
            acciones.push(("layer.add.group", json!({})));
        }
        ui.add_enabled_ui(sel.is_some(), |ui| {
            ui.menu_button("Máscara", |ui| menu_mascara(m, ui, None, &mut acciones));
            if ui.small_button("Duplicar").clicked() {
                acciones.push(("layer.duplicate", json!({})));
            }
            if ui.small_button("Eliminar").clicked() {
                acciones.push(("layer.remove", json!({})));
            }
        });
    });
    ui.add_space(2.0);
    let capas = m.capas().clone();
    let resp = egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for c in capas.as_array().into_iter().flatten() {
                fila(m, ui, c, 0, sel, &mut acciones);
            }
            ui.allocate_rect(ui.available_rect_before_wrap(), egui::Sense::hover())
        })
        .inner;
    // Material soltado bajo la última capa: encima de la selección.
    if let Some(a) = resp.dnd_release_payload::<Arrastre>() {
        super::estante::soltar(m, &a);
    }
    for (id, p) in acciones {
        m.run(id, p);
    }
}

fn menu_mascara(m: &ModoTextura, ui: &mut Ui, capa: Option<u64>, acciones: &mut Vec<(&'static str, Value)>) {
    let con = |mut p: Value| {
        if let Some(c) = capa {
            p["layer"] = c.into();
        }
        p
    };
    for (n, id) in [("Blanca", "mask.add.white"), ("Negra", "mask.add.black")] {
        if ui.button(n).clicked() {
            acciones.push((id, con(json!({}))));
            ui.close();
        }
    }
    ui.menu_button("Inteligente", |ui| {
        for (id, nombre) in m.estante.mascaras.iter().flatten() {
            if ui.button(tr(nombre)).clicked() {
                acciones.push(("mask.smart.apply", con(json!({ "name": id }))));
                ui.close();
            }
        }
    });
    ui.separator();
    for (n, id) in [("Generador de máscara", "effect.add.generator"), ("Pintura en la máscara", "effect.add.paint"), ("Invertir", "mask.invert"), ("Activar / desactivar", "mask.toggle"), ("Quitar", "mask.remove")] {
        if ui.button(n).clicked() {
            let p = if id == "effect.add.paint" { json!({ "target": "mask" }) } else { json!({}) };
            acciones.push((id, con(p)));
            ui.close();
        }
    }
}

fn icono(kind: &str, abierta: bool) -> &'static str {
    match kind {
        "fill" => "▣",
        "group" if abierta => "▾",
        "group" => "▸",
        "instance" => "⧉",
        _ => "✎",
    }
}

fn fila(m: &mut ModoTextura, ui: &mut Ui, c: &Value, nivel: usize, sel: Option<u64>, acciones: &mut Vec<(&'static str, Value)>) {
    let id = c["id"].as_u64().unwrap_or(0);
    let nombre = c["name"].as_str().unwrap_or("").to_string();
    let visible = c["visible"].as_bool().unwrap_or(true);
    let kind = c["kind"].as_str().unwrap_or("paint");
    let colapsada = c["collapsed"].as_bool().unwrap_or(false);
    let es_sel = sel == Some(id);
    let objetivo = m.t.s.target;
    let alto = 26.0;
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), alto), egui::Sense::click_and_drag());
    let p = ui.painter().clone();
    if es_sel {
        p.rect_filled(rect, 3.0, theme::ACCENT_SOFT);
    } else if resp.hovered() {
        p.rect_filled(rect, 3.0, theme::ROW_HOVER);
    }
    let mut x = rect.left() + 4.0 + nivel as f32 * 14.0;
    let ojo = egui::Rect::from_min_size(egui::pos2(x, rect.top() + 3.0), egui::vec2(20.0, 20.0));
    let ojo_r = ui.interact(ojo, ui.scope_id().with(("ojo", id)), egui::Sense::click());
    p.text(ojo.center(), egui::Align2::CENTER_CENTER, if visible { "👁" } else { "–" }, egui::FontId::proportional(12.0), if visible { theme::TEXT2 } else { theme::TEXT4 });
    if ojo_r.clicked() {
        acciones.push(("layer.visible", json!({ "layer": id, "visible": !visible })));
    }
    x += 22.0;
    let tipo = egui::Rect::from_min_size(egui::pos2(x, rect.top() + 3.0), egui::vec2(18.0, 20.0));
    let tipo_r = ui.interact(tipo, ui.scope_id().with(("tipo", id)), egui::Sense::click());
    p.text(tipo.center(), egui::Align2::CENTER_CENTER, icono(kind, !colapsada), egui::FontId::proportional(13.0), theme::TEXT3);
    if kind == "group" && tipo_r.clicked() {
        // Plegar/desplegar es estado del documento.
        if let Ok(l) = m.t.s.doc.project.layer_mut(LayerId(id))
            && let maker_textura::tc::core::LayerKind::Group { collapsed, .. } = &mut l.kind
        {
            *collapsed = !*collapsed;
            m.t.s.touched();
        }
    }
    x += 22.0;
    // Chip de la máscara: clic la selecciona para pintarla o editar sus efectos.
    let tiene_mascara = c["mask"].is_object();
    let derecha = if tiene_mascara { 30.0 } else { 6.0 };
    let texto = egui::Rect::from_min_max(egui::pos2(x, rect.top()), egui::pos2(rect.right() - derecha, rect.bottom()));
    if m.renombrar.as_ref().is_some_and(|r| r.0 == id) {
        let mut ui2 = ui.new_child(egui::UiBuilder::new().max_rect(texto.shrink2(egui::vec2(0.0, 2.0))));
        let (_, t) = m.renombrar.as_mut().unwrap();
        let r = ui2.add(egui::TextEdit::singleline(t).desired_width(texto.width()));
        r.request_focus();
        if r.lost_focus() {
            let (_, t) = m.renombrar.take().unwrap();
            if ui.input(|i| !i.key_pressed(egui::Key::Escape)) && !t.trim().is_empty() {
                acciones.push(("layer.rename", json!({ "layer": id, "name": t.trim() })));
            }
        }
    } else {
        let color = if es_sel && objetivo == StackTarget::Content { theme::ACCENT } else { theme::TEXT1 };
        p.with_clip_rect(texto).text(egui::pos2(texto.left(), rect.center().y), egui::Align2::LEFT_CENTER, tr(&nombre), egui::FontId::proportional(13.0), color);
    }
    if tiene_mascara {
        let chip = egui::Rect::from_min_size(egui::pos2(rect.right() - 26.0, rect.top() + 4.0), egui::vec2(22.0, 18.0));
        let activa = c["mask"]["enabled"].as_bool().unwrap_or(true);
        let fondo_negro = c["mask"]["background"].as_str() == Some("black");
        let mascara_sel = es_sel && objetivo == StackTarget::Mask;
        p.rect(chip, 2.0, if fondo_negro { egui::Color32::from_gray(30) } else { egui::Color32::from_gray(235) }, egui::Stroke::new(if mascara_sel { 2.0 } else { 1.0 }, if mascara_sel { theme::ACCENT } else { theme::STROKE2 }), egui::StrokeKind::Inside);
        if !activa {
            p.line_segment([chip.left_top(), chip.right_bottom()], egui::Stroke::new(1.5, theme::RED));
        }
        let chip_r = ui.interact(chip, ui.scope_id().with(("mascara", id)), egui::Sense::click()).on_hover_text("Máscara (Alt+clic: verla sola)");
        if chip_r.clicked() {
            if ui.input(|i| i.modifiers.alt) {
                acciones.push(("viewport.view", json!({ "mode": "mask", "layer": id })));
            } else {
                acciones.push(("layer.select", json!({ "layer": id, "target": "mask" })));
            }
        }
    }
    if resp.clicked() {
        acciones.push(("layer.select", json!({ "layer": id })));
    }
    if resp.double_clicked() {
        m.renombrar = Some((id, nombre.clone()));
    }
    resp.dnd_set_drag_payload(CapaArrastre(id));
    if let Some(a) = resp.dnd_release_payload::<CapaArrastre>()
        && a.0 != id
    {
        let dentro = kind == "group";
        acciones.push(("layer.move", json!({ "layer": a.0, "above": id, "inside": dentro })));
    }
    if let Some(a) = resp.dnd_release_payload::<Arrastre>() {
        match &*a {
            Arrastre::Material(mat) => acciones.push(("material.apply", json!({ "material": mat, "above": id }))),
            Arrastre::Mascara(n) => acciones.push(("mask.smart.apply", json!({ "name": n, "layer": id }))),
        }
    }
    if resp.dnd_hover_payload::<CapaArrastre>().is_some() || resp.dnd_hover_payload::<Arrastre>().is_some() {
        ui.painter().hline(rect.x_range(), rect.top(), egui::Stroke::new(2.0, theme::ACCENT));
    }
    resp.context_menu(|ui| {
        if ui.button("Renombrar").clicked() {
            m.renombrar = Some((id, nombre.clone()));
            ui.close();
        }
        for (n, cmd) in [("Duplicar", "layer.duplicate"), ("Agrupar en carpeta", "layer.group"), ("Aplanar", "layer.flatten"), ("Subir", "up"), ("Bajar", "down")] {
            if ui.button(n).clicked() {
                let p = match cmd {
                    "up" => ("layer.move", json!({ "layer": id, "delta": 1 })),
                    "down" => ("layer.move", json!({ "layer": id, "delta": -1 })),
                    c => (c, json!({ "layer": id })),
                };
                acciones.push((p.0, p.1));
                ui.close();
            }
        }
        ui.separator();
        ui.menu_button("Máscara", |ui| menu_mascara(m, ui, Some(id), acciones));
        ui.separator();
        if ui.button(RichText::new("Eliminar").color(theme::RED)).clicked() {
            acciones.push(("layer.remove", json!({ "layer": id })));
            ui.close();
        }
    });
    if kind == "group" && !colapsada {
        for h in c["children"].as_array().into_iter().flatten() {
            fila(m, ui, h, nivel + 1, sel, acciones);
        }
    }
}
