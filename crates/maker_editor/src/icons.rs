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
    Personaje,
    Biblioteca,
    Ojo,
    OjoCerrado,
    Escena,
    Script,
    Archivo,
    Carpeta,
    Lienzo,
    PanelUi,
    Imagen,
    Texto,
    Boton,
    Barra,
    Deslizador,
    Casilla,
    Campo,
    Mascara,
    Contenedor,
    Personalizado,
    Estilo,
    Tema,
    Plantilla,
    Fuente,
    /// Escena de arranque (primera del orden de escenas) y Play desde ella.
    Inicio,
    EstiloVisual,
    /// Modo Esculpir y pinceles.
    Esculpir,
    PincelEstandar,
    PincelArcilla,
    PincelSuavizar,
    PincelMover,
    PincelInflar,
    PincelPliegue,
    PincelAplanar,
    PincelPellizcar,
    PincelPliegueFino,
    PincelBloque,
    PincelRecortar,
    PincelCapa,
    PincelEmpujar,
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
            Icon::Personaje => &px::PERSONAJE,
            Icon::Biblioteca => &px::BIBLIOTECA,
            Icon::Ojo => &px::OJO,
            Icon::OjoCerrado => &px::OJO_CERRADO,
            Icon::Escena => &px::ESCENA,
            Icon::Script => &px::SCRIPT,
            Icon::Archivo => &px::ARCHIVO,
            Icon::Carpeta => &px::CARPETA,
            Icon::Lienzo => &px::LIENZO,
            Icon::PanelUi => &px::PANEL_UI,
            Icon::Imagen => &px::IMAGEN,
            Icon::Texto => &px::TEXTO,
            Icon::Boton => &px::BOTON,
            Icon::Barra => &px::BARRA,
            Icon::Deslizador => &px::DESLIZADOR,
            Icon::Casilla => &px::CASILLA,
            Icon::Campo => &px::CAMPO,
            Icon::Mascara => &px::MASCARA,
            Icon::Contenedor => &px::CONTENEDOR,
            Icon::Personalizado => &px::PERSONALIZADO,
            Icon::Estilo => &px::ESTILO,
            Icon::Tema => &px::TEMA,
            Icon::Plantilla => &px::PLANTILLA,
            Icon::Fuente => &px::FUENTE,
            Icon::Inicio => &px::INICIO,
            Icon::EstiloVisual => &px::ESTILO_VISUAL,
            Icon::Esculpir => &px::ESCULPIR,
            Icon::PincelEstandar => &px::PINCEL_ESTANDAR,
            Icon::PincelArcilla => &px::PINCEL_ARCILLA,
            Icon::PincelSuavizar => &px::PINCEL_SUAVIZAR,
            Icon::PincelMover => &px::PINCEL_MOVER,
            Icon::PincelInflar => &px::PINCEL_INFLAR,
            Icon::PincelPliegue => &px::PINCEL_PLIEGUE,
            Icon::PincelAplanar => &px::PINCEL_APLANAR,
            Icon::PincelPellizcar => &px::PINCEL_PELLIZCAR,
            Icon::PincelPliegueFino => &px::PINCEL_PLIEGUE_FINO,
            Icon::PincelBloque => &px::PINCEL_BLOQUE,
            Icon::PincelRecortar => &px::PINCEL_RECORTAR,
            Icon::PincelCapa => &px::PINCEL_CAPA,
            Icon::PincelEmpujar => &px::PINCEL_EMPUJAR,
        }
    }
}

impl Icon {
    /// Icono de un pincel de esculpir.
    pub fn pincel(b: maker_sculpt::Brush) -> Icon {
        use maker_sculpt::Brush as B;
        match b {
            B::Standard => Icon::PincelEstandar,
            B::Clay => Icon::PincelArcilla,
            B::Smooth => Icon::PincelSuavizar,
            B::Move => Icon::PincelMover,
            B::Inflate => Icon::PincelInflar,
            B::Crease => Icon::PincelPliegue,
            B::Flatten => Icon::PincelAplanar,
            B::Pinch => Icon::PincelPellizcar,
            B::SharpCrease => Icon::PincelPliegueFino,
            B::ClayBlock => Icon::PincelBloque,
            B::Trim => Icon::PincelRecortar,
            B::Layer => Icon::PincelCapa,
            B::Nudge => Icon::PincelEmpujar,
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
        let pinceles = maker_sculpt::Brush::ALL.map(Icon::pincel);
        for i in [Icon::Camara, Icon::Sol, Icon::Cubo, Icon::Toro, Icon::Modelo, Icon::Cerrar, Icon::Personaje, Icon::Biblioteca, Icon::Esculpir].into_iter().chain(pinceles) {
            let p = rgba(i);
            assert_eq!(p.len(), 16 * 16 * 4);
            assert!(p.chunks(4).any(|c| c[3] == 255), "{i:?} vacío");
        }
    }
}
