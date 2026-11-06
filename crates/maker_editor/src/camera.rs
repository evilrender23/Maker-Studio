//! Cámara del editor: órbita (Alt+clic), vuelo con clic derecho + WASDQE
//! (Unity/Unreal), paneo con botón central y zoom con la rueda.

use glam::{Mat4, Vec3};
use maker_engine::math::{look_at, perspective};
use maker_render::RenderView;

#[derive(Clone, Copy, Debug)]
pub struct EditorCamera {
    pub pivot: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub distance: f32,
    pub fov: f32,
    /// Velocidad de vuelo (m/s); la rueda la cambia mientras se vuela.
    pub fly_speed: f32,
}

impl Default for EditorCamera {
    fn default() -> Self {
        Self { pivot: Vec3::new(0.0, 0.5, 0.0), yaw: 0.6, pitch: -0.45, distance: 11.0, fov: 60.0, fly_speed: 6.0 }
    }
}

impl EditorCamera {
    /// Dirección hacia la que mira.
    pub fn forward(&self) -> Vec3 {
        Vec3::new(-self.yaw.sin() * self.pitch.cos(), self.pitch.sin(), -self.yaw.cos() * self.pitch.cos()).normalize()
    }
    pub fn right(&self) -> Vec3 {
        self.forward().cross(Vec3::Y).normalize_or(Vec3::X)
    }
    pub fn up(&self) -> Vec3 {
        self.right().cross(self.forward())
    }
    pub fn eye(&self) -> Vec3 {
        self.pivot - self.forward() * self.distance
    }

    pub fn view(&self) -> Mat4 {
        look_at(self.eye(), self.pivot, Vec3::Y)
    }

    pub fn render_view(&self, aspect: f32) -> RenderView {
        RenderView { view: self.view(), proj: perspective(self.fov, aspect, 0.05, 2000.0), eye: self.eye() }
    }

    /// Órbita alrededor del pivote (delta en píxeles).
    pub fn orbit(&mut self, dx: f32, dy: f32) {
        self.yaw -= dx * 0.008;
        self.pitch = (self.pitch - dy * 0.008).clamp(-1.55, 1.55);
    }

    /// Mirar alrededor sin mover el ojo (modo vuelo).
    pub fn look(&mut self, dx: f32, dy: f32) {
        let eye = self.eye();
        self.orbit(dx * 0.6, dy * 0.6);
        self.pivot = eye + self.forward() * self.distance;
    }

    /// Paneo en el plano de la pantalla, escalado por la distancia.
    pub fn pan(&mut self, dx: f32, dy: f32, viewport_h: f32) {
        let k = 2.0 * self.distance * (self.fov.to_radians() * 0.5).tan() / viewport_h.max(1.0);
        self.pivot += (-self.right() * dx + self.up() * dy) * k;
    }

    /// Zoom proporcional: `steps` positivos acercan.
    pub fn dolly(&mut self, steps: f32) {
        self.distance = (self.distance * (-steps * 0.12).exp()).clamp(0.05, 5000.0);
    }

    /// Vuelo: mueve ojo y pivote juntos (en espacio de cámara).
    pub fn fly(&mut self, local: Vec3, dt: f32, fast: bool) {
        let speed = self.fly_speed * if fast { 3.0 } else { 1.0 };
        let d = (self.right() * local.x + Vec3::Y * local.y + self.forward() * local.z) * speed * dt;
        self.pivot += d;
    }

    /// Encuadra una caja (centro, radio) como la tecla F de Unity.
    pub fn focus(&mut self, center: Vec3, radius: f32) {
        self.pivot = center;
        let r = radius.max(0.25);
        self.distance = r / (self.fov.to_radians() * 0.5).sin() * 1.15;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn look_keeps_eye_fixed() {
        let mut c = EditorCamera::default();
        let eye = c.eye();
        c.look(120.0, -40.0);
        assert!((c.eye() - eye).length() < 1e-4);
    }

    #[test]
    fn dolly_is_proportional_and_split_invariant() {
        let mut a = EditorCamera::default();
        let mut b = EditorCamera::default();
        a.dolly(1.0);
        for _ in 0..10 {
            b.dolly(0.1);
        }
        assert!((a.distance - b.distance).abs() < 1e-3);
    }

    #[test]
    fn focus_frames_target() {
        let mut c = EditorCamera::default();
        c.focus(Vec3::new(10.0, 0.0, 0.0), 1.0);
        assert_eq!(c.pivot, Vec3::new(10.0, 0.0, 0.0));
        assert!(c.forward().dot((c.pivot - c.eye()).normalize()) > 0.999);
    }
}
