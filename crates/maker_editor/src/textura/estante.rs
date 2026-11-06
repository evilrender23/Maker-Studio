//! Estante: materiales, materiales inteligentes, máscaras inteligentes y alfas de pincel, con
//! miniaturas procedurales (esferas sombreadas en la CPU, unas pocas por fotograma). El
//! sombreado de las esferas viene de `crates/ui-egui/src/thumbs.rs` de TextureCraft (MIT).

use super::{ModoTextura, tr};
use crate::theme;
use egui::{Color32, ColorImage, TextureHandle, TextureOptions, Ui};
use maker_textura::tc::paint::alphas;
use maker_textura::tc::preview::{self, Sample};
use maker_textura::tc::texel::linear_to_srgb;
use serde_json::json;
use std::collections::HashMap;

const POR_FOTOGRAMA: usize = 3;
const LADO_PX: usize = 72;
const LADO: f32 = 64.0;

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Pestana {
    #[default]
    Materiales,
    Inteligentes,
    Mascaras,
    Pinceles,
}

/// Lo que se arrastra del estante al visor o a la pila de capas.
#[derive(Clone, Debug)]
pub enum Arrastre {
    Material(String),
    Mascara(String),
}

#[derive(Default)]
pub struct Estante {
    pestana: Pestana,
    buscar: String,
    /// (id, nombre, inteligente).
    materiales: Option<Vec<(String, String, bool)>>,
    pub(super) mascaras: Option<Vec<(String, String)>>,
    miniaturas: HashMap<String, TextureHandle>,
    hechas: (u64, usize),
}

fn shade(n: [f32; 3], base: [f32; 3], metal: f32, rough: f32) -> [f32; 3] {
    let l = [0.45f32, 0.7, 0.55];
    let len = (l[0] * l[0] + l[1] * l[1] + l[2] * l[2]).sqrt();
    let l = l.map(|v| v / len);
    let ndl = (n[0] * l[0] + n[1] * l[1] + n[2] * l[2]).max(0.0);
    let ndv = n[2].max(0.0);
    let h = [l[0], l[1], l[2] + 1.0];
    let hl = (h[0] * h[0] + h[1] * h[1] + h[2] * h[2]).sqrt();
    let ndh = ((n[0] * h[0] + n[1] * h[1] + n[2] * h[2]) / hl).max(0.0);
    let shin = 2.0 / (rough * rough * rough * rough).max(1e-3) - 2.0;
    let spec = ndh.powf(shin.clamp(1.0, 4096.0)) * (shin + 2.0) / 8.0;
    let ry = 2.0 * ndv * n[1];
    let w = 0.06 + rough * 0.9;
    let t = ((ry + w) / (2.0 * w)).clamp(0.0, 1.0);
    let sharp = 0.04 + 0.03 * (ry + 1.0) + (0.2 + 0.55 * ry.max(0.0)) * t * t * (3.0 - 2.0 * t);
    let env = sharp + (0.28 - sharp) * (rough * rough);
    let fres = (1.0 - ndv).powi(5);
    let sky = 0.35 + 0.25 * n[1];
    let mut out = [0.0; 3];
    for k in 0..3 {
        let f0 = 0.04 + (base[k] - 0.04) * metal;
        let f = f0 + ((1.0 - rough).max(f0) - f0) * fres;
        let diffuse = base[k] * (1.0 - metal) * (ndl * 0.85 + sky * 0.5);
        out[k] = diffuse * (1.0 - f) + f * env + f0 * spec * ndl * 1.5;
    }
    out
}

fn aces(x: f32) -> f32 {
    let v = x * 0.6;
    ((v * (2.51 * v + 0.03)) / (v * (2.43 * v + 0.59) + 0.14)).clamp(0.0, 1.0)
}

fn esfera(size: usize, samples: &[Option<Sample>]) -> ColorImage {
    let mut img = ColorImage::filled([size, size], Color32::TRANSPARENT);
    let r = (size as f32 * 0.5 - 1.0).max(0.5);
    let h = |x: Option<usize>, y: Option<usize>| {
        let (x, y) = (x.filter(|x| *x < size)?, y?);
        samples.get(y.checked_mul(size)?.checked_add(x)?).copied().flatten().map(|s| s.height)
    };
    for (i, n) in preview::sphere_normals(size).enumerate() {
        let (Some(n), Some(Some(s))) = (n, samples.get(i)) else { continue };
        let (x, y) = (i % size, i / size);
        let c = s.height;
        let dx = h(Some(x + 1), Some(y)).unwrap_or(c) - h(x.checked_sub(1), Some(y)).unwrap_or(c);
        let dy = h(Some(x), y.checked_sub(1)).unwrap_or(c) - h(Some(x), Some(y + 1)).unwrap_or(c);
        let k = r * 0.75;
        let b = [n.x - dx * k, n.y - dy * k, n.z];
        let len = (b[0] * b[0] + b[1] * b[1] + b[2] * b[2]).sqrt().max(1e-6);
        let col = shade(b.map(|v| v / len), s.base, s.metallic, s.roughness).map(|v| (linear_to_srgb(aces(v * 1.4)) * 255.0 + 0.5) as u8);
        let borde = ((1.0 - (n.x * n.x + n.y * n.y).sqrt()) * r).clamp(0.0, 1.0);
        img.pixels[i] = Color32::from_rgba_unmultiplied(col[0], col[1], col[2], (borde * 255.0) as u8);
    }
    img
}

impl Estante {
    /// Miniatura cacheada; las nuevas se generan como mucho unas pocas por fotograma.
    fn mini(&mut self, ctx: &egui::Context, clave: &str, crear: impl FnOnce() -> ColorImage) -> Option<TextureHandle> {
        if let Some(t) = self.miniaturas.get(clave) {
            return Some(t.clone());
        }
        let f = ctx.cumulative_frame_nr();
        if self.hechas.0 != f {
            self.hechas = (f, 0);
        }
        if self.hechas.1 >= POR_FOTOGRAMA {
            ctx.request_repaint();
            return None;
        }
        self.hechas.1 += 1;
        let t = ctx.load_texture(format!("textura-{clave}"), crear(), TextureOptions::LINEAR);
        self.miniaturas.insert(clave.to_string(), t.clone());
        Some(t)
    }
}

/// Aplica lo soltado sobre el visor o la pila.
pub fn soltar(m: &mut ModoTextura, a: &Arrastre) {
    match a {
        Arrastre::Material(id) => {
            m.run("material.apply", json!({ "material": id }));
        }
        Arrastre::Mascara(id) => {
            m.run("mask.smart.apply", json!({ "name": id }));
        }
    }
}

pub fn mostrar(m: &mut ModoTextura, ui: &mut Ui) {
    let e = &mut m.estante;
    if e.materiales.is_none() {
        let l = maker_textura::tc::execute(&mut m.t.s, "material.list", &json!({})).unwrap_or_default();
        e.materiales = Some(l.as_array().into_iter().flatten().map(|x| (x["id"].as_str().unwrap_or("").to_string(), x["label"].as_str().unwrap_or("").to_string(), x["smart"].as_bool().unwrap_or(false))).collect());
        let l = maker_textura::tc::execute(&mut m.t.s, "mask.smart.list", &json!({})).unwrap_or_default();
        e.mascaras = Some(l.as_array().into_iter().flatten().map(|x| (x["id"].as_str().unwrap_or("").to_string(), x["label"].as_str().unwrap_or("").to_string())).collect());
    }
    ui.horizontal_wrapped(|ui| {
        for (p, n) in [(Pestana::Materiales, "Materiales"), (Pestana::Inteligentes, "Inteligentes"), (Pestana::Mascaras, "Máscaras"), (Pestana::Pinceles, "Pinceles")] {
            ui.selectable_value(&mut e.pestana, p, n);
        }
    });
    ui.add(egui::TextEdit::singleline(&mut e.buscar).hint_text("Buscar").desired_width(f32::INFINITY));
    ui.add_space(4.0);
    let buscar = e.buscar.to_lowercase();
    let coincide = |s: &str| buscar.is_empty() || s.to_lowercase().contains(&buscar) || tr(s).to_lowercase().contains(&buscar);
    // (clave de miniatura, id, nombre)
    let items: Vec<(String, String, String)> = match e.pestana {
        Pestana::Materiales | Pestana::Inteligentes => {
            let smart = e.pestana == Pestana::Inteligentes;
            e.materiales.iter().flatten().filter(|x| x.2 == smart && coincide(&x.1)).map(|x| (format!("mat:{}", x.0), x.0.clone(), x.1.clone())).collect()
        }
        Pestana::Mascaras => e.mascaras.iter().flatten().filter(|x| coincide(&x.1)).map(|x| (format!("mask:{}", x.0), x.0.clone(), x.1.clone())).collect(),
        Pestana::Pinceles => alphas::library().iter().filter(|a| coincide(&a.label)).map(|a| (format!("alpha:{}", a.id), a.id.clone(), a.label.clone())).collect(),
    };
    let pestana = e.pestana;
    let alpha_actual = m.t.s.tool.brush.alpha_library.clone();
    let mut clic: Option<String> = None;
    let ctx = ui.ctx().clone();
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
            for (clave, id, nombre) in &items {
                let tex = m.estante.mini(&ctx, clave, || match pestana {
                    Pestana::Materiales | Pestana::Inteligentes => esfera(LADO_PX, &preview::material(id, LADO_PX).unwrap_or_default()),
                    Pestana::Mascaras => {
                        let px = preview::smart_mask(id, LADO_PX).unwrap_or_default();
                        let s: Vec<Option<Sample>> = px.iter().map(|v| v.map(|v| Sample { base: [0.04 + 0.76 * v; 3], metallic: 0.0, roughness: 0.6, height: 0.0 })).collect();
                        esfera(LADO_PX, &s)
                    }
                    Pestana::Pinceles => {
                        let a = alphas::find(id);
                        let n = 64usize;
                        let v = a.map(|a| a.render(n, 2)).unwrap_or_else(|| vec![0.0; n * n]);
                        ColorImage::new([n, n], v.into_iter().map(|v| Color32::from_gray((v * 255.0 + 0.5) as u8)).collect())
                    }
                });
                let (rect, resp) = ui.allocate_exact_size(egui::vec2(LADO + 8.0, LADO + 22.0), egui::Sense::click_and_drag());
                let activo = pestana == Pestana::Pinceles && alpha_actual.as_deref() == Some(id.as_str());
                if resp.hovered() || activo {
                    ui.painter().rect_filled(rect, 4.0, if activo { theme::ACCENT_SOFT } else { theme::ROW_HOVER });
                }
                let img = egui::Rect::from_min_size(rect.min + egui::vec2(4.0, 2.0), egui::vec2(LADO, LADO));
                match &tex {
                    Some(t) => {
                        ui.painter().image(t.id(), img, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
                    }
                    None => {
                        ui.painter().circle_filled(img.center(), LADO * 0.4, theme::FILL_TERTIARY);
                    }
                }
                let galley = ui.painter().layout(tr(nombre).to_string(), egui::FontId::proportional(10.5), theme::TEXT2, LADO + 6.0);
                let pos = egui::pos2(rect.center().x - galley.size().x / 2.0, img.bottom() + 2.0);
                ui.painter().with_clip_rect(rect).galley(pos, galley, theme::TEXT2);
                let resp = resp.on_hover_text(tr(nombre));
                if resp.clicked() {
                    clic = Some(id.clone());
                }
                if pestana != Pestana::Pinceles {
                    let a = if pestana == Pestana::Mascaras { Arrastre::Mascara(id.clone()) } else { Arrastre::Material(id.clone()) };
                    resp.dnd_set_drag_payload(a);
                }
            }
        });
    });
    if let Some(id) = clic {
        match pestana {
            Pestana::Materiales | Pestana::Inteligentes => soltar(m, &Arrastre::Material(id)),
            Pestana::Mascaras => soltar(m, &Arrastre::Mascara(id)),
            Pestana::Pinceles => {
                m.run("tool.brush", json!({ "alpha": { "library": id } }));
            }
        }
    }
}
