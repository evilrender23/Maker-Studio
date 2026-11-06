//! Maker Studio — editor 3D. MIT © EvilRender23.

mod app;
mod camera;
mod code;
mod editor;
mod gizmo;
mod hub;
mod panels;
mod theme;
mod viewport;

use maker_engine::{Key, Project};
use std::path::PathBuf;

const USAGE: &str = "Uso: maker-studio [PROYECTO] [opciones]

  PROYECTO                 carpeta con project.maker (sin ella se abre el Project Hub)
  --new-demo DIR           crea un proyecto de demostración en DIR y lo abre
  --build-scripts          compila los scripts del proyecto antes de abrir
  --screenshot PNG         guarda una captura de la ventana y sale
  --frames N               fotogramas antes de la captura (por defecto 30)
  --select NOMBRE          selecciona y encuadra una entidad
  --play                   entra en Play al arrancar
  --hold TECLA             mantiene una tecla pulsada durante Play (W, A, Space…)
  --tab content|console|script
  --gizmo move|rotate|scale
  --size ANCHOxALTO        tamaño de ventana
  --export                 exporta el juego a PROYECTO/export/ y sale
";

fn parse_key(s: &str) -> Option<Key> {
    let s = s.to_ascii_uppercase();
    if s.len() == 1 {
        let c = s.as_bytes()[0];
        if c.is_ascii_uppercase() {
            return Some(unsafe { std::mem::transmute::<u32, Key>((c - b'A') as u32) });
        }
    }
    Some(match s.as_str() {
        "SPACE" => Key::Space,
        "UP" => Key::Up,
        "DOWN" => Key::Down,
        "LEFT" => Key::Left,
        "RIGHT" => Key::Right,
        "SHIFT" => Key::Shift,
        _ => return None,
    })
}

fn main() -> eframe::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let mut args = std::env::args().skip(1);
    let mut project: Option<PathBuf> = None;
    let mut new_demo: Option<PathBuf> = None;
    let mut build = false;
    let mut auto = app::Auto::default();
    let mut size = [1600.0f32, 960.0];
    while let Some(a) = args.next() {
        match a.as_str() {
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(());
            }
            "--new-demo" => new_demo = args.next().map(PathBuf::from),
            "--build-scripts" => build = true,
            "--screenshot" => auto.screenshot = args.next().map(PathBuf::from),
            "--frames" => auto.frames = args.next().and_then(|v| v.parse().ok()).unwrap_or(30),
            "--select" => auto.select = args.next(),
            "--play" => auto.play = true,
            "--hold" => auto.hold.extend(args.next().as_deref().and_then(parse_key)),
            "--open-script" => auto.open_script = true,
            "--export" => auto.export = true,
            "--tab" => {
                auto.tab = match args.next().as_deref() {
                    Some("console") => Some(editor::BottomTab::Console),
                    Some("script") => Some(editor::BottomTab::Script),
                    _ => Some(editor::BottomTab::Content),
                }
            }
            "--gizmo" => {
                auto.gizmo = match args.next().as_deref() {
                    Some("rotate") => Some(gizmo::GizmoMode::Rotate),
                    Some("scale") => Some(gizmo::GizmoMode::Scale),
                    _ => Some(gizmo::GizmoMode::Translate),
                }
            }
            "--size" => {
                if let Some((w, h)) = args.next().as_deref().and_then(|s| s.split_once('x')) {
                    size = [w.parse().unwrap_or(size[0]), h.parse().unwrap_or(size[1])];
                }
            }
            other if !other.starts_with("--") => project = Some(PathBuf::from(other)),
            other => {
                eprintln!("opción desconocida: {other}\n\n{USAGE}");
                std::process::exit(2);
            }
        }
    }

    let mut opened = None;
    if let Some(dir) = new_demo {
        let name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Demo".into());
        match Project::create(&dir, &name, true) {
            Ok(p) => opened = Some(p),
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
        }
    } else if let Some(p) = project {
        match Project::open(&p) {
            Ok(p) => opened = Some(p),
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
        }
    }
    if build && let Some(p) = &opened {
        eprintln!("compilando scripts de {}…", p.name());
        match p.build_scripts_blocking() {
            Ok(_) => eprintln!("scripts compilados"),
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
        }
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_title("Maker Studio").with_inner_size(size).with_min_inner_size([1000.0, 620.0]).with_app_id("maker-studio"),
        ..Default::default()
    };
    eframe::run_native("Maker Studio", options, Box::new(move |cc| Ok(Box::new(app::MakerApp::new(cc, opened, auto)))))
}
