//! Mide el tiempo de GPU de la iluminación global con consultas de marca de
//! tiempo, por nivel de calidad, a 1920×1080.
//!
//! `cargo run -p maker_render --example gi_bench [ancho alto]`

use maker_engine::iluminacion::{CalidadGi, IluminacionGlobal, ModoGi};
use maker_engine::scene::Scene;
use maker_render::{RenderOptions, RenderView, SceneRenderer};

fn main() {
    let args: Vec<u32> = std::env::args().skip(1).filter_map(|a| a.parse().ok()).collect();
    let (w, h) = (args.first().copied().unwrap_or(1920), args.get(1).copied().unwrap_or(1080));
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions { power_preference: wgpu::PowerPreference::HighPerformance, ..Default::default() })).expect("adaptador");
    let info = adapter.get_info();
    // Solo las capacidades que el adaptador declara; sin marcas de tiempo no hay medida.
    let required_features = adapter.features() & wgpu::Features::TIMESTAMP_QUERY;
    if required_features.is_empty() {
        eprintln!("{} no admite consultas de marca de tiempo", info.name);
        return;
    }
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor { required_features, required_limits: adapter.limits(), ..Default::default() })).expect("dispositivo");
    println!("GPU: {} ({:?}), {w}×{h}", info.name, info.backend);
    let escenas: [(&str, fn() -> Scene); 2] = [("demo", maker_engine::project::demo_scene), ("cornell", maker_engine::iluminacion::cornell_scene)];
    for (nombre, crear) in escenas {
        for calidad in CalidadGi::ALL {
            let mut s = crear();
            s.env.iluminacion_global = IluminacionGlobal { modo: ModoGi::Software, calidad, ..Default::default() };
            let mut r = SceneRenderer::new(&device, &queue);
            r.resize(w, h);
            let (id, cam) = s.primary_camera().unwrap();
            let view = RenderView::from_camera(s.global(id), &cam, w as f32 / h as f32);
            let opts = RenderOptions { shadows: true, post: true, gi: true, ..Default::default() };
            let mut suma = maker_render::gi::GiTiempos::default();
            let mut n = 0;
            let mut todo = 0.0;
            for f in 0..90 {
                r.render(&s, &view, &opts);
                let t = r.gi_tiempos(true);
                if f == 0 {
                    // Primer fotograma: todas las cascadas se voxelizan.
                    todo = t.map(|t| t.voxelizar).unwrap_or(0.0);
                }
                if f >= 30
                    && let Some(t) = t
                {
                    suma.voxelizar += t.voxelizar;
                    suma.inyectar += t.inyectar;
                    suma.prepase += t.prepase;
                    suma.recoger += t.recoger;
                    suma.filtrar += t.filtrar;
                    n += 1;
                }
            }
            let k = 1.0 / n.max(1) as f32;
            println!(
                "{nombre:8} {:6}: total {:5.2} ms (voxelizar {:.2}, inyectar {:.2}, prepase {:.2}, recoger {:.2}, temporal+filtro {:.2}) · voxelizar todo {:.2} ms · {} MB",
                calidad.label(),
                suma.total() * k,
                suma.voxelizar * k,
                suma.inyectar * k,
                suma.prepase * k,
                suma.recoger * k,
                suma.filtrar * k,
                todo,
                r.gi_memoria() / (1024 * 1024)
            );
        }
    }
}
