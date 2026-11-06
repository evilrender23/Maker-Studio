//! Renderiza las miniaturas de la biblioteca con el renderizador del motor.
//!
//!     cargo run --release -p maker_render --example miniaturas -- assets/biblioteca [--solo pack,pack] [--tam 96]
//!
//! Lee `catalogo.json` y escribe cada `thumbnail` (PNG). Los personajes se
//! posan con el primer fotograma de `Idle_Loop` de la Universal Animation Library.

use maker_engine::biblioteca::{self, Catalog};
use maker_engine::components::*;
use maker_render::SceneRenderer;
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
    let mut pack = String::new();
    let mut done = 0;
    for e in catalog.models.iter().filter(|m| (solo.is_empty() || solo.contains(&m.pack)) && m.path.contains(&filtro)) {
        if e.pack != pack {
            // Libera la GPU entre packs.
            r.clear_model_cache();
            pack = e.pack.clone();
        }
        let src = MeshSource::Model(e.uri());
        let animator = e.has_skin.then(|| {
            if let Some(idle) = e.clips.iter().find(|c| c.contains("Idle")) {
                Animator::new("", idle)
            } else if e.has_animations {
                Animator::new("", "")
            } else {
                Animator::new(biblioteca::DEFAULT_CLIPS, biblioteca::DEFAULT_IDLE)
            }
        });
        let Some(small) = maker_render::miniatura::miniatura(&mut r, &src, animator, size) else {
            eprintln!("✗ {}", e.path);
            continue;
        };
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
