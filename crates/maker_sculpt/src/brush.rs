//! Pinceles y sus ajustes.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Brush {
    Standard,
    Clay,
    Smooth,
    Move,
    Inflate,
    Crease,
    Flatten,
    Pinch,
    SharpCrease,
    ClayBlock,
    Trim,
    Layer,
    Nudge,
}

impl Brush {
    /// En el orden del estante; los diez primeros llevan las teclas 1..9 y 0.
    pub const ALL: [Brush; 13] = [
        Brush::Standard,
        Brush::Clay,
        Brush::Smooth,
        Brush::Move,
        Brush::Inflate,
        Brush::Crease,
        Brush::Flatten,
        Brush::Pinch,
        Brush::SharpCrease,
        Brush::ClayBlock,
        Brush::Trim,
        Brush::Layer,
        Brush::Nudge,
    ];
    pub const COUNT: usize = Self::ALL.len();

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|b| *b == self).unwrap_or(0)
    }

    /// Tecla numérica (1..9, 0) de los diez primeros.
    pub fn key(self) -> Option<u8> {
        match self.index() {
            i @ 0..=8 => Some(i as u8 + 1),
            9 => Some(0),
            _ => None,
        }
    }

    /// Pincel de una tecla numérica.
    pub fn from_key(k: u8) -> Option<Brush> {
        let i = if k == 0 { 9 } else { k as usize - 1 };
        Self::ALL.get(i).copied().filter(|_| k <= 9)
    }

    pub fn name(self) -> &'static str {
        match self {
            Brush::Standard => "Estándar",
            Brush::Clay => "Arcilla",
            Brush::Smooth => "Suavizar",
            Brush::Move => "Mover",
            Brush::Inflate => "Inflar",
            Brush::Crease => "Pliegue",
            Brush::Flatten => "Aplanar",
            Brush::Pinch => "Pellizcar",
            Brush::SharpCrease => "Pliegue fino",
            Brush::ClayBlock => "Bloque",
            Brush::Trim => "Recortar",
            Brush::Layer => "Capa",
            Brush::Nudge => "Empujar",
        }
    }

    /// Qué hace (para la ayuda emergente).
    pub fn tooltip(self) -> &'static str {
        match self {
            Brush::Standard => "Levanta la superficie a lo largo del trazo. Alt: hunde.",
            Brush::Clay => "Añade arcilla que rellena primero los huecos, con el tope algo plano. Alt: quita.",
            Brush::Smooth => "Relaja la superficie (también con Mayús). Con «Volumen» no encoge la forma.",
            Brush::Move => "Agarra la superficie y la arrastra paralela a la pantalla. Alt: a lo largo de la normal.",
            Brush::Inflate => "Hincha la superficie a lo largo de sus normales. Alt: deshincha.",
            Brush::Crease => "Marca un pliegue y pellizca sus lados. Alt: levanta una cresta.",
            Brush::Flatten => "Aplana hacia el plano de la zona. Alt: aleja del plano.",
            Brush::Pinch => "Atrae la superficie hacia el trazo para afilar aristas. Alt: separa.",
            Brush::SharpCrease => "Pliegue estrecho y profundo en V. Alt: cresta afilada.",
            Brush::ClayBlock => "Arcilla de punta cuadrada y tope plano para encajar formas. Alt: talla plano.",
            Brush::Trim => "Rasura todo lo que sobresale de un plano. Alt: rellena hasta el plano.",
            Brush::Layer => "Sube hasta una altura fija por trazo; las pasadas no se acumulan. Alt: baja.",
            Brush::Nudge => "Desliza la superficie de lado, a lo largo del trazo, sin levantarla.",
        }
    }

    /// El pincel no pone *dabs*: arrastra lo que agarró al empezar.
    pub fn is_grab(self) -> bool {
        self == Brush::Move
    }

    /// Alt no invierte (no tiene sentido contrario).
    pub fn invertible(self) -> bool {
        !matches!(self, Brush::Smooth | Brush::Move | Brush::Nudge)
    }
}

/// Tamaño, intensidad y caída de un pincel.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct BrushSettings {
    /// Radio en píxeles de pantalla (2..800).
    pub size: f32,
    /// 1..100.
    pub intensity: f32,
    /// −100..100.
    pub falloff: f32,
}

pub const SIZE_RANGE: std::ops::RangeInclusive<f32> = 2.0..=800.0;
pub const INTENSITY_RANGE: std::ops::RangeInclusive<f32> = 1.0..=100.0;
pub const FALLOFF_RANGE: std::ops::RangeInclusive<f32> = -100.0..=100.0;

impl Default for BrushSettings {
    fn default() -> Self {
        Self { size: 60.0, intensity: 50.0, falloff: 0.0 }
    }
}

impl BrushSettings {
    pub fn clamped(self) -> Self {
        Self {
            size: self.size.clamp(*SIZE_RANGE.start(), *SIZE_RANGE.end()),
            intensity: self.intensity.clamp(*INTENSITY_RANGE.start(), *INTENSITY_RANGE.end()),
            falloff: self.falloff.clamp(*FALLOFF_RANGE.start(), *FALLOFF_RANGE.end()),
        }
    }
}

/// Fuerza de cada pincel por *dab* (fracción del radio o de la distancia al objetivo, a
/// intensidad 100 y presión 1). Ajustada para que una pasada a intensidad 50 con el
/// espaciado por defecto levante del orden de una décima del radio.
pub(crate) mod k {
    pub const STANDARD: f32 = 0.04;
    pub const INFLATE: f32 = 0.03;
    pub const CLAY_HEIGHT: f32 = 0.08;
    pub const CLAY_RATE: f32 = 0.5;
    pub const CLAY_BLOCK_RATE: f32 = 0.8;
    pub const FLATTEN: f32 = 0.25;
    pub const TRIM_OFFSET: f32 = 0.03;
    pub const TRIM_RATE: f32 = 0.5;
    pub const PINCH: f32 = 0.15;
    pub const CREASE_DEPTH: f32 = 0.03;
    pub const CREASE_PINCH: f32 = 0.12;
    pub const SHARP_DEPTH: f32 = 0.045;
    pub const SHARP_PINCH: f32 = 0.25;
    pub const LAYER: f32 = 0.25;
    pub const NUDGE: f32 = 0.5;
    pub const SMOOTH: f32 = 0.5;
    pub const MASK: f32 = 0.5;
    /// Taubin: el paso de vuelta es algo mayor que el de ida (μ = −1,06·λ).
    pub const TAUBIN_MU: f32 = 1.06;
    pub const TAUBIN_LAMBDA: f32 = 0.6;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn number_keys_cover_first_ten() {
        for (i, b) in Brush::ALL.iter().enumerate() {
            match b.key() {
                Some(k) => {
                    assert!(i < 10);
                    assert_eq!(Brush::from_key(k), Some(*b));
                }
                None => assert!(i >= 10),
            }
        }
        assert_eq!(Brush::from_key(1), Some(Brush::Standard));
        assert_eq!(Brush::from_key(0), Some(Brush::ClayBlock));
        assert_eq!(Brush::from_key(4), Some(Brush::Move));
    }
}
