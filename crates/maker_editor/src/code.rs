//! Editor de scripts en Rust con resaltado (paleta github-light de Hermes).

use crate::theme;
use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, FontId};

const KEYWORDS: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern", "false", "fn", "for", "if", "impl", "in", "let",
    "loop", "match", "mod", "move", "mut", "pub", "ref", "return", "self", "Self", "static", "struct", "super", "trait", "true", "type", "unsafe",
    "use", "where", "while",
];
const TYPES: &[&str] = &[
    "f32", "f64", "i8", "i16", "i32", "i64", "i128", "isize", "u8", "u16", "u32", "u64", "u128", "usize", "bool", "char", "str", "String", "Vec",
    "Option", "Some", "None", "Result", "Ok", "Err", "Box",
];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tok {
    Text,
    Keyword,
    Type,
    String,
    Number,
    Comment,
    Function,
    Macro,
    Attribute,
}

/// Tokeniza Rust en tramos `(inicio, fin, tipo)` (sin solapes, cubre todo).
pub fn tokenize(src: &str) -> Vec<(usize, usize, Tok)> {
    let b = src.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    let n = b.len();
    let push = |out: &mut Vec<(usize, usize, Tok)>, s: usize, e: usize, t: Tok| {
        if e > s {
            out.push((s, e, t));
        }
    };
    let mut text_start = 0;
    while i < n {
        let c = b[i];
        let start = i;
        let tok;
        if c == b'/' && i + 1 < n && b[i + 1] == b'/' {
            while i < n && b[i] != b'\n' {
                i += 1;
            }
            tok = Tok::Comment;
        } else if c == b'/' && i + 1 < n && b[i + 1] == b'*' {
            i += 2;
            while i + 1 < n && !(b[i] == b'*' && b[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(n);
            tok = Tok::Comment;
        } else if c == b'"' {
            i += 1;
            while i < n && b[i] != b'"' {
                if b[i] == b'\\' {
                    i += 1;
                }
                i += 1;
            }
            i = (i + 1).min(n);
            tok = Tok::String;
        } else if c == b'#' && i + 1 < n && (b[i + 1] == b'[' || b[i + 1] == b'!') {
            while i < n && b[i] != b']' && b[i] != b'\n' {
                i += 1;
            }
            i = (i + 1).min(n);
            tok = Tok::Attribute;
        } else if c.is_ascii_digit() {
            while i < n && (b[i].is_ascii_alphanumeric() || b[i] == b'.' || b[i] == b'_') {
                if b[i] == b'.' && i + 1 < n && !b[i + 1].is_ascii_digit() {
                    break;
                }
                i += 1;
            }
            tok = Tok::Number;
        } else if c.is_ascii_alphabetic() || c == b'_' {
            while i < n && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            let word = &src[start..i];
            tok = if i < n && b[i] == b'!' {
                i += 1;
                Tok::Macro
            } else if KEYWORDS.contains(&word) {
                Tok::Keyword
            } else if TYPES.contains(&word) || word.chars().next().is_some_and(|c| c.is_ascii_uppercase()) {
                Tok::Type
            } else if i < n && b[i] == b'(' {
                Tok::Function
            } else {
                Tok::Text
            };
        } else {
            // Avanza un carácter UTF-8 completo.
            i += src[i..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
            continue;
        }
        if tok == Tok::Text {
            continue;
        }
        push(&mut out, text_start, start, Tok::Text);
        push(&mut out, start, i, tok);
        text_start = i;
    }
    push(&mut out, text_start, n, Tok::Text);
    out
}

pub fn highlight(src: &str, font: FontId) -> LayoutJob {
    let p = theme::code_palette();
    let mut job = LayoutJob::default();
    for (s, e, t) in tokenize(src) {
        let color: Color32 = match t {
            Tok::Text => p.text,
            Tok::Keyword => p.keyword,
            Tok::Type => p.type_,
            Tok::String => p.string,
            Tok::Number => p.number,
            Tok::Comment => p.comment,
            Tok::Function => p.function,
            Tok::Macro => p.macro_,
            Tok::Attribute => p.attribute,
        };
        job.append(&src[s..e], 0.0, TextFormat::simple(font.clone(), color));
    }
    job
}

/// Editor de código multilínea con resaltado y números de línea.
pub fn code_editor(ui: &mut egui::Ui, text: &mut String, id: &str) -> egui::Response {
    let mut layouter = |ui: &egui::Ui, buf: &dyn egui::TextBuffer, wrap: f32| {
        let mut job = highlight(buf.as_str(), FontId::monospace(13.0));
        job.wrap.max_width = wrap;
        ui.ctx().fonts_mut(|f| f.layout_job(job))
    };
    let lines = text.lines().count().max(1) + 1;
    let mut resp = None;
    ui.horizontal_top(|ui| {
        let nums: String = (1..=lines).map(|n| format!("{n:>4}\n")).collect();
        ui.add(egui::Label::new(egui::RichText::new(nums).font(FontId::monospace(13.0)).color(theme::TEXT4)).selectable(false));
        resp = Some(ui.add(
            egui::TextEdit::multiline(text)
                .id_salt(id)
                .code_editor()
                .desired_rows(30)
                .desired_width(f32::INFINITY)
                .lock_focus(true)
                .frame(egui::Frame::NONE)
                .layouter(&mut layouter),
        ));
    });
    resp.expect("el editor siempre se dibuja")
}

/// Plantilla de un script nuevo.
pub fn new_script_source(name: &str) -> String {
    format!(
        r#"
/// {name}: describe aquí qué hace.
#[derive(Default)]
pub struct {name} {{
    speed: f32,
}}

impl Script for {name} {{
    fn props(&mut self, p: &mut Props) {{
        p.float("speed", &mut self.speed);
    }}
    fn start(&mut self, ctx: &mut Ctx) {{
        ctx.log("{name} listo");
    }}
    fn update(&mut self, ctx: &mut Ctx) {{
        let _ = ctx.dt * self.speed;
    }}
}}
"#
    )
}

/// Añade un script a `lib.rs`: inserta el tipo antes de `export_scripts!` y lo
/// registra en la lista. Devuelve el texto nuevo.
pub fn add_script_to_lib(src: &str, name: &str) -> Result<String, String> {
    if !name.chars().next().is_some_and(|c| c.is_ascii_uppercase()) || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err("el nombre debe ser un identificador en PascalCase (p. ej. EnemyAI)".into());
    }
    if src.contains(&format!("struct {name} ")) || src.contains(&format!("struct {name}{{")) {
        return Err(format!("ya existe un script llamado {name}"));
    }
    let Some(pos) = src.rfind("export_scripts!(") else {
        return Err("no se encontró export_scripts!(...) en lib.rs".into());
    };
    let close = src[pos..].find(')').map(|c| pos + c).ok_or("export_scripts! sin cerrar")?;
    let inner = src[pos + "export_scripts!(".len()..close].trim().trim_end_matches(',').to_string();
    let list = if inner.is_empty() { name.to_string() } else { format!("{inner}, {name}") };
    let mut out = String::new();
    out.push_str(src[..pos].trim_end());
    out.push('\n');
    out.push_str(&new_script_source(name));
    out.push('\n');
    out.push_str(&format!("export_scripts!({list});"));
    out.push_str(&src[close + 1..].trim_start_matches(';'));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_cover_source_exactly() {
        let src = "// hola\nfn main() { let x: f32 = 1.5; println!(\"á\"); } #[derive(Default)] struct Ñ;";
        let toks = tokenize(src);
        let mut pos = 0;
        for (s, e, _) in &toks {
            assert_eq!(*s, pos);
            pos = *e;
        }
        assert_eq!(pos, src.len());
        assert!(toks.iter().any(|t| t.2 == Tok::Macro));
        assert!(toks.iter().any(|t| t.2 == Tok::Keyword && &src[t.0..t.1] == "fn"));
        assert!(toks.iter().any(|t| t.2 == Tok::Function && &src[t.0..t.1] == "main"));
    }

    #[test]
    fn add_script_registers_it() {
        let src = "use x;\nexport_scripts!(A, B);\n";
        let out = add_script_to_lib(src, "Enemy").unwrap();
        assert!(out.contains("pub struct Enemy"));
        assert!(out.contains("export_scripts!(A, B, Enemy);"));
        assert!(add_script_to_lib(&out, "Enemy").is_err());
        assert!(add_script_to_lib(src, "enemy").is_err());
    }
}
