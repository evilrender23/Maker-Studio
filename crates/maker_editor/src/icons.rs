//! Texturas de los iconos pixel art (`pixel_icons.rs`) y del logo de Maker.
//!
//! Los iconos se suben en blanco y se tiñen al dibujarlos; se filtran con NEAREST y se dibujan
//! a un múltiplo entero de píxeles físicos para que cada píxel del dibujo quede nítido.

use std::collections::HashMap;

use egui::{Color32, Context, Id, TextureHandle, TextureOptions, Vec2};

use crate::pixel_icons as px;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Icon {
    Camara,
    Sol,
    Bombilla,
    Cubo,
    Esfera,
    Plano,
    Cilindro,
    Cono,
    Capsula,
    Toro,
    Modelo,
    Vacio,
    Minimizar,
    Maximizar,
    Restaurar,
    Cerrar,
}

impl Icon {
    fn filas(self) -> &'static [&'static str; px::SIZE] {
        match self {
            Icon::Camara => &px::CAMARA,
            Icon::Sol => &px::SOL,
            Icon::Bombilla => &px::BOMBILLA,
            Icon::Cubo => &px::CUBO,
            Icon::Esfera => &px::ESFERA,
            Icon::Plano => &px::PLANO,
            Icon::Cilindro => &px::CILINDRO,
            Icon::Cono => &px::CONO,
            Icon::Capsula => &px::CAPSULA,
            Icon::Toro => &px::TORO,
            Icon::Modelo => &px::MODELO,
            Icon::Vacio => &px::VACIO,
            Icon::Minimizar => &px::MIN,
            Icon::Maximizar => &px::MAX,
            Icon::Restaurar => &px::RESTAURAR,
            Icon::Cerrar => &px::CERRAR,
        }
    }
}

/// RGBA blanco del icono: '#' opaco, '+' relleno suave.
pub fn rgba(icon: Icon) -> Vec<u8> {
    let mut out = Vec::with_capacity(px::SIZE * px::SIZE * 4);
    for fila in icon.filas() {
        for c in fila.bytes() {
            let a = match c {
                b'#' => 255,
                b'+' => 70,
                _ => 0,
            };
            out.extend_from_slice(&[255, 255, 255, a]);
        }
    }
    out
}

#[derive(Clone, Default)]
struct Cache {
    iconos: HashMap<Icon, TextureHandle>,
    /// Logo por altura en píxeles físicos.
    logos: HashMap<u32, TextureHandle>,
}

fn cache_id() -> Id {
    Id::unique("maker-icon-cache")
}

pub fn texture(ctx: &Context, icon: Icon) -> TextureHandle {
    if let Some(t) = ctx.data(|d| d.get_temp::<Cache>(cache_id()).and_then(|c| c.iconos.get(&icon).cloned())) {
        return t;
    }
    let img = egui::ColorImage::from_rgba_unmultiplied([px::SIZE, px::SIZE], &rgba(icon));
    let t = ctx.load_texture(format!("icono-{icon:?}"), img, TextureOptions::NEAREST);
    ctx.data_mut(|d| d.get_temp_mut_or_default::<Cache>(cache_id()).iconos.insert(icon, t.clone()));
    t
}

/// Lado en puntos para que el icono ocupe un número entero de píxeles físicos por píxel del dibujo.
pub fn side(ctx: &Context) -> f32 {
    let ppp = ctx.pixels_per_point();
    px::SIZE as f32 * ppp.round().max(1.0) / ppp
}

/// El icono como imagen de egui, ya teñido.
pub fn image(ctx: &Context, icon: Icon, tint: Color32) -> egui::Image<'static> {
    let t = texture(ctx, icon);
    let s = side(ctx);
    egui::Image::new((t.id(), Vec2::splat(s))).tint(tint)
}

const LOGO_PNG: &[u8] = include_bytes!("../../../assets/logo/maker-logo-black.png");

/// El logo de Maker a `alto` puntos, reescalado una vez a la resolución física exacta
/// (con Lanczos), para que no se vea dentado al dibujarlo pequeño.
pub fn logo(ctx: &Context, alto: f32) -> (TextureHandle, Vec2) {
    let px_alto = (alto * ctx.pixels_per_point()).round().max(1.0) as u32;
    let cached = ctx.data(|d| d.get_temp::<Cache>(cache_id()).and_then(|c| c.logos.get(&px_alto).cloned()));
    let t = cached.unwrap_or_else(|| {
        let img = image::load_from_memory(LOGO_PNG).expect("logo PNG válido").to_rgba8();
        let ancho = (img.width() as f32 * px_alto as f32 / img.height() as f32).round().max(1.0) as u32;
        let img = image::imageops::resize(&img, ancho, px_alto, image::imageops::FilterType::Lanczos3);
        let color = egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw());
        let t = ctx.load_texture(format!("maker-logo-{px_alto}"), color, TextureOptions::LINEAR);
        ctx.data_mut(|d| d.get_temp_mut_or_default::<Cache>(cache_id()).logos.insert(px_alto, t.clone()));
        t
    });
    let [w, h] = t.size();
    (t, Vec2::new(alto * w as f32 / h as f32, alto))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iconos_de_16x16() {
        for i in [Icon::Camara, Icon::Sol, Icon::Cubo, Icon::Toro, Icon::Modelo, Icon::Cerrar] {
            let p = rgba(i);
            assert_eq!(p.len(), 16 * 16 * 4);
            assert!(p.chunks(4).any(|c| c[3] == 255), "{i:?} vacío");
        }
    }
}
