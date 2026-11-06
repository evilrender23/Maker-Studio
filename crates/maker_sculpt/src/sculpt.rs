//! El motor de esculpido: aplica *dabs* sobre la malla, arrastra (Mover), guarda el deshacer,
//! recalcula normales y cajas de lo tocado y dice qué rangos subir a la GPU.
//!
//! Cada *dab* trabaja por hojas del BVH en paralelo (rayon): las hojas tienen rangos de
//! vértices disjuntos, así cada hilo escribe solo en los suyos. Los búferes de trabajo tienen el
//! tamaño de la malla y se reutilizan: en régimen estable un *dab* no reserva memoria.

use crate::brush::{Brush, k};
use crate::bvh::{Bvh, Leaf};
use crate::falloff::FalloffLut;
use crate::mesh::{NONE, SculptMesh};
use crate::symmetry::{self, Symmetry};
use crate::undo::{Record, Target, UndoStack};
use glam::{Mat3, Vec3, Vec4};
use maker_engine::mesh::MeshData;
use maker_engine::model::ModelData;
use rayon::prelude::*;
use std::ops::Range;

/// Puntero compartido para escribir desde varios hilos en rangos disjuntos de un búfer.
#[derive(Clone, Copy)]
struct Ptr<T>(*mut T);
unsafe impl<T> Send for Ptr<T> {}
unsafe impl<T> Sync for Ptr<T> {}
impl<T> Ptr<T> {
    fn new(s: &mut [T]) -> Self {
        Ptr(s.as_mut_ptr())
    }
    /// SAFETY: quien llama garantiza que ningún otro hilo toca `[a, b)` a la vez y que el
    /// rango está dentro del búfer original, que sigue vivo.
    #[inline]
    unsafe fn slice<'a>(self, a: usize, b: usize) -> &'a mut [T] {
        unsafe { std::slice::from_raw_parts_mut(self.0.add(a), b - a) }
    }
    /// SAFETY: como `slice`, para un elemento.
    #[inline]
    #[allow(clippy::mut_from_ref)]
    unsafe fn at<'a>(self, i: usize) -> &'a mut T {
        unsafe { &mut *self.0.add(i) }
    }
}

const EMPTY_BOX: (Vec3, Vec3) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));

/// Un toque del pincel, en el espacio local de la malla.
#[derive(Clone, Copy, Debug)]
pub struct Dab {
    pub center: Vec3,
    /// Normal de la superficie en el centro.
    pub normal: Vec3,
    pub radius: f32,
    /// Intensidad × presión, 0..1.
    pub strength: f32,
    /// Dirección de la vista (de la cámara hacia la escena), unitaria.
    pub view_dir: Vec3,
    /// Movimiento del centro desde el *dab* anterior (Empujar).
    pub delta: Vec3,
    /// Derecha de la pantalla (orienta la punta cuadrada).
    pub screen_x: Vec3,
}

impl Dab {
    pub fn at(center: Vec3, normal: Vec3, radius: f32, strength: f32) -> Dab {
        Dab { center, normal, radius, strength, view_dir: -normal, delta: Vec3::ZERO, screen_x: normal.any_orthonormal_vector() }
    }
    fn transformed(&self, m: &Mat3, c: Vec3) -> Dab {
        Dab {
            center: symmetry::apply(m, c, self.center),
            normal: *m * self.normal,
            radius: self.radius,
            strength: self.strength,
            view_dir: *m * self.view_dir,
            delta: *m * self.delta,
            screen_x: *m * self.screen_x,
        }
    }
}

/// Qué hace un trazo.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    Brush { brush: Brush, invert: bool },
    /// Suavizado (pincel Suavizar o Mayús). `taubin` conserva el volumen.
    Smooth { taubin: bool },
    /// Pintar (o borrar) máscara.
    Mask { erase: bool },
}

impl Action {
    fn target(self) -> Target {
        if matches!(self, Action::Mask { .. }) { Target::Mask } else { Target::Positions }
    }
}

/// Opciones de caída y simetría de un *dab*.
#[derive(Clone, Copy, Debug)]
pub struct DabOptions {
    /// −100..100.
    pub falloff: f32,
    pub front_only: bool,
    pub connected: bool,
    /// Caída medida en pantalla: cilindro a lo largo de la vista.
    pub view_falloff: bool,
    pub symmetry: Symmetry,
    /// Centro de la simetría (origen o centro de la caja).
    pub center: Vec3,
}

impl Default for DabOptions {
    fn default() -> Self {
        Self { falloff: 0.0, front_only: false, connected: false, view_falloff: false, symmetry: Symmetry { mirror: [false; 3], ..Default::default() }, center: Vec3::ZERO }
    }
}

/// Impacto de un rayo en la malla.
#[derive(Clone, Copy, Debug)]
pub struct Hit {
    pub point: Vec3,
    pub normal: Vec3,
    pub t: f32,
    pub tri: u32,
}

/// Búferes de trabajo, del tamaño de la malla y reutilizados.
#[derive(Default)]
struct Scratch {
    leaves: Vec<u32>,
    weight: Vec<f32>,
    tmp: Vec<Vec3>,
    tmpf: Vec<f32>,
    stamp: Vec<u32>,
    vgen: u32,
    queue: Vec<u32>,
    leaf_stamp: Vec<u32>,
    leaf_gen: u32,
    halo: Vec<u32>,
    xforms: Vec<Mat3>,
    /// Posición de cada vértice en la lista de Mover (`NONE` fuera de ella).
    slot: Vec<u32>,
}

impl Scratch {
    fn next_gen(&mut self) -> u32 {
        self.vgen = self.vgen.wrapping_add(1);
        if self.vgen == 0 {
            self.stamp.fill(0);
            self.vgen = 1;
        }
        self.vgen
    }
    fn next_leaf_gen(&mut self) -> u32 {
        self.leaf_gen = self.leaf_gen.wrapping_add(1);
        if self.leaf_gen == 0 {
            self.leaf_stamp.fill(0);
            self.leaf_gen = 1;
        }
        self.leaf_gen
    }
}

/// Estado de Capa: posición y normal al empezar el trazo y desplazamiento ya aplicado.
struct LayerBuf {
    orig: Vec<Vec3>,
    n0: Vec<Vec3>,
    disp: Vec<f32>,
    stamp: Vec<u32>,
}

/// Lo agarrado por Mover: vértices, posiciones originales y peso por copia de simetría.
struct Grab {
    verts: Vec<u32>,
    orig: Vec<Vec3>,
    weights: Vec<f32>,
    mats: Vec<Mat3>,
    leaves: Vec<u32>,
}

pub struct Sculpt {
    pub mesh: SculptMesh,
    pub bvh: Bvh,
    pub undo: UndoStack,
    sc: Scratch,
    stroke: Option<Record>,
    stroke_id: u32,
    leaf_saved: Vec<u32>,
    pending: Vec<u32>,
    pending_flag: Vec<bool>,
    gpu: Vec<u32>,
    gpu_flag: Vec<bool>,
    layer: Option<LayerBuf>,
    grab: Option<Grab>,
    /// Sube con cada cambio de la geometría o la máscara.
    pub version: u64,
}

/// Normal de vértice a partir de los arrays (sin tomar prestada la malla entera).
#[inline]
fn vertex_normal(pos: &[Vec3], tris: &[[u32; 3]], vt_off: &[u32], vt: &[u32], v: usize) -> Vec3 {
    let mut s = Vec3::ZERO;
    for &t in &vt[vt_off[v] as usize..vt_off[v + 1] as usize] {
        let [a, b, c] = tris[t as usize];
        let (a, b, c) = (pos[a as usize], pos[b as usize], pos[c as usize]);
        s += (b - a).cross(c - a);
    }
    s.normalize_or(Vec3::Y)
}

/// Aplica `f` a los vértices con peso de las hojas y ajusta la caja de vértices de cada hoja.
fn displace<F>(pos: &mut [Vec3], leaf_data: &mut [Leaf], weight: &[f32], leaves: &[u32], f: F)
where
    F: Fn(usize, Vec3, f32) -> Vec3 + Sync,
{
    let pp = Ptr::new(pos);
    let lp = Ptr::new(leaf_data);
    leaves.par_iter().for_each(|&l| {
        // SAFETY: `leaves` no repite hojas y cada hoja tiene su propio rango de vértices.
        let lf = unsafe { lp.at(l as usize) };
        let ps = unsafe { pp.slice(lf.v0 as usize, lf.v1 as usize) };
        let (mut lo, mut hi) = EMPTY_BOX;
        for (i, p) in ps.iter_mut().enumerate() {
            let v = lf.v0 as usize + i;
            let w = weight[v];
            if w > 0.0 {
                *p = f(v, *p, w);
            }
            lo = lo.min(*p);
            hi = hi.max(*p);
        }
        lf.vmin = lo;
        lf.vmax = hi;
    });
}

impl Sculpt {
    /// Prepara una malla soldada: BVH, renumeración por hojas, adyacencia y normales.
    pub fn new(mut mesh: SculptMesh) -> Sculpt {
        let (mut bvh, perm) = Bvh::build(&mesh.pos, &mesh.tris);
        mesh.permute(&perm);
        mesh.build_adjacency();
        mesh.compute_normals();
        bvh.refit_all(&mesh.pos, &mesh.tris);
        let n = mesh.pos.len();
        let nl = bvh.leaves.len();
        let sc = Scratch {
            weight: vec![0.0; n],
            tmp: vec![Vec3::ZERO; n],
            tmpf: vec![0.0; n],
            stamp: vec![0; n],
            slot: vec![NONE; n],
            leaf_stamp: vec![0; nl],
            ..Default::default()
        };
        Sculpt {
            mesh,
            bvh,
            undo: UndoStack::default(),
            sc,
            stroke: None,
            stroke_id: 0,
            leaf_saved: vec![0; nl],
            pending: Vec::new(),
            pending_flag: vec![false; nl],
            gpu: Vec::new(),
            gpu_flag: vec![false; nl],
            layer: None,
            grab: None,
            version: 0,
        }
    }

    pub fn from_model(data: &ModelData) -> Result<Sculpt, String> {
        Ok(Sculpt::new(SculptMesh::from_model(data)?))
    }

    pub fn from_mesh(m: MeshData) -> Sculpt {
        Sculpt::from_model(&ModelData::from_mesh(m)).expect("malla válida")
    }

    pub fn vertex_count(&self) -> usize {
        self.mesh.pos.len()
    }
    pub fn triangle_count(&self) -> usize {
        self.mesh.tris.len()
    }

    /// Centro de la caja de la malla (simetría «Local»).
    pub fn bounds_center(&self) -> Vec3 {
        let (a, b) = self.mesh.bounds();
        (a + b) * 0.5
    }

    // ───────────────────────────── rayos ─────────────────────────────

    pub fn raycast(&self, o: Vec3, d: Vec3) -> Option<Hit> {
        let h = self.bvh.ray(o, d, &self.mesh.pos, &self.mesh.tris)?;
        let [a, b, c] = self.mesh.tris[h.tri as usize];
        let n = self.mesh.nrm[a as usize] * (1.0 - h.u - h.v) + self.mesh.nrm[b as usize] * h.u + self.mesh.nrm[c as usize] * h.v;
        let n = n.try_normalize().unwrap_or_else(|| self.mesh.face_area_normal(h.tri).normalize_or(Vec3::Y));
        Some(Hit { point: o + d * h.t, normal: n, t: h.t, tri: h.tri })
    }

    /// Normal media de la superficie dentro de la esfera (plano estable del cursor).
    pub fn area_normal(&self, c: Vec3, r: f32) -> Option<Vec3> {
        let mut leaves = Vec::new();
        self.bvh.leaves_in_sphere(c, r, &mut leaves);
        let r2 = r * r;
        let mut s = Vec3::ZERO;
        for l in leaves {
            let lf = &self.bvh.leaves[l as usize];
            for v in lf.v0 as usize..lf.v1 as usize {
                let d2 = self.mesh.pos[v].distance_squared(c);
                if d2 < r2 {
                    s += self.mesh.nrm[v] * (1.0 - d2 / r2);
                }
            }
        }
        s.try_normalize()
    }

    // ───────────────────────────── trazos ─────────────────────────────

    /// Empieza un paso de deshacer. Los *dabs* fuera de un trazo abren uno solo.
    pub fn begin_stroke(&mut self, label: &str, mask: bool) {
        self.end_stroke();
        self.stroke_id = self.stroke_id.wrapping_add(1);
        if self.stroke_id == 0 {
            self.leaf_saved.fill(0);
            if let Some(l) = &mut self.layer {
                l.stamp.fill(0);
            }
            self.stroke_id = 1;
        }
        self.stroke = Some(Record::new(label, if mask { Target::Mask } else { Target::Positions }));
    }

    /// Cierra el trazo: guarda el paso de deshacer y pone al día normales y cajas.
    pub fn end_stroke(&mut self) {
        self.grab = None;
        if let Some(r) = self.stroke.take()
            && !r.is_empty()
        {
            self.undo.push(r);
        }
        self.flush();
    }

    pub fn in_stroke(&self) -> bool {
        self.stroke.is_some()
    }

    fn save_leaves(&mut self, leaves: &[u32], target: Target) {
        if self.stroke.is_none() {
            self.begin_stroke("Trazo", target == Target::Mask);
        }
        if target == Target::Mask {
            self.mesh.ensure_mask();
        }
        let id = self.stroke_id;
        let rec = self.stroke.as_mut().unwrap();
        for &l in leaves {
            if self.leaf_saved[l as usize] == id {
                continue;
            }
            self.leaf_saved[l as usize] = id;
            let lf = &self.bvh.leaves[l as usize];
            let (a, b) = (lf.v0 as usize, lf.v1 as usize);
            rec.leaves.push(l);
            match rec.target {
                Target::Positions => {
                    rec.offsets.push(rec.pos.len() as u32);
                    rec.pos.extend_from_slice(&self.mesh.pos[a..b]);
                }
                Target::Mask => {
                    rec.offsets.push(rec.mask.len() as u32);
                    rec.mask.extend_from_slice(&self.mesh.mask[a..b]);
                }
            }
        }
    }

    fn mark(&mut self, leaves: &[u32]) {
        for &l in leaves {
            if !self.pending_flag[l as usize] {
                self.pending_flag[l as usize] = true;
                self.pending.push(l);
            }
        }
        self.version = self.version.wrapping_add(1);
    }

    /// Pesos (caída × máscara × caras frontales) de los vértices de las hojas candidatas.
    /// Devuelve la suma ponderada de normales, de posiciones y de pesos.
    fn weigh(&mut self, leaves: &[u32], dab: &Dab, opts: &DabOptions, square: bool, use_mask: bool) -> (Vec3, Vec3, f32) {
        let r = dab.radius.max(1e-9);
        let inv_r = 1.0 / r;
        let lut = FalloffLut::new(opts.falloff);
        let n0 = dab.normal.normalize_or(Vec3::Y);
        let tx = (dab.screen_x - n0 * dab.screen_x.dot(n0)).try_normalize().unwrap_or_else(|| n0.any_orthonormal_vector());
        let ty = n0.cross(tx);
        let vd = dab.view_dir.normalize_or(Vec3::NEG_Z);
        let use_mask = use_mask && self.mesh.mask.len() == self.mesh.pos.len();
        let wp = Ptr::new(&mut self.sc.weight);
        let (pos, nrm, mask, lfs) = (&self.mesh.pos, &self.mesh.nrm, &self.mesh.mask, &self.bvh.leaves);
        leaves.par_iter().for_each(|&l| {
            let lf = &lfs[l as usize];
            // SAFETY: hojas sin repetir, rangos disjuntos.
            let w = unsafe { wp.slice(lf.v0 as usize, lf.v1 as usize) };
            for (i, v) in (lf.v0 as usize..lf.v1 as usize).enumerate() {
                let q = pos[v] - dab.center;
                let x = if opts.view_falloff {
                    let a = q.dot(vd);
                    (q.length_squared() - a * a).max(0.0).sqrt() * inv_r
                } else if square {
                    if q.dot(n0).abs() > r { 1.0 } else { q.dot(tx).abs().max(q.dot(ty).abs()) * inv_r }
                } else {
                    q.length() * inv_r
                };
                let mut wt = lut.at(x);
                if wt > 0.0 {
                    if opts.front_only && nrm[v].dot(vd) > 0.0 {
                        wt = 0.0;
                    } else if use_mask {
                        wt *= 1.0 - mask[v];
                    }
                }
                w[i] = wt;
            }
        });
        if opts.connected {
            self.connect(leaves, dab.center);
        }
        let (weight, pos, nrm, lfs) = (&self.sc.weight, &self.mesh.pos, &self.mesh.nrm, &self.bvh.leaves);
        leaves
            .par_iter()
            .map(|&l| {
                let lf = &lfs[l as usize];
                let mut acc = (Vec3::ZERO, Vec3::ZERO, 0.0f32);
                for v in lf.v0 as usize..lf.v1 as usize {
                    let w = weight[v];
                    if w > 0.0 {
                        acc.0 += nrm[v] * w;
                        acc.1 += pos[v] * w;
                        acc.2 += w;
                    }
                }
                acc
            })
            .reduce(|| (Vec3::ZERO, Vec3::ZERO, 0.0), |a, b| (a.0 + b.0, a.1 + b.1, a.2 + b.2))
    }

    /// «Solo conectado»: deja peso solo a lo unido (por aristas con peso) al vértice más
    /// cercano al centro.
    fn connect(&mut self, leaves: &[u32], center: Vec3) {
        let (weight, pos, lfs) = (&self.sc.weight, &self.mesh.pos, &self.bvh.leaves);
        let seed = leaves
            .par_iter()
            .map(|&l| {
                let lf = &lfs[l as usize];
                let mut best = (f32::MAX, NONE);
                for v in lf.v0..lf.v1 {
                    if weight[v as usize] > 0.0 {
                        let d = pos[v as usize].distance_squared(center);
                        if d < best.0 {
                            best = (d, v);
                        }
                    }
                }
                best
            })
            .reduce(|| (f32::MAX, NONE), |a, b| if b.0 < a.0 { b } else { a });
        let lg = self.sc.next_leaf_gen();
        for &l in leaves {
            self.sc.leaf_stamp[l as usize] = lg;
        }
        let g = self.sc.next_gen();
        if seed.1 != NONE {
            let mut q = std::mem::take(&mut self.sc.queue);
            q.clear();
            q.push(seed.1);
            self.sc.stamp[seed.1 as usize] = g;
            while let Some(v) = q.pop() {
                for &u in self.mesh.ring(v) {
                    let u = u as usize;
                    let l = self.bvh.vert_leaf[u];
                    if l != NONE && self.sc.leaf_stamp[l as usize] == lg && self.sc.stamp[u] != g && self.sc.weight[u] > 0.0 {
                        self.sc.stamp[u] = g;
                        q.push(u as u32);
                    }
                }
            }
            self.sc.queue = q;
        }
        let wp = Ptr::new(&mut self.sc.weight);
        let (stamp, lfs) = (&self.sc.stamp, &self.bvh.leaves);
        leaves.par_iter().for_each(|&l| {
            let lf = &lfs[l as usize];
            // SAFETY: hojas sin repetir, rangos disjuntos.
            let w = unsafe { wp.slice(lf.v0 as usize, lf.v1 as usize) };
            for (i, v) in (lf.v0 as usize..lf.v1 as usize).enumerate() {
                if stamp[v] != g {
                    w[i] = 0.0;
                }
            }
        });
    }

    /// Un *dab* con todas sus copias de simetría.
    pub fn dab(&mut self, action: Action, dab: &Dab, opts: &DabOptions) {
        if matches!(action, Action::Brush { brush: Brush::Move, .. }) {
            return;
        }
        let mut xf = std::mem::take(&mut self.sc.xforms);
        opts.symmetry.transforms(&mut xf);
        for m in &xf {
            let d = dab.transformed(m, opts.center);
            self.dab_one(action, &d, opts);
        }
        self.sc.xforms = xf;
    }

    fn dab_one(&mut self, action: Action, dab: &Dab, opts: &DabOptions) {
        let r = dab.radius.max(1e-9);
        let square = matches!(action, Action::Brush { brush: Brush::ClayBlock, .. });
        let qr = if square { r * std::f32::consts::SQRT_2 } else { r };
        let mut leaves = std::mem::take(&mut self.sc.leaves);
        if opts.view_falloff {
            self.bvh.leaves_in_cylinder(dab.center, dab.view_dir.normalize_or(Vec3::NEG_Z), qr, &mut leaves);
        } else {
            self.bvh.leaves_in_sphere(dab.center, qr, &mut leaves);
        }
        if leaves.is_empty() {
            self.sc.leaves = leaves;
            return;
        }
        self.save_leaves(&leaves, action.target());
        let (sn, sp, sw) = self.weigh(&leaves, dab, opts, square, !matches!(action, Action::Mask { .. }));
        if sw <= 0.0 {
            self.sc.leaves = leaves;
            return;
        }
        let an = sn.try_normalize().unwrap_or(dab.normal);
        let ac = sp / sw;
        let s = dab.strength.clamp(0.0, 1.0);
        match action {
            Action::Mask { erase } => {
                let sign = if erase { -1.0 } else { 1.0 };
                let mp = Ptr::new(&mut self.mesh.mask);
                let (weight, lfs) = (&self.sc.weight, &self.bvh.leaves);
                leaves.par_iter().for_each(|&l| {
                    let lf = &lfs[l as usize];
                    // SAFETY: hojas sin repetir, rangos disjuntos.
                    let m = unsafe { mp.slice(lf.v0 as usize, lf.v1 as usize) };
                    for (i, v) in (lf.v0 as usize..lf.v1 as usize).enumerate() {
                        let w = weight[v];
                        if w > 0.0 {
                            m[i] = (m[i] + sign * w * s * k::MASK * 2.0).clamp(0.0, 1.0);
                        }
                    }
                });
            }
            Action::Smooth { taubin } => self.smooth_leaves(&leaves, s * k::SMOOTH * 2.0, taubin),
            Action::Brush { brush, invert } => {
                let sg = if invert && brush.invertible() { -1.0 } else { 1.0 };
                self.brush_pass(brush, sg, s, r, an, ac, dab, &leaves);
            }
        }
        self.mark(&leaves);
        self.bvh.refit_up(&leaves);
        self.sc.leaves = leaves;
    }

    #[allow(clippy::too_many_arguments)]
    fn brush_pass(&mut self, brush: Brush, sg: f32, s: f32, r: f32, an: Vec3, ac: Vec3, dab: &Dab, leaves: &[u32]) {
        let (pos, lfs, weight) = (&mut self.mesh.pos, &mut self.bvh.leaves, &self.sc.weight);
        let nrm = &self.mesh.nrm;
        let tangential = |q: Vec3| q - an * q.dot(an);
        match brush {
            Brush::Standard => {
                let d = an * (sg * s * r * k::STANDARD);
                displace(pos, lfs, weight, leaves, |_, p, w| p + d * w);
            }
            Brush::Inflate => {
                let d = sg * s * r * k::INFLATE;
                displace(pos, lfs, weight, leaves, |v, p, w| p + nrm[v] * (d * w));
            }
            Brush::Clay | Brush::ClayBlock => {
                let rate = if brush == Brush::Clay { k::CLAY_RATE } else { k::CLAY_BLOCK_RATE };
                let plane = ac + an * (sg * r * k::CLAY_HEIGHT);
                displace(pos, lfs, weight, leaves, |_, p, w| {
                    let d = (p - plane).dot(an);
                    if d * sg < 0.0 { p - an * (d * (w * s * rate).min(1.0)) } else { p }
                });
            }
            Brush::Flatten => {
                let f = sg * s * k::FLATTEN;
                displace(pos, lfs, weight, leaves, |_, p, w| p - an * ((p - ac).dot(an) * (w * f).clamp(-1.0, 1.0)));
            }
            Brush::Trim => {
                let plane = ac - an * (sg * r * k::TRIM_OFFSET);
                displace(pos, lfs, weight, leaves, |_, p, w| {
                    let d = (p - plane).dot(an);
                    if d * sg > 0.0 { p - an * (d * (w * s * k::TRIM_RATE).min(1.0)) } else { p }
                });
            }
            Brush::Pinch => {
                let c = dab.center;
                displace(pos, lfs, weight, leaves, |_, p, w| p + tangential(c - p) * (sg * w * s * k::PINCH).clamp(-1.0, 1.0));
            }
            Brush::Crease | Brush::SharpCrease => {
                let (kd, kp, sharp) = if brush == Brush::Crease { (k::CREASE_DEPTH, k::CREASE_PINCH, false) } else { (k::SHARP_DEPTH, k::SHARP_PINCH, true) };
                let depth = -sg * s * r * kd;
                let c = dab.center;
                displace(pos, lfs, weight, leaves, |_, p, w| {
                    let wd = if sharp { w * w * w } else { w };
                    p + an * (depth * wd) + tangential(c - p) * (w * s * kp).min(1.0)
                });
            }
            Brush::Nudge => {
                let d = tangential(dab.delta);
                displace(pos, lfs, weight, leaves, |_, p, w| p + d * (w * s * k::NUDGE).min(1.0));
            }
            Brush::Layer => {
                let n = pos.len();
                let lb = self.layer.get_or_insert_with(|| LayerBuf { orig: vec![Vec3::ZERO; n], n0: vec![Vec3::ZERO; n], disp: vec![0.0; n], stamp: vec![0; n] });
                let height = sg * s * r * k::LAYER;
                let id = self.stroke_id;
                let (op, np, dp, sp) = (Ptr::new(&mut lb.orig), Ptr::new(&mut lb.n0), Ptr::new(&mut lb.disp), Ptr::new(&mut lb.stamp));
                displace(pos, lfs, weight, leaves, |v, p, w| {
                    // SAFETY: cada vértice lo escribe solo el hilo de su hoja.
                    let (orig, n0, disp, st) = unsafe { (op.at(v), np.at(v), dp.at(v), sp.at(v)) };
                    if *st != id {
                        *st = id;
                        *orig = p;
                        *n0 = nrm[v];
                        *disp = 0.0;
                    }
                    let target = height * w;
                    *disp = if sg > 0.0 { disp.max(target) } else { disp.min(target) };
                    *orig + *n0 * *disp
                });
            }
            Brush::Smooth => self.smooth_leaves(leaves, s * k::SMOOTH * 2.0, false),
            Brush::Move => {}
        }
    }

    /// Suavizado laplaciano (o Taubin: un paso hacia dentro y otro algo mayor hacia fuera)
    /// de los vértices con peso. Los de borde solo promedian con vecinos de borde.
    fn smooth_leaves(&mut self, leaves: &[u32], amount: f32, taubin: bool) {
        if taubin {
            self.laplacian_pass(leaves, amount * k::TAUBIN_LAMBDA, false);
            self.laplacian_pass(leaves, -amount * k::TAUBIN_LAMBDA * k::TAUBIN_MU, false);
        } else {
            self.laplacian_pass(leaves, amount, false);
        }
    }

    /// Un paso de Laplace: `p += (media − p)·λ·w`. Con `tangent` se quita la componente normal
    /// (Relajar: reparte los vértices sin cambiar la forma).
    fn laplacian_pass(&mut self, leaves: &[u32], lambda: f32, tangent: bool) {
        let tp = Ptr::new(&mut self.sc.tmp);
        {
            let (pos, nrm, weight, lfs, mesh) = (&self.mesh.pos, &self.mesh.nrm, &self.sc.weight, &self.bvh.leaves, &self.mesh);
            leaves.par_iter().for_each(|&l| {
                let lf = &lfs[l as usize];
                // SAFETY: hojas sin repetir, rangos disjuntos.
                let out = unsafe { tp.slice(lf.v0 as usize, lf.v1 as usize) };
                for (i, v) in (lf.v0 as usize..lf.v1 as usize).enumerate() {
                    let w = weight[v];
                    if w <= 0.0 {
                        continue;
                    }
                    let p = pos[v];
                    let ring = mesh.ring(v as u32);
                    let on_border = mesh.boundary[v];
                    let (mut s, mut c) = (Vec3::ZERO, 0u32);
                    for &u in ring {
                        if !on_border || mesh.boundary[u as usize] {
                            s += pos[u as usize];
                            c += 1;
                        }
                    }
                    if c == 0 {
                        out[i] = p;
                        continue;
                    }
                    let mut d = s / c as f32 - p;
                    if tangent {
                        d -= nrm[v] * d.dot(nrm[v]);
                    }
                    out[i] = p + d * (lambda * w).clamp(-1.0, 1.0);
                }
            });
        }
        let tmp = &self.sc.tmp;
        displace(&mut self.mesh.pos, &mut self.bvh.leaves, &self.sc.weight, leaves, |v, _, _| tmp[v]);
    }

    // ───────────────────────────── Mover ─────────────────────────────

    /// Agarra lo que hay bajo el pincel (y sus copias de simetría). Devuelve si agarró algo.
    pub fn grab_begin(&mut self, dab: &Dab, opts: &DabOptions) -> bool {
        if self.stroke.is_none() {
            self.begin_stroke("Mover", false);
        }
        let mut xf = std::mem::take(&mut self.sc.xforms);
        opts.symmetry.transforms(&mut xf);
        let copies = xf.len();
        let mut g = Grab { verts: Vec::new(), orig: Vec::new(), weights: Vec::new(), mats: xf.clone(), leaves: Vec::new() };
        for (ki, m) in xf.iter().enumerate() {
            let d = dab.transformed(m, opts.center);
            let mut leaves = std::mem::take(&mut self.sc.leaves);
            self.bvh.leaves_in_sphere(d.center, d.radius, &mut leaves);
            if !leaves.is_empty() {
                self.save_leaves(&leaves, Target::Positions);
                self.weigh(&leaves, &d, opts, false, true);
                for &l in &leaves {
                    let lf = self.bvh.leaves[l as usize];
                    let mut used = false;
                    for v in lf.v0..lf.v1 {
                        let w = self.sc.weight[v as usize];
                        if w <= 0.0 {
                            continue;
                        }
                        used = true;
                        let mut i = self.sc.slot[v as usize];
                        if i == NONE {
                            i = g.verts.len() as u32;
                            g.verts.push(v);
                            g.orig.push(self.mesh.pos[v as usize]);
                            g.weights.extend(std::iter::repeat_n(0.0, copies));
                            self.sc.slot[v as usize] = i;
                        }
                        g.weights[i as usize * copies + ki] = w;
                    }
                    if used {
                        g.leaves.push(l);
                    }
                }
            }
            self.sc.leaves = leaves;
        }
        for &v in &g.verts {
            self.sc.slot[v as usize] = NONE;
        }
        g.leaves.sort_unstable();
        g.leaves.dedup();
        self.sc.xforms = xf;
        let ok = !g.verts.is_empty();
        self.grab = ok.then_some(g);
        ok
    }

    /// Mueve lo agarrado: `delta` es el desplazamiento de la copia original (espacio local).
    pub fn grab_update(&mut self, delta: Vec3) {
        let Some(g) = &self.grab else { return };
        let copies = g.mats.len();
        let ds: Vec<Vec3> = g.mats.iter().map(|m| *m * delta).collect();
        let pp = Ptr::new(&mut self.mesh.pos);
        g.verts.par_iter().enumerate().for_each(|(i, &v)| {
            let mut p = g.orig[i];
            for (k, d) in ds.iter().enumerate() {
                p += *d * g.weights[i * copies + k];
            }
            // SAFETY: cada vértice aparece una sola vez en la lista.
            unsafe { *pp.at(v as usize) = p };
        });
        let leaves = std::mem::take(&mut self.grab.as_mut().unwrap().leaves);
        let (pos, lp) = (&self.mesh.pos, Ptr::new(&mut self.bvh.leaves));
        leaves.par_iter().for_each(|&l| {
            // SAFETY: hojas sin repetir.
            let lf = unsafe { lp.at(l as usize) };
            let (a, b) = Bvh::leaf_verts_box(lf, pos);
            lf.vmin = a;
            lf.vmax = b;
        });
        self.mark(&leaves);
        self.bvh.refit_up(&leaves);
        self.grab.as_mut().unwrap().leaves = leaves;
    }

    pub fn grabbing(&self) -> bool {
        self.grab.is_some()
    }

    // ───────────────────────────── vaciado ─────────────────────────────

    /// Pone al día normales y cajas de triángulos de lo tocado desde la última vez (y de las
    /// hojas vecinas, cuyas normales dependen de lo movido). Devuelve las hojas actualizadas.
    pub fn flush(&mut self) -> usize {
        if self.pending.is_empty() {
            return 0;
        }
        let lg = self.sc.next_leaf_gen();
        let mut halo = std::mem::take(&mut self.sc.halo);
        halo.clear();
        for &l in &self.pending {
            self.pending_flag[l as usize] = false;
            if self.sc.leaf_stamp[l as usize] != lg {
                self.sc.leaf_stamp[l as usize] = lg;
                halo.push(l);
            }
            for &m in self.bvh.neighbours(l) {
                if self.sc.leaf_stamp[m as usize] != lg {
                    self.sc.leaf_stamp[m as usize] = lg;
                    halo.push(m);
                }
            }
        }
        self.pending.clear();
        let np = Ptr::new(&mut self.mesh.nrm);
        let (pos, tris, vt_off, vt) = (&self.mesh.pos, &self.mesh.tris, &self.mesh.vt_off, &self.mesh.vt);
        let lp = Ptr::new(&mut self.bvh.leaves);
        let order = &self.bvh.tri_order;
        halo.par_iter().for_each(|&l| {
            // SAFETY: hojas sin repetir, rangos de vértices disjuntos.
            let lf = unsafe { lp.at(l as usize) };
            let ns = unsafe { np.slice(lf.v0 as usize, lf.v1 as usize) };
            for (i, n) in ns.iter_mut().enumerate() {
                *n = vertex_normal(pos, tris, vt_off, vt, lf.v0 as usize + i);
            }
            let mut bx = EMPTY_BOX;
            for &t in &order[lf.t0 as usize..lf.t1 as usize] {
                for &v in &tris[t as usize] {
                    bx.0 = bx.0.min(pos[v as usize]);
                    bx.1 = bx.1.max(pos[v as usize]);
                }
            }
            lf.tmin = bx.0;
            lf.tmax = bx.1;
            let (a, b) = Bvh::leaf_verts_box(lf, pos);
            lf.vmin = a;
            lf.vmax = b;
        });
        self.bvh.refit_up(&halo);
        for &l in &halo {
            if !self.gpu_flag[l as usize] {
                self.gpu_flag[l as usize] = true;
                self.gpu.push(l);
            }
        }
        let n = halo.len();
        self.sc.halo = halo;
        n
    }

    /// Marca toda la malla para subir (y recalcular) en el próximo vaciado.
    pub fn mark_all(&mut self) {
        let all: Vec<u32> = (0..self.bvh.leaves.len() as u32).collect();
        self.mark(&all);
    }

    /// Rangos de vértices de render cambiados desde la última llamada (ordenados y unidos).
    pub fn take_gpu_ranges(&mut self, out: &mut Vec<Range<u32>>) {
        out.clear();
        if self.gpu.is_empty() {
            return;
        }
        self.gpu.sort_unstable();
        let s = self.mesh.pos.len() as u32;
        let push = |out: &mut Vec<Range<u32>>, r: Range<u32>| {
            if r.is_empty() {
                return;
            }
            match out.last_mut() {
                Some(last) if last.end >= r.start => last.end = last.end.max(r.end),
                _ => out.push(r),
            }
        };
        for &l in &self.gpu {
            self.gpu_flag[l as usize] = false;
            let lf = &self.bvh.leaves[l as usize];
            push(out, lf.v0..lf.v1);
        }
        for &l in &self.gpu {
            let lf = &self.bvh.leaves[l as usize];
            let e = self.mesh.render.extras_in(lf.v0, lf.v1);
            push(out, s + e.start as u32..s + e.end as u32);
        }
        self.gpu.clear();
    }

    pub fn has_gpu_changes(&self) -> bool {
        !self.gpu.is_empty() || !self.pending.is_empty()
    }

    // ───────────────────────────── deshacer ─────────────────────────────

    fn swap_record(&mut self, r: &mut Record) {
        if r.target == Target::Mask {
            self.mesh.ensure_mask();
        }
        for (i, &l) in r.leaves.iter().enumerate() {
            let lf = self.bvh.leaves[l as usize];
            let (a, b) = (lf.v0 as usize, lf.v1 as usize);
            let o = r.offsets[i] as usize;
            match r.target {
                Target::Positions => self.mesh.pos[a..b].swap_with_slice(&mut r.pos[o..o + (b - a)]),
                Target::Mask => self.mesh.mask[a..b].swap_with_slice(&mut r.mask[o..o + (b - a)]),
            }
            let (lo, hi) = Bvh::leaf_verts_box(&lf, &self.mesh.pos);
            self.bvh.leaves[l as usize].vmin = lo;
            self.bvh.leaves[l as usize].vmax = hi;
        }
        let leaves = r.leaves.clone();
        self.mark(&leaves);
        self.bvh.refit_up(&leaves);
        self.flush();
    }

    /// Deshace el último trazo. Devuelve su nombre.
    pub fn undo(&mut self) -> Option<String> {
        self.end_stroke();
        let mut r = self.undo.pop_undo()?;
        self.swap_record(&mut r);
        let label = r.label.clone();
        self.undo.push_redo(r);
        Some(label)
    }

    pub fn redo(&mut self) -> Option<String> {
        self.end_stroke();
        let mut r = self.undo.pop_redo()?;
        self.swap_record(&mut r);
        let label = r.label.clone();
        self.undo.push_undo_keep_redo(r);
        Some(label)
    }

    // ───────────────────────────── malla completa ─────────────────────────────

    /// Todas las hojas como candidatas, con peso `1 − máscara`.
    fn whole(&mut self, label: &str, target: Target) -> Vec<u32> {
        let leaves: Vec<u32> = (0..self.bvh.leaves.len() as u32).collect();
        self.begin_stroke(label, target == Target::Mask);
        self.save_leaves(&leaves, target);
        let has_mask = target == Target::Positions && self.mesh.mask.len() == self.mesh.pos.len();
        let (mask, w) = (&self.mesh.mask, &mut self.sc.weight);
        w.par_iter_mut().enumerate().for_each(|(v, x)| *x = if has_mask { 1.0 - mask[v] } else { 1.0 });
        leaves
    }

    fn whole_end(&mut self, leaves: Vec<u32>) {
        self.mark(&leaves);
        self.bvh.refit_up(&leaves);
        self.end_stroke();
    }

    /// Suavizar toda la malla. `amount` 0..100 (pasadas).
    pub fn smooth_all(&mut self, amount: f32, taubin: bool) {
        let leaves = self.whole("Suavizar malla", Target::Positions);
        for _ in 0..passes(amount) {
            self.smooth_leaves(&leaves, 0.5, taubin);
        }
        self.whole_end(leaves);
    }

    /// Relajar: reparte los vértices por la superficie sin cambiar la forma.
    pub fn relax_all(&mut self, amount: f32) {
        let leaves = self.whole("Relajar malla", Target::Positions);
        for _ in 0..passes(amount) {
            self.laplacian_pass(&leaves, 0.5, true);
            // Normales al día entre pasadas: la proyección tangente las usa.
            self.mark(&leaves);
            self.flush();
        }
        self.whole_end(leaves);
    }

    /// Hinchar como un globo: cada vértice sale por su normal (`amount` 0..100 = hasta el 5 % de
    /// la diagonal de la caja).
    pub fn inflate_all(&mut self, amount: f32) {
        let (a, b) = self.mesh.bounds();
        let dist = (b - a).length() * 0.05 * amount.clamp(-100.0, 100.0) / 100.0;
        let leaves = self.whole("Hinchar malla", Target::Positions);
        let nrm = &self.mesh.nrm;
        displace(&mut self.mesh.pos, &mut self.bvh.leaves, &self.sc.weight, &leaves, |v, p, w| p + nrm[v] * (dist * w));
        self.whole_end(leaves);
    }

    // ───────────────────────────── máscara ─────────────────────────────

    fn mask_op(&mut self, label: &str, f: impl Fn(&SculptMesh, usize, f32) -> f32 + Sync) {
        let leaves = self.whole(label, Target::Mask);
        let mesh = &self.mesh;
        self.sc.tmpf.par_iter_mut().enumerate().for_each(|(v, out)| *out = f(mesh, v, mesh.mask[v]));
        std::mem::swap(&mut self.mesh.mask, &mut self.sc.tmpf);
        self.whole_end(leaves);
    }

    pub fn mask_invert(&mut self) {
        self.mask_op("Invertir máscara", |_, _, m| 1.0 - m);
    }

    pub fn mask_clear(&mut self) {
        self.mask_op("Borrar máscara", |_, _, _| 0.0);
    }

    /// Difumina los bordes de la máscara (media con los vecinos).
    pub fn mask_blur(&mut self) {
        self.mask_op("Difuminar máscara", |mesh, v, m| {
            let ring = mesh.ring(v as u32);
            if ring.is_empty() {
                return m;
            }
            let avg = ring.iter().map(|&u| mesh.mask[u as usize]).sum::<f32>() / ring.len() as f32;
            (m + avg) * 0.5
        });
    }

    /// Afila los bordes de la máscara (realce: se aleja de la media de los vecinos).
    pub fn mask_sharpen(&mut self) {
        self.mask_op("Afilar máscara", |mesh, v, m| {
            let ring = mesh.ring(v as u32);
            if ring.is_empty() {
                return m;
            }
            let avg = ring.iter().map(|&u| mesh.mask[u as usize]).sum::<f32>() / ring.len() as f32;
            let s = m + (m - avg) * 2.0;
            // Y un poco de contraste alrededor de 0,5.
            ((s - 0.5) * 1.2 + 0.5).clamp(0.0, 1.0)
        });
    }

    pub fn has_mask(&self) -> bool {
        self.mesh.mask.iter().any(|&m| m > 0.0)
    }

    // ───────────────────────────── exportar ─────────────────────────────

    /// Modelo con los vértices de render (costuras, UV, colores y materiales originales).
    pub fn to_model(&self) -> ModelData {
        let m = &self.mesh;
        let rm = &m.render;
        let s = m.pos.len();
        let src = |r: usize| if r < s { r } else { rm.extra_src[r - s] as usize };
        let rn = rm.len();
        let positions: Vec<Vec3> = (0..rn).map(|r| m.pos[src(r)]).collect();
        let normals: Vec<Vec3> = (0..rn).map(|r| m.nrm[src(r)]).collect();
        let tangents = if rm.tangents.len() == rn {
            (0..rn)
                .map(|r| {
                    let n = normals[r];
                    let t = rm.tangents[r];
                    let tt = (t.truncate() - n * t.truncate().dot(n)).try_normalize().unwrap_or_else(|| n.any_orthonormal_vector());
                    tt.extend(if t.w < 0.0 { -1.0 } else { 1.0 })
                })
                .collect::<Vec<Vec4>>()
        } else {
            Vec::new()
        };
        let mut mesh = MeshData { positions, normals, uvs: rm.uvs.clone(), tangents, indices: rm.indices.clone(), colors: rm.colors.clone(), joints: Vec::new(), weights: Vec::new() };
        if mesh.tangents.is_empty() {
            // Sin tangentes, quien lo cargue las calcularía (y podría duplicar vértices).
            mesh.compute_tangents();
        }
        ModelData { mesh, submeshes: rm.submeshes.clone(), materials: rm.materials.clone(), anim: Default::default() }
    }
}

fn passes(amount: f32) -> u32 {
    1 + (amount.clamp(0.0, 100.0) / 10.0).round() as u32
}

#[cfg(test)]
mod tests;
