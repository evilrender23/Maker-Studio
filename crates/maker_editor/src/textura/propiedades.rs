//! Propiedades: la herramienta (pincel y material que pinta) y la capa seleccionada (fusión por
//! canal, fuentes de relleno, proyección y efectos de contenido y máscara).

use super::{ModoTextura, tr};
use crate::theme;
use egui::{Id, Response, RichText, Ui};
use maker_textura::tc::core::{BlendMode, Channel, GeneratorKind, ProceduralKind};
use maker_textura::tc::{StackTarget, Tool};
use serde_json::{Value, json};

/// Un control que edita un valor del documento: mientras el ratón está pulsado el valor vive
/// en la interfaz y al soltar se confirma con un solo paso de deshacer.
fn diferido<T: Clone + Send + Sync + PartialEq + 'static>(ui: &mut Ui, clave: Id, actual: T, w: impl FnOnce(&mut Ui, &mut T) -> Response) -> Option<T> {
    let previo = ui.data(|d| d.get_temp::<T>(clave));
    let mut v = previo.clone().unwrap_or_else(|| actual.clone());
    let r = w(ui, &mut v);
    if !r.changed() && previo.is_none() {
        return None;
    }
    if ui.input(|i| i.pointer.any_down()) {
        ui.data_mut(|d| d.insert_temp(clave, v));
        None
    } else {
        ui.data_mut(|d| d.remove::<T>(clave));
        (v != actual).then_some(v)
    }
}

fn num(v: &Value) -> f32 {
    v.as_f64().unwrap_or(0.0) as f32
}

fn camel(s: &str) -> String {
    let mut out = String::new();
    let mut up = false;
    for c in s.chars() {
        if c == '_' {
            up = true;
        } else if up {
            out.extend(c.to_uppercase());
            up = false;
        } else {
            out.push(c);
        }
    }
    out
}

pub fn mostrar(m: &mut ModoTextura, ui: &mut Ui) {
    herramienta(m, ui);
    ui.add_space(6.0);
    capa(m, ui);
}

fn herramienta(m: &mut ModoTextura, ui: &mut Ui) {
    let t = m.t.s.tool.tool;
    theme::section_label(ui, tr(t.label()));
    if !matches!(t, Tool::Paint | Tool::Eraser | Tool::Projection | Tool::Smudge | Tool::Clone | Tool::PolygonFill) {
        return;
    }
    let canales: Vec<Channel> = m.t.s.doc.project.texture_sets.get(m.t.s.ts()).map(|s| s.channels.iter().map(|c| c.0).collect()).unwrap_or_default();
    let tool = &mut m.t.s.tool;
    let b = &mut tool.brush;
    egui::Grid::new("textura-pincel").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
        if t != Tool::PolygonFill {
            ui.label("Tamaño");
            let mut v = b.size / 0.25 * 100.0;
            if ui.add(egui::Slider::new(&mut v, 0.2..=100.0).logarithmic(true).max_decimals(1)).on_hover_text("[ ]  ·  Ctrl + rueda").changed() {
                b.size = v / 100.0 * 0.25;
            }
            ui.end_row();
            ui.label("Flujo");
            ui.add(egui::Slider::new(&mut b.flow, 0.0..=1.0));
            ui.end_row();
            ui.label("Opacidad");
            ui.add(egui::Slider::new(&mut b.opacity, 0.0..=1.0));
            ui.end_row();
            ui.label("Dureza");
            ui.add(egui::Slider::new(&mut b.hardness, 0.0..=1.0));
            ui.end_row();
            ui.label("Espaciado");
            ui.add(egui::Slider::new(&mut b.spacing, 0.01..=4.0).logarithmic(true));
            ui.end_row();
            ui.label("Ángulo");
            ui.add(egui::Slider::new(&mut b.angle, -180.0..=180.0).suffix("°"));
            ui.end_row();
            ui.label("Seguir trazo");
            ui.checkbox(&mut b.follow_path, "");
            ui.end_row();
            ui.label("Solo caras visibles");
            ui.checkbox(&mut b.backface_culling, "");
            ui.end_row();
            ui.label("Ratón perezoso");
            ui.add(egui::Slider::new(&mut b.lazy, 0.0..=60.0).suffix(" px"));
            ui.end_row();
        }
    });
    if m.t.s.target != StackTarget::Content {
        ui.horizontal(|ui| {
            ui.label("Valor en la máscara");
            ui.add(egui::Slider::new(&mut m.t.s.tool.mask_value, 0.0..=1.0));
        });
        return;
    }
    if t == Tool::Eraser || t == Tool::Smudge || t == Tool::Clone {
        return;
    }
    ui.add_space(4.0);
    let tool = &mut m.t.s.tool;
    egui::Grid::new("textura-material-pincel").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
        for c in canales {
            let mut on = tool.enabled.contains(&c);
            if ui.checkbox(&mut on, tr(c.label())).changed() {
                if on {
                    tool.enabled.insert(c);
                } else {
                    tool.enabled.remove(&c);
                }
            }
            let v = tool.values.entry(c).or_insert([0.5; 3]);
            ui.add_enabled_ui(on, |ui| {
                if matches!(c, Channel::BaseColor | Channel::Emissive | Channel::SheenColor | Channel::Specular) {
                    egui::color_picker::color_edit_button_rgb(ui, v);
                } else if c == Channel::Normal {
                    ui.label(RichText::new("—").color(theme::TEXT4));
                } else {
                    let mut x = v[0];
                    if ui.add(egui::Slider::new(&mut x, 0.0..=1.0)).changed() {
                        *v = [x; 3];
                    }
                }
            });
            ui.end_row();
        }
    });
}

fn capa(m: &mut ModoTextura, ui: &mut Ui) {
    let Some(sel) = m.t.s.selected_layer().map(|l| l.0) else { return };
    let capas = m.capas().clone();
    let Some(c) = buscar(&capas, sel) else { return };
    let mut acciones: Vec<(&'static str, Value)> = Vec::new();
    let nombre = c["name"].as_str().unwrap_or("");
    theme::section_label(ui, &format!("Capa · {}", tr(nombre)));
    let kind = c["kind"].as_str().unwrap_or("");
    // Fusión por canal.
    egui::CollapsingHeader::new("Fusión").id_salt(("fusion", sel)).default_open(false).show(ui, |ui| {
        egui::Grid::new(("textura-fusion", sel)).num_columns(3).spacing([6.0, 3.0]).show(ui, |ui| {
            for (canal, b) in c["channels"].as_object().into_iter().flatten() {
                let ch = Channel::parse(canal);
                ui.label(ch.map(|c| tr(c.label())).unwrap_or(canal));
                let modo = b["mode"].as_str().unwrap_or("Normal").to_string();
                let mut nuevo = modo.clone();
                egui::ComboBox::from_id_salt(("modo", sel, canal)).selected_text(tr(&modo)).width(110.0).show_ui(ui, |ui| {
                    for bm in BLENDS {
                        let l = bm.label();
                        ui.selectable_value(&mut nuevo, l.to_string(), tr(l));
                    }
                });
                if nuevo != modo {
                    acciones.push(("layer.blend", json!({ "layer": sel, "channel": canal, "mode": nuevo })));
                }
                if let Some(o) = diferido(ui, Id::unique(("opac", sel, canal)), num(&b["opacity"]), |ui, v| ui.add(egui::DragValue::new(v).range(0.0..=1.0).speed(0.01))) {
                    acciones.push(("layer.blend", json!({ "layer": sel, "channel": canal, "opacity": o })));
                }
                ui.end_row();
            }
        });
    });
    if kind == "fill" {
        relleno(ui, sel, &c["fill"], &mut acciones);
    }
    if let Some(e) = c["contentEffects"].as_array().filter(|e| !e.is_empty()) {
        efectos(ui, "Efectos del contenido", sel, e, &mut acciones);
    }
    if let Some(e) = c["mask"]["effects"].as_array() {
        efectos(ui, "Máscara", sel, e, &mut acciones);
    }
    let destino = if m.t.s.target == StackTarget::Content { "content" } else { "mask" };
    ui.menu_button(if destino == "mask" { "+ Efecto en la máscara" } else { "+ Efecto en el contenido" }, |ui| {
        if destino == "mask" {
            ui.menu_button("Generador", |ui| {
                for g in GeneratorKind::ALL {
                    if ui.button(tr(g.label())).clicked() {
                        acciones.push(("effect.add.generator", json!({ "layer": sel, "generator": generador_id(g) })));
                        ui.close();
                    }
                }
            });
        }
        ui.menu_button("Filtro", |ui| {
            for (id, l, mascara) in filtros(m) {
                if (destino == "content" || mascara) && ui.button(tr(&l)).clicked() {
                    acciones.push(("effect.add.filter", json!({ "layer": sel, "target": destino, "filter": id })));
                    ui.close();
                }
            }
        });
        for (n, id) in [("Niveles", "effect.add.levels"), ("Relleno", "effect.add.fill"), ("Pintura", "effect.add.paint")] {
            if ui.button(n).clicked() {
                acciones.push((id, json!({ "layer": sel, "target": destino })));
                ui.close();
            }
        }
    });
    for (id, p) in acciones {
        m.run(id, p);
    }
}

const BLENDS: [BlendMode; 14] = [
    BlendMode::Normal,
    BlendMode::Passthrough,
    BlendMode::Replace,
    BlendMode::Multiply,
    BlendMode::Screen,
    BlendMode::Overlay,
    BlendMode::SoftLight,
    BlendMode::Darken,
    BlendMode::Lighten,
    BlendMode::LinearDodge,
    BlendMode::Subtract,
    BlendMode::Difference,
    BlendMode::Color,
    BlendMode::NormalMapCombine,
];

fn generador_id(g: GeneratorKind) -> &'static str {
    match g {
        GeneratorKind::EdgeWear => "edgeWear",
        GeneratorKind::Dirt => "dirt",
        GeneratorKind::ThinAreas => "thinAreas",
        GeneratorKind::Curvature => "curvature",
        GeneratorKind::AmbientOcclusion => "ambientOcclusion",
        GeneratorKind::Position => "position",
        GeneratorKind::Direction => "direction",
    }
}

fn filtros(m: &mut ModoTextura) -> Vec<(String, String, bool)> {
    let l = maker_textura::tc::execute(&mut m.t.s, "filter.list", &json!({})).unwrap_or_default();
    l.as_array().into_iter().flatten().map(|f| (f["id"].as_str().unwrap_or("").to_string(), f["label"].as_str().unwrap_or("").to_string(), f["mask"].as_bool().unwrap_or(false))).collect()
}

fn buscar(capas: &Value, id: u64) -> Option<Value> {
    for c in capas.as_array().into_iter().flatten() {
        if c["id"].as_u64() == Some(id) {
            return Some(c.clone());
        }
        if let Some(h) = buscar(&c["children"], id) {
            return Some(h);
        }
    }
    None
}

fn relleno(ui: &mut Ui, sel: u64, fill: &Value, acciones: &mut Vec<(&'static str, Value)>) {
    egui::CollapsingHeader::new("Relleno").id_salt(("relleno", sel)).default_open(true).show(ui, |ui| {
        let proy = fill["projection"].as_str().unwrap_or("UV projection").to_string();
        ui.horizontal(|ui| {
            ui.label("Proyección");
            egui::ComboBox::from_id_salt(("proy", sel)).selected_text(tr(&proy)).show_ui(ui, |ui| {
                for (id, l) in [("uv", "UV projection"), ("triplanar", "Tri-planar projection"), ("planar", "Planar projection"), ("spherical", "Spherical projection"), ("cylindrical", "Cylindrical projection")] {
                    if ui.selectable_label(proy == l, tr(l)).clicked() && proy != l {
                        acciones.push(("layer.fill.projection", json!({ "layer": sel, "projection": id })));
                    }
                }
            });
        });
        egui::Grid::new(("fuentes", sel)).num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
            for (canal, src) in fill["sources"].as_object().into_iter().flatten() {
                let ch = Channel::parse(canal);
                ui.label(ch.map(|c| tr(c.label())).unwrap_or(canal));
                ui.horizontal(|ui| {
                    let proc_actual = src["procedural"].as_str().map(str::to_string);
                    let texto = proc_actual.as_deref().map(tr).unwrap_or(if src["image"].is_string() { "Imagen" } else { "Uniforme" }).to_string();
                    egui::ComboBox::from_id_salt(("fuente", sel, canal)).selected_text(texto).width(96.0).show_ui(ui, |ui| {
                        if ui.selectable_label(src["uniform"].is_array(), "Uniforme").clicked() {
                            acciones.push(("layer.fill.set", json!({ "layer": sel, "channel": canal, "value": 0.5 })));
                        }
                        for k in ProceduralKind::ALL {
                            if ui.selectable_label(proc_actual.as_deref() == Some(k.label()), tr(k.label())).clicked() {
                                acciones.push(("layer.fill.set", json!({ "layer": sel, "channel": canal, "procedural": k.label() })));
                            }
                        }
                    });
                    if let Some(u) = src["uniform"].as_array() {
                        let rgb = [num(&u[0]), num(u.get(1).unwrap_or(&u[0])), num(u.get(2).unwrap_or(&u[0]))];
                        if matches!(ch, Some(Channel::BaseColor | Channel::Emissive | Channel::SheenColor | Channel::Specular)) {
                            if let Some(c) = diferido(ui, Id::unique(("color", sel, canal)), rgb, |ui, v| egui::color_picker::color_edit_button_rgb(ui, v)) {
                                acciones.push(("layer.fill.set", json!({ "layer": sel, "channel": canal, "color": c })));
                            }
                        } else if let Some(x) = diferido(ui, Id::unique(("valor", sel, canal)), rgb[0], |ui, v| ui.add(egui::Slider::new(v, 0.0..=1.0))) {
                            acciones.push(("layer.fill.set", json!({ "layer": sel, "channel": canal, "value": x })));
                        }
                    } else if proc_actual.is_some() {
                        if let Some(x) = diferido(ui, Id::unique(("escala", sel, canal)), num(&src["scale"]), |ui, v| ui.add(egui::DragValue::new(v).range(0.05..=64.0).speed(0.05).prefix("× "))) {
                            acciones.push(("layer.fill.set", json!({ "layer": sel, "channel": canal, "procedural": proc_actual, "scale": x })));
                        }
                        if let Some(x) = diferido(ui, Id::unique(("semilla", sel, canal)), src["seed"].as_u64().unwrap_or(0) as i64, |ui, v| ui.add(egui::DragValue::new(v).range(0..=9999).prefix("#"))) {
                            acciones.push(("layer.fill.set", json!({ "layer": sel, "channel": canal, "procedural": proc_actual, "seed": x })));
                        }
                    }
                });
                ui.end_row();
            }
        });
    });
}

const META: [&str; 8] = ["id", "name", "kind", "blend", "opacity", "visible", "type", "projection"];

/// Parámetros numéricos y booleanos de un efecto, editables con `effect.set`.
fn params(ui: &mut Ui, eid: u64, v: &Value, acciones: &mut Vec<(&'static str, Value)>) {
    let Some(o) = v.as_object() else { return };
    for (k, x) in o {
        if META.contains(&k.as_str()) {
            continue;
        }
        let clave = camel(k);
        match x {
            Value::Number(n) => {
                ui.label(tr(&k.replace('_', " ")));
                if let Some(nv) = diferido(ui, Id::unique(("efecto", eid, k)), n.as_f64().unwrap_or(0.0) as f32, |ui, v| ui.add(egui::DragValue::new(v).speed(0.01))) {
                    acciones.push(("effect.set", json!({ "effect": eid, clave: nv })));
                }
                ui.end_row();
            }
            Value::Bool(b) => {
                ui.label(tr(&k.replace('_', " ")));
                let mut bb = *b;
                if ui.checkbox(&mut bb, "").changed() {
                    acciones.push(("effect.set", json!({ "effect": eid, clave: bb })));
                }
                ui.end_row();
            }
            Value::Object(_) => params(ui, eid, x, acciones),
            _ => {}
        }
    }
}

fn efectos(ui: &mut Ui, titulo: &str, sel: u64, lista: &[Value], acciones: &mut Vec<(&'static str, Value)>) {
    egui::CollapsingHeader::new(titulo).id_salt((titulo, sel)).default_open(true).show(ui, |ui| {
        for e in lista.iter().rev() {
            let eid = e["id"].as_u64().unwrap_or(0);
            let tipo = e["generator"]["type"].as_str().or(e["filter"].as_str()).or(e["kind"].as_str()).unwrap_or("");
            ui.horizontal(|ui| {
                let mut vis = e["visible"].as_bool().unwrap_or(true);
                if ui.checkbox(&mut vis, "").changed() {
                    acciones.push(("effect.set", json!({ "effect": eid, "visible": vis })));
                }
                ui.label(RichText::new(tr(tipo)).strong());
                if let Some(o) = diferido(ui, Id::unique(("efecto-op", eid)), num(&e["opacity"]), |ui, v| ui.add(egui::DragValue::new(v).range(0.0..=1.0).speed(0.01))) {
                    acciones.push(("effect.set", json!({ "effect": eid, "opacity": o })));
                }
                if ui.small_button("✕").on_hover_text("Quitar").clicked() {
                    acciones.push(("effect.remove", json!({ "effect": eid })));
                }
            });
            ui.indent(("efecto", eid), |ui| {
                egui::Grid::new(("efecto-grid", eid)).num_columns(2).spacing([8.0, 3.0]).show(ui, |ui| {
                    params(ui, eid, e, acciones);
                });
            });
        }
    });
}
