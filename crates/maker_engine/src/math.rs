//! Rayos, selección y cámaras.

use crate::mesh::Aabb;
use glam::{Mat4, Vec2, Vec3};

#[derive(Clone, Copy, Debug)]
pub struct Ray {
    pub origin: Vec3,
    pub dir: Vec3,
}

impl Ray {
    pub fn at(&self, t: f32) -> Vec3 {
        self.origin + self.dir * t
    }

    /// Rayo desde la cámara a través de un punto de pantalla en NDC (-1..1, y arriba).
    pub fn from_ndc(inv_view_proj: Mat4, ndc: Vec2) -> Ray {
        // Profundidad 0..1 (convención wgpu).
        let near = inv_view_proj.project_point3(Vec3::new(ndc.x, ndc.y, 0.0));
        let far = inv_view_proj.project_point3(Vec3::new(ndc.x, ndc.y, 1.0));
        Ray { origin: near, dir: (far - near).normalize() }
    }

    /// Intersección con AABB (método de las losas). Devuelve t ≥ 0.
    pub fn hit_aabb(&self, b: &Aabb) -> Option<f32> {
        let inv = Vec3::ONE / self.dir;
        let t0 = (b.min - self.origin) * inv;
        let t1 = (b.max - self.origin) * inv;
        let tmin = t0.min(t1).max_element();
        let tmax = t0.max(t1).min_element();
        if tmax >= tmin.max(0.0) { Some(tmin.max(0.0)) } else { None }
    }

    /// Intersección con una caja orientada (AABB local + matriz de mundo).
    /// Devuelve la distancia en espacio mundo.
    pub fn hit_obb(&self, local: &Aabb, world: &Mat4) -> Option<f32> {
        let inv = world.inverse();
        let o = inv.transform_point3(self.origin);
        let d = inv.transform_vector3(self.dir);
        let lr = Ray { origin: o, dir: d };
        // Cajas planas (plano): engrosar un poco para poder seleccionarlas.
        let mut b = *local;
        let pad = (b.size().max_element() * 0.01).max(1e-3);
        for i in 0..3 {
            if b.max[i] - b.min[i] < pad {
                b.min[i] -= pad;
                b.max[i] += pad;
            }
        }
        let t = lr.hit_aabb(&b)?;
        let p = world.transform_point3(lr.at(t));
        Some((p - self.origin).length())
    }

    /// Intersección con el plano por `p` con normal `n`.
    pub fn hit_plane(&self, p: Vec3, n: Vec3) -> Option<f32> {
        let d = self.dir.dot(n);
        if d.abs() < 1e-6 {
            return None;
        }
        let t = (p - self.origin).dot(n) / d;
        (t >= 0.0).then_some(t)
    }
}

/// Proyección perspectiva con profundidad 0..1 (wgpu), mano derecha.
pub fn perspective(fov_y_deg: f32, aspect: f32, near: f32, far: f32) -> Mat4 {
    glam::camera::rh::proj::directx::perspective(fov_y_deg.to_radians(), aspect.max(1e-4), near, far)
}

/// Matriz de vista (mano derecha).
pub fn look_at(eye: Vec3, target: Vec3, up: Vec3) -> Mat4 {
    glam::camera::rh::view::look_at_mat4(eye, target, up)
}

/// Proyecta un punto de mundo a píxeles dentro de un rectángulo de `size`.
/// `None` si está detrás de la cámara.
pub fn world_to_screen(view_proj: Mat4, p: Vec3, size: Vec2) -> Option<Vec2> {
    let c = view_proj * p.extend(1.0);
    if c.w <= 1e-5 {
        return None;
    }
    let ndc = c.truncate() / c.w;
    Some(Vec2::new((ndc.x * 0.5 + 0.5) * size.x, (1.0 - (ndc.y * 0.5 + 0.5)) * size.y))
}

/// Píxel (origen arriba-izquierda) → NDC.
pub fn screen_to_ndc(px: Vec2, size: Vec2) -> Vec2 {
    Vec2::new(px.x / size.x * 2.0 - 1.0, 1.0 - px.y / size.y * 2.0)
}

/// Encuadre ortonormal de una luz direccional para el mapa de sombras,
/// centrado en `focus` y cubriendo `radius`.
pub fn directional_light_view_proj(dir: Vec3, focus: Vec3, radius: f32) -> Mat4 {
    let d = dir.normalize_or(Vec3::NEG_Y);
    let up = if d.y.abs() > 0.99 { Vec3::Z } else { Vec3::Y };
    let eye = focus - d * radius * 2.0;
    let view = look_at(eye, focus, up);
    // Ajuste a texel para evitar parpadeo al moverse la cámara.
    let proj = glam::camera::rh::proj::directx::orthographic(-radius, radius, -radius, radius, 0.01, radius * 4.0);
    proj * view
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ray_hits_box_in_front_only() {
        let r = Ray { origin: Vec3::new(0.0, 0.0, 5.0), dir: Vec3::NEG_Z };
        let b = Aabb { min: Vec3::splat(-0.5), max: Vec3::splat(0.5) };
        assert!((r.hit_aabb(&b).unwrap() - 4.5).abs() < 1e-5);
        let back = Ray { origin: Vec3::new(0.0, 0.0, 5.0), dir: Vec3::Z };
        assert!(back.hit_aabb(&b).is_none());
    }

    #[test]
    fn obb_respects_transform() {
        let r = Ray { origin: Vec3::new(10.0, 0.0, 5.0), dir: Vec3::NEG_Z };
        let b = Aabb { min: Vec3::splat(-0.5), max: Vec3::splat(0.5) };
        let m = Mat4::from_translation(Vec3::new(10.0, 0.0, 0.0)) * Mat4::from_scale(Vec3::splat(2.0));
        assert!((r.hit_obb(&b, &m).unwrap() - 4.0).abs() < 1e-4);
        assert!(r.hit_obb(&b, &Mat4::IDENTITY).is_none());
    }

    #[test]
    fn screen_roundtrip() {
        let view = look_at(Vec3::new(0.0, 2.0, 6.0), Vec3::ZERO, Vec3::Y);
        let proj = perspective(60.0, 16.0 / 9.0, 0.1, 100.0);
        let vp = proj * view;
        let size = Vec2::new(1600.0, 900.0);
        let p = Vec3::new(1.0, 0.5, -1.0);
        let s = world_to_screen(vp, p, size).unwrap();
        let ray = Ray::from_ndc(vp.inverse(), screen_to_ndc(s, size));
        let to_p = (p - ray.origin).normalize();
        assert!(to_p.dot(ray.dir) > 0.9999);
    }
}
