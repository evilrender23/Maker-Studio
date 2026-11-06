//! Miniaturas de modelos renderizadas con el propio motor (biblioteca y pestaña Contenido).

use crate::{RenderOptions, RenderView, SceneRenderer};
use glam::{Quat, Vec3};
use maker_engine::animation::Animation;
use maker_engine::components::*;
use maker_engine::scene::Scene;

/// Renderiza `src` centrado sobre fondo claro y lo reduce a `size`×`size`. `animator` posa los
/// modelos con piel (primer fotograma del clip). `None` si el modelo no carga.
pub fn miniatura(r: &mut SceneRenderer, src: &MeshSource, animator: Option<Animator>, size: u32) -> Option<image::RgbaImage> {
    let render_size = size * 2;
    r.resize(render_size, render_size);
    let mut s = Scene::new();
    s.env.sky_top = [0.93, 0.94, 0.96];
    s.env.sky_horizon = [0.97, 0.97, 0.98];
    s.env.ground = [0.82, 0.82, 0.84];
    s.env.fog_density = 0.0;
    s.env.ambient_intensity = 0.8;
    let sun = s.spawn("Sol");
    let mut t = Transform::default();
    t.set_euler_degrees(Vec3::new(-45.0, -30.0, 0.0));
    s.set_transform(sun, t);
    s.insert(sun, Light { intensity: 3.2, ..Light::directional() });
    let id = s.spawn("Modelo");
    s.insert(id, MeshRenderer { mesh: src.clone(), material: Material::default(), cast_shadows: true });
    let con_piel = animator.is_some();
    if let Some(a) = animator {
        s.insert(id, a);
    }
    s.update_transforms();
    Animation::default().update(&mut s, None, 0.0, |_| false);
    let aabb = r.mesh_aabb(src)?;
    let center = aabb.center();
    // Las mallas con piel tienen la caja ampliada ×1.5 para la animación.
    let radius = (aabb.size().length() * 0.5 / if con_piel { 1.5 } else { 1.0 }).max(0.05);
    let fov = 30.0f32;
    let dist = radius / (fov.to_radians() * 0.5).sin() * 1.02;
    let dir_cam = Quat::from_euler(glam::EulerRot::YXZ, 35f32.to_radians(), -22f32.to_radians(), 0.0) * Vec3::Z;
    let eye = center + dir_cam * dist;
    let view = RenderView {
        view: maker_engine::math::look_at(eye, center, Vec3::Y),
        proj: maker_engine::math::perspective(fov, 1.0, (dist - radius * 1.5).max(0.01), dist + radius * 2.0),
        eye,
    };
    r.render(&s, &view, &RenderOptions { shadows: true, ..Default::default() });
    let (w, h, px) = r.read_pixels()?;
    let img = image::RgbaImage::from_raw(w, h, px)?;
    Some(image::imageops::resize(&img, size, size, image::imageops::FilterType::Triangle))
}
