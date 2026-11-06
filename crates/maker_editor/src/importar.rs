//! Importar assets al proyecto: uno a uno o en bloque (varios archivos, carpetas enteras o
//! soltándolos sobre Contenido / el visor). La copia va en otro hilo con progreso.
//!
//! - Archivos sueltos: a la carpeta abierta en Contenido. Si el nombre existe, «nombre (2).ext».
//! - Carpetas: se copian con su estructura dentro de la carpeta abierta.
//! - `.gltf` con archivos externos (`.bin`, texturas): se lleva sus dependencias con sus rutas
//!   relativas; si se importa suelto, todo va a una subcarpeta con su nombre para no mezclarlas.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};

/// Una copia planificada.
#[derive(Clone, Debug, PartialEq)]
pub struct Copia {
    pub origen: PathBuf,
    pub destino: PathBuf,
}

/// Resultado de una importación.
#[derive(Debug, Default)]
pub struct Informe {
    /// Archivos principales importados (no dependencias), ya en el proyecto.
    pub importados: Vec<PathBuf>,
    pub copiados: usize,
    pub errores: Vec<String>,
    /// Si se importa para añadir los modelos a la escena (soltados en el visor).
    pub a_escena: bool,
}

/// Importación en curso.
pub struct Tarea {
    pub hechos: Arc<AtomicUsize>,
    pub total: usize,
    rx: Receiver<Informe>,
}

pub struct Importador {
    pub tarea: Option<Tarea>,
    /// Texto del campo «Desde ruta».
    pub ruta: String,
    /// Lo elegido en el selector nativo, con la carpeta de destino de cuando se abrió.
    pub elegidos: (Sender<Vec<PathBuf>>, Receiver<Vec<PathBuf>>),
    pub destino_elegidos: PathBuf,
    /// Dónde está Contenido en la ventana (para soltar archivos encima).
    pub rect_contenido: egui::Rect,
}

impl Default for Importador {
    fn default() -> Self {
        Self { tarea: None, ruta: String::new(), elegidos: channel(), destino_elegidos: PathBuf::new(), rect_contenido: egui::Rect::NOTHING }
    }
}

/// Carpeta para lo que se suelta en el visor.
pub const CARPETA_VISOR: &str = "assets/importados";

fn es_modelo(p: &Path) -> bool {
    p.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("glb") || e.eq_ignore_ascii_case("gltf"))
}

/// Cada fotograma: selector, archivos soltados y fin de la copia.
pub fn tick(ed: &mut crate::editor::Editor, ctx: &egui::Context) {
    let raiz = ed.project.root.clone();
    while let Ok(v) = ed.importador.elegidos.1.try_recv() {
        let d = std::mem::take(&mut ed.importador.destino_elegidos);
        ed.importador.empezar(v, if d.as_os_str().is_empty() { raiz.clone() } else { d }, false, ctx.clone());
    }
    // Archivos soltados desde el sistema: sobre Contenido → carpeta abierta; sobre el visor →
    // `assets/importados` y los modelos se añaden a la escena.
    let (soltados, pos, flotando) = ctx.input(|i| {
        let s: Vec<PathBuf> = i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).filter(|p| !p.as_os_str().is_empty()).collect();
        (s, i.pointer.hover_pos().or(i.pointer.latest_pos()), !i.raw.hovered_files.is_empty())
    });
    let sobre_contenido = pos.is_some_and(|p| ed.importador.rect_contenido.contains(p));
    let sobre_visor = pos.is_some_and(|p| ed.viewport_rect.contains(p)) && !ed.is_playing();
    if flotando && (sobre_contenido || sobre_visor) {
        let r = if sobre_contenido { ed.importador.rect_contenido } else { ed.viewport_rect }.shrink(4.0);
        let pintor = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("importar-soltar")));
        pintor.rect(r, 8.0, crate::theme::ACCENT_SOFT.gamma_multiply(0.9), egui::Stroke::new(2.0, crate::theme::ACCENT), egui::StrokeKind::Inside);
        pintor.text(r.center(), egui::Align2::CENTER_CENTER, if sobre_contenido { "Soltar para importar aquí" } else { "Soltar para importar y añadir a la escena" }, egui::FontId::proportional(16.0), crate::theme::ACCENT);
    }
    if !soltados.is_empty() {
        if sobre_contenido {
            let d = raiz.join(ed.contenido.carpeta());
            ed.importador.empezar(soltados, d, false, ctx.clone());
        } else if sobre_visor {
            ed.importador.empezar(soltados, raiz.join(CARPETA_VISOR), true, ctx.clone());
        }
    }
    if let Some(inf) = ed.importador.terminado() {
        use maker_engine::log;
        for e in &inf.errores {
            log::error("importar", e.clone());
        }
        let n = inf.importados.len();
        match n {
            0 => {}
            1 => log::info("importar", format!("importado: {}", ed.project.relative(&inf.importados[0]))),
            _ => log::info("importar", format!("{n} archivos importados ({} copiados con sus dependencias)", inf.copiados)),
        }
        if inf.a_escena {
            for p in inf.importados.iter().filter(|p| es_modelo(p)) {
                let rel = ed.project.relative(p);
                ed.add_model(&rel);
            }
        }
        if let Some(p) = inf.importados.first() {
            ed.contenido.mostrar(&raiz, p);
        }
    }
}

/// Menú «Importar»: archivos, carpetas o una ruta escrita, hacia `destino`.
pub fn menu(ed: &mut crate::editor::Editor, ui: &mut egui::Ui, destino: PathBuf) {
    let imp = &mut ed.importador;
    let titulo = match &imp.tarea {
        Some(t) => format!("Importando {}/{}", t.hechos.load(Ordering::Relaxed), t.total),
        None => "Importar".into(),
    };
    ui.add_enabled_ui(imp.tarea.is_none(), |ui| {
        ui.menu_button(titulo, |ui| {
            if ui.button("Archivos…").on_hover_text("Uno o varios archivos").clicked() {
                imp.destino_elegidos = destino.clone();
                elegir(false, destino.clone(), imp.elegidos.0.clone(), ui.ctx().clone());
                ui.close();
            }
            if ui.button("Carpetas…").on_hover_text("Carpetas enteras, con su estructura").clicked() {
                imp.destino_elegidos = destino.clone();
                elegir(true, destino.clone(), imp.elegidos.0.clone(), ui.ctx().clone());
                ui.close();
            }
            ui.separator();
            ui.horizontal(|ui| {
                let r = ui.add(egui::TextEdit::singleline(&mut imp.ruta).hint_text("Ruta de un archivo o carpeta").desired_width(260.0));
                let ok = ruta_escrita(&imp.ruta);
                let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if (ui.add_enabled(ok.is_some(), egui::Button::new("Importar")).clicked() || enter)
                    && let Some(p) = ok
                {
                    imp.ruta.clear();
                    imp.empezar(vec![p], destino.clone(), false, ui.ctx().clone());
                    ui.close();
                }
            });
        })
        .response
        .on_hover_text("También puedes soltar archivos y carpetas sobre Contenido o el visor");
    });
}

fn oculto(p: &Path) -> bool {
    p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.'))
}

/// URIs externas de un `.gltf` (buffers e imágenes), ya decodificadas. Sin las `data:`.
pub fn dependencias_gltf(gltf: &Path) -> Vec<String> {
    let Ok(txt) = std::fs::read_to_string(gltf) else { return Vec::new() };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&txt) else { return Vec::new() };
    let mut out = Vec::new();
    for clave in ["buffers", "images"] {
        for e in v.get(clave).and_then(|b| b.as_array()).into_iter().flatten() {
            if let Some(u) = e.get("uri").and_then(|u| u.as_str())
                && !u.starts_with("data:")
                && !u.contains("://")
            {
                out.push(decodificar(u));
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// `%20` → espacio (URIs relativas de glTF).
fn decodificar(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16)
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Ruta libre: `dir/nombre.ext`, o `dir/nombre (2).ext`… si ya existe (o ya está planificada).
fn libre(dir: &Path, nombre: &str, ocupados: &[PathBuf]) -> PathBuf {
    let p = dir.join(nombre);
    if !p.exists() && !ocupados.contains(&p) {
        return p;
    }
    let (base, ext) = match nombre.rsplit_once('.') {
        Some((b, e)) if !b.is_empty() => (b.to_string(), format!(".{e}")),
        _ => (nombre.to_string(), String::new()),
    };
    (2..)
        .map(|n| dir.join(format!("{base} ({n}){ext}")))
        .find(|p| !p.exists() && !ocupados.contains(p))
        .unwrap()
}

/// Qué copiar y dónde. Devuelve también los archivos principales (los que verá el usuario).
pub fn planificar(fuentes: &[PathBuf], destino: &Path) -> (Vec<Copia>, Vec<PathBuf>) {
    let mut copias: Vec<Copia> = Vec::new();
    let mut principales = Vec::new();
    for f in fuentes {
        if f.is_dir() {
            let nombre = f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "importado".into());
            let ocupados: Vec<PathBuf> = copias.iter().map(|c| c.destino.clone()).collect();
            let raiz = libre(destino, &nombre, &ocupados);
            let mut pila = vec![f.clone()];
            while let Some(d) = pila.pop() {
                let Ok(rd) = std::fs::read_dir(&d) else { continue };
                let mut hijos: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| !oculto(p)).collect();
                hijos.sort();
                for h in hijos {
                    if h.is_dir() {
                        pila.push(h);
                    } else if let Ok(rel) = h.strip_prefix(f) {
                        let dst = raiz.join(rel);
                        principales.push(dst.clone());
                        copias.push(Copia { origen: h, destino: dst });
                    }
                }
            }
        } else if f.is_file() {
            let nombre = f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let es_gltf = f.extension().is_some_and(|e| e.eq_ignore_ascii_case("gltf"));
            let deps = if es_gltf { dependencias_gltf(f) } else { Vec::new() };
            let ocupados: Vec<PathBuf> = copias.iter().map(|c| c.destino.clone()).collect();
            if deps.is_empty() {
                let dst = libre(destino, &nombre, &ocupados);
                principales.push(dst.clone());
                copias.push(Copia { origen: f.clone(), destino: dst });
            } else {
                // Con dependencias: subcarpeta propia, conservando las rutas relativas.
                let base = f.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "modelo".into());
                let dir = libre(destino, &base, &ocupados);
                let dst = dir.join(&nombre);
                principales.push(dst.clone());
                copias.push(Copia { origen: f.clone(), destino: dst });
                let src_dir = f.parent().unwrap_or(Path::new("."));
                for d in deps {
                    let rel = Path::new(&d);
                    // Sin salir de la carpeta: «../x.png» se queda junto al modelo.
                    let rel_seguro: PathBuf = if rel.components().any(|c| matches!(c, std::path::Component::ParentDir | std::path::Component::RootDir | std::path::Component::Prefix(_))) {
                        PathBuf::from(rel.file_name().unwrap_or_default())
                    } else {
                        rel.to_path_buf()
                    };
                    copias.push(Copia { origen: src_dir.join(rel), destino: dir.join(rel_seguro) });
                }
            }
        }
    }
    (copias, principales)
}

impl Importador {
    pub fn ocupado(&self) -> bool {
        self.tarea.is_some()
    }

    /// Empieza a importar `fuentes` en `destino` (carpeta del proyecto) en otro hilo.
    pub fn empezar(&mut self, fuentes: Vec<PathBuf>, destino: PathBuf, a_escena: bool, ctx: egui::Context) {
        if self.ocupado() || fuentes.is_empty() {
            return;
        }
        let (copias, principales) = planificar(&fuentes, &destino);
        if copias.is_empty() {
            maker_engine::log::warn("importar", "no hay nada que importar");
            return;
        }
        let hechos = Arc::new(AtomicUsize::new(0));
        let (tx, rx): (Sender<Informe>, Receiver<Informe>) = channel();
        let total = copias.len();
        let h = hechos.clone();
        std::thread::spawn(move || {
            let mut inf = Informe { a_escena, ..Default::default() };
            for c in &copias {
                if let Some(d) = c.destino.parent()
                    && let Err(e) = std::fs::create_dir_all(d)
                {
                    inf.errores.push(format!("{}: {e}", d.display()));
                } else {
                    match std::fs::copy(&c.origen, &c.destino) {
                        Ok(_) => inf.copiados += 1,
                        Err(e) => inf.errores.push(format!("{}: {e}", c.origen.display())),
                    }
                }
                h.fetch_add(1, Ordering::Relaxed);
                ctx.request_repaint();
            }
            inf.importados = principales.into_iter().filter(|p| p.is_file()).collect();
            let _ = tx.send(inf);
            ctx.request_repaint();
        });
        self.tarea = Some(Tarea { hechos, total, rx });
    }

    /// Recoge el informe cuando la copia termina.
    pub fn terminado(&mut self) -> Option<Informe> {
        let inf = self.tarea.as_ref()?.rx.try_recv().ok()?;
        self.tarea = None;
        Some(inf)
    }
}

/// Selector nativo de archivos o carpetas (varios), en otro hilo.
pub fn elegir(carpetas: bool, desde: PathBuf, tx: Sender<Vec<PathBuf>>, ctx: egui::Context) {
    std::thread::spawn(move || {
        let d = rfd::FileDialog::new().set_directory(desde);
        let v = if carpetas {
            d.set_title("Importar carpetas").pick_folders()
        } else {
            d.set_title("Importar archivos").add_filter("Todos los archivos", &["*"]).add_filter("Modelos", &["glb", "gltf"]).add_filter("Imágenes", &["png", "jpg", "jpeg", "webp", "bmp", "gif", "tga"]).add_filter("Fuentes", &["ttf", "otf"]).pick_files()
        };
        let _ = tx.send(v.unwrap_or_default());
        ctx.request_repaint();
    });
}

/// Ruta escrita a mano: comillas, `file://` y `~/`.
pub fn ruta_escrita(s: &str) -> Option<PathBuf> {
    let raw = s.trim().trim_matches(['"', '\'']).trim_start_matches("file://");
    if raw.is_empty() {
        return None;
    }
    let p = match raw.strip_prefix("~/") {
        Some(rest) => PathBuf::from(std::env::var("HOME").ok()?).join(rest),
        None => PathBuf::from(decodificar(raw)),
    };
    p.exists().then_some(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(n: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("maker-importar-{n}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn archivos_sueltos_carpetas_y_gltf_con_dependencias() {
        let src = tmp("src");
        let dst = tmp("dst");
        std::fs::write(src.join("a.png"), b"x").unwrap();
        std::fs::write(dst.join("a.png"), b"ya estaba").unwrap();
        std::fs::create_dir_all(src.join("pack/sub")).unwrap();
        std::fs::write(src.join("pack/uno.glb"), b"g").unwrap();
        std::fs::write(src.join("pack/sub/dos.ogg"), b"o").unwrap();
        std::fs::write(src.join("pack/.oculto"), b"h").unwrap();
        std::fs::create_dir_all(src.join("tex")).unwrap();
        std::fs::write(src.join("tex/piel%.png"), b"t").unwrap();
        std::fs::write(src.join("casa.bin"), b"b").unwrap();
        std::fs::write(src.join("casa.gltf"), r#"{"buffers":[{"uri":"casa.bin"}],"images":[{"uri":"tex/piel%25.png"},{"uri":"data:image/png;base64,AA"}]}"#).unwrap();

        let (copias, principales) = planificar(&[src.join("a.png"), src.join("a.png"), src.join("pack"), src.join("casa.gltf")], &dst);
        let destinos: Vec<String> = copias.iter().map(|c| c.destino.strip_prefix(&dst).unwrap().to_string_lossy().replace('\\', "/")).collect();
        assert_eq!(destinos, ["a (2).png", "a (3).png", "pack/uno.glb", "pack/sub/dos.ogg", "casa/casa.gltf", "casa/casa.bin", "casa/tex/piel%.png"]);
        assert_eq!(principales.len(), 5, "las dependencias del gltf no cuentan como principales");

        let mut imp = Importador::default();
        imp.empezar(vec![src.join("pack"), src.join("casa.gltf")], dst.clone(), false, egui::Context::default());
        let inf = loop {
            if let Some(i) = imp.terminado() {
                break i;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        };
        assert!(inf.errores.is_empty(), "{:?}", inf.errores);
        assert_eq!((inf.copiados, inf.importados.len()), (5, 3));
        assert_eq!(std::fs::read(dst.join("casa/tex/piel%.png")).unwrap(), b"t");
        assert!(!dst.join("pack/.oculto").exists());
        assert_eq!(std::fs::read(dst.join("a.png")).unwrap(), b"ya estaba", "nunca se sobrescribe");
        let _ = std::fs::remove_dir_all(&src);
        let _ = std::fs::remove_dir_all(&dst);
    }
}
