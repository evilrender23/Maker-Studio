//! maker-player — ejecuta un proyecto de Maker Studio sin editor (el binario
//! que se distribuye al exportar un juego). MIT © EvilRender23.

use egui::{Color32, Pos2, Rect};
use maker_api::Key;
use maker_engine::{CursorCapture, CursorMode, Input, Project, Runtime, Scene, ScriptLib, log};
use maker_render::{RenderOptions, RenderView, SceneRenderer};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

struct Player {
    rs: egui_wgpu::RenderState,
    scene: Scene,
    runtime: Runtime,
    renderer: SceneRenderer,
    input: Input,
    tex: Option<egui::TextureId>,
    tex_gen: u64,
    last: Instant,
    screenshot: Option<PathBuf>,
    frames: u32,
    frame: u32,
    hold: Vec<Key>,
    done: Arc<AtomicBool>,
    shot: bool,
    fullscreen: bool,
    cursor: CursorCapture,
}

/// Letra A..Z → código de la ABI (0..25).
fn letter_key(c: u8) -> Option<Key> {
    c.is_ascii_uppercase().then(|| unsafe { std::mem::transmute::<u32, Key>((c - b'A') as u32) })
}

fn map_key(k: egui::Key) -> Option<Key> {
    use egui::Key as E;
    let letters = [
        E::A, E::B, E::C, E::D, E::E, E::F, E::G, E::H, E::I, E::J, E::K, E::L, E::M, E::N, E::O, E::P, E::Q, E::R, E::S, E::T, E::U, E::V, E::W, E::X,
        E::Y, E::Z,
    ];
    if let Some(i) = letters.iter().position(|l| *l == k) {
        return letter_key(b'A' + i as u8);
    }
    let digits = [E::Num0, E::Num1, E::Num2, E::Num3, E::Num4, E::Num5, E::Num6, E::Num7, E::Num8, E::Num9];
    if let Some(i) = digits.iter().position(|d| *d == k) {
        return Some(unsafe { std::mem::transmute::<u32, Key>(26 + i as u32) });
    }
    Some(match k {
        E::Space => Key::Space,
        E::Enter => Key::Enter,
        E::Escape => Key::Escape,
        E::Tab => Key::Tab,
        E::Backspace => Key::Backspace,
        E::ArrowUp => Key::Up,
        E::ArrowDown => Key::Down,
        E::ArrowLeft => Key::Left,
        E::ArrowRight => Key::Right,
        _ => return None,
    })
}

/// Modo de cursor → órdenes de ventana. Bloqueado pide primero Confinado y
/// luego Bloqueado: donde winit no sabe bloquear (X11) queda confinado, y
/// donde no sabe confinar (macOS) queda bloqueado.
fn aplicar_modo(ctx: &egui::Context, modo: CursorMode, centro: Option<Pos2>) {
    use egui::ViewportCommand as V;
    use egui::viewport::CursorGrab as G;
    match modo {
        CursorMode::Libre => {
            if let Some(c) = centro {
                ctx.send_viewport_cmd(V::CursorPosition(c));
            }
            ctx.send_viewport_cmd(V::CursorGrab(G::None));
        }
        CursorMode::Confinado => ctx.send_viewport_cmd(V::CursorGrab(G::Confined)),
        CursorMode::Bloqueado => {
            ctx.send_viewport_cmd(V::CursorGrab(G::Confined));
            ctx.send_viewport_cmd(V::CursorGrab(G::Locked));
        }
    }
}

impl eframe::App for Player {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let now = Instant::now();
        let dt = (now - self.last).as_secs_f32();
        self.last = now;
        self.frame += 1;

        // Ratón: Esc / Mayús+F1 lo sueltan sin parar el juego; un clic lo recaptura.
        let cursor = &mut self.cursor;
        ctx.input_mut(|i| {
            use egui::{Key as K, KeyboardShortcut as S, Modifiers as M};
            if cursor.capturado() && (i.consume_key(M::NONE, K::Escape) || i.consume_shortcut(&S::new(M::SHIFT, K::F1))) {
                cursor.soltar();
            } else if !cursor.capturado() && i.pointer.any_pressed() {
                cursor.clic_en_juego();
            }
        });

        ctx.input(|i| {
            for e in &i.events {
                match e {
                    egui::Event::Key { key, pressed, repeat: false, .. } => {
                        if *key == egui::Key::F11 && *pressed {
                            self.fullscreen = !self.fullscreen;
                            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.fullscreen));
                        }
                        if let Some(k) = map_key(*key) {
                            self.input.set_key(k, *pressed);
                        }
                    }
                    egui::Event::MouseMoved(d) => self.input.mouse_delta += glam::Vec2::new(d.x, d.y),
                    _ => {}
                }
            }
            self.input.set_key(Key::Shift, i.modifiers.shift);
            self.input.set_key(Key::Ctrl, i.modifiers.ctrl);
            self.input.set_key(Key::Alt, i.modifiers.alt);
            self.input.mouse_buttons = self.cursor.botones([
                i.pointer.button_down(egui::PointerButton::Primary),
                i.pointer.button_down(egui::PointerButton::Secondary),
                i.pointer.button_down(egui::PointerButton::Middle),
            ]);
        });
        for k in &self.hold {
            self.input.set_key(*k, true);
        }
        self.runtime.tick(&mut self.scene, &self.input, dt);
        self.input.end_frame();
        self.cursor.modo_script(self.runtime.cursor_mode, true);
        let foco = ctx.input(|i| i.focused);
        self.cursor.foco_ventana(foco);
        if let Some(m) = self.cursor.cambio() {
            let centro = (foco && m == CursorMode::Libre).then(|| ctx.content_rect().center());
            aplicar_modo(&ctx, m, centro);
        }
        if self.cursor.modo() == CursorMode::Bloqueado {
            ctx.set_cursor_icon(egui::CursorIcon::None);
        }

        egui::CentralPanel::no_frame().show(ui, |ui| {
            let rect = ui.max_rect();
            let ppp = ctx.pixels_per_point();
            self.renderer.resize((rect.width() * ppp) as u32, (rect.height() * ppp) as u32);
            let aspect = rect.width() / rect.height().max(1.0);
            let view = match self.scene.primary_camera() {
                Some((id, cam)) => RenderView::from_camera(self.scene.global(id), &cam, aspect),
                None => {
                    let eye = glam::Vec3::new(0.0, 4.0, 10.0);
                    RenderView {
                        view: maker_engine::math::look_at(eye, glam::Vec3::ZERO, glam::Vec3::Y),
                        proj: maker_engine::math::perspective(60.0, aspect, 0.1, 1000.0),
                        eye,
                    }
                }
            };
            self.renderer.render(&self.scene, &view, &RenderOptions { shadows: true, ..Default::default() });
            if let Some(v) = self.renderer.output_view() {
                let mut r = self.rs.renderer.write();
                match self.tex {
                    None => self.tex = Some(r.register_native_texture(&self.rs.device, v, wgpu::FilterMode::Linear)),
                    Some(id) if self.tex_gen != self.renderer.target_generation => {
                        r.update_egui_texture_from_wgpu_texture(&self.rs.device, v, wgpu::FilterMode::Linear, id)
                    }
                    _ => {}
                }
                self.tex_gen = self.renderer.target_generation;
            }
            if let Some(id) = self.tex {
                ui.painter().image(id, rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
            }
        });
        ctx.request_repaint();

        if let Some(path) = self.screenshot.clone() {
            if self.frame >= self.frames.max(10) && !self.shot {
                self.shot = true;
                let done = self.done.clone();
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::ScreenshotCallback::new(move |img| {
                    let bytes: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_array()).collect();
                    match image::save_buffer(&path, &bytes, img.size[0] as u32, img.size[1] as u32, image::ExtendedColorType::Rgba8) {
                        Ok(()) => println!("captura guardada en {}", path.display()),
                        Err(e) => eprintln!("captura: {e}"),
                    }
                    done.store(true, Ordering::SeqCst);
                })));
            }
            if self.done.load(Ordering::SeqCst) {
                println!("t={:.2}s scripts={} cuerpos={}", self.runtime.time, self.runtime.script_count(), self.runtime.physics.body_count());
                for e in log::since(0) {
                    println!("[{:?}] [{}] {}", e.level, e.source, e.text);
                }
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }
}

fn main() -> eframe::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let mut args = std::env::args().skip(1);
    let mut root: Option<PathBuf> = None;
    let mut scene_path: Option<PathBuf> = None;
    let mut screenshot = None;
    let mut frames = 60;
    let mut hold = Vec::new();
    while let Some(a) = args.next() {
        match a.as_str() {
            "--scene" => scene_path = args.next().map(PathBuf::from),
            "--screenshot" => screenshot = args.next().map(PathBuf::from),
            "--frames" => frames = args.next().and_then(|v| v.parse().ok()).unwrap_or(60),
            "--hold" => {
                if let Some(k) = args.next() {
                    let up = k.to_ascii_uppercase();
                    if up == "SPACE" {
                        hold.push(Key::Space);
                    } else if up.len() == 1 {
                        hold.extend(letter_key(up.as_bytes()[0]));
                    }
                }
            }
            "-h" | "--help" => {
                println!("Uso: maker-player [CARPETA_DEL_PROYECTO] [--scene ESCENA] [--screenshot PNG --frames N] [--hold TECLA]");
                println!("Ratón capturado al jugar: Esc o Mayús+F1 lo suelta, un clic lo recaptura. F11: pantalla completa.");
                return Ok(());
            }
            other => root = Some(PathBuf::from(other)),
        }
    }
    // Sin argumento: el proyecto está junto al ejecutable (juego exportado).
    let root = root.unwrap_or_else(|| std::env::current_exe().ok().and_then(|e| e.parent().map(|p| p.to_path_buf())).unwrap_or_else(|| PathBuf::from(".")));
    let project = match Project::open(&root) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("No se encontró un proyecto de Maker Studio en {}: {e}", root.display());
            std::process::exit(1);
        }
    };
    let scene_path = scene_path.unwrap_or_else(|| project.main_scene_path());
    let mut scene = match Scene::load(&scene_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    let lib = match ScriptLib::load(&project.script_lib_path()) {
        Ok(l) => Some(Arc::new(l)),
        Err(e) => {
            log::warn("player", e);
            None
        }
    };
    let title = project.name().to_string();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_title(&title).with_inner_size([1280.0, 720.0]).with_app_id("maker-player"),
        ..Default::default()
    };
    eframe::run_native(
        &title,
        options,
        Box::new(move |cc| {
            let rs = cc.wgpu_render_state.clone().expect("wgpu");
            let mut renderer = SceneRenderer::new(&rs.device, &rs.queue);
            renderer.project_root = Some(project.root.clone());
            let mut runtime = Runtime::start_en(&mut scene, lib, Some(&project.root));
            runtime.project_root = Some(project.root.clone());
            // Las capturas automáticas no se quedan con el ratón del escritorio.
            let mut cursor = CursorCapture::default();
            cursor.iniciar(project.file.capturar_raton && screenshot.is_none(), true);
            Ok(Box::new(Player {
                rs,
                scene,
                runtime,
                renderer,
                input: Input::default(),
                tex: None,
                tex_gen: 0,
                last: Instant::now(),
                screenshot,
                frames,
                frame: 0,
                hold,
                done: Arc::new(AtomicBool::new(false)),
                shot: false,
                fullscreen: false,
                cursor,
            }))
        }),
    )
}
