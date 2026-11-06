//! Ventanas del modo: horneado de mapas de malla (en otro hilo, con progreso) y exportación de
//! texturas y malla con los presets de TextureCraft.

use super::{ModoTextura, tr};
use crate::theme;
use egui::RichText;
use maker_textura::tc::bake::{BakeOutput, Progress};
use maker_textura::tc::core::BakedMap;
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::mpsc::Receiver;

type Trabajo = (usize, Arc<Progress>, Receiver<Result<BakeOutput, String>>);

pub struct Ventanas {
    pub hornear: bool,
    pub exportar: bool,
    resolucion: u32,
    mapas: BTreeSet<BakedMap>,
    todos_los_sets: bool,
    cola: Vec<usize>,
    trabajo: Option<Trabajo>,
    presets: Vec<String>,
    preset: String,
    carpeta: String,
    formato: String,
    tamano: u32,
}

impl Default for Ventanas {
    fn default() -> Self {
        Self {
            hornear: false,
            exportar: false,
            resolucion: 1024,
            mapas: [BakedMap::AmbientOcclusion, BakedMap::Curvature, BakedMap::Thickness, BakedMap::WorldNormal, BakedMap::Position, BakedMap::Id].into_iter().collect(),
            todos_los_sets: true,
            cola: Vec::new(),
            trabajo: None,
            presets: Vec::new(),
            preset: maker_textura::sesion::PRESET.into(),
            carpeta: String::new(),
            formato: "png".into(),
            tamano: 0,
        }
    }
}

/// El trazador de rayos por GPU de TextureCraft para hornear (una vez por proceso).
fn gpu_horneado() {
    static HECHO: std::sync::Once = std::sync::Once::new();
    HECHO.call_once(|| maker_textura::tc::bake_cmds::install_gpu_tracer(Arc::new(maker_textura::tc_render::gpu_bake::GpuBaker::new())));
}

pub fn mostrar(m: &mut ModoTextura, ctx: &egui::Context) {
    avanzar(m, ctx);
    if m.ventanas.hornear {
        let mut abierta = true;
        egui::Window::new("Hornear mapas de malla").open(&mut abierta).resizable(false).collapsible(false).show(ctx, |ui| hornear(m, ui));
        m.ventanas.hornear = abierta;
    }
    if m.ventanas.exportar {
        let mut abierta = true;
        egui::Window::new("Exportar texturas").open(&mut abierta).resizable(false).collapsible(false).show(ctx, |ui| exportar(m, ui));
        m.ventanas.exportar = abierta;
    }
}

/// Recoge el horneado terminado y lanza el siguiente texture set de la cola.
fn avanzar(m: &mut ModoTextura, ctx: &egui::Context) {
    if let Some((ts, _, rx)) = &m.ventanas.trabajo {
        match rx.try_recv() {
            Ok(r) => {
                let ts = *ts;
                m.ventanas.trabajo = None;
                match r.and_then(|out| m.t.s.apply_bake(ts, out).map_err(|e| e.to_string())) {
                    Ok(()) => maker_engine::log::info("texturizado", format!("horneado el texture set {ts}")),
                    Err(e) => {
                        maker_engine::log::error("texturizado", format!("horneado: {e}"));
                        m.ventanas.cola.clear();
                    }
                }
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
                return;
            }
            Err(_) => m.ventanas.trabajo = None,
        }
    }
    if m.ventanas.cola.is_empty() {
        return;
    }
    let ts = m.ventanas.cola.remove(0);
    gpu_horneado();
    match m.t.s.bake_task(ts) {
        Ok(task) => {
            let p = Arc::new(Progress::new());
            let p2 = p.clone();
            let (tx, rx) = std::sync::mpsc::channel();
            let lanzado = std::thread::Builder::new().name("horneado".into()).spawn(move || {
                let _ = tx.send(task.run(&p2).map_err(|e| e.to_string()));
            });
            match lanzado {
                Ok(_) => m.ventanas.trabajo = Some((ts, p, rx)),
                Err(e) => maker_engine::log::error("texturizado", e.to_string()),
            }
        }
        Err(e) => maker_engine::log::error("texturizado", e.to_string()),
    }
    ctx.request_repaint();
}

fn hornear(m: &mut ModoTextura, ui: &mut egui::Ui) {
    let v = &mut m.ventanas;
    egui::Grid::new("hornear-grid").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
        ui.label("Resolución");
        egui::ComboBox::from_id_salt("hornear-res").selected_text(format!("{} px", v.resolucion)).show_ui(ui, |ui| {
            for r in [256, 512, 1024, 2048, 4096] {
                ui.selectable_value(&mut v.resolucion, r, format!("{r} px"));
            }
        });
        ui.end_row();
        ui.label("Texture sets");
        ui.horizontal(|ui| {
            ui.radio_value(&mut v.todos_los_sets, true, "Todos");
            ui.radio_value(&mut v.todos_los_sets, false, "El activo");
        });
        ui.end_row();
    });
    ui.add_space(4.0);
    ui.label(RichText::new("Mapas").strong());
    ui.columns(2, |cols| {
        for (i, b) in BakedMap::ALL.into_iter().enumerate() {
            let mut on = v.mapas.contains(&b);
            if cols[i % 2].checkbox(&mut on, tr(b.label())).changed() {
                if on {
                    v.mapas.insert(b);
                } else {
                    v.mapas.remove(&b);
                }
            }
        }
    });
    ui.add_space(6.0);
    let ocupado = v.trabajo.is_some() || !v.cola.is_empty();
    if let Some((ts, p, _)) = &v.trabajo {
        let frac = p.done.load(std::sync::atomic::Ordering::Relaxed) as f32 / 10_000.0;
        ui.add(egui::ProgressBar::new(frac).text(format!("Texture set {} · {:.0} %", ts + 1, frac * 100.0)));
        if ui.button("Cancelar").clicked() {
            p.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            v.cola.clear();
        }
        return;
    }
    if ui.add_enabled(!ocupado && !v.mapas.is_empty(), egui::Button::new(RichText::new("Hornear").color(egui::Color32::WHITE)).fill(theme::ACCENT)).clicked() {
        let sets: Vec<usize> = if v.todos_los_sets { (0..m.t.s.set_count()).collect() } else { vec![m.t.s.ts()] };
        let mapas: Vec<&str> = v.mapas.iter().map(|b| b.id()).collect();
        let (res, sets_v) = (v.resolucion, sets.clone());
        if m.run("bake.settings", json!({ "ts": sets_v, "resolution": res, "maps": mapas })).is_some() {
            m.ventanas.cola = sets;
        }
    }
}

fn exportar(m: &mut ModoTextura, ui: &mut egui::Ui) {
    if m.ventanas.presets.is_empty() {
        let l = maker_textura::tc::execute(&mut m.t.s, "export.presets", &json!({})).unwrap_or_default();
        m.ventanas.presets = l.as_array().into_iter().flatten().filter_map(|p| p["name"].as_str().map(str::to_string)).collect();
    }
    if m.ventanas.carpeta.is_empty() {
        m.ventanas.carpeta = m.t.carpeta.join("export").display().to_string();
    }
    let v = &mut m.ventanas;
    egui::Grid::new("exportar-grid").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
        ui.label("Preset");
        egui::ComboBox::from_id_salt("exportar-preset").selected_text(tr(&v.preset)).width(220.0).show_ui(ui, |ui| {
            for p in &v.presets {
                ui.selectable_value(&mut v.preset, p.clone(), tr(p));
            }
        });
        ui.end_row();
        ui.label("Carpeta");
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut v.carpeta).desired_width(200.0));
            if ui.button("…").clicked()
                && let Some(d) = rfd::FileDialog::new().set_directory(&v.carpeta).pick_folder()
            {
                v.carpeta = d.display().to_string();
            }
        });
        ui.end_row();
        ui.label("Formato");
        egui::ComboBox::from_id_salt("exportar-formato").selected_text(v.formato.to_uppercase()).show_ui(ui, |ui| {
            for f in ["png", "jpg", "tga", "tif", "exr", "webp"] {
                ui.selectable_value(&mut v.formato, f.to_string(), f.to_uppercase());
            }
        });
        ui.end_row();
        ui.label("Tamaño");
        egui::ComboBox::from_id_salt("exportar-tam").selected_text(if v.tamano == 0 { "El del texture set".to_string() } else { format!("{} px", v.tamano) }).show_ui(ui, |ui| {
            ui.selectable_value(&mut v.tamano, 0, "El del texture set");
            for r in [512, 1024, 2048, 4096] {
                ui.selectable_value(&mut v.tamano, r, format!("{r} px"));
            }
        });
        ui.end_row();
    });
    ui.add_space(8.0);
    let mut accion: Option<(&str, Value)> = None;
    ui.horizontal(|ui| {
        if ui.add(egui::Button::new(RichText::new("Exportar texturas").color(egui::Color32::WHITE)).fill(theme::ACCENT)).clicked() {
            let mut p = json!({ "dir": v.carpeta, "preset": v.preset, "ts": "all", "format": v.formato });
            if v.tamano > 0 {
                p["size"] = v.tamano.into();
            }
            accion = Some(("file.exportTextures", p));
        }
        if ui.button("Exportar malla (GLB)").clicked() {
            let ruta = std::path::Path::new(&v.carpeta).join(format!("{}.glb", m.t.nombre));
            accion = Some(("file.exportMesh", json!({ "path": ruta.display().to_string() })));
        }
    });
    if let Some((id, p)) = accion {
        let _ = std::fs::create_dir_all(&m.ventanas.carpeta);
        if let Some(r) = m.run(id, p) {
            let n = r["files"].as_array().map(|f| f.len()).or_else(|| r["textures"].as_array().map(|f| f.len()));
            maker_engine::log::info("texturizado", format!("exportado en {}{}", m.ventanas.carpeta, n.map(|n| format!(" ({n} archivos)")).unwrap_or_default()));
        }
    }
}
