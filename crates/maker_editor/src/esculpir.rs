//! Modo Esculpir (como el *Sculpt Mode* de Blender, con los pinceles y atajos de SculptForge).
//!
//! Al entrar, la malla de la entidad seleccionada (primitiva o modelo) se convierte en una
//! escultura (`maker_sculpt`) y el renderizador la dibuja desde un búfer dinámico del que cada
//! fotograma solo se suben los rangos tocados. Arrastrar con el botón izquierdo sobre el modelo
//! esculpe y fuera de él orbita. Al salir (o con Ctrl+S) la escultura se hornea a un `.glb` en
//! `assets/esculturas/` y la entidad lo referencia con un paso normal del historial.

use crate::editor::Editor;
use crate::gizmo::View;
use crate::icons::{self, Icon};
use crate::theme;
use egui::{Color32, Pos2, Rect, Sense, Stroke};
use glam::Vec2;
use maker_engine::components::*;
use maker_engine::model::ModelData;
use maker_engine::{biblioteca, log};
use maker_sculpt::brush::{FALLOFF_RANGE, INTENSITY_RANGE, SIZE_RANGE};
use maker_sculpt::{Axis, Brush, Controller, CursorInfo, PointerState, Sculpt, ToolSettings, ViewParams, glb, primitives};
use std::ops::Range;
use std::path::Path;
use std::time::SystemTime;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Modo {
    #[default]
    Objeto,
    Esculpir,
}

/// Carpeta de las esculturas dentro del proyecto.
pub const CARPETA: &str = "assets/esculturas";
/// Vértices de la base al esculpir una primitiva.
const BASE_VERTICES: usize = 60_000;
/// Celdas por arista de las esferas de «Nueva escultura».
pub const RESOLUCIONES: [u32; 4] = [64, 128, 256, 512];
/// Más allá de esto «Subdividir» no sigue (cada paso multiplica por cuatro).
const MAX_SUBDIVIDIR: usize = 4_000_000;
/// Alto del estante de pinceles.
const ESTANTE: f32 = 58.0;

/// Una escultura abierta.
pub struct Sesion {
    pub entidad: EntityId,
    pub sculpt: Sculpt,
    ctrl: Controller,
    /// Cambios sin hornear al `.glb`.
    pub cambios: bool,
    /// El arrastre izquierdo en curso orbita (empezó fuera del modelo).
    orbitando: bool,
    rangos: Vec<Range<u32>>,
    version_caja: u64,
}

impl Sesion {
    fn new(entidad: EntityId, sculpt: Sculpt) -> Self {
        Sesion { entidad, sculpt, ctrl: Controller::default(), cambios: false, orbitando: false, rangos: Vec::new(), version_caja: u64::MAX }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Deslizador {
    Tamano,
    Intensidad,
    Caida,
}

/// Deslizador emergente en el cursor (teclas S, U y O).
pub struct Emergente {
    pub que: Deslizador,
    pub pos: Pos2,
    pub texto: String,
    foco: bool,
}

pub struct Esculpir {
    pub herramienta: ToolSettings,
    pub sesion: Option<Sesion>,
    /// Última escultura horneada (ruta, fecha del archivo): se sigue con su historial si se
    /// vuelve a ella sin que el archivo haya cambiado.
    guardada: Option<(String, Option<SystemTime>, Sesion)>,
    pub alambre: bool,
    pub emergente: Option<Emergente>,
    /// Cantidad de las operaciones de malla completa (0..100).
    pub cantidad: f32,
    presion: f32,
    /// Resolución pedida para la próxima esfera nueva.
    pub base: Option<u32>,
}

impl Default for Esculpir {
    fn default() -> Self {
        Self { herramienta: ToolSettings::default(), sesion: None, guardada: None, alambre: false, emergente: None, cantidad: 30.0, presion: 1.0, base: None }
    }
}

/// Hay una escultura abierta.
pub fn activo(ed: &Editor) -> bool {
    ed.modo == Modo::Esculpir && ed.esculpir.sesion.is_some()
}

/// La selección se puede esculpir.
pub fn puede_entrar(ed: &Editor) -> bool {
    !ed.is_playing() && ed.selection.is_some_and(|s| ed.scene.has::<MeshRenderer>(s))
}

fn fecha(p: &Path) -> Option<SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

/// Entra en el modo con la entidad seleccionada.
pub fn entrar(ed: &mut Editor) {
    if activo(ed) {
        return;
    }
    if ed.is_playing() {
        ed.status = "No se puede esculpir durante Play".into();
        return;
    }
    let Some(id) = ed.selection else {
        ed.status = "Selecciona una entidad con malla para esculpir".into();
        return;
    };
    let Some(mr) = ed.scene.get::<MeshRenderer>(id) else {
        ed.status = "Selecciona una entidad con malla para esculpir".into();
        return;
    };
    let t0 = std::time::Instant::now();
    let sesion = match &mr.mesh {
        MeshSource::Primitive(p) => {
            let base = match (ed.esculpir.base.take(), p) {
                (Some(res), PrimitiveShape::Sphere) => primitives::quad_sphere(res, 0.5),
                _ => primitives::sculptable(*p, BASE_VERTICES),
            };
            Sesion::new(id, Sculpt::from_mesh(base))
        }
        MeshSource::Model(rel) => {
            let full = biblioteca::resolve(rel, Some(&ed.project.root));
            let previa = ed.esculpir.guardada.take().filter(|(r, t, s)| r == rel && s.entidad == id && *t == fecha(&full));
            match previa {
                Some((_, _, s)) => s,
                None => match ModelData::load(&full).and_then(|d| Sculpt::from_model(&d)) {
                    Ok(s) => Sesion::new(id, s),
                    Err(e) => {
                        ed.status = format!("No se puede esculpir: {e}");
                        log::error("esculpir", e);
                        return;
                    }
                },
            }
        }
    };
    ed.renderer.sculpt_begin(id, &sesion.sculpt.render_model());
    let (n, t) = (sesion.sculpt.vertex_count(), sesion.sculpt.triangle_count());
    ed.esculpir.sesion = Some(sesion);
    ed.modo = Modo::Esculpir;
    ed.gizmo.end();
    let nombre = ed.scene.name(id);
    ed.status = format!("Esculpir {nombre}: {n} vértices, {t} triángulos");
    log::info("esculpir", format!("{} en {:.0} ms", ed.status, t0.elapsed().as_secs_f64() * 1000.0));
}

/// Hornea y vuelve al modo Objeto.
pub fn salir(ed: &mut Editor) {
    if ed.modo != Modo::Esculpir {
        return;
    }
    hornear(ed);
    ed.renderer.sculpt_end();
    ed.modo = Modo::Objeto;
    ed.esculpir.emergente = None;
    if let Some(s) = ed.esculpir.sesion.take()
        && let Some(MeshSource::Model(rel)) = ed.scene.get::<MeshRenderer>(s.entidad).map(|m| m.mesh)
        && !s.cambios
    {
        let t = fecha(&biblioteca::resolve(&rel, Some(&ed.project.root)));
        ed.esculpir.guardada = Some((rel, t, s));
    }
}

/// Deja la escultura sin hornear (la entidad desapareció o se cambió de escena).
fn abandonar(ed: &mut Editor) {
    ed.renderer.sculpt_end();
    ed.esculpir.sesion = None;
    ed.esculpir.emergente = None;
    ed.modo = Modo::Objeto;
}

/// Nombre de archivo sin caracteres problemáticos.
pub fn nombre_archivo(nombre: &str) -> String {
    let s: String = nombre.trim().chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    let s = s.trim_matches('_').to_string();
    if s.is_empty() { "escultura".into() } else { s }
}

/// Ruta libre `assets/esculturas/<nombre>[_N].glb`.
pub fn ruta_libre(raiz: &Path, nombre: &str) -> String {
    let base = nombre_archivo(nombre);
    (1..)
        .map(|i| if i == 1 { format!("{CARPETA}/{base}.glb") } else { format!("{CARPETA}/{base}_{i}.glb") })
        .find(|r| !raiz.join(r).exists())
        .unwrap()
}

/// Escribe la escultura a su `.glb` y hace que la entidad lo use (un paso del historial).
pub fn hornear(ed: &mut Editor) -> bool {
    let Some(s) = ed.esculpir.sesion.as_mut() else { return false };
    if s.ctrl.active() {
        s.ctrl.release(&mut s.sculpt);
    }
    if !s.cambios {
        return true;
    }
    let id = s.entidad;
    let Some(mr) = ed.scene.get::<MeshRenderer>(id) else { return false };
    let nombre = ed.scene.name(id);
    let rel = match &mr.mesh {
        MeshSource::Model(r) if r.starts_with(&format!("{CARPETA}/")) => r.clone(),
        _ => ruta_libre(&ed.project.root, &nombre),
    };
    let path = ed.project.root.join(&rel);
    let t0 = std::time::Instant::now();
    if let Err(e) = glb::write_glb(&s.sculpt.to_model(), &path, &nombre) {
        ed.status = format!("No se pudo guardar la escultura: {e}");
        log::error("esculpir", e);
        return false;
    }
    s.cambios = false;
    let src = MeshSource::Model(rel.clone());
    if mr.mesh != src
        && let Some(mut m) = ed.scene.get_mut::<MeshRenderer>(id)
    {
        m.mesh = src.clone();
    }
    ed.renderer.reload_model(&src);
    let raiz = ed.project.root.clone();
    ed.contenido.invalidar_miniatura(&raiz, &rel);
    ed.commit(&format!("Esculpir {nombre}"));
    ed.dirty = true;
    ed.status = format!("Escultura guardada en {rel}");
    log::info("esculpir", format!("{} ({:.0} ms)", ed.status, t0.elapsed().as_secs_f64() * 1000.0));
    true
}

/// Deshacer dentro del modo (no toca el historial de la escena).
pub fn deshacer(ed: &mut Editor) {
    if let Some(s) = ed.esculpir.sesion.as_mut() {
        s.ctrl.release(&mut s.sculpt);
        match s.sculpt.undo() {
            Some(l) => {
                s.cambios = true;
                ed.status = format!("Deshecho: {l}");
            }
            None => ed.status = "No hay más trazos que deshacer".into(),
        }
    }
}

pub fn rehacer(ed: &mut Editor) {
    if let Some(s) = ed.esculpir.sesion.as_mut() {
        s.ctrl.release(&mut s.sculpt);
        if let Some(l) = s.sculpt.redo() {
            s.cambios = true;
            ed.status = format!("Rehecho: {l}");
        }
    }
}

/// Etiquetas de deshacer y rehacer del modo, para el menú Editar.
pub fn etiquetas(ed: &Editor) -> Option<(Option<String>, Option<String>)> {
    let s = ed.esculpir.sesion.as_ref().filter(|_| ed.modo == Modo::Esculpir)?;
    Some((s.sculpt.undo.undo_label().map(str::to_string), s.sculpt.undo.redo_label().map(str::to_string)))
}

/// Cada fotograma: comprobar que la escultura sigue siendo válida, vaciar cambios y subir a la
/// GPU solo los rangos tocados.
pub fn actualizar(ed: &mut Editor) {
    if ed.modo != Modo::Esculpir {
        return;
    }
    let Some(ent) = ed.esculpir.sesion.as_ref().map(|s| s.entidad) else {
        ed.modo = Modo::Objeto;
        return;
    };
    if !ed.scene.contains(ent) || !ed.scene.has::<MeshRenderer>(ent) {
        abandonar(ed);
        return;
    }
    if ed.is_playing() || ed.selection != Some(ent) {
        salir(ed);
        return;
    }
    let s = ed.esculpir.sesion.as_mut().unwrap();
    if ed.renderer.sculpt_entity() != Some(ent) {
        // La caché de modelos se vació (reimportación): volver a subirla entera.
        ed.renderer.sculpt_begin(ent, &s.sculpt.render_model());
        s.version_caja = u64::MAX;
    }
    s.sculpt.flush();
    s.sculpt.take_gpu_ranges(&mut s.rangos);
    if !s.rangos.is_empty() {
        let sc = &s.sculpt;
        ed.renderer.sculpt_write(&s.rangos, |i| sc.render_vertex(i));
    }
    if s.sculpt.version != s.version_caja && !s.ctrl.active() {
        let (a, b) = s.sculpt.mesh.bounds();
        ed.renderer.sculpt_set_bounds(maker_engine::mesh::Aabb { min: a, max: b });
        s.version_caja = s.sculpt.version;
    }
    if ed.esculpir.alambre != ed.renderer.sculpt_has_wire() {
        let aristas = ed.esculpir.alambre.then(|| s.sculpt.wire_edges());
        ed.renderer.sculpt_set_wire(aristas.as_deref());
    }
    if s.cambios {
        ed.dirty = true;
    }
}

fn vista(view: &View, rect: Rect) -> ViewParams {
    ViewParams { view_proj: view.view_proj, eye: view.eye, size: Vec2::new(rect.width(), rect.height()), world_per_px_at_1: view.world_per_px_at_1 }
}

/// Entrada del visor en el modo: esculpir sobre el modelo, orbitar fuera de él.
pub fn entrada(ed: &mut Editor, ctx: &egui::Context, resp: &egui::Response, rect: Rect, view: &View) {
    let Some(ent) = ed.esculpir.sesion.as_ref().map(|s| s.entidad) else { return };
    let world = ed.scene.global(ent);
    let vp = vista(view, rect);
    let (pressed, down, pos, origin, mods, delta, toque) = ctx.input(|i| {
        let toque = i.events.iter().rev().find_map(|e| if let egui::Event::Touch { force: Some(f), .. } = e { Some(*f) } else { None });
        (i.pointer.primary_pressed(), i.pointer.primary_down(), i.pointer.latest_pos(), i.pointer.press_origin(), i.modifiers, i.pointer.delta(), toque)
    });
    let esc = &mut ed.esculpir;
    // Presión del lápiz si el sistema la da (eventos táctiles con fuerza); con ratón, 1.
    match toque {
        Some(f) => esc.presion = f.clamp(0.0, 1.0),
        None if pressed => esc.presion = 1.0,
        None => {}
    }
    let local = |p: Pos2| Vec2::new(p.x - rect.min.x, p.y - rect.min.y);
    let estado = |p: Pos2, presion: f32| PointerState { pos: local(p), pressure: presion, shift: mods.shift, ctrl: mods.ctrl, alt: mods.alt };
    let s = esc.sesion.as_mut().unwrap();
    if pressed
        && resp.hovered()
        && esc.emergente.is_none()
        && let Some(p) = origin.or(pos)
    {
        if s.ctrl.press(&mut s.sculpt, &esc.herramienta, &vp, &world, &estado(p, esc.presion)) {
            s.cambios = true;
        } else {
            s.orbitando = true;
        }
    }
    if down {
        if s.ctrl.active() {
            if let Some(p) = pos {
                s.ctrl.drag(&mut s.sculpt, &esc.herramienta, &vp, &world, &estado(p, esc.presion));
            }
            ctx.request_repaint();
        } else if s.orbitando {
            ed.cam.orbit(delta.x, delta.y);
            ctx.request_repaint();
        }
    } else {
        if s.ctrl.active() {
            s.ctrl.release(&mut s.sculpt);
        }
        s.orbitando = false;
    }
}

/// Anillos del pincel sobre la superficie y puntos de las copias de simetría.
pub fn dibujar_cursor(ed: &Editor, ctx: &egui::Context, painter: &egui::Painter, resp: &egui::Response, rect: Rect, view: &View) {
    let Some(s) = ed.esculpir.sesion.as_ref() else { return };
    if s.orbitando || ed.esculpir.emergente.is_some() || !resp.hovered() {
        return;
    }
    let (pos, mods) = ctx.input(|i| (i.pointer.latest_pos(), i.modifiers));
    let Some(p) = pos else { return };
    let world = ed.scene.global(s.entidad);
    let t = &ed.esculpir.herramienta;
    let screen = Vec2::new(p.x - rect.min.x, p.y - rect.min.y);
    let Some(info) = Controller::cursor(&s.sculpt, t, &vista(view, rect), &world, screen, mods.shift, mods.ctrl, mods.alt) else { return };
    let color = if mods.ctrl {
        theme::TEXT1
    } else if (mods.alt ^ t.subtract) && t.brush.invertible() && !mods.shift {
        theme::RED
    } else {
        theme::ACCENT
    };
    ctx.set_cursor_icon(egui::CursorIcon::Crosshair);
    anillos(painter, view, &info, color);
}

fn anillos(painter: &egui::Painter, view: &View, info: &CursorInfo, color: Color32) {
    let n = info.normal;
    let (a, b) = n.any_orthonormal_pair();
    let circulo = |r: f32| -> Vec<Pos2> {
        (0..=64)
            .filter_map(|k| {
                let t = k as f32 / 64.0 * std::f32::consts::TAU;
                view.to_screen(info.center + (a * t.cos() + b * t.sin()) * r)
            })
            .collect()
    };
    let sombra = Color32::from_rgba_unmultiplied(255, 255, 255, 170);
    for (r, w) in [(info.radius, 1.6), (info.radius * info.inner, 1.0)] {
        let pts = circulo(r);
        painter.add(egui::Shape::line(pts.clone(), Stroke::new(w + 2.0, sombra)));
        painter.add(egui::Shape::line(pts, Stroke::new(w, color)));
    }
    if let (Some(c), Some(e)) = (view.to_screen(info.center), view.to_screen(info.center + n * info.radius * 0.35)) {
        painter.line_segment([c, e], Stroke::new(1.4, color));
    }
    for c in &info.copies {
        if let Some(q) = view.to_screen(*c) {
            painter.circle(q, 3.0, color, Stroke::new(1.0, sombra));
        }
    }
}

// ───────────────────────────── interfaz ─────────────────────────────

/// Selector de modo al principio de la barra del visor.
pub fn selector_modo(ed: &mut Editor, ui: &mut egui::Ui) {
    let esculpiendo = ed.modo == Modo::Esculpir;
    if ui.add(egui::Button::selectable(!esculpiendo, "Objeto")).on_hover_text("Mover, rotar y escalar entidades (Tab)").clicked() && esculpiendo {
        salir(ed);
    }
    let puede = esculpiendo || puede_entrar(ed);
    let r = ui
        .add_enabled(puede, egui::Button::image_and_text(icons::image(ui.ctx(), Icon::Esculpir, if esculpiendo { theme::ACCENT } else { theme::TEXT2 }), "Esculpir").selected(esculpiendo))
        .on_hover_text("Esculpir la malla seleccionada (Tab)")
        .on_disabled_hover_text("Selecciona una entidad con malla");
    if r.clicked() && !esculpiendo {
        entrar(ed);
    }
}

/// Valor y rango del deslizador (la intensidad de Suavizar es la suya propia).
fn valor(t: &mut ToolSettings, d: Deslizador) -> (&mut f32, std::ops::RangeInclusive<f32>) {
    match d {
        Deslizador::Tamano if t.world_size => (&mut t.world_radius, 0.0005..=100.0),
        Deslizador::Tamano => (&mut t.current_mut().size, SIZE_RANGE),
        Deslizador::Intensidad => (&mut t.current_mut().intensity, INTENSITY_RANGE),
        Deslizador::Caida => (&mut t.current_mut().falloff, FALLOFF_RANGE),
    }
}

/// Barra superior del modo: tamaño, intensidad, caída, sumar/restar, simetría, opciones,
/// máscara y malla.
pub fn barra(ed: &mut Editor, ui: &mut egui::Ui) {
    let t = &mut ed.esculpir.herramienta;
    let mundo = t.world_size;
    {
        let (v, r) = valor(t, Deslizador::Tamano);
        let dv = if mundo { egui::DragValue::new(v).range(r).speed(0.002).prefix("Tamaño ").suffix(" m") } else { egui::DragValue::new(v).range(r).speed(0.5).prefix("Tamaño ").suffix(" px").max_decimals(0) };
        ui.add(dv).on_hover_text("Radio del pincel (S, [ y ])");
    }
    {
        let (v, r) = valor(t, Deslizador::Intensidad);
        ui.add(egui::DragValue::new(v).range(r).speed(0.5).prefix("Intensidad ").max_decimals(0)).on_hover_text(if t.brush == Brush::Smooth { "Intensidad propia de Suavizar, la que usa Mayús (U)" } else { "Fuerza de cada toque (U)" });
    }
    {
        let (v, r) = valor(t, Deslizador::Caida);
        ui.add(egui::DragValue::new(v).range(r).speed(0.5).prefix("Caída ").max_decimals(0)).on_hover_text("Por debajo de 0 la punta se ensancha y endurece; por encima, se estrecha y suaviza (O)");
    }
    ui.separator();
    if ui.add(egui::Button::selectable(!t.subtract, "Añadir")).on_hover_text("Los pinceles añaden (Alt invierte durante el trazo)").clicked() {
        t.subtract = false;
    }
    if ui.add(egui::Button::selectable(t.subtract, "Restar")).on_hover_text("Los pinceles restan (Alt invierte durante el trazo)").clicked() {
        t.subtract = true;
    }
    ui.separator();
    for (i, eje) in ["X", "Y", "Z"].into_iter().enumerate() {
        let tip = if i == 0 { "Simetría en X (X)" } else if i == 1 { "Simetría en Y" } else { "Simetría en Z" };
        ui.toggle_value(&mut t.symmetry.mirror[i], eje).on_hover_text(tip);
    }
    ui.toggle_value(&mut t.symmetry.radial, "R").on_hover_text("Simetría radial: copias alrededor de un eje");
    if t.symmetry.radial {
        ui.add(egui::DragValue::new(&mut t.symmetry.radial_count).range(maker_sculpt::symmetry::RADIAL_MIN..=maker_sculpt::symmetry::RADIAL_MAX).speed(0.1)).on_hover_text("Copias radiales");
        egui::ComboBox::from_id_salt("esc-eje-radial").width(34.0).selected_text(t.symmetry.radial_axis.label()).show_ui(ui, |ui| {
            for a in Axis::ALL {
                ui.selectable_value(&mut t.symmetry.radial_axis, a, a.label());
            }
        });
    }
    ui.toggle_value(&mut t.symmetry.local, "Local").on_hover_text("Simetría por el centro de la pieza en vez del origen");
    ui.separator();
    ui.toggle_value(&mut t.front_only, "Frontal").on_hover_text("Solo caras frontales: no toca la cara de atrás de las partes finas");
    ui.toggle_value(&mut t.connected, "Conectado").on_hover_text("Solo lo unido al punto bajo el cursor (no salta a piezas cercanas)");
    ui.toggle_value(&mut t.taubin, "Taubin").on_hover_text("Suavizado que conserva el volumen");
    ui.menu_button("Trazo", |ui| {
        ui.checkbox(&mut t.view_falloff, "Caída de vista").on_hover_text("Un cilindro a lo largo de la vista que atraviesa el modelo");
        ui.checkbox(&mut t.world_size, "Tamaño en el mundo").on_hover_text("El tamaño en metros en vez de en píxeles de pantalla");
        ui.checkbox(&mut t.pressure_strength, "Presión → intensidad").on_hover_text("La presión del lápiz escala la intensidad (si el sistema la da)");
        ui.add(egui::Slider::new(&mut t.lazy, 0.0..=200.0).text("Cuerda (px)")).on_hover_text("El pincel va detrás del cursor con una cuerda: trazos firmes");
        ui.add(egui::Slider::new(&mut t.spacing, 2.0..=50.0).text("Espaciado (%)")).on_hover_text("Distancia entre toques, en % del radio");
    });
    let Some(s) = ed.esculpir.sesion.as_mut() else { return };
    ui.menu_button("Máscara", |ui| {
        let mut hecho = true;
        if ui.button("Invertir").clicked() {
            s.sculpt.mask_invert();
        } else if ui.button("Borrar").clicked() {
            s.sculpt.mask_clear();
        } else if ui.button("Difuminar").clicked() {
            s.sculpt.mask_blur();
        } else if ui.button("Afilar").clicked() {
            s.sculpt.mask_sharpen();
        } else {
            hecho = false;
        }
        if hecho {
            s.cambios = true;
        }
    })
    .response
    .on_hover_text("Ctrl pinta la máscara, Ctrl+Alt la borra");
    let cantidad = &mut ed.esculpir.cantidad;
    let mut subdividir = false;
    ui.menu_button("Malla", |ui| {
        ui.add(egui::Slider::new(cantidad, 0.0..=100.0).text("Cantidad"));
        let mut hecho = true;
        if ui.button("Suavizar").on_hover_text("Suaviza toda la malla (respeta la máscara y «Taubin»)").clicked() {
            s.sculpt.smooth_all(*cantidad, t.taubin);
        } else if ui.button("Relajar").on_hover_text("Reparte los vértices por la superficie sin cambiar la forma").clicked() {
            s.sculpt.relax_all(*cantidad);
        } else if ui.button("Hinchar").on_hover_text("Hincha toda la malla como un globo").clicked() {
            s.sculpt.inflate_all(*cantidad);
        } else {
            hecho = false;
        }
        if hecho {
            s.cambios = true;
        }
        ui.separator();
        let n = s.sculpt.vertex_count();
        if ui
            .add_enabled(n * 4 <= MAX_SUBDIVIDIR, egui::Button::new("Subdividir"))
            .on_hover_text(format!("Divide cada triángulo en cuatro (~{} vértices). Borra el historial de trazos y la máscara.", n * 4))
            .on_disabled_hover_text("Demasiados vértices")
            .clicked()
        {
            subdividir = true;
            ui.close();
        }
    });
    ui.toggle_value(&mut ed.esculpir.alambre, "Alambre").on_hover_text("Ver las aristas (Mayús+F)");
    if subdividir {
        dividir(ed);
    }
}

/// Subdivide la escultura (cambia la topología: historial nuevo).
fn dividir(ed: &mut Editor) {
    let Some(s) = ed.esculpir.sesion.as_mut() else { return };
    let m = primitives::subdivide(&s.sculpt.render_model().mesh);
    let mut data = s.sculpt.render_model();
    data.mesh = m;
    match Sculpt::from_model(&data) {
        Ok(nueva) => {
            s.sculpt = nueva;
            s.cambios = true;
            s.version_caja = u64::MAX;
            ed.renderer.sculpt_begin(s.entidad, &s.sculpt.render_model());
            ed.status = format!("Subdividida: {} vértices", s.sculpt.vertex_count());
        }
        Err(e) => ed.status = format!("No se pudo subdividir: {e}"),
    }
}

/// Alto que ocupa el estante bajo el visor.
pub fn alto_estante() -> f32 {
    ESTANTE
}

/// Estante de pinceles: icono, nombre y tecla.
pub fn estante(ed: &mut Editor, ui: &mut egui::Ui, rect: Rect) {
    let ctx = ui.ctx().clone();
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, theme::CHROME);
    let n = Brush::ALL.len() as f32;
    let gap = 4.0;
    let w = ((rect.width() - 8.0 - gap * (n - 1.0)) / n).clamp(40.0, 72.0);
    let h = rect.height() - 8.0;
    let lado = icons::side(&ctx) * 2.0;
    let t = &mut ed.esculpir.herramienta;
    for (i, b) in Brush::ALL.into_iter().enumerate() {
        let r = Rect::from_min_size(Pos2::new(rect.min.x + 4.0 + i as f32 * (w + gap), rect.min.y + 4.0), egui::vec2(w, h));
        if r.min.x > rect.max.x {
            break;
        }
        let resp = ui.interact(r, ui.scope_id().with(("esc-pincel", i)), Sense::click());
        let activo = t.brush == b;
        let fondo = if activo {
            theme::ACCENT_SOFT
        } else if resp.hovered() {
            theme::CONTROL_HOVER
        } else {
            theme::CARD
        };
        painter.rect(r, 4.0, fondo, Stroke::new(if activo { 1.5 } else { 1.0 }, if activo { theme::ACCENT } else { theme::STROKE3 }), egui::StrokeKind::Inside);
        let tex = icons::texture(&ctx, Icon::pincel(b));
        let ir = Rect::from_center_size(Pos2::new(r.center().x, r.min.y + 4.0 + lado * 0.5), egui::vec2(lado, lado));
        painter.image(tex.id(), ir, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), if activo { theme::ACCENT } else { theme::TEXT1 });
        painter.text(Pos2::new(r.center().x, r.max.y - 3.0), egui::Align2::CENTER_BOTTOM, b.name(), egui::FontId::proportional(10.0), if activo { theme::ACCENT } else { theme::TEXT2 });
        if let Some(k) = b.key() {
            painter.text(r.min + egui::vec2(4.0, 2.0), egui::Align2::LEFT_TOP, k.to_string(), egui::FontId::monospace(9.0), theme::TEXT3);
        }
        let tip = match b.key() {
            Some(k) => format!("{} ({k}): {}", b.name(), b.tooltip()),
            None => format!("{}: {}", b.name(), b.tooltip()),
        };
        if resp.on_hover_text(tip).clicked() {
            t.brush = b;
        }
    }
}

/// Atajos del modo (solo sin escribir en un campo). `pos` es el puntero.
pub fn atajos(ed: &mut Editor, i: &mut egui::InputState) {
    use egui::{Key as K, Modifiers as M};
    if i.consume_key(M::NONE, K::Tab) {
        salir(ed);
        return;
    }
    // Borrar o duplicar la entidad que se esculpe no tiene sentido aquí.
    i.consume_key(M::NONE, K::Delete);
    i.consume_key(M::COMMAND, K::D);
    let flying = i.pointer.secondary_down();
    let t = &mut ed.esculpir.herramienta;
    for (k, key) in [(1u8, K::Num1), (2, K::Num2), (3, K::Num3), (4, K::Num4), (5, K::Num5), (6, K::Num6), (7, K::Num7), (8, K::Num8), (9, K::Num9), (0, K::Num0)] {
        if i.consume_key(M::NONE, key)
            && let Some(b) = Brush::from_key(k)
        {
            t.brush = b;
        }
    }
    let menor = i.consume_key(M::NONE, K::OpenBracket) || i.events.iter().any(|e| matches!(e, egui::Event::Text(s) if s == "["));
    let mayor = i.consume_key(M::NONE, K::CloseBracket) || i.events.iter().any(|e| matches!(e, egui::Event::Text(s) if s == "]"));
    if menor || mayor {
        let f = if mayor { 1.1 } else { 1.0 / 1.1 };
        if t.world_size {
            t.world_radius = (t.world_radius * f).max(0.0005);
        } else {
            let c = t.current_mut();
            c.size = (c.size * f).clamp(*SIZE_RANGE.start(), *SIZE_RANGE.end());
        }
    }
    if i.consume_key(M::SHIFT, K::F) {
        ed.esculpir.alambre = !ed.esculpir.alambre;
    }
    if flying {
        return;
    }
    if i.consume_key(M::NONE, K::X) {
        t.symmetry.mirror[0] = !t.symmetry.mirror[0];
    }
    let pos = i.pointer.latest_pos();
    for (key, que) in [(K::S, Deslizador::Tamano), (K::U, Deslizador::Intensidad), (K::O, Deslizador::Caida)] {
        if i.consume_key(M::NONE, key)
            && let Some(p) = pos
        {
            ed.esculpir.emergente = Some(Emergente { que, pos: p + egui::vec2(12.0, 12.0), texto: String::new(), foco: true });
        }
    }
}

/// El deslizador emergente: arrastrar y soltar lo cierra, o escribir un número y Enter; Esc
/// lo cierra sin más.
pub fn emergente(ed: &mut Editor, ctx: &egui::Context) {
    let Some(e) = ed.esculpir.emergente.as_mut() else { return };
    let t = &mut ed.esculpir.herramienta;
    let mut cerrar = false;
    let area = egui::Area::new(egui::Id::unique("esc-emergente")).order(egui::Order::Foreground).fixed_pos(e.pos).show(ctx, |ui| {
        egui::Frame::popup(ui.style()).show(ui, |ui| {
            ui.horizontal(|ui| {
                let nombre = match e.que {
                    Deslizador::Tamano => "Tamaño",
                    Deslizador::Intensidad => "Intensidad",
                    Deslizador::Caida => "Caída",
                };
                let (v, r) = valor(t, e.que);
                let s = ui.add(egui::Slider::new(v, r).text(nombre));
                if s.drag_stopped() {
                    cerrar = true;
                }
                let te = ui.add(egui::TextEdit::singleline(&mut e.texto).desired_width(48.0));
                if e.foco {
                    te.request_focus();
                    e.foco = false;
                }
                if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    if let Ok(x) = e.texto.trim().replace(',', ".").parse::<f32>() {
                        let (v, r) = valor(t, e.que);
                        *v = x.clamp(*r.start(), *r.end());
                    }
                    cerrar = true;
                }
            });
        });
    });
    let fuera = ctx.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !area.response.rect.contains(p)));
    if cerrar || fuera || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        ed.esculpir.emergente = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nombres_de_archivo_y_rutas_libres() {
        assert_eq!(nombre_archivo("Mi escultura 1"), "Mi_escultura_1");
        assert_eq!(nombre_archivo("  ¿?  "), "escultura");
        assert_eq!(nombre_archivo("Cabeza-ñú"), "Cabeza-ñú");
        let raiz = std::env::temp_dir().join(format!("maker-esc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&raiz);
        assert_eq!(ruta_libre(&raiz, "Busto"), "assets/esculturas/Busto.glb");
        std::fs::create_dir_all(raiz.join(CARPETA)).unwrap();
        std::fs::write(raiz.join("assets/esculturas/Busto.glb"), b"x").unwrap();
        assert_eq!(ruta_libre(&raiz, "Busto"), "assets/esculturas/Busto_2.glb");
        let _ = std::fs::remove_dir_all(&raiz);
    }
}
