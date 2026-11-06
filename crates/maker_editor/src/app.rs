//! Aplicación eframe: Hub ↔ Editor, menús, barra de Play, atajos y
//! automatización para capturas (`--screenshot`).

use crate::editor::{BottomTab, Editor};
use crate::hub::Hub;
use crate::panels;
use crate::theme;
use egui::{RichText, Ui};
use maker_engine::{Key, Project, log};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;
use crate::agente::Agente;
use crate::agente::cli::Catalogo;
use crate::prefs::Preferencias;

/// Opciones de línea de comandos para pruebas y capturas.
#[derive(Default, Clone)]
pub struct Auto {
    pub screenshot: Option<PathBuf>,
    pub frames: u32,
    pub select: Option<String>,
    /// Escena a abrir en lugar de la principal.
    pub scene: Option<String>,
    pub play: bool,
    pub tab: Option<BottomTab>,
    pub hold: Vec<Key>,
    pub open_script: bool,
    pub export: bool,
    pub gizmo: Option<crate::gizmo::GizmoMode>,
    pub prefs: bool,
    pub agent: AutoAgente,
    pub frame: u32,
    pub shot_requested: bool,
    pub done: Arc<AtomicBool>,
    /// Fotogramas seguidos sin turno del agente en marcha.
    pub agent_idle: u32,
}

/// Automatización del modo agente (`--agent-*`).
#[derive(Default, Clone)]
pub struct AutoAgente {
    pub prompt: Option<String>,
    pub cli: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub perm: Option<String>,
    pub chips: Option<String>,
    pub continuar: bool,
    pub compact: bool,
    pub panel: bool,
}

impl AutoAgente {
    fn activo(&self) -> bool {
        self.prompt.is_some() || self.panel || self.compact
    }
}

pub struct MakerApp {
    rs: egui_wgpu::RenderState,
    hub: Hub,
    ed: Option<Editor>,
    auto: Auto,
    last: Instant,
    title: String,
    close_prompt: bool,
    quitting: bool,
    upd: crate::update::Actualizador,
    upd_oculta: bool,
    prefs: Preferencias,
    prefs_abierta: bool,
    catalogo: Catalogo,
    agente: Option<Agente>,
}

impl MakerApp {
    pub fn new(cc: &eframe::CreationContext, project: Option<Project>, auto: Auto) -> Self {
        theme::install_fonts(&cc.egui_ctx);
        theme::apply(&cc.egui_ctx);
        let rs = cc.wgpu_render_state.clone().expect("Maker Studio necesita el backend wgpu");
        let info = rs.adapter.get_info();
        log::info("render", format!("GPU: {} ({:?}, {:?})", info.name, info.backend, info.device_type));
        let mut hub = Hub::new();
        let ed = project.map(|p| {
            hub.config.touch(&p.root);
            let mut ed = Editor::open(p, &rs);
            if let Some(s) = &auto.scene {
                ed.open_scene(s);
            }
            ed
        });
        let upd = crate::update::Actualizador::default();
        // En capturas automáticas no se busca nada en segundo plano.
        if auto.screenshot.is_none() && !auto.export {
            upd.vigilar(&cc.egui_ctx);
        }
        let mut prefs = Preferencias::cargar();
        if auto.agent.activo() {
            prefs.modo_agente = true;
            prefs.panel_agente = true;
        }
        let catalogo = Catalogo::default();
        // Con el modo agente apagado no se busca ninguna CLI.
        if prefs.modo_agente {
            catalogo.detectar(prefs.rutas(), Some(cc.egui_ctx.clone()));
        }
        let prefs_abierta = auto.prefs;
        Self { rs, hub, ed, auto, last: Instant::now(), title: String::new(), close_prompt: false, quitting: false, upd, upd_oculta: false, prefs, prefs_abierta, catalogo, agente: None }
    }

    fn set_title(&mut self, ctx: &egui::Context) {
        let t = match &self.ed {
            Some(ed) => format!("Maker Studio — {} — {}{}", ed.project.name(), ed.scene_name(), if ed.dirty { " •" } else { "" }),
            None => "Maker Studio".into(),
        };
        if t != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(t.clone()));
            self.title = t;
        }
    }

    fn automation(&mut self, ctx: &egui::Context) {
        let a = &mut self.auto;
        a.frame += 1;
        if let Some(ed) = &mut self.ed {
            if a.frame == 2 {
                if let Some(name) = &a.select {
                    ed.selection = ed.scene.find_by_name(name);
                    if ed.selection.is_some() {
                        ed.focus_selected();
                        ed.cam.dolly(-0.6);
                    }
                }
                if let Some(t) = a.tab {
                    ed.bottom_tab = t;
                }
                if let Some(g) = a.gizmo {
                    ed.gizmo.mode = g;
                }
                if a.open_script {
                    ed.open_script(ed.project.scripts_dir().join("src/lib.rs"));
                }
            }
            if a.frame == 3 && a.export {
                match ed.export_game() {
                    Ok(p) => println!("exportado en {}", p.display()),
                    Err(e) => eprintln!("exportación fallida: {e}"),
                }
                self.quitting = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                return;
            }
            if a.frame == 4 && a.play {
                ed.play();
                ed.game_focus = true;
                // Las capturas automáticas no se quedan con el ratón del escritorio.
                if a.screenshot.is_some() {
                    ed.cursor.parar();
                }
            }
            if ed.is_playing() {
                for k in &a.hold {
                    ed.input.set_key(*k, true);
                }
            }
        }
        if a.screenshot.is_none() {
            return;
        }
        ctx.request_repaint();
        let building = self.ed.as_ref().is_some_and(|e| e.build.is_some());
        let agente_ocupado = self.agente.as_ref().is_some_and(|g| g.ocupado() || g.midiendo());
        a.agent_idle = if agente_ocupado || building { 0 } else { a.agent_idle + 1 };
        let agente_listo = !a.agent.activo() || (a.frame > 3 && a.agent_idle > 20);
        if a.frame >= a.frames.max(30) && !a.shot_requested && !building && agente_listo {
            a.shot_requested = true;
            let path = a.screenshot.clone().unwrap();
            let done = a.done.clone();
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::ScreenshotCallback::new(move |img| {
                let bytes: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_array()).collect();
                match image::save_buffer(&path, &bytes, img.size[0] as u32, img.size[1] as u32, image::ExtendedColorType::Rgba8) {
                    Ok(()) => println!("captura guardada en {}", path.display()),
                    Err(e) => eprintln!("captura: {e}"),
                }
                done.store(true, Ordering::SeqCst);
            })));
        }
        if a.done.load(Ordering::SeqCst) && !self.quitting {
            if let Some(ed) = &self.ed
                && let Some(p) = &ed.play
            {
                println!("play: t={:.2}s scripts={} cuerpos={}", p.runtime.time, p.runtime.script_count(), p.runtime.physics.body_count());
                if let Some(pl) = ed.scene.find_by_name("Jugador") {
                    let g = ed.scene.global(pl).w_axis;
                    println!("Jugador en ({:.2}, {:.2}, {:.2})", g.x, g.y, g.z);
                }
            }
            if a.agent.activo()
                && let Some(g) = &self.agente
            {
                crate::agente::imprimir_sesion(g);
            }
            for e in log::since(0).iter().filter(|e| e.level != log::Level::Info || (a.agent.activo() && (e.source == "agente" || e.source == "scripts"))) {
                println!("[{:?}] [{}] {}", e.level, e.source, e.text);
            }
            self.quitting = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}

impl eframe::App for MakerApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let now = Instant::now();
        let dt = (now - self.last).as_secs_f32();
        self.last = now;
        self.set_title(&ctx);

        // Cerrar con cambios sin guardar.
        if ctx.input(|i| i.viewport().close_requested()) && !self.quitting && self.ed.as_ref().is_some_and(|e| e.dirty) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.close_prompt = true;
        }

        crate::titlebar::bordes(&ctx);
        let mut back_to_hub = false;
        {
            let (ed, upd, oculta) = (&mut self.ed, &self.upd, &mut self.upd_oculta);
            let (prefs, prefs_abierta) = (&mut self.prefs, &mut self.prefs_abierta);
            crate::titlebar::show(ui, ed.is_some(), |ui| {
                if let Some(ed) = ed {
                    back_to_hub = menu_bar(ed, ui, upd, oculta, prefs, prefs_abierta);
                }
            });
        }
        self.update_bar(ui);
        if ctx.input_mut(|i| i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::Comma))) {
            self.prefs_abierta = !self.prefs_abierta;
        }
        let antes = self.prefs.modo_agente;
        crate::prefs::ventana(&ctx, &mut self.prefs_abierta, &mut self.prefs, &self.catalogo);
        if antes && !self.prefs.modo_agente
            && let Some(g) = &mut self.agente
        {
            g.parar();
        }
        self.automation_agente(&ctx);
        let Some(ed) = &mut self.ed else {
            if let Some(p) = self.hub.ui(ui) {
                self.ed = Some(Editor::open(p, &self.rs));
            }
            self.automation(&ctx);
            return;
        };

        crate::importar::tick(ed, &ctx);
        ed.frame_times.push(dt);
        if ed.frame_times.len() > 60 {
            ed.frame_times.remove(0);
        }
        ed.auto_build = self.prefs.compilar_auto;
        ed.tick(dt);
        shortcuts(ed, &ctx);

        // Modo agente: estado por proyecto, eventos de la CLI e integración tras cada turno.
        let modo = self.prefs.modo_agente;
        if modo {
            if self.agente.as_ref().is_none_or(|g| g.raiz != ed.project.root) {
                self.agente = Some(Agente::new(&ed.project.root, &self.prefs, self.catalogo.clone(), &ctx));
            }
            if ctx.input_mut(|i| i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, egui::Key::A))) {
                self.prefs.panel_agente = !self.prefs.panel_agente;
                self.prefs.guardar();
            }
        }
        if let Some(g) = &mut self.agente {
            g.actualizar(ed, &self.prefs);
            if !modo && !g.ocupado() {
                self.agente = None;
            }
        }
        let mut panel_agente = modo.then_some(self.prefs.panel_agente);
        egui::Panel::top("toolbar")
            .frame(egui::Frame::new().fill(theme::CHROME).inner_margin(egui::Margin::symmetric(8, 4)).stroke(egui::Stroke::new(1.0, theme::STROKE4)))
            .show(ui, |ui| toolbar(ed, ui, panel_agente.as_mut()));
        if let Some(v) = panel_agente
            && v != self.prefs.panel_agente
        {
            self.prefs.panel_agente = v;
            self.prefs.guardar();
        }
        egui::Panel::bottom("status").frame(egui::Frame::new().fill(theme::CHROME).inner_margin(egui::Margin::symmetric(10, 3))).show(ui, |ui| {
            ui.horizontal(|ui| {
                let msg = if ed.status.is_empty() { "Listo".to_string() } else { ed.status.clone() };
                ui.label(RichText::new(msg).small().color(theme::TEXT3));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(ed.project.root.display().to_string()).small().color(theme::TEXT4));
                    if ed.dirty {
                        ui.label(RichText::new("● sin guardar").small().color(theme::ORANGE));
                    }
                });
            });
        });
        egui::Panel::left("hierarchy").frame(theme::sidebar_frame()).resizable(true).default_size(250.0).min_size(170.0).show(ui, |ui| panels::hierarchy(ed, ui));
        if modo
            && self.prefs.panel_agente
            && let Some(g) = &mut self.agente
        {
            egui::Panel::right("agente").frame(theme::sidebar_frame()).resizable(true).default_size(400.0).min_size(320.0).max_size(760.0).show(ui, |ui| crate::agente::panel::mostrar(g, ed, &self.prefs, ui));
        }
        egui::Panel::right("inspector").frame(theme::sidebar_frame()).resizable(true).default_size(330.0).min_size(240.0).show(ui, |ui| panels::inspector(ed, ui));
        egui::Panel::bottom("bottom").frame(theme::sidebar_frame()).resizable(true).default_size(240.0).min_size(110.0).show(ui, |ui| panels::bottom(ed, ui, &self.rs));
        egui::CentralPanel::no_frame().show(ui, |ui| {
            egui::Frame::new().fill(theme::CHROME).inner_margin(egui::Margin::symmetric(6, 4)).show(ui, |ui| {
                crate::viewport::show(ed, ui, &self.rs);
            });
        });
        crate::viewport::capturar_raton(ed, &ctx);

        // Confirmar ediciones del inspector cuando termina la interacción.
        if ed.pending_edit.is_some() && !ctx.input(|i| i.pointer.any_down()) && !ctx.egui_wants_keyboard_input() {
            let label = ed.pending_edit.take().unwrap();
            ed.commit(&label);
        }

        if ed.is_playing() || ed.build.is_some() {
            ctx.request_repaint();
        } else {
            ctx.request_repaint_after(std::time::Duration::from_millis(500));
        }

        if self.close_prompt {
            let mut choice = None;
            egui::Modal::new(egui::Id::unique("close-prompt")).show(&ctx, |ui| {
                ui.set_width(340.0);
                ui.label(RichText::new("¿Guardar los cambios de la escena?").strong());
                ui.label(RichText::new(format!("{} tiene cambios sin guardar.", ed.scene_name())).color(theme::TEXT3));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.add(egui::Button::new(RichText::new("Guardar").color(egui::Color32::WHITE)).fill(theme::ACCENT)).clicked() {
                        choice = Some(1);
                    }
                    if ui.button("Descartar").clicked() {
                        choice = Some(2);
                    }
                    if ui.button("Cancelar").clicked() {
                        choice = Some(0);
                    }
                });
            });
            match choice {
                Some(1) => {
                    ed.stop();
                    ed.save_scene();
                    self.quitting = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                Some(2) => {
                    self.quitting = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                Some(_) => self.close_prompt = false,
                None => {}
            }
        }

        if back_to_hub {
            if let Some(g) = &mut self.agente {
                g.parar();
            }
            self.agente = None;
            if let Some(ed) = &mut self.ed {
                ed.stop();
                crate::viewport::capturar_raton(ed, &ctx);
                if ed.dirty {
                    ed.save_scene();
                }
            }
            self.ed = None;
            self.hub.config = crate::hub::Config::load();
            return;
        }
        self.automation(&ctx);
    }

    fn on_exit(&mut self) {
        self.upd.al_salir();
        if let Some(t) = self.agente.as_ref().and_then(|g| g.turno.as_ref()) {
            crate::agente::cli::matar_arbol(t.ej.pid);
        }
        if let Some(ed) = &mut self.ed {
            ed.stop();
        }
    }

    fn clear_color(&self, _v: &egui::Visuals) -> [f32; 4] {
        theme::CHROME.to_normalized_gamma_f32()
    }
}

impl MakerApp {
    /// `--agent-*`: activa el panel, prepara la sesión y envía el mensaje en el tercer fotograma.
    fn automation_agente(&mut self, ctx: &egui::Context) {
        let a = self.auto.agent.clone();
        if !a.activo() || self.auto.frame != 2 {
            return;
        }
        let Some(ed) = &mut self.ed else { return };
        use crate::agente::cli::{Backend, Permisos};
        if let Some(b) = a.cli.as_deref().and_then(Backend::desde_clave) {
            self.prefs.cli = b;
        }
        let g = self.agente.get_or_insert_with(|| Agente::new(&ed.project.root, &self.prefs, self.catalogo.clone(), ctx));
        if !a.continuar {
            g.nueva_sesion(&self.prefs);
        }
        if let Some(b) = a.cli.as_deref().and_then(Backend::desde_clave) {
            g.cambiar_backend(b, &self.prefs);
        }
        let s = g.sesion_mut();
        if let Some(m) = &a.model {
            s.modelo = m.clone();
        }
        if let Some(e) = &a.effort {
            s.esfuerzo = e.clone();
        }
        if let Some(p) = a.perm.as_deref().and_then(Permisos::desde_clave) {
            s.permisos = p;
        }
        if let Some(c) = &a.chips {
            g.chips = c.split(',').filter_map(|x| crate::agente::contexto::Chip::desde_clave(x.trim())).collect();
        }
        if let Some(p) = &a.prompt {
            if a.compact {
                g.compactar_al_terminar();
            }
            g.enviar(ed, &self.prefs, p);
        } else if a.compact {
            g.compactar(ed, &self.prefs);
        }
    }

    /// Barra fina bajo el menú mientras se busca, compila o está lista una actualización.
    fn update_bar(&mut self, ui: &mut Ui) {
        use crate::update::Estado;
        let estado = self.upd.estado();
        let mostrar = match &estado {
            Estado::Inactiva | Estado::Buscando => false,
            Estado::AlDia | Estado::Error(_) | Estado::Nueva { .. } => !self.upd_oculta,
            Estado::Compilando { .. } | Estado::Lista { .. } => true,
        };
        if !mostrar {
            return;
        }
        let ctx = ui.ctx().clone();
        let frame = egui::Frame::new().fill(theme::ACCENT_SOFT).inner_margin(egui::Margin::symmetric(10, 4)).stroke(egui::Stroke::new(1.0, theme::STROKE3));
        let mut cerrar = false;
        egui::Panel::top("update_bar").frame(frame).show(ui, |ui| {
            ui.horizontal(|ui| match &estado {
                Estado::AlDia => {
                    ui.colored_label(theme::GREEN, format!("✔ Maker Studio está al día ({})", crate::update::descripcion_version()));
                    if ui.small_button("✕").clicked() {
                        self.upd_oculta = true;
                    }
                }
                Estado::Error(e) => {
                    ui.colored_label(theme::RED, format!("No se pudo actualizar: {e}"));
                    if ui.small_button("✕").clicked() {
                        self.upd_oculta = true;
                    }
                }
                Estado::Nueva { ultimo } => {
                    ui.label(format!("Hay una versión nueva: {} — {}", crate::update::corto(&ultimo.sha), ultimo.mensaje.clone().unwrap_or_default()));
                    if ui.add(egui::Button::new(RichText::new("Compilar e instalar").color(egui::Color32::WHITE)).fill(theme::ACCENT)).clicked()
                        && let Err(e) = self.upd.instalar()
                    {
                        log::error("actualizador", e);
                    }
                    if ui.small_button("Ahora no").clicked() {
                        self.upd_oculta = true;
                    }
                }
                Estado::Compilando { paso, total, texto } => {
                    ui.spinner();
                    ui.label(if *total > 0 { format!("Actualizando Maker Studio en segundo plano · [{paso}/{total}] {texto}") } else { "Actualizando Maker Studio en segundo plano…".into() });
                }
                Estado::Lista { commit } => {
                    ui.colored_label(theme::GREEN, format!("La versión {commit} está lista. Se aplica al reiniciar el editor."));
                    if ui.add(egui::Button::new(RichText::new("Reiniciar ahora").color(egui::Color32::WHITE)).fill(theme::ACCENT)).clicked() {
                        cerrar = true;
                    }
                }
                _ => {}
            });
        });
        if cerrar {
            if let Some(ed) = &mut self.ed {
                ed.stop();
                if ed.dirty {
                    ed.save_scene();
                }
                if let Some(s) = &ed.script
                    && s.text != s.saved
                {
                    ed.save_script();
                }
            }
            self.upd.aplicar();
            self.quitting = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}

fn shortcuts(ed: &mut Editor, ctx: &egui::Context) {
    use egui::{Key as K, KeyboardShortcut as S, Modifiers as M};
    let typing = ctx.egui_wants_keyboard_input();
    let game = ed.is_playing() && ed.game_focus;
    ctx.input_mut(|i| {
        if i.consume_shortcut(&S::new(M::COMMAND, K::P)) || i.consume_key(M::NONE, K::F5) {
            if ed.is_playing() { ed.stop() } else { ed.play() }
        }
        if i.consume_key(M::NONE, K::F7) {
            ed.build_scripts();
        }
        if !game && i.consume_shortcut(&S::new(M::COMMAND, K::I)) {
            let d = ed.project.root.join(ed.contenido.carpeta());
            ed.importador.abrir(false, d, &ctx.clone());
        }
        // Esc suelta el ratón capturado sin parar el juego (el siguiente Esc es del juego).
        if ed.cursor.capturado() && (i.consume_key(M::NONE, K::Escape) || i.consume_shortcut(&S::new(M::SHIFT, K::F1))) {
            ed.cursor.soltar();
        }
        if typing {
            return;
        }
        if i.consume_shortcut(&S::new(M::COMMAND, K::S)) {
            ed.save_scene();
        }
        if i.consume_shortcut(&S::new(M::COMMAND | M::SHIFT, K::Z)) || i.consume_shortcut(&S::new(M::COMMAND, K::Y)) {
            ed.redo();
        }
        if i.consume_shortcut(&S::new(M::COMMAND, K::Z)) {
            ed.undo();
        }
        if game {
            return;
        }
        if i.consume_shortcut(&S::new(M::COMMAND, K::D)) {
            ed.duplicate_selected();
        }
        if i.consume_key(M::NONE, K::Delete) {
            ed.delete_selected();
        }
        if i.consume_key(M::NONE, K::F2)
            && let Some(id) = ed.selection
        {
            ed.rename = Some((id, ed.scene.name(id)));
        }
    });
}

fn menu_bar(ed: &mut Editor, ui: &mut Ui, upd: &crate::update::Actualizador, upd_oculta: &mut bool, prefs: &mut Preferencias, prefs_abierta: &mut bool) -> bool {
    let mut hub = false;
    egui::MenuBar::new().ui(ui, |ui| {
        ui.menu_button("Archivo", |ui| {
            if ui.button("Guardar escena            Ctrl+S").clicked() {
                ed.save_scene();
            }
            if ui.button("Importar…                   Ctrl+I").clicked() {
                let d = ed.project.root.join(ed.contenido.carpeta());
                ed.importador.abrir(false, d, ui.ctx());
                ui.close();
            }
            if ui.button("Importar carpeta…").clicked() {
                let d = ed.project.root.join(ed.contenido.carpeta());
                ed.importador.abrir(true, d, ui.ctx());
                ui.close();
            }
            ui.menu_button("Abrir escena", |ui| {
                for s in ed.project.scenes() {
                    if ui.button(&s).clicked() {
                        if ed.dirty {
                            ed.save_scene();
                        }
                        ed.open_scene(&s);
                    }
                }
            });
            let mut capturar = ed.project.file.capturar_raton;
            if ui.checkbox(&mut capturar, "Capturar ratón en Play").on_hover_text("Esc lo suelta · clic en el juego lo recaptura").changed() {
                ed.project.file.capturar_raton = capturar;
                if let Err(e) = ed.project.save() {
                    log::error("editor", e);
                }
            }
            if ui.button("Hacer esta escena la principal").clicked() {
                ed.project.file.main_scene = ed.project.relative(&ed.scene_path);
                match ed.project.save() {
                    Ok(()) => ed.status = "Escena principal actualizada".into(),
                    Err(e) => log::error("editor", e),
                }
            }
            ui.separator();
            if ui.button("Ejecutar en ventana aparte").clicked() {
                ed.run_standalone();
            }
            if ui.button("Exportar juego…").clicked() {
                match ed.export_game() {
                    Ok(p) => ed.status = format!("Exportado en {}", p.display()),
                    Err(e) => log::error("editor", e),
                }
            }
            ui.separator();
            if ui.button("Preferencias…            Ctrl+,").clicked() {
                *prefs_abierta = true;
            }
            ui.separator();
            if ui.button("Volver al Project Hub").clicked() {
                hub = true;
            }
            if ui.button("Salir").clicked() {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            }
        });
        ui.menu_button("Editar", |ui| {
            let ul = ed.undo.undo_label().map(|l| format!("Deshacer: {l}")).unwrap_or_else(|| "Deshacer".into());
            if ui.add_enabled(ed.undo.can_undo() && !ed.is_playing(), egui::Button::new(format!("{ul}   Ctrl+Z"))).clicked() {
                ed.undo();
            }
            let rl = ed.undo.redo_label().map(|l| format!("Rehacer: {l}")).unwrap_or_else(|| "Rehacer".into());
            if ui.add_enabled(ed.undo.can_redo() && !ed.is_playing(), egui::Button::new(format!("{rl}   Ctrl+Y"))).clicked() {
                ed.redo();
            }
            ui.separator();
            let has = ed.selection.is_some();
            if ui.add_enabled(has, egui::Button::new("Duplicar   Ctrl+D")).clicked() {
                ed.duplicate_selected();
            }
            if ui.add_enabled(has, egui::Button::new("Eliminar   Supr")).clicked() {
                ed.delete_selected();
            }
            if ui.add_enabled(has, egui::Button::new("Encuadrar   F")).clicked() {
                ed.focus_selected();
            }
        });
        ui.menu_button("Crear", |ui| {
            if let Some(c) = panels::create_menu(ui) {
                ed.create(c);
            }
        });
        ui.menu_button("Scripts", |ui| {
            if ui.button("Compilar y recargar   F7").clicked() {
                ed.build_scripts();
            }
            if ui.button("Recargar biblioteca").clicked() {
                ed.reload_scripts(true);
            }
            if ui.button("Abrir lib.rs").clicked() {
                ed.open_script(ed.project.scripts_dir().join("src/lib.rs"));
            }
        });
        if prefs.modo_agente {
            ui.menu_button("Agente", |ui| {
                if ui.checkbox(&mut prefs.panel_agente, "Panel del agente   Ctrl+Mayús+A").changed() {
                    prefs.guardar();
                }
            });
        }
        ui.menu_button("Ayuda", |ui| {
            ui.label(RichText::new(format!("Versión {}", crate::update::descripcion_version())).color(theme::TEXT3));
            let instalada = crate::update::version_instalada().is_some();
            if ui
                .add_enabled(instalada, egui::Button::new("Buscar actualizaciones"))
                .on_disabled_hover_text("Solo las copias instaladas con instalar.sh / instalar.ps1 se actualizan solas")
                .clicked()
            {
                *upd_oculta = false;
                upd.buscar_ahora(crate::update::automatico());
                ui.close();
            }
            ui.separator();
            ui.label(RichText::new("Controles del viewport").strong());
            for (k, v) in [
                ("Clic derecho + WASDQE", "volar (Shift = rápido, rueda = velocidad)"),
                ("Alt + clic izquierdo", "orbitar"),
                ("Botón central", "desplazar"),
                ("Rueda", "acercar / alejar"),
                ("W / E / R", "mover / rotar / escalar"),
                ("Ctrl al arrastrar", "ajuste a rejilla"),
                ("F", "encuadrar selección"),
                ("F5 · Ctrl+P", "Play / Stop"),
                ("Esc · Mayús+F1", "soltar el ratón del juego"),
                ("F7", "compilar scripts"),
            ] {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(k).monospace());
                    ui.label(RichText::new(v).color(theme::TEXT3));
                });
            }
        });
    });
    hub
}

fn toolbar(ed: &mut Editor, ui: &mut Ui, panel_agente: Option<&mut bool>) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("{} / {}", ed.project.name(), ed.scene_name())).color(theme::TEXT2));
        let center = ui.available_width() * 0.5 - 90.0;
        ui.add_space((center - ui.min_rect().width()).max(8.0));
        let playing = ed.is_playing();
        let paused = ed.play.as_ref().is_some_and(|p| p.runtime.paused);
        let play_btn = if playing {
            egui::Button::new(RichText::new("■  Stop").color(egui::Color32::WHITE)).fill(theme::RED)
        } else {
            egui::Button::new(RichText::new("▶  Play").color(egui::Color32::WHITE)).fill(theme::ACCENT)
        };
        if ui.add(play_btn.min_size(egui::vec2(84.0, 24.0))).on_hover_text("F5 / Ctrl+P").clicked() {
            if playing { ed.stop() } else { ed.play() }
        }
        if ui.add_enabled(playing, egui::Button::selectable(paused, "❚❚ Pausa")).clicked() {
            ed.toggle_pause();
        }
        if ui.add_enabled(paused, egui::Button::new("▶❚ Paso")).on_hover_text("Avanzar un fotograma").clicked() {
            ed.step_frame();
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if let Some(v) = panel_agente {
                if ui.add(egui::Button::selectable(*v, "✦ Agente")).on_hover_text("Ctrl+Mayús+A").clicked() {
                    *v = !*v;
                }
                ui.separator();
            }
            let building = ed.build.is_some();
            if ui.add_enabled(!building, egui::Button::new(if building { "Compilando…" } else { "Compilar scripts (F7)" })).clicked() {
                ed.build_scripts();
            }
            if building {
                ui.spinner();
            }
            match (&ed.lib, &ed.lib_error) {
                (Some(l), _) => ui.label(RichText::new(format!("ƒ {} scripts", l.names().len())).color(theme::GREEN)),
                (None, Some(e)) => ui.label(RichText::new(e).color(theme::YELLOW)),
                _ => ui.label(""),
            };
        });
    });
}
