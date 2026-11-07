//! Rutas absolutas sin el prefijo «verbatim» de Windows.
//!
//! En Windows `Path::canonicalize` devuelve rutas `\\?\C:\…` (y `\\?\UNC\servidor\…`).
//! Funcionan con la API de archivos, pero no son rutas normales: cargo no las acepta en un
//! `Cargo.toml` (`invalid path url //?/C:/…`), muchas herramientas externas tampoco y en la
//! interfaz se ven raras. Todo lo que canonicaliza el motor pasa por aquí.

use std::path::{Path, PathBuf};

/// `canonicalize` sin el prefijo `\\?\` cuando la ruta se puede escribir de forma normal.
/// Si falla (la ruta no existe…), devuelve la ruta tal cual, también sin prefijo.
pub fn canonicalizar(p: &Path) -> PathBuf {
    sin_verbatim(p.canonicalize().unwrap_or_else(|_| p.to_path_buf()))
}

/// Como [`canonicalizar`] pero informando del error.
pub fn canonicalizar_estricto(p: &Path) -> std::io::Result<PathBuf> {
    p.canonicalize().map(sin_verbatim)
}

/// Quita `\\?\` (`\\?\C:\x` → `C:\x`, `\\?\UNC\srv\x` → `\\srv\x`). Deja la ruta como está
/// si no lleva el prefijo o si sin él cambiaría de significado (demasiado larga, nombres
/// reservados, componentes con punto final…).
pub fn sin_verbatim(p: PathBuf) -> PathBuf {
    let Some(s) = p.to_str() else { return p };
    let (resto, unc) = if let Some(r) = s.strip_prefix(r"\\?\UNC\") {
        (r, true)
    } else if let Some(r) = s.strip_prefix(r"\\?\") {
        (r, false)
    } else {
        return p;
    };
    if !unc {
        // Solo unidades con letra: `C:\…` (o `C:` a secas).
        let b = resto.as_bytes();
        if b.len() < 2 || !b[0].is_ascii_alphabetic() || b[1] != b':' || (b.len() > 2 && b[2] != b'\\') {
            return p;
        }
    }
    if resto.len() > 250 || !componentes_normales(resto) {
        return p;
    }
    if unc { PathBuf::from(format!(r"\\{resto}")) } else { PathBuf::from(resto) }
}

/// Lo que sin `\\?\` Windows reinterpretaría: `.`/`..`, puntos o espacios finales, `/`,
/// caracteres no válidos y nombres reservados de dispositivo.
fn componentes_normales(resto: &str) -> bool {
    const RESERVADOS: [&str; 22] = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6",
        "LPT7", "LPT8", "LPT9",
    ];
    resto.split('\\').enumerate().all(|(i, c)| {
        if i == 0 {
            return true; // `C:` o el servidor
        }
        if c.is_empty() {
            return true; // barra final
        }
        if c == "." || c == ".." || c.ends_with('.') || c.ends_with(' ') {
            return false;
        }
        if c.chars().any(|ch| matches!(ch, '/' | '<' | '>' | '"' | '|' | '?' | '*' | ':') || (ch as u32) < 32) {
            return false;
        }
        let base = c.split('.').next().unwrap_or("").trim_end();
        !RESERVADOS.iter().any(|r| r.eq_ignore_ascii_case(base))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(p: &str) -> String {
        sin_verbatim(PathBuf::from(p)).to_string_lossy().into_owned()
    }

    #[test]
    fn quita_el_prefijo() {
        assert_eq!(s(r"\\?\C:\Users\ozx\Mi juego"), r"C:\Users\ozx\Mi juego");
        assert_eq!(s(r"\\?\C:\"), r"C:\");
        assert_eq!(s(r"\\?\UNC\srv\comp\x"), r"\\srv\comp\x");
    }

    #[test]
    fn deja_lo_demas() {
        assert_eq!(s(r"C:\a\b"), r"C:\a\b");
        assert_eq!(s("/home/a/b"), "/home/a/b");
        assert_eq!(s(r"\\?\Volume{1234}\x"), r"\\?\Volume{1234}\x");
        assert_eq!(s(r"\\?\C:\a\con.txt"), r"\\?\C:\a\con.txt");
        assert_eq!(s(r"\\?\C:\a\b."), r"\\?\C:\a\b.");
        let largo = format!(r"\\?\C:\{}", "x".repeat(300));
        assert_eq!(s(&largo), largo);
    }

    #[test]
    fn canonicalizar_no_devuelve_verbatim() {
        let d = std::env::temp_dir();
        let c = canonicalizar(&d);
        assert!(!c.to_string_lossy().starts_with(r"\\?\"), "{}", c.display());
        assert!(c.is_dir());
    }
}
