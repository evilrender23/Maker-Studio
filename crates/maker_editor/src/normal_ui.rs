//! Normal maps en el editor: filas del Inspector, generación de los PNG en un
//! hilo aparte (con una espera tras el último cambio de parámetros, para no
//! regenerar a cada paso de un deslizador) y la acción en lote «Generar normal
//! maps que falten».

use egui::Ui;
use maker_engine::components::*;
use maker_engine::model::TextureSource;
use maker_engine::{log, normal_gen};
use maker_render::{NormalInfo, SceneRenderer};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};

use crate::editor::Editor;

/// Espera tras el último cambio antes de regenerar.
const ESPERA: Duration = Duration::from_millis(300);

struct Trabajo {
    src: TextureSource,
    out: PathBuf,
    params: NormalGen,
}

struct Hecho {
    out: PathBuf,
    r: Result<bool, String>,
}

#[derive(Default)]
pub struct Normales {
    tx: Option<Sender<Trabajo>>,
    rx: Option<Receiver<Hecho>>,
    pendientes: usize,
    /// Cambio de parámetros esperando a que pase `ESPERA`.
    espera: Option<(EntityId, NormalGen, Instant)>,
    /// Parámetros ya pedidos por entidad (detecta cambios, también al deshacer).
    pedidos: HashMap<EntityId, NormalGen>,
    /// Normal maps del modelo del Inspector (se recalcula al cambiar de malla).
    info: Option<(MeshSource, Option<NormalInfo>)>,
}

impl Normales {
    /// Qué normal maps tiene y le faltan a una malla.
    pub fn info(&mut self, renderer: &mut SceneRenderer, mesh: &MeshSource) -> Option<NormalInfo> {
        if self.info.as_ref().is_none_or(|(m, _)| m != mesh) {
            self.info = Some((mesh.clone(), renderer.normal_info(mesh)));
        }
        self.info.as_ref().and_then(|(_, i)| i.clone())
    }

    pub fn generando(&self) -> bool {
        self.pendientes > 0
    }

    fn enviar(&mut self, info: &NormalInfo, params: NormalGen) {
        if self.tx.is_none() {
            let (tx, rx) = std::sync::mpsc::channel::<Trabajo>();
            let (htx, hrx) = std::sync::mpsc::channel::<Hecho>();
            let hilo = std::thread::Builder::new().name("generar normal maps".into()).spawn(move || {
                for t in rx {
                    let r = normal_gen::generate_file(&t.src, &t.out, &t.params);
                    if htx.send(Hecho { out: t.out, r }).is_err() {
                        break;
                    }
                }
            });
            if let Err(e) = hilo {
                log::error("normal maps", e.to_string());
                return;
            }
            self.tx = Some(tx);
            self.rx = Some(hrx);
        }
        let Some(tx) = &self.tx else { return };
        for (src, out) in &info.missing {
            if tx.send(Trabajo { src: src.clone(), out: out.clone(), params }).is_ok() {
                self.pendientes += 1;
            }
        }
    }
}

/// Cada fotograma: recoge lo generado y, si la entidad seleccionada usa
/// normal maps generados con otros parámetros que los pedidos, los regenera
/// cuando deja de cambiar.
pub fn update(ed: &mut Editor) {
    if let Some(rx) = &ed.normales.rx {
        let hechos: Vec<Hecho> = rx.try_iter().collect();
        for h in hechos {
            ed.normales.pendientes = ed.normales.pendientes.saturating_sub(1);
            match h.r {
                Ok(true) => {
                    ed.renderer.reload_normal_map(&h.out);
                    log::info("normal maps", format!("generado {}", ed.project.relative(&h.out)));
                }
                Ok(false) => {}
                Err(e) => log::error("normal maps", e),
            }
        }
    }
    if ed.is_playing() {
        return;
    }
    let Some(id) = ed.selection else { return };
    let Some((mesh, params)) = ed.scene.get::<MeshRenderer>(id).filter(|mr| mr.material.normal.source == NormalSource::Generated).map(|mr| (mr.mesh, mr.material.normal.generate)) else {
        return;
    };
    if ed.normales.pedidos.get(&id) == Some(&params) {
        return;
    }
    match ed.normales.espera {
        Some((e, p, t)) if e == id && p == params => {
            if t.elapsed() >= ESPERA {
                ed.normales.espera = None;
                ed.normales.pedidos.insert(id, params);
                if let Some(info) = ed.normales.info(&mut ed.renderer, &mesh) {
                    ed.normales.enviar(&info, params);
                }
            }
        }
        _ => ed.normales.espera = Some((id, params, Instant::now())),
    }
}

/// Pone normal maps generados en todas las mallas de `raiz` y sus
/// descendientes que tienen textura de color sin normal map. Devuelve cuántas.
pub fn generar_que_faltan(ed: &mut Editor, raiz: EntityId) -> usize {
    let h = ed.scene.hierarchy();
    let mut ids = vec![raiz];
    let mut i = 0;
    while i < ids.len() {
        ids.extend_from_slice(h.children_of(ids[i]));
        i += 1;
    }
    let mut n = 0;
    for id in ids {
        let Some(mut mr) = ed.scene.get::<MeshRenderer>(id) else { continue };
        if mr.material.normal.source != NormalSource::Model {
            continue;
        }
        let Some(info) = ed.renderer.normal_info(&mr.mesh).filter(|i| !i.missing.is_empty()) else { continue };
        mr.material.normal.source = NormalSource::Generated;
        let params = mr.material.normal.generate;
        ed.scene.insert(id, mr);
        ed.normales.pedidos.insert(id, params);
        ed.normales.enviar(&info, params);
        n += 1;
    }
    if n > 0 {
        ed.commit("Generar normal maps que faltan");
    }
    ed.status = match n {
        0 => "Ninguna malla necesita normal map generado".into(),
        1 => "Generando el normal map de 1 malla".into(),
        n => format!("Generando normal maps de {n} mallas"),
    };
    n
}

/// Por qué no se puede generar (None = se puede).
fn motivo(info: Option<&NormalInfo>) -> Option<&'static str> {
    match info {
        Some(i) if !i.missing.is_empty() => None,
        Some(i) if i.textured > 0 => Some("Todas sus texturas de color ya traen normal map"),
        _ => Some("El material no tiene textura de color de la que sacar el relieve"),
    }
}

/// Filas del normal map dentro de la rejilla del material.
pub fn filas(ui: &mut Ui, m: &mut Material, info: Option<&NormalInfo>, generando: bool, root: &Path) {
    if m.shading == ShadingModel::Unlit {
        return;
    }
    let no_gen = motivo(info);
    let n = &mut m.normal;
    ui.label("Normal map");
    ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt("mat-normal").selected_text(n.source.label()).width(130.0).show_ui(ui, |ui| {
            for s in NormalSource::ALL {
                match no_gen {
                    Some(why) if s.generates() && n.source != s => {
                        ui.add_enabled(false, egui::Button::selectable(false, s.label())).on_disabled_hover_text(why);
                    }
                    _ => {
                        let tip = match s {
                            NormalSource::Model => "Los que trae el modelo (glTF); sin ellos, ninguno",
                            NormalSource::Texture => "Una textura para todo el material",
                            NormalSource::Generated => "Calculado del color base y guardado como PNG junto a la textura (o en assets/generados/)",
                            NormalSource::Auto => "Calculado del color base al cargar, en memoria, sin archivo",
                        };
                        ui.selectable_value(&mut n.source, s, s.label()).on_hover_text(tip);
                    }
                }
            }
        });
        if generando && n.source == NormalSource::Generated {
            ui.spinner().on_hover_text("Generando…");
        }
    });
    ui.end_row();
    if n.source == NormalSource::Model {
        ui.label("");
        let b = ui.add_enabled(no_gen.is_none(), egui::Button::new("Generar normal map"));
        let b = match no_gen {
            Some(why) => b.on_disabled_hover_text(why),
            None => b.on_hover_text("Saca el relieve de la luminancia del color base (filtro de Sobel) y lo guarda como PNG"),
        };
        if b.clicked() {
            n.source = NormalSource::Generated;
        }
        ui.end_row();
    }
    if n.source == NormalSource::Texture {
        ui.label("Textura");
        texture_combo(ui, &mut n.texture, root);
        ui.end_row();
    }
    let activo = n.source != NormalSource::Model || info.is_some_and(|i| i.with_normal > 0);
    if activo {
        ui.label("Intensidad");
        ui.add(egui::Slider::new(&mut n.strength, 0.0..=2.0));
        ui.end_row();
        ui.label("Verde DirectX");
        ui.checkbox(&mut n.flip_green, "").on_hover_text("Para normal maps con el verde hacia abajo (Unreal, 3ds Max). glTF, Blender y Godot usan la convención OpenGL");
        ui.end_row();
    }
    if n.source.generates() {
        let g = &mut n.generate;
        ui.label("Relieve");
        ui.add(egui::Slider::new(&mut g.strength, 0.0..=10.0));
        ui.end_row();
        ui.label("Suavizado");
        ui.add(egui::Slider::new(&mut g.radius, 0.0..=8.0).step_by(1.0).suffix(" px")).on_hover_text("Radio del desenfoque previo: más radio, menos detalle fino");
        ui.end_row();
        ui.label("Invertir");
        ui.checkbox(&mut g.invert, "").on_hover_text("Lo claro se hunde en vez de sobresalir");
        ui.end_row();
    }
}

/// Elegir un normal map: imágenes del proyecto (`assets/`) y los normal maps
/// de la biblioteca.
fn texture_combo(ui: &mut Ui, path: &mut String, root: &Path) {
    let text = if path.is_empty() { "Elegir…".to_string() } else { path.rsplit('/').next().unwrap_or(path).to_string() };
    let r = egui::ComboBox::from_id_salt("mat-normal-tex").selected_text(text).width(170.0).show_ui(ui, |ui| {
        let es_imagen = |p: &Path| p.extension().and_then(|e| e.to_str()).is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "png" | "jpg" | "jpeg"));
        let mut propias = Vec::new();
        maker_engine::project::walk(&root.join("assets"), &mut |p| {
            if es_imagen(p)
                && let Ok(rel) = p.strip_prefix(root)
            {
                propias.push(rel.to_string_lossy().replace('\\', "/"));
            }
        });
        propias.sort();
        for p in propias {
            ui.selectable_value(path, p.clone(), p);
        }
        if let Some(lib) = maker_engine::biblioteca::library_dir() {
            let mut de_lib = Vec::new();
            maker_engine::project::walk(&lib, &mut |p| {
                let nombre = p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
                if es_imagen(p)
                    && nombre.contains("normal")
                    && let Ok(rel) = p.strip_prefix(&lib)
                {
                    de_lib.push(maker_engine::biblioteca::uri(&rel.to_string_lossy().replace('\\', "/")));
                }
            });
            de_lib.sort();
            if !de_lib.is_empty() {
                ui.separator();
            }
            for p in de_lib {
                ui.selectable_value(path, p.clone(), p);
            }
        }
    });
    if !path.is_empty() {
        r.response.on_hover_text(path.as_str());
    }
}
