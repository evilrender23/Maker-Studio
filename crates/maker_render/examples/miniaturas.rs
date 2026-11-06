//! Renderiza las miniaturas de la biblioteca con el renderizador del motor.
//!
//!     cargo run --release -p maker_render --example miniaturas -- assets/biblioteca [--solo pack,pack] [--tam 96]
//!
//! Lee `catalogo.json` y escribe cada `thumbnail` (PNG). Los personajes se
//! posan con el primer fotograma de `Idle_Loop` de la Universal Animation Library.

use glam::{Quat, Vec3};
use maker_engine::animation::Animation;
use maker_engine::biblioteca::{self, Catalog};
use maker_engine::components::*;
use maker_engine::scene::Scene;
use maker_render::{RenderOptions, RenderView, SceneRenderer};
use std::path::PathBuf;

fn main() {
    let mut args = std::env::args().skip(1);
    let mut dir: Option<PathBuf> = None;
    let mut solo: Vec<String> = Vec::new();
    let mut size = 96u32;
    let mut salida: Option<PathBuf> = None;
    let mut filtro = String::new();
    while let Some(a) = args.next() {
        match a.as_str() {
            "--solo" => solo = args.next().unwrap_or_default().split(',').map(str::to_string).collect(),
            "--salida" => salida = args.next().map(PathBuf::from),
            "--filtro" => filtro = args.next().unwrap_or_default(),
            "--tam" => size = args.next().and_then(|v| v.parse().ok()).unwrap_or(96),
            other => dir = Some(PathBuf::from(other)),
        }
    }
    let dir = dir.unwrap_or_else(biblioteca::compiled_dir).canonicalize().expect("carpeta de la biblioteca");
    // SAFETY: aún no hay otros hilos.
    unsafe { std::env::set_var(biblioteca::ENV_VAR, &dir) };
    let catalog = Catalog::load(&dir).expect("catálogo");

    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions { power_preference: wgpu::PowerPreference::HighPerformance, ..Default::default() })).expect("GPU");
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).expect("dispositivo");
    let mut r = SceneRenderer::new(&device, &queue);
    let render_size = size * 2;
    r.resize(render_size, render_size);
    let mut anim = Animation::default();
    let mut pack = String::new();
    let mut done = 0;
    for e in catalog.models.iter().filter(|m| (solo.is_empty() || solo.contains(&m.pack)) && m.path.contains(&filtro)) {
        if e.pack != pack {
            // Libera la GPU entre packs.
            r.clear_model_cache();
            pack = e.pack.clone();
        }
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
        let id = s.spawn(&e.name);
        let src = MeshSource::Model(e.uri());
        s.insert(id, MeshRenderer { mesh: src.clone(), material: Material::default(), cast_shadows: true });
        if e.has_skin {
            let own_idle = e.clips.iter().find(|c| c.contains("Idle")).cloned();
            let a = if let Some(idle) = own_idle {
                Animator::new("", &idle)
            } else if e.has_animations {
                Animator::new("", "")
            } else {
                Animator::new(biblioteca::DEFAULT_CLIPS, biblioteca::DEFAULT_IDLE)
            };
            s.insert(id, a);
        }
        s.update_transforms();
        anim.update(&mut s, None, 0.0, |_| false);
        let Some(aabb) = r.mesh_aabb(&src) else {
            eprintln!("✗ {}", e.path);
            continue;
        };
        let center = aabb.center();
        // Las mallas con piel tienen la caja ampliada ×1.5 para la animación.
        let radius = (aabb.size().length() * 0.5 / if e.has_skin { 1.5 } else { 1.0 }).max(0.05);
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
        let Some((w, h, px)) = r.read_pixels() else { continue };
        let img = image::RgbaImage::from_raw(w, h, px).expect("imagen");
        let small = image::imageops::resize(&img, size, size, image::imageops::FilterType::Triangle);
        let out = match &salida {
            Some(d) => d.join(e.thumbnail.replace('/', "_")),
            None => dir.join(&e.thumbnail),
        };
        if let Some(p) = out.parent() {
            std::fs::create_dir_all(p).expect("carpeta de miniaturas");
        }
        small.save(&out).expect("guardar miniatura");
        done += 1;
    }
    println!("{done} miniaturas en {}", dir.display());
}
