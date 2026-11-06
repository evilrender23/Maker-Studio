//! Del ratón al pincel: espaciado de *dabs*, trazo con cuerda (*lazy*), línea recta con Mayús,
//! filtrado del punto de la superficie, radio en píxeles de pantalla y modificadores
//! (Alt invierte, Mayús suaviza, Ctrl pinta máscara).

use crate::brush::{Brush, BrushSettings};
use crate::falloff;
use crate::sculpt::{Action, Dab, DabOptions, Sculpt};
use crate::symmetry::{self, Symmetry};
use glam::{Mat3, Mat4, Vec2, Vec3};
use serde::{Deserialize, Serialize};

/// Cámara del visor para un fotograma. Coordenadas de pantalla en puntos, relativas a la
/// esquina superior izquierda del visor.
#[derive(Clone, Copy, Debug)]
pub struct ViewParams {
    pub view_proj: Mat4,
    pub eye: Vec3,
    pub size: Vec2,
    /// Unidades de mundo por punto de pantalla a distancia 1 de la cámara.
    pub world_per_px_at_1: f32,
}

impl ViewParams {
    /// Rayo de mundo por un punto de pantalla (origen en el plano cercano).
    pub fn ray(&self, s: Vec2) -> (Vec3, Vec3) {
        let ndc = Vec2::new(s.x / self.size.x.max(1.0) * 2.0 - 1.0, 1.0 - s.y / self.size.y.max(1.0) * 2.0);
        let inv = self.view_proj.inverse();
        let near = inv.project_point3(ndc.extend(0.0));
        let far = inv.project_point3(ndc.extend(1.0));
        (near, (far - near).normalize_or(Vec3::NEG_Z))
    }
    pub fn project(&self, p: Vec3) -> Option<Vec2> {
        let c = self.view_proj * p.extend(1.0);
        if c.w <= 1e-6 {
            return None;
        }
        let n = c.truncate() / c.w;
        Some(Vec2::new((n.x + 1.0) * 0.5 * self.size.x, (1.0 - n.y) * 0.5 * self.size.y))
    }
    /// Longitud de mundo que miden `px` puntos a la distancia de `at`.
    pub fn px_to_world(&self, at: Vec3, px: f32) -> f32 {
        (at - self.eye).length() * self.world_per_px_at_1 * px
    }
    /// Dirección hacia la que mira la cámara.
    pub fn forward(&self) -> Vec3 {
        self.ray(self.size * 0.5).1
    }
}

/// Todo lo que el usuario ajusta de las herramientas de esculpir.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ToolSettings {
    pub brush: Brush,
    /// Ajustes de cada pincel, en el orden de `Brush::ALL`.
    pub brushes: Vec<BrushSettings>,
    /// Restar en vez de añadir (Alt lo invierte durante un trazo).
    pub subtract: bool,
    pub front_only: bool,
    pub connected: bool,
    pub view_falloff: bool,
    /// Suavizado que conserva el volumen (Taubin).
    pub taubin: bool,
    pub symmetry: Symmetry,
    /// Longitud de la cuerda del trazo en píxeles (0 = sin cuerda).
    pub lazy: f32,
    /// Separación de los *dabs* en % del radio.
    pub spacing: f32,
    /// Radio en unidades del mundo en vez de píxeles.
    pub world_size: bool,
    pub world_radius: f32,
    /// La presión del lápiz escala la intensidad.
    pub pressure_strength: bool,
}

impl Default for ToolSettings {
    fn default() -> Self {
        Self {
            brush: Brush::Standard,
            brushes: vec![BrushSettings::default(); Brush::COUNT],
            subtract: false,
            front_only: true,
            connected: false,
            view_falloff: false,
            taubin: false,
            symmetry: Symmetry::default(),
            lazy: 0.0,
            spacing: 10.0,
            world_size: false,
            world_radius: 0.1,
            pressure_strength: true,
        }
    }
}

impl ToolSettings {
    pub fn of(&self, b: Brush) -> BrushSettings {
        self.brushes.get(b.index()).copied().unwrap_or_default()
    }
    pub fn of_mut(&mut self, b: Brush) -> &mut BrushSettings {
        if self.brushes.len() < Brush::COUNT {
            self.brushes.resize(Brush::COUNT, BrushSettings::default());
        }
        &mut self.brushes[b.index()]
    }
    pub fn current(&self) -> BrushSettings {
        self.of(self.brush)
    }
    pub fn current_mut(&mut self) -> &mut BrushSettings {
        let b = self.brush;
        self.of_mut(b)
    }
}

/// Estado del puntero en un fotograma.
#[derive(Clone, Copy, Debug)]
pub struct PointerState {
    pub pos: Vec2,
    /// 0..1 (1 con ratón).
    pub pressure: f32,
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
}

impl PointerState {
    pub fn at(pos: Vec2) -> Self {
        Self { pos, pressure: 1.0, shift: false, ctrl: false, alt: false }
    }
}

/// Lleva el pincel por la pantalla y suelta un *dab* cada `step` píxeles.
#[derive(Clone, Copy, Debug, Default)]
pub struct Sampler {
    /// Último *dab*.
    pub last: Vec2,
    /// Posición del pincel (con cuerda, va por detrás del cursor).
    pub brush: Vec2,
    /// Inicio de la línea recta (Mayús a mitad del trazo).
    pub anchor: Option<Vec2>,
}

/// Lleva `v` a la dirección de 45° más cercana (proyección sobre ella).
pub fn snap45(v: Vec2) -> Vec2 {
    if v.length_squared() < 1e-12 {
        return Vec2::ZERO;
    }
    let step = std::f32::consts::FRAC_PI_4;
    let a = (v.y.atan2(v.x) / step).round() * step;
    let d = Vec2::new(a.cos(), a.sin());
    d * v.dot(d).max(0.0)
}

impl Sampler {
    pub fn start(p: Vec2) -> Self {
        Self { last: p, brush: p, anchor: None }
    }

    /// Avanza hacia el cursor y deja en `out` los puntos donde caen *dabs* nuevos.
    pub fn advance(&mut self, cursor: Vec2, step: f32, lazy: f32, line: bool, out: &mut Vec<Vec2>) {
        out.clear();
        if line {
            if self.anchor.is_none() {
                self.anchor = Some(self.brush);
            }
        } else {
            self.anchor = None;
        }
        let target = match self.anchor {
            Some(a) => a + snap45(cursor - a),
            None => cursor,
        };
        if lazy > 0.0 && self.anchor.is_none() {
            let d = target - self.brush;
            let len = d.length();
            if len > lazy {
                self.brush += d * ((len - lazy) / len);
            }
        } else {
            self.brush = target;
        }
        let step = step.max(0.5);
        loop {
            let d = self.brush - self.last;
            let len = d.length();
            if len < step {
                break;
            }
            self.last += d / len * step;
            out.push(self.last);
        }
    }
}

/// Filtro ligero del punto de la superficie: atenúa el temblor a lo largo de la normal (los
/// bultos finos de una malla densa) y sigue al instante un escalón real.
#[derive(Clone, Copy, Debug, Default)]
pub struct PickSmoother {
    prev: Option<(Vec3, Vec3)>,
}

impl PickSmoother {
    pub fn filter(&mut self, p: Vec3, n: Vec3, radius: f32) -> (Vec3, Vec3) {
        let out = match self.prev {
            Some((pp, pn)) => {
                let off = (p - pp).dot(pn);
                if off.abs() > radius * 0.25 { (p, n) } else { (p - pn * (off * 0.5), (pn + n).normalize_or(n)) }
            }
            None => (p, n),
        };
        self.prev = Some(out);
        out
    }
}

/// Escala media de una matriz de mundo (para pasar radios del mundo a la malla).
pub fn world_scale(m: &Mat4) -> f32 {
    Mat3::from_mat4(*m).determinant().abs().cbrt().max(1e-9)
}

/// Dónde dibujar el cursor del pincel.
#[derive(Clone, Debug)]
pub struct CursorInfo {
    pub center: Vec3,
    /// Normal media de la zona (plano de los anillos).
    pub normal: Vec3,
    pub radius: f32,
    /// Radio relativo donde el pincel trabaja a media fuerza.
    pub inner: f32,
    /// Centros de las copias de simetría (sin la original).
    pub copies: Vec<Vec3>,
}

/// Mover: dónde empezó y, con Alt, a lo largo de qué normal.
#[derive(Clone, Copy, Debug)]
struct GrabState {
    press: Vec2,
    start: Vec3,
    normal: Option<(Vec3, Vec2)>,
}

struct Active {
    action: Action,
    settings: BrushSettings,
    started_shift: bool,
    sampler: Sampler,
    smoother: PickSmoother,
    opts: DabOptions,
    last_local: Vec3,
    last_world: Vec3,
    last_normal: Vec3,
    grab: Option<GrabState>,
    pressure: f32,
}

/// Convierte la entrada del puntero en trazos sobre una escultura.
#[derive(Default)]
pub struct Controller {
    st: Option<Active>,
    samples: Vec<Vec2>,
}

/// Qué hará un trazo que empiece con estos modificadores.
pub fn action_for(tool: &ToolSettings, shift: bool, ctrl: bool, alt: bool) -> (Action, BrushSettings) {
    let cur = tool.current();
    if ctrl {
        (Action::Mask { erase: alt }, cur)
    } else if shift || tool.brush == Brush::Smooth {
        let own = tool.of(Brush::Smooth);
        (Action::Smooth { taubin: tool.taubin }, BrushSettings { intensity: own.intensity, ..cur })
    } else {
        (Action::Brush { brush: tool.brush, invert: alt ^ tool.subtract }, cur)
    }
}

fn label(a: Action) -> String {
    match a {
        Action::Mask { erase: false } => "Pintar máscara".into(),
        Action::Mask { erase: true } => "Borrar máscara".into(),
        Action::Smooth { .. } => "Suavizar".into(),
        Action::Brush { brush, .. } => brush.name().into(),
    }
}

impl Controller {
    pub fn active(&self) -> bool {
        self.st.is_some()
    }

    /// Acción del trazo en curso.
    pub fn action(&self) -> Option<Action> {
        self.st.as_ref().map(|s| s.action)
    }

    fn options(tool: &ToolSettings, settings: &BrushSettings, sc: &Sculpt) -> DabOptions {
        DabOptions {
            falloff: settings.falloff,
            front_only: tool.front_only,
            connected: tool.connected,
            view_falloff: tool.view_falloff,
            symmetry: tool.symmetry,
            center: if tool.symmetry.local { sc.bounds_center() } else { Vec3::ZERO },
        }
    }

    fn radius_world(tool: &ToolSettings, settings: &BrushSettings, view: &ViewParams, at: Vec3) -> f32 {
        if tool.world_size { tool.world_radius.max(1e-6) } else { view.px_to_world(at, settings.size) }
    }

    /// Botón pulsado. Devuelve `false` si no toca el modelo (el visor orbita).
    pub fn press(&mut self, sc: &mut Sculpt, tool: &ToolSettings, view: &ViewParams, world: &Mat4, p: &PointerState) -> bool {
        self.st = None;
        let inv = world.inverse();
        let (o, d) = view.ray(p.pos);
        let (lo, ld) = (inv.transform_point3(o), inv.transform_vector3(d).normalize_or(Vec3::NEG_Z));
        let Some(hit) = sc.raycast(lo, ld) else { return false };
        let world_hit = world.transform_point3(hit.point);
        let (action, settings) = action_for(tool, p.shift, p.ctrl, p.alt);
        let opts = Self::options(tool, &settings, sc);
        let scale = world_scale(world);
        let radius = Self::radius_world(tool, &settings, view, world_hit) / scale;
        sc.begin_stroke(&label(action), matches!(action, Action::Mask { .. }));
        let pressure = p.pressure.clamp(0.0, 1.0);
        let mut st = Active {
            action,
            settings,
            started_shift: p.shift,
            sampler: Sampler::start(p.pos),
            smoother: PickSmoother::default(),
            opts,
            last_local: hit.point,
            last_world: world_hit,
            last_normal: hit.normal,
            grab: None,
            pressure,
        };
        let (_, hn) = st.smoother.filter(hit.point, hit.normal, radius);
        let dab = Dab { center: hit.point, normal: hn, radius, strength: self.strength(tool, &st), view_dir: ld, delta: Vec3::ZERO, screen_x: screen_x(view, p.pos, &inv) };
        if let Action::Brush { brush: Brush::Move, .. } = action {
            if sc.grab_begin(&dab, &st.opts) {
                let normal = if p.alt {
                    let n_local = sc.area_normal(hit.point, radius).unwrap_or(hit.normal);
                    let nw = inv.transpose().transform_vector3(n_local).normalize_or(Vec3::Y);
                    let a = view.project(world_hit);
                    let b = view.project(world_hit + nw * view.px_to_world(world_hit, 50.0));
                    let dir = match (a, b) {
                        (Some(a), Some(b)) if (b - a).length() > 5.0 => (b - a).normalize(),
                        _ => Vec2::new(0.0, -1.0),
                    };
                    Some((nw, dir))
                } else {
                    None
                };
                st.grab = Some(GrabState { press: p.pos, start: world_hit, normal });
            }
        } else {
            sc.dab(action, &dab, &st.opts);
        }
        self.st = Some(st);
        true
    }

    fn strength(&self, tool: &ToolSettings, st: &Active) -> f32 {
        let pr = if tool.pressure_strength { st.pressure } else { 1.0 };
        (st.settings.intensity / 100.0 * pr).clamp(0.0, 1.0)
    }

    /// El puntero se mueve con el botón pulsado.
    pub fn drag(&mut self, sc: &mut Sculpt, tool: &ToolSettings, view: &ViewParams, world: &Mat4, p: &PointerState) {
        let Some(mut st) = self.st.take() else { return };
        st.pressure = p.pressure.clamp(0.0, 1.0);
        let inv = world.inverse();
        if let Some(g) = st.grab {
            let delta_world = match g.normal {
                Some((nw, dir)) => {
                    let px = (p.pos - g.press).dot(dir);
                    nw * view.px_to_world(g.start, px)
                }
                None => {
                    let (o, d) = view.ray(p.pos);
                    let f = view.forward();
                    let den = d.dot(f);
                    if den.abs() < 1e-6 {
                        Vec3::ZERO
                    } else {
                        o + d * ((g.start - o).dot(f) / den) - g.start
                    }
                }
            };
            sc.grab_update(inv.transform_vector3(delta_world));
            self.st = Some(st);
            return;
        }
        let line = p.shift && !st.started_shift;
        let radius_px = if tool.world_size {
            let r = tool.world_radius;
            r / view.px_to_world(st.last_world, 1.0).max(1e-9)
        } else {
            st.settings.size
        };
        let step = (tool.spacing.clamp(1.0, 200.0) / 100.0 * radius_px).max(0.5);
        let mut samples = std::mem::take(&mut self.samples);
        st.sampler.advance(p.pos, step, tool.lazy, line, &mut samples);
        let scale = world_scale(world);
        for &s in &samples {
            let (o, d) = view.ray(s);
            let (lo, ld) = (inv.transform_point3(o), inv.transform_vector3(d).normalize_or(Vec3::NEG_Z));
            let (point, normal) = match sc.raycast(lo, ld) {
                Some(h) => (h.point, h.normal),
                None => {
                    // Fuera de la silueta: el pincel sigue a la profundidad del último punto
                    // (solo actúa si su esfera aún toca la malla).
                    let f = view.forward();
                    let den = d.dot(f);
                    if den.abs() < 1e-6 {
                        continue;
                    }
                    let pw = o + d * ((st.last_world - o).dot(f) / den);
                    (inv.transform_point3(pw), st.last_normal)
                }
            };
            let world_p = world.transform_point3(point);
            let radius = Self::radius_world(tool, &st.settings, view, world_p) / scale;
            let (pf, nf) = st.smoother.filter(point, normal, radius);
            let dab = Dab { center: pf, normal: nf, radius, strength: self.strength(tool, &st), view_dir: ld, delta: pf - st.last_local, screen_x: screen_x(view, s, &inv) };
            sc.dab(st.action, &dab, &st.opts);
            st.last_local = pf;
            st.last_world = world_p;
            st.last_normal = nf;
        }
        self.samples = samples;
        self.st = Some(st);
    }

    /// Botón suelto: cierra el trazo (un paso de deshacer).
    pub fn release(&mut self, sc: &mut Sculpt) {
        if self.st.take().is_some() {
            sc.end_stroke();
        }
    }

    /// Cursor del pincel bajo un punto de pantalla (en coordenadas de mundo).
    pub fn cursor(sc: &Sculpt, tool: &ToolSettings, view: &ViewParams, world: &Mat4, screen: Vec2, shift: bool, ctrl: bool, alt: bool) -> Option<CursorInfo> {
        let inv = world.inverse();
        let (o, d) = view.ray(screen);
        let hit = sc.raycast(inv.transform_point3(o), inv.transform_vector3(d).normalize_or(Vec3::NEG_Z))?;
        let (_, settings) = action_for(tool, shift, ctrl, alt);
        let world_hit = world.transform_point3(hit.point);
        let rw = Self::radius_world(tool, &settings, view, world_hit);
        let rl = rw / world_scale(world);
        let n_local = sc.area_normal(hit.point, rl).unwrap_or(hit.normal);
        let normal = inv.transpose().transform_vector3(n_local).normalize_or(Vec3::Y);
        let center = if tool.symmetry.local { sc.bounds_center() } else { Vec3::ZERO };
        let mut xf = Vec::new();
        tool.symmetry.transforms(&mut xf);
        let copies = xf.iter().skip(1).map(|m| world.transform_point3(symmetry::apply(m, center, hit.point))).collect();
        Some(CursorInfo { center: world_hit, normal, radius: rw, inner: falloff::half_radius(settings.falloff), copies })
    }
}

/// Derecha de la pantalla en el espacio local, en un punto.
fn screen_x(view: &ViewParams, s: Vec2, inv: &Mat4) -> Vec3 {
    let a = view.ray(s).1;
    let b = view.ray(s + Vec2::new(4.0, 0.0)).1;
    inv.transform_vector3(b - a).normalize_or(Vec3::X)
}

/// Simetría por defecto sin espejos (para pruebas y API).
pub fn no_symmetry() -> Symmetry {
    Symmetry { mirror: [false; 3], ..Default::default() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives;
    use maker_engine::math::{look_at, perspective};

    #[test]
    fn sampler_spaces_dabs_evenly() {
        let mut s = Sampler::start(Vec2::ZERO);
        let mut out = Vec::new();
        s.advance(Vec2::new(100.0, 0.0), 10.0, 0.0, false, &mut out);
        assert_eq!(out.len(), 10);
        for (i, p) in out.iter().enumerate() {
            assert!((p.x - 10.0 * (i + 1) as f32).abs() < 1e-3);
        }
        // Un movimiento corto no suelta nada, pero se acumula.
        s.advance(Vec2::new(104.0, 0.0), 10.0, 0.0, false, &mut out);
        assert!(out.is_empty());
        s.advance(Vec2::new(111.0, 0.0), 10.0, 0.0, false, &mut out);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn lazy_string_trails_cursor() {
        let mut s = Sampler::start(Vec2::ZERO);
        let mut out = Vec::new();
        s.advance(Vec2::new(30.0, 0.0), 5.0, 50.0, false, &mut out);
        assert!(out.is_empty(), "la cuerda no está tensa");
        s.advance(Vec2::new(80.0, 0.0), 5.0, 50.0, false, &mut out);
        assert!((s.brush.x - 30.0).abs() < 1e-3, "{}", s.brush.x);
        assert_eq!(out.len(), 6);
    }

    #[test]
    fn shift_line_snaps_to_45_degrees() {
        assert_eq!(snap45(Vec2::new(10.0, 1.0)), Vec2::new(10.0, 0.0));
        let d = snap45(Vec2::new(10.0, 9.0));
        assert!((d.x - d.y).abs() < 1e-4 && d.x > 0.0);
        let mut s = Sampler::start(Vec2::ZERO);
        let mut out = Vec::new();
        s.advance(Vec2::new(50.0, 3.0), 5.0, 0.0, true, &mut out);
        assert!(out.iter().all(|p| p.y.abs() < 1e-4), "{out:?}");
        // Al soltar Mayús vuelve a seguir al cursor.
        s.advance(Vec2::new(50.0, 40.0), 5.0, 0.0, false, &mut out);
        assert!(out.last().unwrap().y > 30.0);
    }

    #[test]
    fn pick_smoother_halves_jitter_and_follows_steps() {
        let mut f = PickSmoother::default();
        f.filter(Vec3::ZERO, Vec3::Y, 1.0);
        let (p, _) = f.filter(Vec3::new(0.1, 0.02, 0.0), Vec3::Y, 1.0);
        assert!((p.y - 0.01).abs() < 1e-6 && (p.x - 0.1).abs() < 1e-6);
        let (p, _) = f.filter(Vec3::new(0.2, 0.8, 0.0), Vec3::Y, 1.0);
        assert_eq!(p.y, 0.8, "un escalón real se sigue");
    }

    fn view() -> ViewParams {
        let eye = Vec3::new(0.0, 0.0, 3.0);
        let vp = perspective(40.0, 1.0, 0.05, 100.0) * look_at(eye, Vec3::ZERO, Vec3::Y);
        ViewParams { view_proj: vp, eye, size: Vec2::splat(400.0), world_per_px_at_1: 2.0 * (20f32.to_radians()).tan() / 400.0 }
    }

    #[test]
    fn view_params_ray_and_project_roundtrip() {
        let v = view();
        let (o, d) = v.ray(Vec2::new(200.0, 200.0));
        assert!((d - Vec3::NEG_Z).length() < 1e-4 && o.z > 2.9);
        let p = v.project(Vec3::new(0.3, 0.2, 0.0)).unwrap();
        let (o, d) = v.ray(p);
        let t = -o.z / d.z;
        assert!((o + d * t - Vec3::new(0.3, 0.2, 0.0)).length() < 1e-3);
        // 60 px a 3 m.
        let r = v.px_to_world(Vec3::ZERO, 60.0);
        assert!((r - 3.0 * 2.0 * 20f32.to_radians().tan() / 400.0 * 60.0).abs() < 1e-5);
    }

    #[test]
    fn controller_sculpts_on_model_and_orbits_on_background() {
        let mut sc = Sculpt::from_mesh(primitives::quad_sphere(24, 0.5));
        let tool = ToolSettings { symmetry: no_symmetry(), ..Default::default() };
        let v = view();
        let mut c = Controller::default();
        assert!(!c.press(&mut sc, &tool, &v, &Mat4::IDENTITY, &PointerState::at(Vec2::new(5.0, 5.0))), "fondo");
        let before = sc.mesh.pos.clone();
        assert!(c.press(&mut sc, &tool, &v, &Mat4::IDENTITY, &PointerState::at(Vec2::new(200.0, 200.0))));
        for x in 0..20 {
            c.drag(&mut sc, &tool, &v, &Mat4::IDENTITY, &PointerState::at(Vec2::new(200.0 + x as f32 * 3.0, 200.0)));
        }
        c.release(&mut sc);
        let moved = before.iter().zip(&sc.mesh.pos).filter(|(a, b)| a != b).count();
        assert!(moved > 0);
        // Los movidos salen hacia fuera (Estándar añade).
        let out = before.iter().zip(&sc.mesh.pos).filter(|(a, b)| a != b).all(|(a, b)| b.length() >= a.length() - 1e-6);
        assert!(out);
        assert_eq!(sc.undo.len(), 1, "un trazo = un paso");
        // Mayús → suavizar; Ctrl → máscara.
        let (a, s) = action_for(&tool, true, false, false);
        assert!(matches!(a, Action::Smooth { .. }));
        assert_eq!(s.intensity, tool.of(Brush::Smooth).intensity);
        assert_eq!(action_for(&tool, false, true, true).0, Action::Mask { erase: true });
        assert_eq!(action_for(&tool, false, false, true).0, Action::Brush { brush: Brush::Standard, invert: true });
    }

    #[test]
    fn controller_move_follows_cursor_in_screen_plane() {
        let mut sc = Sculpt::from_mesh(primitives::quad_sphere(24, 0.5));
        let tool = ToolSettings { brush: Brush::Move, symmetry: no_symmetry(), ..Default::default() };
        let v = view();
        let mut c = Controller::default();
        assert!(c.press(&mut sc, &tool, &v, &Mat4::IDENTITY, &PointerState::at(Vec2::new(200.0, 200.0))));
        let front = (0..sc.mesh.pos.len()).max_by(|&a, &b| sc.mesh.pos[a].z.total_cmp(&sc.mesh.pos[b].z)).unwrap();
        let p0 = sc.mesh.pos[front];
        c.drag(&mut sc, &tool, &v, &Mat4::IDENTITY, &PointerState::at(Vec2::new(240.0, 200.0)));
        c.release(&mut sc);
        let d = sc.mesh.pos[front] - p0;
        assert!(d.x > 0.05 && d.y.abs() < 0.02 && d.z.abs() < 0.02, "{d}");
        // Con Alt va a lo largo de la normal (hacia la cámara al arrastrar hacia arriba).
        let mut sc = Sculpt::from_mesh(primitives::quad_sphere(24, 0.5));
        let p0 = sc.mesh.pos[front];
        let mut pr = PointerState::at(Vec2::new(200.0, 200.0));
        pr.alt = true;
        assert!(c.press(&mut sc, &tool, &v, &Mat4::IDENTITY, &pr));
        pr.pos = Vec2::new(200.0, 160.0);
        c.drag(&mut sc, &tool, &v, &Mat4::IDENTITY, &pr);
        c.release(&mut sc);
        let d = sc.mesh.pos[front] - p0;
        assert!(d.z > 0.05 && d.x.abs() < 0.01, "{d}");
    }
}
