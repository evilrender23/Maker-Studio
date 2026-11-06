//! Archivos adjuntos a un mensaje del agente: se añaden con «Adjuntar» (archivos del proyecto o
//! una ruta), soltándolos desde el sistema o desde Contenido/Biblioteca, o pegando una imagen.
//! Se pasan por ruta: la CLI los lee con sus propias herramientas (las imágenes también como
//! imagen en Codex, `-i`), y las carpetas de fuera del proyecto se le abren con `--add-dir`.

use std::path::{Path, PathBuf};

const IMAGENES: [&str; 6] = ["png", "jpg", "jpeg", "webp", "gif", "bmp"];

pub fn es_imagen(p: &Path) -> bool {
    p.extension().and_then(|e| e.to_str()).is_some_and(|e| IMAGENES.contains(&e.to_ascii_lowercase().as_str()))
}

/// Ruta escrita o pegada por el usuario: comillas, `file://` y `~` fuera.
pub fn normalizar(texto: &str, raiz: &Path) -> Option<PathBuf> {
    let t = texto.trim().trim_matches(['"', '\'']).trim();
    let t = t.strip_prefix("file://").unwrap_or(t);
    if t.is_empty() {
        return None;
    }
    let p = if let Some(r) = t.strip_prefix("~/") {
        PathBuf::from(std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?).join(r)
    } else {
        PathBuf::from(t)
    };
    let p = if p.is_relative() { raiz.join(p) } else { p };
    p.exists().then_some(p)
}

/// Carpetas que la CLI necesita además del proyecto para leer los adjuntos.
pub fn carpetas_extra(adjuntos: &[PathBuf], raiz: &Path) -> Vec<PathBuf> {
    let raiz = raiz.canonicalize().unwrap_or_else(|_| raiz.to_path_buf());
    let mut v: Vec<PathBuf> = Vec::new();
    for a in adjuntos {
        let a = a.canonicalize().unwrap_or_else(|_| a.clone());
        if a.starts_with(&raiz) {
            continue;
        }
        let d = if a.is_dir() { a } else { a.parent().map(Path::to_path_buf).unwrap_or(a) };
        if !v.contains(&d) {
            v.push(d);
        }
    }
    v
}

/// Bloque que se añade al mensaje con la lista de adjuntos.
pub fn bloque(adjuntos: &[PathBuf], raiz: &Path) -> String {
    let mut s = String::from("<archivos_adjuntos>\nEl usuario adjunta estos archivos; léelos antes de responder:\n");
    for a in adjuntos {
        let r = a.strip_prefix(raiz).map(|r| r.to_string_lossy().replace('\\', "/")).unwrap_or_else(|_| a.display().to_string());
        let tipo = if a.is_dir() {
            " (carpeta)"
        } else if es_imagen(a) {
            " (imagen)"
        } else {
            ""
        };
        s.push_str(&format!("- {r}{tipo}\n"));
    }
    s.push_str("</archivos_adjuntos>");
    s
}

pub fn nombre(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.display().to_string())
}

/// Carpeta (fuera del proyecto) donde se guardan las imágenes pegadas; las de más de 7 días se borran.
fn carpeta_pegadas() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("LOCALAPPDATA").map(PathBuf::from))
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))?;
    let d = base.join("maker-studio").join("adjuntos");
    std::fs::create_dir_all(&d).ok()?;
    let semana = std::time::Duration::from_secs(7 * 24 * 3600);
    for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
        if e.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|t| t > semana) {
            let _ = std::fs::remove_file(e.path());
        }
    }
    Some(d)
}

/// Guarda una imagen pegada como PNG y devuelve su ruta.
pub fn guardar_imagen(img: &egui::ColorImage) -> Result<PathBuf, String> {
    let bytes: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_srgba_unmultiplied()).collect();
    guardar_rgba(img.size[0] as u32, img.size[1] as u32, &bytes, "pegada")
}

/// Guarda píxeles RGBA como PNG en la caché de adjuntos.
pub fn guardar_rgba(w: u32, h: u32, rgba: &[u8], prefijo: &str) -> Result<PathBuf, String> {
    let d = carpeta_pegadas().ok_or("no hay carpeta de caché")?;
    let ns = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let p = d.join(format!("{prefijo}-{ns}.png"));
    image::save_buffer(&p, rgba, w, h, image::ExtendedColorType::Rgba8).map_err(|e| e.to_string())?;
    Ok(p)
}

/// Resultado de una operación en segundo plano (selector de archivos, portapapeles).
pub type Entrante = Result<Vec<PathBuf>, String>;

/// Selector de archivos nativo para imágenes (varias a la vez), en otro hilo.
pub fn elegir_imagenes(tx: std::sync::mpsc::Sender<Entrante>, desde: PathBuf, ctx: egui::Context) {
    std::thread::spawn(move || {
        let v = rfd::FileDialog::new().set_title("Adjuntar imágenes").add_filter("Imágenes", &IMAGENES).add_filter("Todos los archivos", &["*"]).set_directory(desde).pick_files().unwrap_or_default();
        let _ = tx.send(Ok(v));
        ctx.request_repaint();
    });
}

/// Lee una imagen del portapapeles del sistema (o rutas de imagen copiadas desde el gestor de
/// archivos) en otro hilo. `silencioso`: sin error si no hay imagen (para Ctrl+V con texto).
pub fn pegar_portapapeles(tx: std::sync::mpsc::Sender<Entrante>, silencioso: bool, ctx: egui::Context) {
    std::thread::spawn(move || {
        let r = (|| -> Entrante {
            let mut cb = arboard::Clipboard::new().map_err(|e| e.to_string())?;
            if let Ok(img) = cb.get_image() {
                return Ok(vec![guardar_rgba(img.width as u32, img.height as u32, &img.bytes, "pegada")?]);
            }
            // Archivos copiados en el gestor de archivos: llegan como texto con rutas o URIs.
            let texto = cb.get_text().unwrap_or_default();
            let v: Vec<PathBuf> = texto.lines().filter_map(|l| {
                let l = l.trim().strip_prefix("file://").unwrap_or(l.trim());
                let p = PathBuf::from(percent_decode(l));
                (p.is_file() && es_imagen(&p)).then_some(p)
            }).collect();
            if v.is_empty() && !silencioso {
                return Err("el portapapeles no tiene ninguna imagen".into());
            }
            Ok(v)
        })();
        let _ = tx.send(r);
        ctx.request_repaint();
    });
}

/// `%20` → espacio, etc. (rutas de URIs `file://`).
fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Miniatura cuadrada para mostrar un adjunto de imagen.
pub fn miniatura(p: &Path, lado: u32) -> Option<egui::ColorImage> {
    let img = image::open(p).ok()?.thumbnail(lado, lado).to_rgba8();
    Some(egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rutas_y_carpetas() {
        let raiz = std::env::temp_dir().join(format!("maker-adj-{}", std::process::id()));
        std::fs::create_dir_all(raiz.join("scripts")).unwrap();
        std::fs::write(raiz.join("scripts/a.rs"), "x").unwrap();
        let fuera = std::env::temp_dir().join(format!("maker-adj-fuera-{}.png", std::process::id()));
        std::fs::write(&fuera, "x").unwrap();
        assert_eq!(normalizar("  'scripts/a.rs' ", &raiz), Some(raiz.join("scripts/a.rs")));
        assert_eq!(normalizar(&format!("file://{}", fuera.display()), &raiz), Some(fuera.clone()));
        assert_eq!(normalizar("no/existe.txt", &raiz), None);
        let extra = carpetas_extra(&[raiz.join("scripts/a.rs"), fuera.clone()], &raiz);
        assert_eq!(extra, vec![fuera.parent().unwrap().canonicalize().unwrap()]);
        let b = bloque(&[raiz.join("scripts/a.rs"), fuera.clone()], &raiz);
        assert!(b.contains("- scripts/a.rs\n") && b.contains("(imagen)"));
        let _ = std::fs::remove_dir_all(&raiz);
        let _ = std::fs::remove_file(&fuera);
        assert_eq!(percent_decode("/a%20b/c%C3%B1.png"), "/a b/cñ.png");
    }
}
