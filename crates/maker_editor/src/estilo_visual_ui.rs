//! Galería «Estilo visual»: tarjetas con una miniatura en vivo de la escena
//! actual (cámara del editor) con cada estilo, un clic para aplicarlo (un solo
//! paso de deshacer), intensidad, aplicar solo el sombreado o solo el
//! posprocesado, y estilos propios del proyecto en `estilos/*.mvisual`.
//!
//! Las miniaturas se renderizan con un renderizador aparte, como mucho una por
//! fotograma, y se rehacen cuando la escena, la cámara o los ajustes llevan un
//! momento quietos (así no se renderiza mientras se arrastra la cámara).

use crate::editor::Editor;
use crate::icons::{self, Icon};
use crate::theme;
use egui::{Color32, RichText, Ui};
use maker_engine::estilo_visual::{self as ev, EstiloVisual, Partes};
use maker_engine::log;
use maker_render::{RenderOptions, SceneRenderer};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Tamaño de la miniatura en puntos.
const MINI: egui::Vec2 = egui::vec2(132.0, 80.0);
/// Tiempo que la escena y la cámara deben estar quietas antes de rehacer miniaturas.
const ESPERA: Duration = Duration::from_millis(300);
/// Cada cuánto se vuelve a leer la carpeta `estilos/` con la ventana abierta.
const RELEER: Duration = Duration::from_secs(2);

/// Una tarjeta de la galería.
#[derive(Clone, Debug, PartialEq)]
pub struct Tarjeta {
    /// Clave estable: `ninguno`, `i:<id>` o `u:<archivo>`.
    pub clave: String,
    pub nombre: String,
    pub estilo: EstiloVisual,
    /// Archivo, si es un estilo del usuario.
    pub archivo: Option<PathBuf>,
}

/// Tarjetas en el orden de la galería: Ninguno, los integrados disponibles en
/// el proyecto (CRT solo si tiene su efecto) y los del usuario.
pub fn tarjetas(raiz: &Path, usuario: &[(PathBuf, EstiloVisual)]) -> Vec<Tarjeta> {
    let mut v = vec![Tarjeta { clave: "ninguno".into(), nombre: "Ninguno".into(), estilo: EstiloVisual::default(), archivo: None }];
    for i in &ev::INTEGRADOS {
        let estilo = i.estilo();
        if estilo.disponible(raiz) {
            v.push(Tarjeta { clave: format!("i:{}", i.id), nombre: i.nombre.into(), estilo, archivo: None });
        }
    }
    for (p, estilo) in usuario {
        let nombre = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let clave = format!("u:{}", p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default());
        v.push(Tarjeta { clave, nombre, estilo: estilo.clone(), archivo: Some(p.clone()) });
    }
    v
}

/// La tarjeta aplicada en `env` (la primera que coincide) y su intensidad.
pub fn activa(tarjetas: &[Tarjeta], env: &maker_engine::scene::Environment, partes: Partes, intensidad: f32) -> Option<(usize, f32)> {
    tarjetas.iter().enumerate().find_map(|(i, t)| t.estilo.intensidad_en(env, partes, &[intensidad]).map(|k| (i, k)))
}

/// Antirrebote: `listo` es verdadero cuando la huella lleva `espera` sin cambiar.
#[derive(Debug, Default)]
pub struct Antirrebote {
    huella: u64,
    desde: Option<Instant>,
}

impl Antirrebote {
    pub fn listo(&mut self, huella: u64, ahora: Instant, espera: Duration) -> bool {
        if self.desde.is_none() || huella != self.huella {
            self.huella = huella;
            self.desde = Some(ahora);
        }
        self.desde.is_some_and(|d| ahora.duration_since(d) >= espera)
    }
}

struct Miniatura {
    huella: u64,
    tex: egui::TextureHandle,
}

pub struct Galeria {
    pub abierta: bool,
    /// Intensidad con la que se aplican los estilos (1 = como están guardados).
    pub intensidad: f32,
    pub partes: Partes,
    /// Pide al inspector que muestre el Entorno y se desplace al sombreado global.
    pub ir_a_entorno: bool,
    usuario: Vec<(PathBuf, EstiloVisual)>,
    leido: Option<(PathBuf, Instant)>,
    minis: HashMap<String, Miniatura>,
    renderer: Option<SceneRenderer>,
    quieto: Antirrebote,
    nombre_nuevo: String,
    renombrar: Option<(PathBuf, String)>,
    borrar: Option<PathBuf>,
}

impl Default for Galeria {
    fn default() -> Self {
        Self {
            abierta: false,
            intensidad: 1.0,
            partes: Partes::Todo,
            ir_a_entorno: false,
            usuario: Vec::new(),
            leido: None,
            minis: HashMap::new(),
            renderer: None,
            quieto: Antirrebote::default(),
            nombre_nuevo: String::new(),
            renombrar: None,
            borrar: None,
        }
    }
}

impl Galeria {
    /// Relee `estilos/*.mvisual` si toca (o si se fuerza).
    fn releer(&mut self, raiz: &Path, forzar: bool) {
        let toca = match &self.leido {
            Some((r, t)) => r != raiz || t.elapsed() >= RELEER,
            None => true,
        };
        if !(toca || forzar) {
            return;
        }
        self.usuario = ev::archivos(raiz)
            .into_iter()
            .filter_map(|p| match EstiloVisual::cargar(&p) {
                Ok(e) => Some((p, e)),
                Err(e) => {
                    log::warn("estilo visual", e);
                    None
                }
            })
            .collect();
        self.leido = Some((raiz.to_path_buf(), Instant::now()));
    }
}

/// Aplica un estilo con los ajustes de la galería como un paso de deshacer.
fn aplicar(ed: &mut Editor, estilo: &EstiloVisual, nombre: &str) {
    if ed.is_playing() {
        return;
    }
    let g = &ed.galeria;
    estilo.escalado(g.intensidad).aplicar(&mut ed.scene.env, g.partes);
    ed.commit(&format!("Estilo visual: {nombre}"));
}

/// Doble clic en Contenido: aplica el estilo de ese archivo.
pub fn aplicar_archivo(ed: &mut Editor, p: &Path) {
    match EstiloVisual::cargar(p) {
        Ok(e) => {
            let nombre = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            aplicar(ed, &e, &nombre);
        }
        Err(e) => log::error("estilo visual", e),
    }
}

/// Huella de lo que se ve en las miniaturas (sin el estilo de cada tarjeta).
fn huella_escena(ed: &Editor, lado: (u32, u32)) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    ed.undo.version.hash(&mut h);
    let c = &ed.cam;
    for f in [c.pivot.x, c.pivot.y, c.pivot.z, c.yaw, c.pitch, c.distance, c.fov, ed.galeria.intensidad] {
        f.to_bits().hash(&mut h);
    }
    (ed.shadows, lado, ed.galeria.partes as u8).hash(&mut h);
    // El entorno puede cambiar sin confirmar aún (deslizadores del inspector).
    serde_json::to_string(&ed.scene.env).unwrap_or_default().hash(&mut h);
    h.finish()
}

fn huella_tarjeta(base: u64, t: &Tarjeta) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    base.hash(&mut h);
    serde_json::to_string(&t.estilo).unwrap_or_default().hash(&mut h);
    h.finish()
}

/// Renderiza la escena con la cámara del editor y el estilo `t` aplicado.
fn renderizar(ed: &mut Editor, t: &Tarjeta, lado: (u32, u32)) -> Option<egui::ColorImage> {
    let g = &mut ed.galeria;
    let r = g.renderer.get_or_insert_with(|| {
        let (d, q) = ed.renderer.gpu();
        SceneRenderer::new(d, q)
    });
    r.project_root = Some(ed.project.root.clone());
    r.resize(lado.0, lado.1);
    let mut env = ed.scene.env.clone();
    t.estilo.escalado(g.intensidad).aplicar(&mut env, g.partes);
    let antes = std::mem::replace(&mut ed.scene.env, env);
    let vista = ed.cam.render_view(lado.0 as f32 / lado.1.max(1) as f32);
    r.render(&ed.scene, &vista, &RenderOptions { shadows: ed.shadows, post: true, ..Default::default() });
    ed.scene.env = antes;
    let (w, h, px) = r.read_pixels()?;
    Some(egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &px))
}

/// Resumen para el tooltip: modelo de sombreado y efectos.
fn resumen(e: &EstiloVisual) -> String {
    let mut partes: Vec<String> = Vec::new();
    if let Some(m) = e.global_shading.model {
        partes.push(m.label().to_string());
    }
    partes.extend(e.post.iter().map(|f| f.kind.label()));
    if partes.is_empty() { "Sin sombreado global ni posprocesado".into() } else { partes.join(" · ") }
}

/// Botón de la barra del viewport y de la cabecera del Entorno.
pub fn boton(ed: &mut Editor, ui: &mut Ui, texto: &str) {
    let img = icons::image(ui.ctx(), Icon::EstiloVisual, theme::TEXT2);
    if ui.add(egui::Button::image_and_text(img, texto).selected(ed.galeria.abierta)).on_hover_text("Galería de estilos visuales").clicked() {
        ed.galeria.abierta = !ed.galeria.abierta;
    }
}

pub fn ventana(ed: &mut Editor, ctx: &egui::Context) {
    if !ed.galeria.abierta {
        return;
    }
    let mut abierta = true;
    egui::Window::new("Estilo visual")
        .open(&mut abierta)
        .collapsible(false)
        .resizable(true)
        .default_width(3.0 * (MINI.x + 10.0) + 20.0)
        .min_width(MINI.x + 30.0)
        .show(ctx, |ui| contenido(ed, ui));
    ed.galeria.abierta &= abierta;
    if !ed.galeria.abierta {
        // Libera la GPU y las texturas al cerrar.
        ed.galeria.renderer = None;
        ed.galeria.minis.clear();
    }
}

fn contenido(ed: &mut Editor, ui: &mut Ui) {
    let raiz = ed.project.root.clone();
    ed.galeria.releer(&raiz, false);
    let lista = tarjetas(&raiz, &ed.galeria.usuario);
    let g = &ed.galeria;
    let activa = activa(&lista, &ed.scene.env, g.partes, g.intensidad);
    let ctx = ui.ctx().clone();
    let ppp = ctx.pixels_per_point().min(2.0);
    let lado = ((MINI.x * ppp).round() as u32, (MINI.y * ppp).round() as u32);

    // Miniaturas: una por fotograma cuando todo lleva un rato quieto.
    let base = huella_escena(ed, lado);
    let listo = ed.galeria.quieto.listo(base, Instant::now(), ESPERA);
    let pendiente = lista.iter().find(|t| ed.galeria.minis.get(&t.clave).is_none_or(|m| m.huella != huella_tarjeta(base, t))).cloned();
    if let Some(t) = pendiente {
        if listo {
            if let Some(img) = renderizar(ed, &t, lado) {
                let h = huella_tarjeta(base, &t);
                match ed.galeria.minis.get_mut(&t.clave) {
                    Some(m) => {
                        m.tex.set(img, egui::TextureOptions::LINEAR);
                        m.huella = h;
                    }
                    None => {
                        let tex = ctx.load_texture(format!("estilo-visual-{}", t.clave), img, egui::TextureOptions::LINEAR);
                        ed.galeria.minis.insert(t.clave.clone(), Miniatura { huella: h, tex });
                    }
                }
            }
            ctx.request_repaint();
        } else {
            ctx.request_repaint_after(ESPERA);
        }
    }
    ed.galeria.minis.retain(|k, _| lista.iter().any(|t| &t.clave == k));

    // Tarjetas.
    let mut clic: Option<usize> = None;
    let mut releer = false;
    egui::ScrollArea::vertical().max_height((ui.available_height() - 90.0).max(MINI.y + 30.0)).auto_shrink([false, true]).show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
            for (i, t) in lista.iter().enumerate() {
                let es_activa = activa.is_some_and(|(a, _)| a == i);
                let tex = ed.galeria.minis.get(&t.clave).map(|m| m.tex.id());
                let resp = tarjeta(ui, t, tex, es_activa);
                if resp.clicked() {
                    clic = Some(i);
                }
                if let Some(p) = &t.archivo {
                    resp.context_menu(|ui| releer |= menu_usuario(&mut ed.galeria, ui, p));
                }
            }
        });
    });
    if let Some(i) = clic {
        let t = &lista[i];
        aplicar(ed, &t.estilo, &t.nombre);
    }

    // Ajustes.
    ui.separator();
    let g = &mut ed.galeria;
    let mut k = g.intensidad;
    let r = ui
        .horizontal(|ui| {
            ui.label("Intensidad");
            ui.add(
                egui::Slider::new(&mut k, 0.0..=1.5)
                    .custom_formatter(|v, _| format!("{:.0} %", v * 100.0))
                    .custom_parser(|s| s.trim().trim_end_matches('%').trim().replace(',', ".").parse::<f64>().ok().map(|v| v / 100.0)),
            )
            .on_hover_text("Escala la fuerza de los efectos del estilo")
        })
        .inner;
    if r.changed() {
        g.intensidad = k;
        if let Some((i, _)) = activa
            && !ed.is_playing()
        {
            // Reaplica el estilo activo; se confirma al soltar.
            let t = &lista[i];
            t.estilo.escalado(k).aplicar(&mut ed.scene.env, ed.galeria.partes);
            ed.pending_edit = Some(format!("Intensidad del estilo: {}", t.nombre));
            ed.dirty = true;
        }
    } else if let Some((_, ka)) = activa
        && !r.dragged()
        && (ka - g.intensidad).abs() > 1e-3
    {
        g.intensidad = ka;
    }
    let g = &mut ed.galeria;
    ui.horizontal(|ui| {
        let mut solo_s = g.partes == Partes::Sombreado;
        let mut solo_p = g.partes == Partes::Posprocesado;
        if ui.toggle_value(&mut solo_s, "Solo sombreado").on_hover_text("Aplicar solo el sombreado global y conservar el posprocesado actual").changed() {
            g.partes = if solo_s { Partes::Sombreado } else { Partes::Todo };
        }
        if ui.toggle_value(&mut solo_p, "Solo posprocesado").on_hover_text("Aplicar solo el posprocesado y conservar el sombreado actual").changed() {
            g.partes = if solo_p { Partes::Posprocesado } else { Partes::Todo };
        }
    });
    ui.horizontal(|ui| {
        if ui.button("Personalizar…").on_hover_text("Editar el sombreado global y el posprocesado en el inspector").clicked() {
            ed.selection = None;
            ed.galeria.ir_a_entorno = true;
        }
        releer |= guardar_como(ed, ui, &raiz);
    });
    if releer {
        ed.galeria.releer(&raiz, true);
    }
}

/// Una tarjeta: miniatura y nombre; resaltada si es la activa.
fn tarjeta(ui: &mut Ui, t: &Tarjeta, tex: Option<egui::TextureId>, activa: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(MINI.x + 8.0, MINI.y + 26.0), egui::Sense::click());
    let pt = ui.painter_at(rect);
    let fondo = if activa {
        theme::ACCENT_SOFT
    } else if resp.hovered() {
        theme::ROW_HOVER
    } else {
        Color32::TRANSPARENT
    };
    pt.rect_filled(rect, 4.0, fondo);
    let img = egui::Rect::from_min_size(rect.min + egui::vec2(4.0, 4.0), MINI);
    match tex {
        Some(id) => {
            pt.image(id, img, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
        }
        None => {
            pt.rect_filled(img, 2.0, theme::FILL_TERTIARY);
        }
    }
    pt.rect_stroke(img, 2.0, egui::Stroke::new(1.0, theme::STROKE3), egui::StrokeKind::Inside);
    if activa {
        pt.rect_stroke(rect, 4.0, egui::Stroke::new(2.0, theme::ACCENT), egui::StrokeKind::Inside);
    }
    let color = if activa { theme::ACCENT } else { theme::TEXT1 };
    pt.text(egui::pos2(rect.center().x, rect.bottom() - 5.0), egui::Align2::CENTER_BOTTOM, &t.nombre, egui::FontId::proportional(12.0), color);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &t.nombre));
    let tip = match &t.archivo {
        Some(p) => format!("{}\n{}", resumen(&t.estilo), p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()),
        None => resumen(&t.estilo),
    };
    resp.on_hover_text(tip)
}

/// Menú contextual de un estilo del usuario. Devuelve `true` si cambió algún archivo.
fn menu_usuario(g: &mut Galeria, ui: &mut Ui, p: &Path) -> bool {
    let mut cambio = false;
    let actual = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    if g.renombrar.as_ref().is_none_or(|(r, _)| r != p) {
        g.renombrar = Some((p.to_path_buf(), actual.clone()));
    }
    let Some((_, nombre)) = &mut g.renombrar else { return false };
    ui.label("Nombre:");
    let r = ui.text_edit_singleline(nombre);
    let valido = ev::nombre_valido(nombre).filter(|n| *n != actual);
    let destino = valido.as_ref().map(|n| p.with_file_name(format!("{n}.{}", ev::EXT)));
    let existe = destino.as_ref().is_some_and(|d| d.exists());
    if existe {
        ui.label(RichText::new("Ya existe un estilo con ese nombre").small().color(theme::YELLOW));
    }
    let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    if (ui.add_enabled(destino.is_some() && !existe, egui::Button::new("Renombrar")).clicked() || enter)
        && let Some(d) = destino.filter(|d| !d.exists())
    {
        match std::fs::rename(p, &d) {
            Ok(()) => cambio = true,
            Err(e) => log::error("estilo visual", format!("{}: {e}", p.display())),
        }
        g.renombrar = None;
        ui.close();
    }
    ui.separator();
    if g.borrar.as_deref() == Some(p) {
        if ui.button(RichText::new("Confirmar: eliminar").color(theme::RED)).clicked() {
            match std::fs::remove_file(p) {
                Ok(()) => cambio = true,
                Err(e) => log::error("estilo visual", format!("{}: {e}", p.display())),
            }
            g.borrar = None;
            ui.close();
        }
    } else if ui.button("Eliminar").clicked() {
        g.borrar = Some(p.to_path_buf());
    }
    cambio
}

/// «Guardar como estilo…»: guarda el sombreado global y la pila actuales.
fn guardar_como(ed: &mut Editor, ui: &mut Ui, raiz: &Path) -> bool {
    let mut guardado = false;
    ui.menu_button("Guardar como estilo…", |ui| {
        ui.label("Nombre:");
        let r = ui.text_edit_singleline(&mut ed.galeria.nombre_nuevo);
        let nombre = ev::nombre_valido(&ed.galeria.nombre_nuevo);
        if let Some(n) = &nombre
            && ev::ruta(raiz, n).exists()
        {
            ui.label(RichText::new("Se reemplazará el estilo existente").small().color(theme::YELLOW));
        }
        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if (ui.add_enabled(nombre.is_some(), egui::Button::new("Guardar")).clicked() || enter)
            && let Some(n) = nombre
        {
            let ruta = ev::ruta(raiz, &n);
            match EstiloVisual::de_entorno(&ed.scene.env).guardar(&ruta) {
                Ok(()) => {
                    log::info("estilo visual", format!("guardado {}", ed.project.relative(&ruta)));
                    ed.galeria.nombre_nuevo.clear();
                    guardado = true;
                }
                Err(e) => log::error("estilo visual", e),
            }
            ui.close();
        }
    })
    .response
    .on_hover_text(format!("Guarda el sombreado global y el posprocesado actuales en {}/", ev::DIR));
    guardado
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tarjetas_en_orden_y_crt_solo_con_su_efecto() {
        let raiz = std::env::temp_dir().join(format!("maker-galeria-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&raiz);
        std::fs::create_dir_all(&raiz).unwrap();
        let usuario = vec![(raiz.join("estilos/Mío.mvisual"), ev::INTEGRADOS[1].estilo().escalado(0.5))];
        let v = tarjetas(&raiz, &usuario);
        assert_eq!(v[0].nombre, "Ninguno");
        assert!(v.iter().all(|t| t.clave != "i:crt"), "sin el efecto CRT no aparece");
        assert_eq!(v.last().map(|t| (t.nombre.as_str(), t.clave.as_str())), Some(("Mío", "u:Mío.mvisual")));
        assert_eq!(v.len(), 1 + ev::INTEGRADOS.len() - 1 + 1);
        let crt = ev::INTEGRADOS.iter().find(|i| i.id == "crt").unwrap().estilo();
        let p = raiz.join(crt.efectos_propios()[0]);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, maker_engine::postproceso::EXAMPLES[0].1).unwrap();
        assert!(tarjetas(&raiz, &[]).iter().any(|t| t.clave == "i:crt"));

        // La activa se reconoce con su intensidad; tras cambiarla, Ninguno.
        let mut env = maker_engine::scene::Environment::default();
        assert_eq!(activa(&v, &env, Partes::Todo, 1.0), Some((0, 1.0)));
        v[2].estilo.escalado(0.7).aplicar(&mut env, Partes::Todo);
        let (i, k) = activa(&v, &env, Partes::Todo, 1.0).unwrap();
        assert_eq!(i, 2);
        assert!((k - 0.7).abs() < 1e-3);
        let _ = std::fs::remove_dir_all(&raiz);
    }

    #[test]
    fn antirrebote_espera_a_que_todo_este_quieto() {
        let mut a = Antirrebote::default();
        let t0 = Instant::now();
        let ms = Duration::from_millis;
        assert!(!a.listo(1, t0, ms(300)));
        assert!(!a.listo(1, t0 + ms(200), ms(300)));
        // La cámara se mueve: vuelve a esperar.
        assert!(!a.listo(2, t0 + ms(250), ms(300)));
        assert!(!a.listo(2, t0 + ms(500), ms(300)));
        assert!(a.listo(2, t0 + ms(560), ms(300)));
        assert!(a.listo(2, t0 + ms(900), ms(300)));
    }
}
