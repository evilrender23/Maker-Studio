//! Texturas de color base: decodificación (PNG/JPEG), mipmaps en espacio
//! lineal y conservación de la cobertura alfa para los recortes (las hojas no
//! se adelgazan con la distancia).

/// Niveles de mip RGBA8 sRGB, del mayor al de 1×1.
pub struct MipChain {
    pub levels: Vec<(u32, u32, Vec<u8>)>,
    /// ¿Hay algún texel no opaco?
    pub has_alpha: bool,
}

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

fn linear_to_srgb(c: f32) -> f32 {
    if c <= 0.003_130_8 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 }
}

struct Luts {
    to_lin: [f32; 256],
    /// lineal (0..=4095) → sRGB u8.
    to_srgb: Vec<u8>,
}

fn luts() -> &'static Luts {
    static L: std::sync::OnceLock<Luts> = std::sync::OnceLock::new();
    L.get_or_init(|| {
        let mut to_lin = [0.0; 256];
        for (i, v) in to_lin.iter_mut().enumerate() {
            *v = srgb_to_linear(i as f32 / 255.0);
        }
        let to_srgb = (0..4096).map(|i| (linear_to_srgb(i as f32 / 4095.0) * 255.0).round() as u8).collect();
        Luts { to_lin, to_srgb }
    })
}

/// Fracción de texels con alfa ≥ `cutoff` (escalando el alfa por `scale`).
fn coverage(px: &[u8], cutoff: f32, scale: f32) -> f32 {
    let n = px.len() / 4;
    if n == 0 {
        return 0.0;
    }
    let lim = cutoff * 255.0;
    px.chunks_exact(4).filter(|p| p[3] as f32 * scale >= lim).count() as f32 / n as f32
}

/// Medio tamaño con filtro de caja 2×2: color en lineal ponderado por alfa.
fn downsample(w: u32, h: u32, src: &[u8]) -> (u32, u32, Vec<u8>) {
    let l = luts();
    let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
    let mut out = vec![0u8; (nw * nh * 4) as usize];
    for y in 0..nh {
        for x in 0..nw {
            let mut acc = [0.0f32; 4];
            let mut wsum = 0.0;
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let sx = (x * 2 + dx).min(w - 1);
                let sy = (y * 2 + dy).min(h - 1);
                let i = ((sy * w + sx) * 4) as usize;
                let a = src[i + 3] as f32 / 255.0;
                let wt = a.max(1.0 / 255.0);
                for c in 0..3 {
                    acc[c] += l.to_lin[src[i + c] as usize] * wt;
                }
                acc[3] += a;
                wsum += wt;
            }
            let o = ((y * nw + x) * 4) as usize;
            for c in 0..3 {
                let v = (acc[c] / wsum).clamp(0.0, 1.0);
                out[o + c] = l.to_srgb[(v * 4095.0) as usize];
            }
            out[o + 3] = ((acc[3] / 4.0) * 255.0).round() as u8;
        }
    }
    (nw, nh, out)
}

/// Cadena de mips completa. Con `cutoff` (material con recorte) cada nivel
/// escala su alfa para conservar la cobertura del nivel 0.
pub fn build_mips(w: u32, h: u32, rgba: Vec<u8>, cutoff: Option<f32>) -> MipChain {
    let has_alpha = rgba.chunks_exact(4).any(|p| p[3] < 255);
    let target = cutoff.filter(|_| has_alpha).map(|c| (c, coverage(&rgba, c, 1.0)));
    let mut levels = vec![(w, h, rgba)];
    loop {
        let (lw, lh, data) = levels.last().unwrap();
        if *lw == 1 && *lh == 1 {
            break;
        }
        let (nw, nh, mut next) = downsample(*lw, *lh, data);
        if let Some((c, want)) = target {
            // Búsqueda binaria del factor que iguala la cobertura.
            let (mut lo, mut hi) = (1.0f32, 8.0f32);
            for _ in 0..12 {
                let mid = (lo + hi) * 0.5;
                if coverage(&next, c, mid) < want { lo = mid } else { hi = mid }
            }
            if hi > 1.0001 {
                for p in next.chunks_exact_mut(4) {
                    p[3] = (p[3] as f32 * hi).ceil().min(255.0) as u8;
                }
            }
        }
        levels.push((nw, nh, next));
    }
    MipChain { levels, has_alpha }
}

/// Decodifica PNG/JPEG a RGBA8.
pub fn decode(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    let img = image::load_from_memory(bytes).map_err(|e| e.to_string())?.to_rgba8();
    let (w, h) = img.dimensions();
    Ok((w, h, img.into_raw()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mip_chain_reaches_one_texel_and_averages_in_linear() {
        // 4×2: mitad negra, mitad blanca, opaco.
        let mut px = Vec::new();
        for _y in 0..2 {
            for x in 0..4 {
                let v = if x < 2 { 0 } else { 255 };
                px.extend_from_slice(&[v, v, v, 255]);
            }
        }
        let m = build_mips(4, 2, px, None);
        let dims: Vec<(u32, u32)> = m.levels.iter().map(|l| (l.0, l.1)).collect();
        assert_eq!(dims, vec![(4, 2), (2, 1), (1, 1)]);
        assert!(!m.has_alpha);
        // Media lineal de negro y blanco = 0.5 → sRGB ≈ 188 (no 128).
        let last = &m.levels[2].2;
        assert!((last[0] as i32 - 188).abs() <= 1, "{last:?}");
        assert_eq!(last[3], 255);
    }

    #[test]
    fn alpha_coverage_is_preserved_for_cutouts() {
        // Rejilla de hojas: un texel opaco de cada 2×2 → cobertura 25 %.
        let (w, h) = (16u32, 16u32);
        let mut px = vec![0u8; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                let i = ((y * w + x) * 4) as usize;
                px[i..i + 3].copy_from_slice(&[40, 160, 40]);
                px[i + 3] = if x % 2 == 0 && y % 2 == 0 { 255 } else { 0 };
            }
        }
        let plain = build_mips(w, h, px.clone(), None);
        // Sin conservar, el nivel 1 queda con alfa 64 (< 0.5): desaparece.
        assert_eq!(coverage(&plain.levels[1].2, 0.5, 1.0), 0.0);
        let kept = build_mips(w, h, px, Some(0.5));
        assert!(kept.has_alpha);
        assert!(coverage(&kept.levels[1].2, 0.5, 1.0) > 0.2, "la cobertura se mantiene");
        // El color de los bordes no se oscurece con los texels transparentes.
        assert!(kept.levels[1].2[1] > 150);
    }
}
