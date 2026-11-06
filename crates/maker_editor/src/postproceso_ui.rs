//! Inspector del entorno: sombreado global y pila de posprocesado (añadir,
//! quitar, reordenar, encender y editar cada efecto, incluidos los efectos
//! WGSL del proyecto).

use crate::editor::Editor;
use crate::panels::{color3, shading_params};
use crate::theme;
use egui::{RichText, Ui};
use maker_engine::components::ShadingModel;
use maker_engine::log;
use maker_engine::postproceso::*;
use maker_engine::scene::Environment;
use maker_render::SceneRenderer;
use std::path::Path;

/// Lo que la pila pide al editor tras dibujarse.
pub enum Accion {
    /// Cambio estructural (añadir, quitar, mover): se confirma ya.
    Commit(&'static str),
    /// Crear un efecto desde la plantilla, añadirlo y abrirlo.
    NuevoEfecto,
    /// Abrir el `.wgsl` de un efecto propio en el editor de código.
    Abrir(String),
}

pub fn apply(ed: &mut Editor, accion: Option<Accion>) {
    match accion {
        Some(Accion::Commit(label)) => ed.commit(label),
        Some(Accion::NuevoEfecto) => {
            if let Some(path) = ed.create_post_effect("efecto") {
                ed.scene.env.post.push(PostEffect::new(PostKind::Custom(CustomEffect { path, ..Default::default() })));
                ed.commit("Añadir efecto");
            }
        }
        Some(Accion::Abrir(rel)) => ed.open_script(ed.project.root.join(rel)),
        None => {}
    }
}

/// Modelo forzado en todos los materiales y sus parámetros (los mismos
/// controles que el material).
pub fn global_shading(ui: &mut Ui, g: &mut GlobalShading) {
    egui::Grid::new("global-shading-grid").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
        ui.label("Modelo");
        let text = g.model.map(|m| m.label()).unwrap_or("Ninguno");
        egui::ComboBox::from_id_salt("global-shading").selected_text(text).width(170.0).show_ui(ui, |ui| {
            ui.selectable_value(&mut g.model, None, "Ninguno");
            for s in ShadingModel::ALL {
                ui.selectable_value(&mut g.model, Some(s), s.label());
            }
        })
        .response
        .on_hover_text("Fuerza este modelo en todos los materiales; cada uno conserva su color y textura");
        ui.end_row();
        if let Some(model) = g.model {
            shading_params(ui, model, &mut g.style, &mut g.metallic, &mut g.roughness);
        }
    });
}

/// Archivos del proyecto con alguna de estas extensiones (relativos).
fn project_files(root: &Path, sub: &str, exts: &[&str]) -> Vec<String> {
    let mut v = Vec::new();
    maker_engine::project::walk(&root.join(sub), &mut |p| {
        if p.extension().and_then(|e| e.to_str()).is_some_and(|e| exts.iter().any(|x| x.eq_ignore_ascii_case(e))) {
            v.push(p.strip_prefix(root).unwrap_or(p).to_string_lossy().replace('\\', "/"));
        }
    });
    v.sort();
    v
}

pub fn post_stack(ui: &mut Ui, env: &mut Environment, renderer: &mut SceneRenderer, root: &Path) -> Option<Accion> {
    let mut accion = None;
    egui::Grid::new("post-exposure-grid").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
        ui.label("Exposición");
        ui.add(egui::Slider::new(&mut env.exposure, 0.05..=8.0).logarithmic(true));
        ui.end_row();
    });
    let n = env.post.len();
    let mut mover: Option<(usize, usize)> = None;
    let mut quitar: Option<usize> = None;
    for i in 0..n {
        ui.push_id(("post", i), |ui| {
            let e = &mut env.post[i];
            let state = egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), ui.make_persistent_id("post-fx"), false);
            let error = match &e.kind {
                PostKind::Custom(c) => renderer.post_fx().custom_info(&c.path, Some(root)).error.map(str::to_string),
                _ => None,
            };
            let (_, _, body) = state
                .show_header(ui, |ui| {
                    ui.checkbox(&mut e.enabled, "").on_hover_text("Activo");
                    let mut t = RichText::new(e.kind.label()).strong();
                    if error.is_some() {
                        t = t.color(theme::RED);
                    } else if !e.enabled {
                        t = t.color(theme::TEXT3);
                    }
                    let r = ui.label(t);
                    if let Some(err) = &error {
                        r.on_hover_text(err);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("✕").on_hover_text("Quitar").clicked() {
                            quitar = Some(i);
                        }
                        if ui.add_enabled(i + 1 < n, egui::Button::new("⏷").small()).on_hover_text("Bajar").clicked() {
                            mover = Some((i, i + 1));
                        }
                        if ui.add_enabled(i > 0, egui::Button::new("⏶").small()).on_hover_text("Subir").clicked() {
                            mover = Some((i, i - 1));
                        }
                    });
                })
                .body(|ui| {
                    if let Some(err) = &error {
                        ui.label(RichText::new(err.lines().next().unwrap_or_default()).small().color(theme::RED)).on_hover_text(err);
                    }
                    egui::Grid::new("post-fx-grid").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
                        if !matches!(e.kind, PostKind::Tonemap(_)) {
                            ui.label("Intensidad");
                            ui.add(egui::Slider::new(&mut e.intensity, 0.0..=1.0));
                            ui.end_row();
                        }
                        if let Some(a) = effect_params(ui, &mut e.kind, renderer, root) {
                            accion = Some(a);
                        }
                    });
                });
            let _ = body;
        });
    }
    if let Some((a, b)) = mover {
        env.post.swap(a, b);
        accion = Some(Accion::Commit("Reordenar efecto"));
    }
    if let Some(i) = quitar {
        env.post.remove(i);
        accion = Some(Accion::Commit("Quitar efecto"));
    }
    ui.add_space(4.0);
    ui.vertical_centered_justified(|ui| {
        ui.menu_button("Añadir efecto", |ui| {
            for k in PostKind::builtins() {
                if ui.button(k.label()).clicked() {
                    env.post.push(PostEffect::new(k));
                    accion = Some(Accion::Commit("Añadir efecto"));
                    ui.close();
                }
            }
            ui.separator();
            for path in project_files(root, CUSTOM_DIR, &["wgsl"]) {
                let label = Path::new(&path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                if ui.button(label).on_hover_text(&path).clicked() {
                    env.post.push(PostEffect::new(PostKind::Custom(CustomEffect { path, ..Default::default() })));
                    accion = Some(Accion::Commit("Añadir efecto"));
                    ui.close();
                }
            }
            if ui.button("Nuevo efecto…").on_hover_text(format!("Crea un .wgsl en {CUSTOM_DIR} desde la plantilla")).clicked() {
                accion = Some(Accion::NuevoEfecto);
                ui.close();
            }
        });
    });
    accion
}

fn drag(ui: &mut Ui, v: &mut f32, speed: f64, range: std::ops::RangeInclusive<f32>, suffix: &str) {
    ui.add(egui::DragValue::new(v).speed(speed).range(range).suffix(suffix));
}

fn rgb_drag(ui: &mut Ui, v: &mut [f32; 3], range: std::ops::RangeInclusive<f32>) {
    ui.horizontal(|ui| {
        for (c, name) in v.iter_mut().zip(["R", "G", "B"]) {
            ui.add(egui::DragValue::new(c).speed(0.005).range(range.clone()).max_decimals(3)).on_hover_text(name);
        }
    });
}

fn row(ui: &mut Ui, label: &str, f: impl FnOnce(&mut Ui)) {
    ui.label(label);
    f(ui);
    ui.end_row();
}

/// Parámetros propios de cada efecto, dentro de la rejilla.
fn effect_params(ui: &mut Ui, kind: &mut PostKind, renderer: &mut SceneRenderer, root: &Path) -> Option<Accion> {
    match kind {
        PostKind::Tonemap(t) => {
            row(ui, "Curva", |ui| {
                egui::ComboBox::from_id_salt("tonemap-op").selected_text(t.op.label()).show_ui(ui, |ui| {
                    for op in Tonemapper::ALL {
                        ui.selectable_value(&mut t.op, op, op.label());
                    }
                });
            });
            row(ui, "Exposición", |ui| drag(ui, &mut t.exposure_ev, 0.02, -10.0..=10.0, " EV"));
        }
        PostKind::ColorGrading(g) => {
            row(ui, "Contraste", |ui| {
                ui.add(egui::Slider::new(&mut g.contrast, 0.0..=2.0));
            });
            row(ui, "Saturación", |ui| {
                ui.add(egui::Slider::new(&mut g.saturation, 0.0..=2.0));
            });
            row(ui, "Temperatura", |ui| {
                ui.add(egui::Slider::new(&mut g.temperature, -1.0..=1.0));
            });
            row(ui, "Matiz", |ui| {
                ui.add(egui::Slider::new(&mut g.tint, -1.0..=1.0)).on_hover_text("Verde ↔ magenta");
            });
            row(ui, "Lift", |ui| rgb_drag(ui, &mut g.lift, -1.0..=1.0));
            row(ui, "Gamma", |ui| rgb_drag(ui, &mut g.gamma, 0.01..=4.0));
            row(ui, "Gain", |ui| rgb_drag(ui, &mut g.gain, 0.0..=4.0));
            row(ui, "LUT", |ui| {
                let text = if g.lut.is_empty() { "Ninguna".to_string() } else { g.lut.rsplit('/').next().unwrap_or(&g.lut).to_string() };
                egui::ComboBox::from_id_salt("grading-lut")
                    .selected_text(text)
                    .width(170.0)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut g.lut, String::new(), "Ninguna");
                        for p in project_files(root, "", &["png", "jpg", "jpeg"]) {
                            let label = p.clone();
                            ui.selectable_value(&mut g.lut, p, label);
                        }
                    })
                    .response
                    .on_hover_text("Imagen en tira N²×N (p. ej. 256×16)");
            });
        }
        PostKind::Bloom(b) => {
            row(ui, "Umbral", |ui| drag(ui, &mut b.threshold, 0.01, 0.0..=20.0, ""));
            row(ui, "Radio", |ui| {
                ui.add(egui::Slider::new(&mut b.radius, 0.0..=1.0));
            });
        }
        PostKind::Vignette(v) => {
            row(ui, "Radio", |ui| {
                ui.add(egui::Slider::new(&mut v.radius, 0.0..=1.5));
            });
            row(ui, "Suavidad", |ui| {
                ui.add(egui::Slider::new(&mut v.softness, 0.0..=1.0));
            });
            row(ui, "Color", |ui| {
                color3(ui, &mut v.color);
            });
        }
        PostKind::ChromaticAberration(c) => row(ui, "Desplazamiento", |ui| {
            ui.add(egui::Slider::new(&mut c.amount, 0.0..=20.0).suffix(" px"));
        }),
        PostKind::FilmGrain(g) => row(ui, "Tamaño", |ui| {
            ui.add(egui::Slider::new(&mut g.size, 1.0..=4.0).suffix(" px"));
        }),
        PostKind::Retro(r) => {
            row(ui, "Píxel", |ui| {
                ui.add(egui::Slider::new(&mut r.pixel_size, 1.0..=16.0).suffix(" px"));
            });
            row(ui, "Niveles", |ui| {
                ui.add(egui::Slider::new(&mut r.levels, 0..=32)).on_hover_text("Colores por canal; 0 = sin posterizar");
            });
            row(ui, "Tramado", |ui| {
                ui.add(egui::Slider::new(&mut r.dither, 0.0..=1.0)).on_hover_text("Patrón Bayer 4×4 entre niveles (necesita niveles)");
            });
        }
        PostKind::Sharpen(s) => row(ui, "Radio", |ui| {
            ui.add(egui::Slider::new(&mut s.radius, 0.5..=3.0).suffix(" px"));
        }),
        PostKind::Fxaa => {}
        PostKind::EdgeOutline(o) => {
            row(ui, "Color", |ui| {
                color3(ui, &mut o.color);
            });
            row(ui, "Grosor", |ui| {
                ui.add(egui::Slider::new(&mut o.thickness, 0.5..=4.0).suffix(" px"));
            });
            row(ui, "Umbral de distancia", |ui| {
                ui.add(egui::Slider::new(&mut o.depth_threshold, 0.01..=1.0).logarithmic(true));
            });
            row(ui, "Umbral de normal", |ui| {
                ui.add(egui::Slider::new(&mut o.normal_threshold, 0.0..=2.0)).on_hover_text("Pliegues interiores; 0 = solo siluetas");
            });
        }
        PostKind::HeightFog(f) => {
            row(ui, "Color", |ui| {
                color3(ui, &mut f.color);
            });
            row(ui, "Densidad", |ui| {
                ui.add(egui::Slider::new(&mut f.density, 0.0..=1.0).logarithmic(true));
            });
            row(ui, "Altura", |ui| drag(ui, &mut f.height, 0.05, -1000.0..=1000.0, " m"));
            row(ui, "Caída", |ui| {
                ui.add(egui::Slider::new(&mut f.falloff, 0.01..=2.0).logarithmic(true));
            });
        }
        PostKind::DepthOfField(d) => {
            row(ui, "Foco", |ui| drag(ui, &mut d.focus, 0.05, 0.0..=1000.0, " m"));
            row(ui, "Rango nítido", |ui| drag(ui, &mut d.range, 0.05, 0.0..=1000.0, " m"));
            row(ui, "Desenfoque", |ui| {
                ui.add(egui::Slider::new(&mut d.max_blur, 0.0..=32.0).suffix(" px"));
            });
        }
        PostKind::Custom(c) => {
            let params = renderer.post_fx().custom_info(&c.path, Some(root)).params.to_vec();
            for p in &params {
                let v = c.values.get(&p.name).copied().unwrap_or(p.default);
                let mut nv = v;
                ui.label(&p.name);
                match p.kind {
                    ParamKind::Float { min, max } => {
                        ui.add(egui::Slider::new(&mut nv[0], min..=max));
                    }
                    ParamKind::Color => {
                        let mut rgb = [nv[0], nv[1], nv[2]];
                        color3(ui, &mut rgb);
                        nv = [rgb[0], rgb[1], rgb[2], 1.0];
                    }
                }
                ui.end_row();
                if nv != v {
                    c.values.insert(p.name.clone(), nv);
                }
            }
            // Valores de parámetros que ya no existen en el archivo.
            c.values.retain(|k, _| params.is_empty() || params.iter().any(|p| &p.name == k));
            ui.label("");
            if ui.small_button("Editar código").on_hover_text(&c.path).clicked() {
                return Some(Accion::Abrir(c.path.clone()));
            }
            ui.end_row();
        }
    }
    None
}

impl Editor {
    /// Crea `shaders/post/<nombre>.wgsl` desde la plantilla y lo abre.
    /// Devuelve la ruta relativa al proyecto.
    pub fn create_post_effect(&mut self, name: &str) -> Option<String> {
        let dir = self.project.root.join(CUSTOM_DIR);
        if let Err(e) = std::fs::create_dir_all(&dir) {
            log::error("editor", format!("{}: {e}", dir.display()));
            return None;
        }
        let file = free_name(&dir, name);
        let path = dir.join(&file);
        if let Err(e) = std::fs::write(&path, CUSTOM_TEMPLATE) {
            log::error("editor", format!("{}: {e}", path.display()));
            return None;
        }
        let rel = format!("{CUSTOM_DIR}/{file}");
        log::info("editor", format!("efecto {rel} creado"));
        self.open_script(path);
        Some(rel)
    }
}
