//! Visor del modo Texturizado: vista 3D o UV con el renderer de TextureCraft, navegación,
//! pinceladas y cursor del pincel.

use super::{ModoTextura, gpu, tr};
use crate::theme;
use egui::{PointerButton, Pos2, Rect, Sense, Stroke, Ui, pos2, vec2};
use maker_textura::tc::core::{BakedMap, Channel};
use maker_textura::tc::geom::glam::{Vec2, Vec3};
use maker_textura::tc::{Tool, ViewMode};
use maker_textura::tc_render::{Frame, Shade, ViewKind, uv_view_scale};
use serde_json::json;

const VISTA_3D: u64 = 0x7e57_3d;
const VISTA_UV: u64 = 0x7e57_2d;

/// Herramientas que el modo ofrece en la barra del visor.
const HERRAMIENTAS: [(Tool, &str, &str); 7] = [
    (Tool::Paint, "Pincel", "B"),
    (Tool::Eraser, "Borrador", "E"),
    (Tool::Projection, "Proyección", ""),
    (Tool::PolygonFill, "Relleno", "P"),
    (Tool::Smudge, "Difuminar", ""),
    (Tool::Clone, "Clonar", "V + clic: origen"),
    (Tool::ColorPicker, "Cuentagotas", "I"),
];

pub fn herramientas(m: &mut ModoTextura, ui: &mut Ui) {
    ui.horizontal(|ui| {
        for (t, nombre, atajo) in HERRAMIENTAS {
            let r = ui.selectable_label(m.t.s.tool.tool == t, nombre);
            let r = if atajo.is_empty() { r } else { r.on_hover_text(atajo) };
            if r.clicked() {
                m.run("tool.select", json!({ "tool": t.id() }));
            }
        }
        ui.separator();
        let b = &m.t.s.tool.brush;
        let sim = [b.symmetry.x, b.symmetry.y, b.symmetry.z];
        ui.label(RichText::new("Simetría").color(theme::TEXT3));
        for (i, eje) in ["X", "Y", "Z"].into_iter().enumerate() {
            if ui.selectable_label(sim[i], eje).clicked() {
                let k = ["symmetryX", "symmetryY", "symmetryZ"][i];
                m.run("tool.brush", json!({ k: !sim[i] }));
            }
        }
    });
}
use egui::RichText;

/// Desplegable con lo que muestra el visor: material, un canal, un mapa horneado o la cuadrícula UV.
pub fn modo_de_vista(m: &mut ModoTextura, ui: &mut Ui) {
    let actual = match m.t.s.view {
        ViewMode::Material => "Material".to_string(),
        ViewMode::Channel(c) => tr(c.label()).to_string(),
        ViewMode::MeshMap(b) => format!("Mapa: {}", tr(b.label())),
        ViewMode::Mask(_) => "Máscara".into(),
        ViewMode::UvChecker => "Cuadrícula UV".into(),
    };
    let canales: Vec<Channel> = m.t.s.doc.project.texture_sets.get(m.t.s.ts()).map(|t| t.channels.iter().map(|c| c.0).collect()).unwrap_or_default();
    let mut elegido = None;
    egui::ComboBox::from_id_salt("textura-modo-vista").selected_text(actual).width(150.0).show_ui(ui, |ui| {
        if ui.selectable_label(m.t.s.view == ViewMode::Material, "Material").clicked() {
            elegido = Some("material".to_string());
        }
        ui.separator();
        for c in canales {
            if ui.selectable_label(m.t.s.view == ViewMode::Channel(c), tr(c.label())).clicked() {
                elegido = Some(c.id().to_string());
            }
        }
        ui.separator();
        for b in BakedMap::ALL {
            if ui.selectable_label(m.t.s.view == ViewMode::MeshMap(b), format!("Mapa: {}", tr(b.label()))).clicked() {
                elegido = Some(format!("map:{}", b.id()));
            }
        }
        ui.separator();
        if ui.selectable_label(m.t.s.view == ViewMode::UvChecker, "Cuadrícula UV").clicked() {
            elegido = Some("uvChecker".into());
        }
    });
    if let Some(v) = elegido {
        m.run("viewport.view", json!({ "mode": v }));
    }
    m.aux = match m.t.s.view {
        ViewMode::MeshMap(b) => Some(gpu::Aux::Mapa(b)),
        ViewMode::Mask(id) => Some(gpu::Aux::Mascara(id)),
        _ => None,
    };
}

fn shade(m: &ModoTextura) -> Shade {
    let (view_mode, channel) = match m.t.s.view {
        ViewMode::Material => (0, 0),
        ViewMode::Channel(c) => (1, c.index() as u32),
        ViewMode::MeshMap(_) | ViewMode::Mask(_) => (2, 0),
        ViewMode::UvChecker => (3, 0),
    };
    let ts = m.t.s.ts();
    let hs = m.t.s.doc.project.texture_sets.get(ts).map(|t| t.atlas_height_strength()).unwrap_or(1.0);
    Shade { height_strength: hs, view_mode, channel, wireframe: m.vista_uv, ..Shade::from_project(&m.t.s.doc.project) }
}

fn presion(ui: &Ui) -> f32 {
    ui.input(|i| {
        i.raw.events.iter().rev().find_map(|e| match e {
            egui::Event::Touch { force: Some(f), .. } if f.is_finite() => Some(f.clamp(0.0, 1.0)),
            _ => None,
        })
    })
    .unwrap_or(1.0)
}

pub fn mostrar(m: &mut ModoTextura, ui: &mut Ui) {
    let rect = ui.available_rect_before_wrap();
    ui.allocate_rect(rect, Sense::hover());
    let resp = ui.interact(rect, ui.scope_id().with("textura-visor"), Sense::click_and_drag());
    let size = rect.size();
    m.t.s.viewport = Vec2::new(size.x.max(16.0), size.y.max(16.0));
    if m.vista_uv { vista_uv(m, ui, rect, &resp) } else { vista_3d(m, ui, rect, &resp) }
    // Soltar un material del estante sobre el visor.
    if let Some(id) = resp.dnd_release_payload::<super::estante::Arrastre>() {
        super::estante::soltar(m, &id);
    }
    let kind = if m.vista_uv { ViewKind::Uv { ts: m.t.s.ts(), zoom: m.uv_zoom, pan: m.uv_pan } } else { ViewKind::Scene };
    let ppp = ui.ctx().pixels_per_point();
    let frame = Frame {
        view_id: if m.vista_uv { VISTA_UV } else { VISTA_3D },
        size: [(size.x * ppp).max(1.0) as u32, (size.y * ppp).max(1.0) as u32],
        kind,
        camera: m.t.s.camera,
        shade: shade(m),
        visible: m.t.s.doc.project.texture_sets.iter().map(|t| t.visible).collect(),
    };
    let aux = m.aux;
    if let Some(enlace) = &mut m.enlace {
        let sync = enlace.sync(&mut m.t.s, aux);
        let cb = enlace.callback(frame, Some(sync), rect);
        ui.painter().add(egui_wgpu::Callback::new_paint_callback(rect, cb));
    }
    if m.t.s.pending_tiles() > 0 {
        ui.painter().text(rect.left_bottom() + vec2(10.0, -10.0), egui::Align2::LEFT_BOTTOM, "Componiendo…", egui::FontId::proportional(12.0), theme::TEXT3);
    }
}

fn vista_3d(m: &mut ModoTextura, ui: &mut Ui, rect: Rect, resp: &egui::Response) {
    let size = rect.size();
    let local = |p: Pos2| Vec2::new(p.x - rect.left(), p.y - rect.top());
    let (alt, shift, ctrl) = ui.input(|i| (i.modifiers.alt, i.modifiers.shift, i.modifiers.command));
    let delta = ui.input(|i| i.pointer.delta());
    let cam = &mut m.t.s.camera;
    // Navegación: botón derecho o Alt + izquierdo orbitan, central desplaza, rueda acerca.
    if resp.dragged_by(PointerButton::Middle) || (shift && resp.dragged_by(PointerButton::Secondary)) {
        cam.pan(delta.x / size.y, delta.y / size.y);
    } else if resp.dragged_by(PointerButton::Secondary) || (alt && resp.dragged_by(PointerButton::Primary)) {
        cam.orbit(delta.x * 0.008, delta.y * 0.008);
    }
    if resp.hovered() {
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        if scroll != 0.0 {
            if ctrl {
                let b = &mut m.t.s.tool.brush;
                b.size = (b.size * (1.0 + scroll * 0.002)).clamp(0.0005, 0.25);
            } else {
                m.t.s.camera.zoom(scroll * 0.0025);
            }
        }
    }
    let v_held = ui.input(|i| i.key_down(egui::Key::V)) && !ui.ctx().egui_wants_keyboard_input();
    if v_held
        && m.t.s.tool.tool == Tool::Clone
        && resp.clicked_by(PointerButton::Primary)
        && let Some(p) = resp.interact_pointer_pos()
    {
        let (q, vp) = (local(p), m.t.s.viewport);
        m.run("tool.cloneSource", json!({ "x": q.x, "y": q.y, "viewport": [vp.x, vp.y] }));
        return;
    }
    let pintar = !alt && !v_held;
    let tool = m.t.s.tool.tool;
    if pintar && matches!(tool, Tool::PolygonFill | Tool::ColorPicker | Tool::GeometryMask) {
        if resp.clicked_by(PointerButton::Primary)
            && let Some(p) = resp.interact_pointer_pos()
        {
            let (q, vp) = (local(p), m.t.s.viewport);
            m.run("tool.click", json!({ "x": q.x, "y": q.y, "viewport": [vp.x, vp.y] }));
        }
    } else if pintar {
        let pr = presion(ui);
        if resp.drag_started_by(PointerButton::Primary) || resp.clicked_by(PointerButton::Primary) {
            // Mayús + clic: línea recta desde el final de la pincelada anterior.
            let desde = if shift { m.ultimo_punto } else { None };
            if let Some(p) = resp.interact_pointer_pos() {
                let vp = m.t.s.viewport;
                match m.t.s.begin_stroke(desde.unwrap_or(local(p)), vp, pr) {
                    Ok(_) => {
                        if desde.is_some() {
                            m.t.s.continue_stroke(local(p), pr);
                        }
                        m.ultimo_punto = Some(local(p));
                    }
                    Err(e) => maker_engine::log::error("texturizado", e.to_string()),
                }
            }
            if resp.clicked_by(PointerButton::Primary)
                && let Err(e) = m.t.s.end_stroke()
            {
                maker_engine::log::error("texturizado", e.to_string());
            }
        } else if resp.dragged_by(PointerButton::Primary)
            && m.t.s.stroke_active()
            && let Some(p) = resp.interact_pointer_pos()
        {
            m.t.s.continue_stroke(local(p), pr);
            m.ultimo_punto = Some(local(p));
        }
        if resp.drag_stopped_by(PointerButton::Primary)
            && let Err(e) = m.t.s.end_stroke()
        {
            maker_engine::log::error("texturizado", e.to_string());
        }
    }
    // Cursor del pincel: un círculo sobre la superficie.
    let navegando = alt || ui.input(|i| i.pointer.middle_down() || i.pointer.secondary_down());
    if !navegando
        && resp.hovered()
        && matches!(tool, Tool::Paint | Tool::Eraser | Tool::Projection | Tool::Smudge | Tool::Clone)
        && let Some(hp) = resp.hover_pos()
        && let Some(hit) = m.t.s.pick(local(hp), m.t.s.viewport)
    {
        ui.ctx().set_cursor_icon(egui::CursorIcon::None);
        cursor(m, ui, rect, hit.pos, hit.nrm);
    }
}

fn proyectar(m: &ModoTextura, rect: Rect, p: Vec3) -> Option<Pos2> {
    let n = m.t.s.camera.project(p, rect.width() / rect.height().max(1.0))?;
    Some(pos2(rect.left() + (n.x * 0.5 + 0.5) * rect.width(), rect.top() + (0.5 - n.y * 0.5) * rect.height()))
}

fn cursor(m: &ModoTextura, ui: &Ui, rect: Rect, pos: Vec3, nrm: Vec3) {
    let (lo, hi) = m.t.s.mesh.bounds();
    let radio = m.t.s.tool.brush.size * (hi - lo).length();
    let t = nrm.any_orthonormal_vector();
    let b = nrm.cross(t);
    let puntos: Vec<Pos2> = (0..=40)
        .filter_map(|i| {
            let a = i as f32 / 40.0 * std::f32::consts::TAU;
            proyectar(m, rect, pos + (t * a.cos() + b * a.sin()) * radio)
        })
        .collect();
    let p = ui.painter_at(rect);
    p.add(egui::Shape::line(puntos.clone(), Stroke::new(2.5, egui::Color32::from_black_alpha(140))));
    p.add(egui::Shape::line(puntos, Stroke::new(1.2, egui::Color32::WHITE)));
}

fn vista_uv(m: &mut ModoTextura, ui: &mut Ui, rect: Rect, resp: &egui::Response) {
    let size = rect.size();
    let delta = ui.input(|i| i.pointer.delta());
    if resp.dragged_by(PointerButton::Middle) || resp.dragged_by(PointerButton::Secondary) {
        m.uv_pan[0] += delta.x / size.x * 2.0;
        m.uv_pan[1] -= delta.y / size.y * 2.0;
    }
    if resp.hovered() {
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        if scroll != 0.0 {
            m.uv_zoom = (m.uv_zoom * (1.0 + scroll * 0.002)).clamp(0.1, 32.0);
        }
    }
    let set = m.t.s.doc.project.texture_sets.get(m.t.s.ts()).map(|t| t.size()).unwrap_or((1, 1));
    let [sx, sy] = uv_view_scale(size.x / size.y.max(1.0), m.uv_zoom, set);
    let pan = m.uv_pan;
    let a_uv = |p: Pos2| {
        let xc = (p.x - rect.left()) / size.x * 2.0 - 1.0;
        let yc = 1.0 - (p.y - rect.top()) / size.y * 2.0;
        Vec2::new(((xc - pan[0]) / sx + 1.0) * 0.5, (1.0 - (yc - pan[1]) / sy) * 0.5)
    };
    if !matches!(m.t.s.tool.tool, Tool::Paint | Tool::Eraser | Tool::Smudge | Tool::Clone) {
        return;
    }
    if (resp.drag_started_by(PointerButton::Primary) || resp.clicked_by(PointerButton::Primary))
        && let Some(p) = resp.interact_pointer_pos()
        && let Err(e) = m.t.s.begin_stroke_uv(a_uv(p), 1.0)
    {
        maker_engine::log::error("texturizado", e.to_string());
    }
    if resp.clicked_by(PointerButton::Primary)
        && let Err(e) = m.t.s.end_stroke()
    {
        maker_engine::log::error("texturizado", e.to_string());
    }
    if resp.dragged_by(PointerButton::Primary)
        && m.t.s.stroke_active()
        && let Some(p) = resp.interact_pointer_pos()
    {
        m.t.s.continue_stroke_uv(a_uv(p), 1.0);
    }
    if resp.drag_stopped_by(PointerButton::Primary)
        && let Err(e) = m.t.s.end_stroke()
    {
        maker_engine::log::error("texturizado", e.to_string());
    }
}
