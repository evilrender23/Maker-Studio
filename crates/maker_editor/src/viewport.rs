//! Viewport 3D: muestra la textura del renderizador, maneja la cámara del
//! editor, la selección por rayo, los gizmos y la entrada del juego en Play.

use crate::editor::Editor;
use crate::gizmo::{GizmoMode, View};
use crate::theme;
use egui::{Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, Vec2 as EVec2};
use glam::{Mat4, Vec2, Vec3};
use maker_engine::Key;
use maker_engine::components::*;
use maker_render::{RenderOptions, RenderView};

/// Asset arrastrado desde el explorador de contenido.
#[derive(Clone, Debug)]
pub struct AssetDrag(pub String);

pub fn map_key(k: egui::Key) -> Option<Key> {
    use egui::Key as E;
    Some(match k {
        E::A => Key::A, E::B => Key::B, E::C => Key::C, E::D => Key::D, E::E => Key::E, E::F => Key::F, E::G => Key::G, E::H => Key::H,
        E::I => Key::I, E::J => Key::J, E::K => Key::K, E::L => Key::L, E::M => Key::M, E::N => Key::N, E::O => Key::O, E::P => Key::P,
        E::Q => Key::Q, E::R => Key::R, E::S => Key::S, E::T => Key::T, E::U => Key::U, E::V => Key::V, E::W => Key::W, E::X => Key::X,
        E::Y => Key::Y, E::Z => Key::Z,
        E::Num0 => Key::Num0, E::Num1 => Key::Num1, E::Num2 => Key::Num2, E::Num3 => Key::Num3, E::Num4 => Key::Num4,
        E::Num5 => Key::Num5, E::Num6 => Key::Num6, E::Num7 => Key::Num7, E::Num8 => Key::Num8, E::Num9 => Key::Num9,
        E::Space => Key::Space, E::Enter => Key::Enter, E::Escape => Key::Escape, E::Tab => Key::Tab, E::Backspace => Key::Backspace,
        E::ArrowUp => Key::Up, E::ArrowDown => Key::Down, E::ArrowLeft => Key::Left, E::ArrowRight => Key::Right,
        E::F1 => Key::F1, E::F2 => Key::F2, E::F3 => Key::F3, E::F4 => Key::F4, E::F5 => Key::F5, E::F6 => Key::F6,
        E::F7 => Key::F7, E::F8 => Key::F8, E::F9 => Key::F9, E::F10 => Key::F10, E::F11 => Key::F11, E::F12 => Key::F12,
        _ => return None,
    })
}

fn toolbar(ed: &mut Editor, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for (mode, label, key) in [(GizmoMode::Translate, "Mover", "W"), (GizmoMode::Rotate, "Rotar", "E"), (GizmoMode::Scale, "Escalar", "R")] {
            if ui.selectable_label(ed.gizmo.mode == mode, label).on_hover_text(format!("Herramienta {label} ({key})")).clicked() {
                ed.gizmo.mode = mode;
            }
        }
        ui.separator();
        let space = if ed.gizmo.local { "Local" } else { "Global" };
        if ui.button(space).on_hover_text("Ejes del gizmo: locales o globales").clicked() {
            ed.gizmo.local = !ed.gizmo.local;
        }
        ui.toggle_value(&mut ed.gizmo.snap, "Ajuste").on_hover_text("Ajustar a 0,5 m · 15° · 0,1 (o mantén Ctrl)");
        ui.separator();
        ui.toggle_value(&mut ed.show_grid, "Rejilla");
        ui.toggle_value(&mut ed.shadows, "Sombras");
        if ed.is_playing() {
            ui.separator();
            ui.toggle_value(&mut ed.show_game_view, "Cámara del juego").on_hover_text("Ver desde la cámara principal o desde la del editor");
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let s = ed.renderer.stats;
            let fps = if ed.frame_times.is_empty() { 0.0 } else { ed.frame_times.len() as f32 / ed.frame_times.iter().sum::<f32>().max(1e-4) };
            // La versión más larga que quepa sin pisar los botones de la izquierda.
            let variantes = [
                format!("{fps:.0} fps · {} draw calls · {} k tris · {} entidades", s.draw_calls, s.triangles / 1000, ed.scene.len()),
                format!("{fps:.0} fps · {} draw calls · {} k tris", s.draw_calls, s.triangles / 1000),
                format!("{fps:.0} fps"),
            ];
            let fuente = egui::TextStyle::Small.resolve(ui.style());
            let hueco = ui.available_width() - 8.0;
            let ancho = |t: &str| ui.fonts_mut(|f| f.layout_no_wrap(t.to_owned(), fuente.clone(), theme::TEXT3).size().x);
            if let Some(t) = variantes.iter().find(|t| ancho(t) <= hueco) {
                ui.label(egui::RichText::new(t).small().color(theme::TEXT3));
            }
        });
    });
}

pub fn show(ed: &mut Editor, ui: &mut egui::Ui, rs: &egui_wgpu::RenderState) {
    toolbar(ed, ui);
    let rect = ui.available_rect_before_wrap();
    ed.viewport_rect = rect;
    let resp = ui.interact(rect, ui.scope_id().with("viewport"), Sense::click_and_drag());
    let ppp = ui.ctx().pixels_per_point();
    let (pw, ph) = ((rect.width() * ppp).round().max(1.0) as u32, (rect.height() * ppp).round().max(1.0) as u32);
    ed.renderer.resize(pw, ph);
    let aspect = rect.width() / rect.height().max(1.0);

    // ¿Qué cámara?
    let game_cam = if ed.is_playing() && ed.show_game_view { ed.scene.primary_camera() } else { None };
    let (rview, fov) = match game_cam {
        Some((id, cam)) => (RenderView::from_camera(ed.scene.global(id), &cam, aspect), cam.fov_y),
        None => (ed.cam.render_view(aspect), ed.cam.fov),
    };
    let editing_view = game_cam.is_none();

    let mut opts = RenderOptions { show_grid: ed.show_grid && editing_view, shadows: ed.shadows, ..Default::default() };
    if editing_view && let Some(s) = ed.selection {
        let h = ed.scene.hierarchy();
        let mut stack = vec![s];
        while let Some(c) = stack.pop() {
            opts.selected.insert(c);
            stack.extend_from_slice(h.children_of(c));
        }
    }
    ed.renderer.render(&ed.scene, &rview, &opts);

    // Textura → egui.
    if let Some(view) = ed.renderer.output_view() {
        let mut r = rs.renderer.write();
        match ed.tex_id {
            None => ed.tex_id = Some(r.register_native_texture(&rs.device, view, wgpu::FilterMode::Linear)),
            Some(id) if ed.tex_gen != ed.renderer.target_generation => r.update_egui_texture_from_wgpu_texture(&rs.device, view, wgpu::FilterMode::Linear, id),
            _ => {}
        }
        ed.tex_gen = ed.renderer.target_generation;
    }
    let painter = ui.painter_at(rect);
    if let Some(id) = ed.tex_id {
        painter.image(id, rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
    }

    let view = View {
        view_proj: rview.view_proj(),
        eye: rview.eye,
        size: Vec2::new(rect.width(), rect.height()),
        origin: rect.min,
        world_per_px_at_1: 2.0 * (fov.to_radians() * 0.5).tan() / rect.height().max(1.0),
    };

    let ctx = ui.ctx().clone();
    let playing_game = ed.is_playing() && ed.game_focus;

    // ── Entrada del juego ──
    if ed.is_playing() {
        if resp.clicked() || resp.drag_started() {
            ed.game_focus = true;
        } else if ctx.input(|i| i.pointer.any_pressed()) && !resp.hovered() {
            ed.game_focus = false;
            ed.input.release_all();
        }
        if ed.game_focus {
            ctx.input(|i| {
                for e in &i.events {
                    match e {
                        egui::Event::Key { key, pressed, repeat: false, .. } => {
                            if let Some(k) = map_key(*key) {
                                ed.input.set_key(k, *pressed);
                            }
                        }
                        egui::Event::MouseMoved(d) => ed.input.mouse_delta += Vec2::new(d.x, d.y),
                        _ => {}
                    }
                }
                ed.input.set_key(Key::Shift, i.modifiers.shift);
                ed.input.set_key(Key::Ctrl, i.modifiers.ctrl);
                ed.input.set_key(Key::Alt, i.modifiers.alt);
                ed.input.mouse_buttons = [
                    i.pointer.button_down(egui::PointerButton::Primary),
                    i.pointer.button_down(egui::PointerButton::Secondary),
                    i.pointer.button_down(egui::PointerButton::Middle),
                ];
                if let Some(p) = i.pointer.hover_pos() {
                    ed.input.mouse_pos = Vec2::new(p.x - rect.min.x, p.y - rect.min.y);
                }
            });
        }
    }

    // ── Controles de cámara y edición ──
    if editing_view {
        let (scroll, rmb, mmb, alt, dt, fast) = ctx.input(|i| {
            (
                i.smooth_scroll_delta.y,
                i.pointer.button_down(egui::PointerButton::Secondary),
                i.pointer.button_down(egui::PointerButton::Middle),
                i.modifiers.alt,
                i.stable_dt.min(0.1),
                i.modifiers.shift,
            )
        });
        let delta = resp.drag_delta();
        let flying = rmb && (resp.dragged_by(egui::PointerButton::Secondary) || resp.hovered());
        if resp.hovered() && scroll != 0.0 {
            if rmb {
                ed.cam.fly_speed = (ed.cam.fly_speed * (scroll / 40.0 * 0.15).exp()).clamp(0.2, 200.0);
            } else {
                ed.cam.dolly(scroll / 40.0);
            }
        }
        if resp.dragged_by(egui::PointerButton::Secondary) {
            ed.cam.look(delta.x, delta.y);
        }
        if resp.dragged_by(egui::PointerButton::Middle) || (mmb && resp.dragged()) {
            ed.cam.pan(delta.x, delta.y, rect.height());
        }
        if alt && resp.dragged_by(egui::PointerButton::Primary) {
            ed.cam.orbit(delta.x, delta.y);
        }
        if flying && !playing_game {
            let mv = ctx.input(|i| {
                let k = |key| if i.key_down(key) { 1.0 } else { 0.0 };
                Vec3::new(k(egui::Key::D) - k(egui::Key::A), k(egui::Key::E) - k(egui::Key::Q), k(egui::Key::W) - k(egui::Key::S))
            });
            if mv != Vec3::ZERO {
                ed.cam.fly(mv, dt, fast);
                ctx.request_repaint();
            }
        }

        // Gizmo.
        let snap_held = ctx.input(|i| i.modifiers.command);
        if let Some(sel) = ed.selection.filter(|s| ed.scene.contains(*s)) {
            let world = ed.scene.global(sel);
            let mouse = ctx.input(|i| i.pointer.hover_pos());
            let mut hovered = None;
            if let Some(m) = mouse
                && !ed.gizmo.is_dragging()
                && resp.hovered()
                && !alt
            {
                hovered = ed.gizmo.hover(&view, &world, m);
            }
            if resp.drag_started_by(egui::PointerButton::Primary)
                && !alt
                && let (Some(axis), Some(m)) = (hovered, ctx.input(|i| i.pointer.press_origin()))
            {
                ed.gizmo.begin(&view, world, m, axis);
            }
            if ed.gizmo.is_dragging() {
                let prev_snap = ed.gizmo.snap;
                ed.gizmo.snap |= snap_held;
                if let Some(m) = mouse
                    && let Some(new_world) = ed.gizmo.update(&view, m)
                {
                    ed.scene.set_world_matrix(sel, new_world);
                    ed.scene.update_transforms();
                    ed.euler_cache = None;
                }
                ed.gizmo.snap = prev_snap;
                if !ctx.input(|i| i.pointer.primary_down()) {
                    ed.gizmo.end();
                    let label = match ed.gizmo.mode {
                        GizmoMode::Translate => "Mover",
                        GizmoMode::Rotate => "Rotar",
                        GizmoMode::Scale => "Escalar",
                    };
                    ed.commit(label);
                }
            }
            if !(ed.is_playing() && game_cam.is_some()) {
                ed.gizmo.draw(&painter, &view, &ed.scene.global(sel), hovered);
            }
        }

        // Selección por clic.
        if resp.clicked_by(egui::PointerButton::Primary)
            && !alt
            && !ed.gizmo.is_dragging()
            && let Some(m) = resp.interact_pointer_pos()
        {
            ed.selection = pick(ed, &view, m);
            ed.euler_cache = None;
        }

        // Atajos del viewport (si no se escribe en un campo de texto).
        if (resp.hovered() || resp.has_focus()) && !ctx.egui_wants_keyboard_input() && !rmb {
            ctx.input(|i| {
                if !i.modifiers.command {
                    if i.key_pressed(egui::Key::W) {
                        ed.gizmo.mode = GizmoMode::Translate;
                    }
                    if i.key_pressed(egui::Key::E) {
                        ed.gizmo.mode = GizmoMode::Rotate;
                    }
                    if i.key_pressed(egui::Key::R) {
                        ed.gizmo.mode = GizmoMode::Scale;
                    }
                }
            });
            if ctx.input(|i| i.key_pressed(egui::Key::F)) {
                ed.focus_selected();
            }
        }
        if resp.dragged() || scroll != 0.0 {
            ctx.request_repaint();
        }

        draw_icons(ed, &painter, &view);
    }

    // Soltar un modelo del explorador de contenido.
    if let Some(p) = resp.dnd_release_payload::<AssetDrag>() {
        let id = ed.add_model(&p.0);
        if let Some(m) = resp.hover_pos() {
            let ray = view.ray(m);
            if let Some(t) = ray.hit_plane(Vec3::ZERO, Vec3::Y) {
                let p = ray.at(t);
                let mut tr = ed.scene.transform(id);
                tr.translation = p;
                ed.scene.set_transform(id, tr);
                ed.scene.update_transforms();
                ed.commit("Colocar modelo");
            }
        }
    } else if resp.dnd_hover_payload::<AssetDrag>().is_some() {
        painter.rect_stroke(rect.shrink(2.0), 4.0, Stroke::new(2.0, theme::ACCENT), egui::StrokeKind::Inside);
    }

    orientation_widget(&painter, rect, &rview.view);

    // Marco de Play (como el tinte de Unity).
    if let Some(p) = &ed.play {
        let col = if p.runtime.paused { theme::YELLOW } else { theme::ACCENT };
        painter.rect_stroke(rect.shrink(1.0), 0.0, Stroke::new(2.0, col), egui::StrokeKind::Inside);
        let text = if p.runtime.paused {
            "EN PAUSA".to_string()
        } else if ed.game_focus {
            format!("JUGANDO · {:.1} s · clic fuera para soltar el teclado", p.runtime.time)
        } else {
            format!("JUGANDO · {:.1} s · clic en la vista para controlar", p.runtime.time)
        };
        let pos = rect.center_top() + EVec2::new(0.0, 10.0);
        let galley = painter.layout_no_wrap(text, FontId::proportional(11.5), Color32::WHITE);
        let bg = Rect::from_center_size(pos + EVec2::new(0.0, galley.size().y * 0.5), galley.size() + EVec2::new(16.0, 6.0));
        painter.rect_filled(bg, 4.0, col);
        painter.galley(bg.center() - galley.size() * 0.5, galley, Color32::WHITE);
        ctx.request_repaint();
    }
    if game_cam.is_none() && ed.is_playing() && ed.show_game_view {
        painter.text(rect.center(), Align2::CENTER_CENTER, "No hay cámara principal en la escena", FontId::proportional(14.0), theme::TEXT2);
    }
}

/// Selección: iconos de luces/cámaras primero, luego mallas por rayo.
fn pick(ed: &mut Editor, view: &View, mouse: Pos2) -> Option<EntityId> {
    let mut best: Option<(f32, EntityId)> = None;
    for (id, gt, l, c) in ed.scene.world.query::<(&EntityId, &GlobalTransform, Option<&Light>, Option<&Camera>)>().iter() {
        if l.is_none() && c.is_none() {
            continue;
        }
        if let Some(s) = view.to_screen(gt.translation())
            && (s - mouse).length() < 14.0
        {
            return Some(*id);
        }
    }
    let ray = view.ray(mouse);
    let items: Vec<(EntityId, MeshSource, Mat4)> = ed
        .scene
        .world
        .query::<(&EntityId, &MeshRenderer, &GlobalTransform)>()
        .iter()
        .map(|(id, mr, gt)| (*id, mr.mesh.clone(), gt.0))
        .collect();
    for (id, mesh, m) in items {
        if !ed.scene.is_active_in_hierarchy(id) {
            continue;
        }
        let Some(aabb) = ed.renderer.mesh_aabb(&mesh) else { continue };
        if let Some(t) = ray.hit_obb(&aabb, &m)
            && best.is_none_or(|b| t < b.0)
        {
            best = Some((t, id));
        }
    }
    best.map(|b| b.1)
}

fn draw_icons(ed: &Editor, painter: &egui::Painter, view: &View) {
    for (id, gt, light, cam) in ed.scene.world.query::<(&EntityId, &GlobalTransform, Option<&Light>, Option<&Camera>)>().iter() {
        if light.is_none() && cam.is_none() {
            continue;
        }
        let Some(s) = view.to_screen(gt.translation()) else { continue };
        let selected = ed.selection == Some(*id);
        let ring = if selected { theme::ACCENT } else { theme::TEXT2 };
        if let Some(l) = light {
            let c = l.color;
            let fill = Color32::from_rgb((c[0] * 255.0) as u8, (c[1] * 255.0) as u8, (c[2] * 255.0) as u8);
            painter.circle(s, 7.0, fill, Stroke::new(if selected { 2.5 } else { 1.5 }, ring));
            for k in 0..8 {
                let a = k as f32 / 8.0 * std::f32::consts::TAU;
                let d = EVec2::new(a.cos(), a.sin());
                painter.line_segment([s + d * 9.5, s + d * 13.0], Stroke::new(1.5, ring));
            }
            if selected {
                let p = gt.translation();
                match l.kind {
                    LightKind::Directional => {
                        let dir = gt.0.transform_vector3(Vec3::NEG_Z).normalize_or(Vec3::NEG_Y);
                        if let Some(e) = view.to_screen(p + dir * view.world_len(p, 70.0)) {
                            painter.arrow(s, e - s, Stroke::new(2.0, theme::YELLOW));
                        }
                    }
                    LightKind::Point => {
                        let mut pts = Vec::new();
                        for k in 0..=48 {
                            let a = k as f32 / 48.0 * std::f32::consts::TAU;
                            if let Some(q) = view.to_screen(p + Vec3::new(a.cos(), 0.0, a.sin()) * l.range) {
                                pts.push(q);
                            }
                        }
                        painter.add(egui::Shape::line(pts, Stroke::new(1.2, theme::YELLOW)));
                    }
                }
            }
        } else if let Some(c) = cam {
            let r = Rect::from_center_size(s, EVec2::new(18.0, 12.0));
            painter.rect(r, 2.0, theme::CARD, Stroke::new(if selected { 2.5 } else { 1.5 }, ring), egui::StrokeKind::Inside);
            painter.add(egui::Shape::convex_polygon(
                vec![r.right_center() + EVec2::new(1.0, 0.0), r.right_center() + EVec2::new(7.0, -5.0), r.right_center() + EVec2::new(7.0, 5.0)],
                ring,
                Stroke::NONE,
            ));
            if selected {
                // Frustum de la cámara.
                let m = gt.0;
                let (_, rot, pos) = m.to_scale_rotation_translation();
                let d = 2.5;
                let h = (c.fov_y.to_radians() * 0.5).tan() * d;
                let w = h * 16.0 / 9.0;
                let corners = [Vec3::new(-w, -h, -d), Vec3::new(w, -h, -d), Vec3::new(w, h, -d), Vec3::new(-w, h, -d)].map(|v| pos + rot * v);
                let cs: Vec<Option<Pos2>> = corners.iter().map(|c| view.to_screen(*c)).collect();
                let stroke = Stroke::new(1.2, theme::ACCENT);
                for i in 0..4 {
                    if let (Some(a), Some(b)) = (cs[i], cs[(i + 1) % 4]) {
                        painter.line_segment([a, b], stroke);
                    }
                    if let Some(a) = cs[i] {
                        painter.line_segment([s, a], stroke);
                    }
                }
            }
        }
        let _ = id;
    }
}

/// Indicador de orientación en la esquina (ejes del mundo vistos desde la cámara).
fn orientation_widget(painter: &egui::Painter, rect: Rect, view: &Mat4) {
    let c = rect.right_bottom() + EVec2::new(-38.0, -38.0);
    painter.circle(c, 30.0, Color32::from_rgba_unmultiplied(253, 253, 253, 200), Stroke::new(1.0, theme::STROKE3));
    let mut axes = [(Vec3::X, theme::AXIS_X, "X"), (Vec3::Y, theme::AXIS_Y, "Y"), (Vec3::Z, theme::AXIS_Z, "Z")];
    axes.sort_by(|a, b| view.transform_vector3(a.0).z.total_cmp(&view.transform_vector3(b.0).z));
    for (a, col, label) in axes {
        let v = view.transform_vector3(a);
        let e = c + EVec2::new(v.x, -v.y) * 22.0;
        painter.line_segment([c, e], Stroke::new(2.0, col));
        painter.circle_filled(e, 7.0, col);
        painter.text(e, Align2::CENTER_CENTER, label, FontId::proportional(9.5), Color32::WHITE);
    }
}
