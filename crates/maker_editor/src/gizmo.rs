//! Gizmos de transformación (mover / rotar / escalar) dibujados sobre el
//! viewport. La matemática es pura y se prueba sin ventana.

use crate::theme;
use egui::{Color32, Pos2, Stroke};
use glam::{Mat4, Quat, Vec2, Vec3};
use maker_engine::math::{Ray, screen_to_ndc, world_to_screen};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GizmoMode {
    Translate,
    Rotate,
    Scale,
}

/// Datos de cámara para un fotograma del viewport.
#[derive(Clone, Copy, Debug)]
pub struct View {
    pub view_proj: Mat4,
    pub eye: Vec3,
    /// Tamaño del viewport en puntos.
    pub size: Vec2,
    /// Esquina superior izquierda del viewport en la pantalla.
    pub origin: Pos2,
    /// Unidades de mundo por punto de pantalla a distancia 1.
    pub world_per_px_at_1: f32,
}

impl View {
    pub fn to_screen(&self, p: Vec3) -> Option<Pos2> {
        world_to_screen(self.view_proj, p, self.size).map(|s| self.origin + egui::vec2(s.x, s.y))
    }
    pub fn ray(&self, mouse: Pos2) -> Ray {
        let local = Vec2::new(mouse.x - self.origin.x, mouse.y - self.origin.y);
        Ray::from_ndc(self.view_proj.inverse(), screen_to_ndc(local, self.size))
    }
    /// Longitud de mundo que mide `px` puntos en `at`.
    pub fn world_len(&self, at: Vec3, px: f32) -> f32 {
        (at - self.eye).length() * self.world_per_px_at_1 * px
    }
}

#[derive(Clone, Copy, Debug)]
struct Drag {
    axis: usize,
    start_world: Mat4,
    start_mouse: Pos2,
    axis_dir: Vec3,
    start_param: f32,
    start_angle: f32,
}

pub struct Gizmo {
    pub mode: GizmoMode,
    pub local: bool,
    pub snap: bool,
    drag: Option<Drag>,
}

const HANDLE_PX: f32 = 95.0;
const HIT_PX: f32 = 9.0;

impl Default for Gizmo {
    fn default() -> Self {
        Self { mode: GizmoMode::Translate, local: false, snap: false, drag: None }
    }
}

fn dist_to_segment(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_sq().max(1e-6)).clamp(0.0, 1.0);
    (a + ab * t - p).length()
}

/// Parámetro `s` del punto de la recta `p0 + s·a` más cercano al rayo.
pub fn closest_param_on_axis(p0: Vec3, a: Vec3, ray: &Ray) -> Option<f32> {
    let w0 = p0 - ray.origin;
    let b = a.dot(ray.dir);
    let d = a.dot(w0);
    let e = ray.dir.dot(w0);
    let denom = 1.0 - b * b;
    if denom.abs() < 1e-4 {
        return None;
    }
    Some((b * e - d) / denom)
}

fn ring_basis(axis: Vec3) -> (Vec3, Vec3) {
    let u = if axis.y.abs() < 0.9 { axis.cross(Vec3::Y).normalize() } else { axis.cross(Vec3::X).normalize() };
    (u, axis.cross(u))
}

fn snap(v: f32, step: f32) -> f32 {
    (v / step).round() * step
}

impl Gizmo {
    pub fn is_dragging(&self) -> bool {
        self.drag.is_some()
    }
    pub fn cancel(&mut self) {
        self.drag = None;
    }

    fn axes(&self, world: &Mat4) -> [Vec3; 3] {
        if self.local || self.mode == GizmoMode::Scale {
            let (_, r, _) = world.to_scale_rotation_translation();
            [r * Vec3::X, r * Vec3::Y, r * Vec3::Z]
        } else {
            [Vec3::X, Vec3::Y, Vec3::Z]
        }
    }

    /// Eje bajo el ratón (0..2; 3 = uniforme/centro).
    pub fn hover(&self, v: &View, world: &Mat4, mouse: Pos2) -> Option<usize> {
        let c = world.w_axis.truncate();
        let cs = v.to_screen(c)?;
        let len = v.world_len(c, HANDLE_PX);
        let axes = self.axes(world);
        match self.mode {
            GizmoMode::Translate | GizmoMode::Scale => {
                if self.mode == GizmoMode::Scale && (mouse - cs).length() < HIT_PX + 2.0 {
                    return Some(3);
                }
                let mut best = None;
                for (i, a) in axes.iter().enumerate() {
                    let Some(e) = v.to_screen(c + *a * len) else { continue };
                    let d = dist_to_segment(mouse, cs, e);
                    if d < HIT_PX && best.is_none_or(|(_, bd)| d < bd) {
                        best = Some((i, d));
                    }
                }
                best.map(|b| b.0)
            }
            GizmoMode::Rotate => {
                let r = len * 0.85;
                let to_cam = (v.eye - c).normalize_or_zero();
                let mut best = None;
                for (i, a) in axes.iter().enumerate() {
                    let (u, w) = ring_basis(*a);
                    // Un anillo de canto se ve como una línea que cruza los
                    // demás: se penaliza para preferir el que mira a la cámara.
                    let edge_penalty = (1.0 - a.dot(to_cam).abs()) * 4.0;
                    let mut prev: Option<Pos2> = None;
                    for k in 0..=48 {
                        let t = k as f32 / 48.0 * std::f32::consts::TAU;
                        let p = c + (u * t.cos() + w * t.sin()) * r;
                        let s = v.to_screen(p);
                        if let (Some(a0), Some(b0)) = (prev, s) {
                            let d = dist_to_segment(mouse, a0, b0) + edge_penalty;
                            if d < HIT_PX && best.is_none_or(|(_, bd)| d < bd) {
                                best = Some((i, d));
                            }
                        }
                        prev = s;
                    }
                }
                best.map(|b| b.0)
            }
        }
    }

    pub fn begin(&mut self, v: &View, world: Mat4, mouse: Pos2, axis: usize) {
        let c = world.w_axis.truncate();
        let axes = self.axes(&world);
        let axis_dir = if axis < 3 { axes[axis] } else { Vec3::ONE };
        let ray = v.ray(mouse);
        let start_param = if axis < 3 { closest_param_on_axis(c, axis_dir, &ray).unwrap_or(0.0) } else { 0.0 };
        let cs = v.to_screen(c).unwrap_or(mouse);
        let start_angle = (mouse.y - cs.y).atan2(mouse.x - cs.x);
        self.drag = Some(Drag { axis, start_world: world, start_mouse: mouse, axis_dir, start_param, start_angle });
    }

    /// Nueva matriz de mundo mientras se arrastra.
    pub fn update(&self, v: &View, mouse: Pos2) -> Option<Mat4> {
        let d = self.drag?;
        let (s0, r0, t0) = d.start_world.to_scale_rotation_translation();
        match self.mode {
            GizmoMode::Translate => {
                let ray = v.ray(mouse);
                let p = closest_param_on_axis(t0, d.axis_dir, &ray)?;
                let mut delta = p - d.start_param;
                if self.snap {
                    delta = snap(delta, 0.5);
                }
                let t = t0 + d.axis_dir * delta;
                Some(Mat4::from_scale_rotation_translation(s0, r0, t))
            }
            GizmoMode::Rotate => {
                let cs = v.to_screen(t0)?;
                let ang = (mouse.y - cs.y).atan2(mouse.x - cs.x);
                let mut delta = ang - d.start_angle;
                // Hacia la cámara, el sentido de giro en pantalla se invierte.
                if d.axis_dir.dot(v.eye - t0) > 0.0 {
                    delta = -delta;
                }
                if self.snap {
                    delta = snap(delta, 15f32.to_radians());
                }
                let r = Quat::from_axis_angle(d.axis_dir, delta) * r0;
                Some(Mat4::from_scale_rotation_translation(s0, r.normalize(), t0))
            }
            GizmoMode::Scale => {
                let cs = v.to_screen(t0)?;
                let mut factor;
                if d.axis < 3 {
                    let e = v.to_screen(t0 + d.axis_dir * v.world_len(t0, HANDLE_PX))?;
                    let dir = (e - cs).normalized();
                    let px = (mouse - d.start_mouse).dot(dir);
                    factor = 1.0 + px / HANDLE_PX;
                } else {
                    let px = (mouse.x - d.start_mouse.x) - (mouse.y - d.start_mouse.y);
                    factor = 1.0 + px / HANDLE_PX;
                }
                factor = factor.max(0.01);
                let mut s = s0;
                if d.axis < 3 {
                    s[d.axis] *= factor;
                } else {
                    s *= factor;
                }
                if self.snap {
                    s = Vec3::new(snap(s.x, 0.1), snap(s.y, 0.1), snap(s.z, 0.1)).max(Vec3::splat(0.1));
                }
                Some(Mat4::from_scale_rotation_translation(s, r0, t0))
            }
        }
    }

    pub fn end(&mut self) {
        self.drag = None;
    }

    pub fn draw(&self, painter: &egui::Painter, v: &View, world: &Mat4, hovered: Option<usize>) {
        let c = world.w_axis.truncate();
        let Some(cs) = v.to_screen(c) else { return };
        let len = v.world_len(c, HANDLE_PX);
        let axes = self.axes(world);
        let colors = [theme::AXIS_X, theme::AXIS_Y, theme::AXIS_Z];
        let active = self.drag.map(|d| d.axis).or(hovered);
        let hl = Color32::from_rgb(0xff, 0xb4, 0x1e);
        match self.mode {
            GizmoMode::Translate | GizmoMode::Scale => {
                // Del eje más lejano al más cercano.
                let mut order = [0usize, 1, 2];
                order.sort_by(|&a, &b| {
                    let da = (c + axes[a] * len - v.eye).length();
                    let db = (c + axes[b] * len - v.eye).length();
                    db.total_cmp(&da)
                });
                for i in order {
                    let Some(e) = v.to_screen(c + axes[i] * len) else { continue };
                    let col = if active == Some(i) { hl } else { colors[i] };
                    let w = if active == Some(i) { 3.5 } else { 2.5 };
                    painter.line_segment([cs, e], Stroke::new(w, col));
                    let dir = (e - cs).normalized();
                    if self.mode == GizmoMode::Translate {
                        let n = egui::vec2(-dir.y, dir.x);
                        let tip = e + dir * 12.0;
                        painter.add(egui::Shape::convex_polygon(vec![tip, e + n * 5.5, e - n * 5.5], col, Stroke::NONE));
                    } else {
                        painter.rect_filled(egui::Rect::from_center_size(e, egui::vec2(9.0, 9.0)), 1.0, col);
                    }
                }
                let center_col = if active == Some(3) { hl } else { Color32::WHITE };
                painter.circle(cs, 4.5, center_col, Stroke::new(1.5, theme::TEXT2));
            }
            GizmoMode::Rotate => {
                let r = len * 0.85;
                for (i, a) in axes.iter().enumerate() {
                    let (u, w) = ring_basis(*a);
                    let col = if active == Some(i) { hl } else { colors[i] };
                    let width = if active == Some(i) { 3.5 } else { 2.2 };
                    let mut pts = Vec::with_capacity(65);
                    for k in 0..=64 {
                        let t = k as f32 / 64.0 * std::f32::consts::TAU;
                        if let Some(p) = v.to_screen(c + (u * t.cos() + w * t.sin()) * r) {
                            pts.push(p);
                        }
                    }
                    painter.add(egui::Shape::line(pts, Stroke::new(width, col)));
                }
                painter.circle_stroke(cs, 3.0, Stroke::new(1.5, theme::TEXT2));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use maker_engine::math::{look_at, perspective};

    fn view() -> View {
        let vp = perspective(60.0, 16.0 / 9.0, 0.1, 100.0) * look_at(Vec3::new(0.0, 0.0, 10.0), Vec3::ZERO, Vec3::Y);
        View { view_proj: vp, eye: Vec3::new(0.0, 0.0, 10.0), size: Vec2::new(1600.0, 900.0), origin: Pos2::ZERO, world_per_px_at_1: (30f32.to_radians().tan() * 2.0) / 900.0 }
    }

    #[test]
    fn translate_drag_follows_mouse_along_x() {
        let v = view();
        let mut g = Gizmo::default();
        let world = Mat4::IDENTITY;
        let start = v.to_screen(Vec3::new(0.5, 0.0, 0.0)).unwrap();
        assert_eq!(g.hover(&v, &world, start), Some(0));
        g.begin(&v, world, start, 0);
        let target = v.to_screen(Vec3::new(2.5, 0.0, 0.0)).unwrap();
        let m = g.update(&v, target).unwrap();
        assert!((m.w_axis.x - 2.0).abs() < 1e-3, "{}", m.w_axis.x);
        assert!(m.w_axis.y.abs() < 1e-4 && m.w_axis.z.abs() < 1e-4);
    }

    #[test]
    fn snapping_rounds_translation() {
        let v = view();
        let mut g = Gizmo { snap: true, ..Default::default() };
        let start = v.to_screen(Vec3::new(0.5, 0.0, 0.0)).unwrap();
        g.begin(&v, Mat4::IDENTITY, start, 0);
        let m = g.update(&v, v.to_screen(Vec3::new(1.7, 0.0, 0.0)).unwrap()).unwrap();
        assert!((m.w_axis.x - 1.0).abs() < 1e-4, "{}", m.w_axis.x);
    }

    #[test]
    fn rotate_ring_is_hit_and_rotates() {
        let v = view();
        let mut g = Gizmo { mode: GizmoMode::Rotate, ..Default::default() };
        let r = v.world_len(Vec3::ZERO, HANDLE_PX) * 0.85;
        // anillo Z (de cara a la cámara): punto a la derecha
        let p = v.to_screen(Vec3::new(r, 0.0, 0.0)).unwrap();
        assert_eq!(g.hover(&v, &Mat4::IDENTITY, p), Some(2));
        g.begin(&v, Mat4::IDENTITY, p, 2);
        let up = v.to_screen(Vec3::new(0.0, r, 0.0)).unwrap();
        let m = g.update(&v, up).unwrap();
        let x = m.transform_vector3(Vec3::X);
        assert!((x - Vec3::Y).length() < 1e-3, "girar 90° en Z lleva X a Y: {x}");
    }
}
