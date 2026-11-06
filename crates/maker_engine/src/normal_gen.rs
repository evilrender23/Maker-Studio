//! Normal maps generados a partir del color base, como las herramientas
//! clásicas de altura → normal: la altura es la luminancia del albedo
//! (opcionalmente invertida y desenfocada) y la pendiente sale de un filtro de
//! Sobel. El resultado sigue la convención OpenGL de glTF (verde = arriba en
//! la imagen) y se guarda en PNG sin pérdidas.
//!
//! Dos usos: el editor escribe el PNG junto a la textura de origen (o en
//! `assets/generados/` si el origen es de la biblioteca, que es de solo
//! lectura) y el renderizador lo genera en memoria en el modo automático.
//! Cada PNG lleva en un bloque `tEXt` la clave (hash del origen + parámetros):
//! si no ha cambiado nada no se vuelve a calcular.

use crate::components::{MeshSource, NormalGen, NormalSource};
use crate::model::{ModelData, TextureSource};
use crate::scene::SceneFile;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Sufijo de los archivos generados (`ladrillo.jpg` → `ladrillo_normal_gen.png`).
pub const SUFFIX: &str = "_normal_gen.png";
/// Carpeta del proyecto para los generados a partir de la biblioteca.
pub const GENERATED_DIR: &str = "assets/generados";
const KEY_TEXT: &str = "maker-normal";
/// Sube si cambia el algoritmo (invalida los PNG ya generados).
const VERSION: u64 = 1;

/// FNV-1a de 64 bits (estable entre ejecuciones y plataformas).
pub fn hash_bytes(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Clave de caché: contenido del origen + parámetros + versión.
pub fn cache_key(source_hash: u64, p: &NormalGen) -> u64 {
    let mut buf = Vec::with_capacity(28);
    buf.extend_from_slice(&source_hash.to_le_bytes());
    for b in p.bits() {
        buf.extend_from_slice(&b.to_le_bytes());
    }
    buf.extend_from_slice(&VERSION.to_le_bytes());
    hash_bytes(&buf)
}

/// Normal map RGBA8 (lineal, OpenGL) a partir de una imagen RGBA8 sRGB.
/// Los bordes se tratan como repetidos (texturas que se repiten).
pub fn generate(w: u32, h: u32, rgba: &[u8], p: &NormalGen) -> Vec<u8> {
    let (w, h) = (w as usize, h as usize);
    let n = w * h;
    let mut height: Vec<f32> = rgba.chunks_exact(4).take(n).map(|c| (0.2126 * c[0] as f32 + 0.7152 * c[1] as f32 + 0.0722 * c[2] as f32) / 255.0).collect();
    if p.invert {
        for v in &mut height {
            *v = 1.0 - *v;
        }
    }
    let r = p.radius.max(0.0).round() as usize;
    if r > 0 {
        // Dos cajas separables ≈ gaussiana.
        let mut tmp = vec![0.0f32; n];
        for _ in 0..2 {
            box_blur(&height, &mut tmp, w, h, r, true);
            box_blur(&tmp, &mut height, w, h, r, false);
        }
    }
    // Pendiente por píxel (Sobel / 8) escalada al tamaño: el mismo relieve en
    // UV con la textura a 1024 o a 2048 px.
    let k = p.strength * (w.max(h) as f32 / 512.0) / 8.0;
    let mut out = vec![255u8; n * 4];
    let at = |x: isize, y: isize| -> f32 {
        let xx = x.rem_euclid(w as isize) as usize;
        let yy = y.rem_euclid(h as isize) as usize;
        height[yy * w + xx]
    };
    for y in 0..h as isize {
        for x in 0..w as isize {
            let gx = (at(x + 1, y - 1) + 2.0 * at(x + 1, y) + at(x + 1, y + 1)) - (at(x - 1, y - 1) + 2.0 * at(x - 1, y) + at(x - 1, y + 1));
            // y hacia abajo en la imagen.
            let gy = (at(x - 1, y + 1) + 2.0 * at(x, y + 1) + at(x + 1, y + 1)) - (at(x - 1, y - 1) + 2.0 * at(x, y - 1) + at(x + 1, y - 1));
            // Superficie z = altura: normal (−∂h/∂x, −∂h/∂y_arriba, 1); y_arriba = −y_imagen.
            let (nx, ny, nz) = (-gx * k, gy * k, 1.0f32);
            let len = (nx * nx + ny * ny + nz * nz).sqrt();
            let o = (y as usize * w + x as usize) * 4;
            out[o] = ((nx / len * 0.5 + 0.5) * 255.0).round() as u8;
            out[o + 1] = ((ny / len * 0.5 + 0.5) * 255.0).round() as u8;
            out[o + 2] = ((nz / len * 0.5 + 0.5) * 255.0).round() as u8;
        }
    }
    out
}

/// Media móvil de radio `r` en horizontal o vertical, con repetición.
fn box_blur(src: &[f32], dst: &mut [f32], w: usize, h: usize, r: usize, horizontal: bool) {
    let (len, lines) = if horizontal { (w, h) } else { (h, w) };
    let idx = |line: usize, i: usize| if horizontal { line * w + i } else { i * w + line };
    let inv = 1.0 / (2 * r + 1) as f32;
    for line in 0..lines {
        let wrap = |i: isize| i.rem_euclid(len as isize) as usize;
        let mut sum = 0.0;
        for i in -(r as isize)..=(r as isize) {
            sum += src[idx(line, wrap(i))];
        }
        for i in 0..len {
            dst[idx(line, i)] = sum * inv;
            sum += src[idx(line, wrap(i as isize + r as isize + 1))] - src[idx(line, wrap(i as isize - r as isize))];
        }
    }
}

/// Bytes de la imagen de origen.
pub fn read_source(base: &TextureSource) -> Result<Vec<u8>, String> {
    match base {
        TextureSource::File(p) => std::fs::read(p).map_err(|e| format!("{}: {e}", p.display())),
        TextureSource::Embedded { bytes, .. } => Ok(bytes.as_ref().clone()),
    }
}

/// Normal map en memoria (modo automático): (ancho, alto, RGBA8).
pub fn generate_from(base: &TextureSource, p: &NormalGen) -> Result<(u32, u32, Vec<u8>), String> {
    let bytes = read_source(base)?;
    let img = image::load_from_memory(&bytes).map_err(|e| format!("{}: {e}", base.key()))?.to_rgba8();
    let (w, h) = img.dimensions();
    Ok((w, h, generate(w, h, img.as_raw(), p)))
}

/// Dónde se escribe el normal generado de una textura de color: junto a ella
/// si es del proyecto; en `assets/generados/<pack>/` si es de la biblioteca;
/// en `assets/generados/` si está en otro sitio. Las imágenes incrustadas en
/// un modelo usan el nombre del modelo y el índice de la imagen.
pub fn output_path(base: &TextureSource, project_root: &Path) -> PathBuf {
    output_path_in(base, project_root, crate::biblioteca::library_dir().as_deref())
}

pub fn output_path_in(base: &TextureSource, project_root: &Path, library: Option<&Path>) -> PathBuf {
    let (file, index) = match base {
        TextureSource::File(p) => (p.clone(), None),
        TextureSource::Embedded { key, .. } => match key.rsplit_once('#') {
            Some((model, i)) => (PathBuf::from(model), Some(i.to_string())),
            None => (PathBuf::from(key), None),
        },
    };
    let stem = file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "textura".into());
    let name = match index {
        Some(i) => format!("{stem}_{i}{SUFFIX}"),
        None => format!("{stem}{SUFFIX}"),
    };
    if let Some(rel) = library.and_then(|l| file.strip_prefix(l).ok()) {
        let pack = rel.components().next().map(|c| c.as_os_str().to_string_lossy().into_owned()).filter(|_| rel.components().count() > 1);
        let dir = project_root.join(GENERATED_DIR);
        return match pack {
            Some(p) => dir.join(p).join(name),
            None => dir.join(name),
        };
    }
    if file.starts_with(project_root)
        && let Some(dir) = file.parent()
    {
        return dir.join(name);
    }
    project_root.join(GENERATED_DIR).join(name)
}

/// Clave guardada en un PNG generado (None si no existe o no es nuestro).
pub fn read_key(path: &Path) -> Option<u64> {
    let f = std::fs::File::open(path).ok()?;
    let reader = png::Decoder::new(std::io::BufReader::new(f)).read_info().ok()?;
    let info = reader.info();
    info.uncompressed_latin1_text.iter().find(|t| t.keyword == KEY_TEXT).and_then(|t| u64::from_str_radix(&t.text, 16).ok())
}

/// Escribe un normal map RGB en PNG con su clave.
pub fn write_png(path: &Path, w: u32, h: u32, rgba: &[u8], key: u64) -> Result<(), String> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    let rgb: Vec<u8> = rgba.chunks_exact(4).flat_map(|c| [c[0], c[1], c[2]]).collect();
    // Se escribe a un temporal y se renombra: nunca queda un PNG a medias que
    // el renderizador pudiera leer.
    let tmp = path.with_extension("png.tmp");
    let file = std::fs::File::create(&tmp).map_err(|e| format!("{}: {e}", tmp.display()))?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    enc.add_text_chunk(KEY_TEXT.to_string(), format!("{key:016x}")).map_err(|e| e.to_string())?;
    let mut wr = enc.write_header().map_err(|e| e.to_string())?;
    wr.write_image_data(&rgb).map_err(|e| e.to_string())?;
    wr.finish().map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
}

/// Genera el PNG de `base` en `out` salvo que ya esté al día (misma clave).
/// Devuelve si escribió algo.
pub fn generate_file(base: &TextureSource, out: &Path, p: &NormalGen) -> Result<bool, String> {
    let bytes = read_source(base)?;
    let key = cache_key(hash_bytes(&bytes), p);
    if read_key(out) == Some(key) {
        return Ok(false);
    }
    let img = image::load_from_memory(&bytes).map_err(|e| format!("{}: {e}", base.key()))?.to_rgba8();
    let (w, h) = img.dimensions();
    write_png(out, w, h, &generate(w, h, img.as_raw(), p), key)?;
    Ok(true)
}

/// Texturas de color de un modelo que no traen normal map (sin repetir).
pub fn missing_normals(model: &ModelData) -> Vec<TextureSource> {
    let mut out: Vec<TextureSource> = Vec::new();
    for s in &model.submeshes {
        let m = &model.materials[s.material];
        if m.normal.is_none()
            && let Some(t) = &m.texture
            && !out.contains(t)
        {
            out.push(t.clone());
        }
    }
    out
}

/// Normal maps que usan las escenas y hay que llevar al juego exportado:
/// texturas elegidas a mano y los PNG generados (que existan). Devuelve rutas
/// del proyecto o `biblioteca://…`.
pub fn files_used(scenes: &[SceneFile], project_root: &Path) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut generated_models = BTreeSet::new();
    for f in scenes {
        for r in &f.entities {
            let Some(mr) = &r.mesh else { continue };
            let n = &mr.material.normal;
            match n.source {
                NormalSource::Texture if !n.texture.is_empty() => {
                    out.insert(n.texture.clone());
                }
                NormalSource::Generated => {
                    if let MeshSource::Model(m) = &mr.mesh {
                        generated_models.insert(m.clone());
                    }
                }
                _ => {}
            }
        }
    }
    for m in generated_models {
        let Ok(model) = ModelData::load(&crate::biblioteca::resolve(&m, Some(project_root))) else { continue };
        for base in missing_normals(&model) {
            let p = output_path(&base, project_root);
            if p.is_file()
                && let Ok(rel) = p.strip_prefix(project_root)
            {
                out.insert(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(px: &[u8], w: usize, x: usize, y: usize) -> [f32; 3] {
        let o = (y * w + x) * 4;
        [0, 1, 2].map(|c| px[o + c] as f32 / 255.0 * 2.0 - 1.0)
    }

    /// Un cuadrado claro sobre fondo oscuro es un relieve: en su borde
    /// izquierdo la normal mira a la izquierda (−X) y en el de arriba hacia
    /// arriba de la imagen (+Y, verde OpenGL). Invertir lo hunde.
    #[test]
    fn bright_square_bulges_with_opengl_green() {
        let (w, h) = (256usize, 256usize);
        let mut img = vec![0u8; w * h * 4];
        for y in 0..h {
            for x in 0..w {
                let v = if (64..192).contains(&x) && (64..192).contains(&y) { 255 } else { 20 };
                img[(y * w + x) * 4..][..4].copy_from_slice(&[v, v, v, 255]);
            }
        }
        let p = NormalGen { strength: 2.0, radius: 0.0, invert: false };
        let out = generate(w as u32, h as u32, &img, &p);
        let left = decode(&out, w, 64, 128);
        let top = decode(&out, w, 128, 64);
        let flat = decode(&out, w, 128, 128);
        assert!(left[0] < -0.3 && left[1].abs() < 0.05, "borde izquierdo: {left:?}");
        assert!(top[1] > 0.3 && top[0].abs() < 0.05, "borde superior: {top:?}");
        assert!(flat[2] > 0.99, "centro plano: {flat:?}");
        let inv = generate(w as u32, h as u32, &img, &NormalGen { invert: true, ..p });
        assert!(decode(&inv, w, 64, 128)[0] > 0.3);
        // Más fuerza, más inclinación; el desenfoque la reparte y la suaviza.
        let strong = generate(w as u32, h as u32, &img, &NormalGen { strength: 6.0, ..p });
        assert!(decode(&strong, w, 64, 128)[2] < left[2]);
        let soft = generate(w as u32, h as u32, &img, &NormalGen { radius: 3.0, ..p });
        assert!(decode(&soft, w, 64, 128)[2] > left[2] && decode(&soft, w, 61, 128)[0] < -0.05);
        // Todas las normales unitarias.
        for c in out.chunks_exact(4) {
            let v = [0, 1, 2].map(|i| c[i] as f32 / 255.0 * 2.0 - 1.0);
            assert!(((v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt() - 1.0).abs() < 0.02);
        }
    }

    #[test]
    fn output_paths_next_to_source_or_in_generated_dir() {
        let root = Path::new("/proj");
        let lib = Path::new("/motor/biblioteca");
        let f = |p: &str| TextureSource::File(PathBuf::from(p));
        assert_eq!(output_path_in(&f("/proj/assets/tex/ladrillo.jpg"), root, Some(lib)), Path::new("/proj/assets/tex/ladrillo_normal_gen.png"));
        assert_eq!(output_path_in(&f("/motor/biblioteca/naturaleza/texturas/Leaves.png"), root, Some(lib)), Path::new("/proj/assets/generados/naturaleza/Leaves_normal_gen.png"));
        assert_eq!(output_path_in(&f("/otro/sitio/x.png"), root, Some(lib)), Path::new("/proj/assets/generados/x_normal_gen.png"));
        let emb = TextureSource::Embedded { key: "/proj/assets/m/robot.glb#2".into(), bytes: Default::default() };
        assert_eq!(output_path_in(&emb, root, Some(lib)), Path::new("/proj/assets/m/robot_2_normal_gen.png"));
    }

    /// El PNG lleva su clave: con el mismo origen y parámetros no se rehace;
    /// al cambiar un parámetro o el origen, sí.
    #[test]
    fn generated_file_is_cached_by_source_and_params() {
        let dir = std::env::temp_dir().join(format!("maker-normal-gen-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("color.png");
        let mut img = image::RgbaImage::new(16, 16);
        for (x, y, p) in img.enumerate_pixels_mut() {
            *p = image::Rgba([(x * 16) as u8, (y * 16) as u8, 90, 255]);
        }
        img.save(&src).unwrap();
        let base = TextureSource::File(src.clone());
        let out = output_path_in(&base, &dir, None);
        assert_eq!(out, dir.join("color_normal_gen.png"));
        let p = NormalGen::default();
        assert!(generate_file(&base, &out, &p).unwrap(), "primera vez: se escribe");
        assert!(!generate_file(&base, &out, &p).unwrap(), "misma clave: caché");
        let decoded = image::open(&out).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (16, 16));
        assert!(generate_file(&base, &out, &NormalGen { radius: 2.0, ..p }).unwrap(), "otros parámetros");
        let mut img2 = img.clone();
        img2.put_pixel(3, 3, image::Rgba([255, 0, 0, 255]));
        img2.save(&src).unwrap();
        assert!(generate_file(&base, &out, &NormalGen { radius: 2.0, ..p }).unwrap(), "otro origen");
        assert!(read_key(&src).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
