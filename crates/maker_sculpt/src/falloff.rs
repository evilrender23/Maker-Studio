//! Curva de caída del pincel, con un parámetro de −100 a 100.
//!
//! La base es la caída suave clásica `1 − (3x² − 2x³)` sobre la distancia normalizada
//! `x = d / r`. Con el parámetro negativo una parte central crece a plena fuerza y el borde se
//! comprime (punta ancha y dura); con el positivo la curva se eleva a una potencia y queda una
//! punta estrecha y blanda.

/// Muestras de la tabla (más una para interpolar el último tramo).
const LUT: usize = 256;

/// Caída en `x = d / r` (0 en el centro, 1 en el borde) con el parámetro `f` (−100..100).
pub fn falloff(x: f32, f: f32) -> f32 {
    if !(x < 1.0) {
        return 0.0;
    }
    let x = x.max(0.0);
    let k = (f / 100.0).clamp(-1.0, 1.0);
    let smooth = |t: f32| 1.0 - t * t * (3.0 - 2.0 * t);
    if k < 0.0 {
        let core = -k * 0.9;
        let t = if x <= core { 0.0 } else { (x - core) / (1.0 - core) };
        smooth(t)
    } else {
        smooth(x).powf(1.0 + 4.0 * k)
    }
}

/// Radio relativo (0..1) en el que la caída vale la mitad: el anillo interior del cursor.
pub fn half_radius(f: f32) -> f32 {
    let (mut lo, mut hi) = (0.0f32, 1.0f32);
    for _ in 0..30 {
        let m = (lo + hi) * 0.5;
        if falloff(m, f) > 0.5 { lo = m } else { hi = m }
    }
    (lo + hi) * 0.5
}

/// Tabla de la curva para un *dab* (evita `powf` por vértice).
#[derive(Clone)]
pub struct FalloffLut {
    t: [f32; LUT + 1],
}

impl FalloffLut {
    pub fn new(f: f32) -> Self {
        let mut t = [0.0; LUT + 1];
        for (i, v) in t.iter_mut().enumerate() {
            *v = falloff(i as f32 / LUT as f32, f);
        }
        t[LUT] = 0.0;
        Self { t }
    }
    /// Caída en `x = d / r`.
    #[inline]
    pub fn at(&self, x: f32) -> f32 {
        if !(x < 1.0) {
            return 0.0;
        }
        let s = x.max(0.0) * LUT as f32;
        let i = s as usize;
        let fr = s - i as f32;
        self.t[i] + (self.t[i + 1] - self.t[i]) * fr
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_curve_is_smooth_and_bounded() {
        assert_eq!(falloff(0.0, 0.0), 1.0);
        assert_eq!(falloff(1.0, 0.0), 0.0);
        assert!((falloff(0.5, 0.0) - 0.5).abs() < 1e-6);
        let mut prev = 1.0;
        for i in 0..=100 {
            let v = falloff(i as f32 / 100.0, 0.0);
            assert!(v <= prev + 1e-6 && (0.0..=1.0).contains(&v));
            prev = v;
        }
    }

    #[test]
    fn negative_widens_core_positive_softens_point() {
        // Con −100 casi todo el radio va a plena fuerza; con +100 la mitad del radio ya es débil.
        assert_eq!(falloff(0.5, -100.0), 1.0);
        assert!(falloff(0.5, 100.0) < 0.1);
        assert!(half_radius(-100.0) > half_radius(0.0));
        assert!(half_radius(100.0) < half_radius(0.0));
        assert!((half_radius(0.0) - 0.5).abs() < 1e-3);
    }

    #[test]
    fn lut_matches_curve() {
        for f in [-100.0, -40.0, 0.0, 35.0, 100.0] {
            let l = FalloffLut::new(f);
            for i in 0..=200 {
                let x = i as f32 / 200.0;
                assert!((l.at(x) - falloff(x, f)).abs() < 0.02, "f={f} x={x}");
            }
            assert_eq!(l.at(1.0), 0.0);
            assert_eq!(l.at(2.0), 0.0);
        }
    }
}
