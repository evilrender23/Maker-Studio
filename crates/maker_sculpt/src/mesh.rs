//! Malla esculpible en estructura de arrays.
//!
//! Los modelos glTF repiten vértices en las costuras (UV, normales duras). Para esculpir, los
//! vértices que comparten posición se sueldan en uno («vértice de esculpido») y la malla guarda
//! aparte el mapa a los vértices originales («vértices de render»), que conservan sus UV,
//! colores y tangentes. Los `S` primeros vértices de render son los `S` vértices de esculpido en
//! el mismo orden; los duplicados de costura van detrás, ordenados por su vértice de origen. Así
//! un rango contiguo de vértices de esculpido corresponde a dos rangos contiguos de render.

use glam::{Vec2, Vec3, Vec4};
use maker_engine::model::{ModelData, ModelMaterial, SubMesh};
use rayon::prelude::*;
use std::collections::HashMap;
use std::ops::Range;

/// Índice nulo.
pub const NONE: u32 = u32::MAX;

/// Atributos de los vértices de render y su relación con los de esculpido.
#[derive(Clone, Debug, Default)]
pub struct RenderMap {
    /// UV por vértice de render.
    pub uvs: Vec<Vec2>,
    /// Color por vértice de render (vacío = blanco).
    pub colors: Vec<[u8; 4]>,
    /// Tangentes por vértice de render (vacío = sin tangentes).
    pub tangents: Vec<Vec4>,
    /// Vértice de esculpido de cada duplicado de costura (render `S + i`), en orden creciente.
    pub extra_src: Vec<u32>,
    /// Triángulos en índices de render, en el mismo orden que `SculptMesh::tris`.
    pub indices: Vec<u32>,
    pub submeshes: Vec<SubMesh>,
    pub materials: Vec<ModelMaterial>,
}

impl RenderMap {
    /// Número de vértices de render.
    pub fn len(&self) -> usize {
        self.uvs.len()
    }
    pub fn is_empty(&self) -> bool {
        self.uvs.is_empty()
    }
    /// Duplicados de costura cuyo origen está en `[a, b)` (posiciones dentro de `extra_src`).
    pub fn extras_in(&self, a: u32, b: u32) -> Range<usize> {
        let lo = self.extra_src.partition_point(|&s| s < a);
        let hi = self.extra_src.partition_point(|&s| s < b);
        lo..hi
    }
}

/// Malla de esculpido: posiciones, normales y máscara por vértice, triángulos y adyacencia.
#[derive(Clone, Debug, Default)]
pub struct SculptMesh {
    pub pos: Vec<Vec3>,
    pub nrm: Vec<Vec3>,
    /// Máscara 0..1 por vértice (1 = protegido). Vacía = sin máscara.
    pub mask: Vec<f32>,
    pub tris: Vec<[u32; 3]>,
    /// Vértice → triángulos (CSR).
    pub vt_off: Vec<u32>,
    pub vt: Vec<u32>,
    /// Vértice → vecinos por arista (CSR, ordenados).
    pub vv_off: Vec<u32>,
    pub vv: Vec<u32>,
    /// Vértices en el borde de una malla abierta.
    pub boundary: Vec<bool>,
    pub render: RenderMap,
}

impl SculptMesh {
    pub fn len(&self) -> usize {
        self.pos.len()
    }
    pub fn is_empty(&self) -> bool {
        self.pos.is_empty()
    }

    /// Suelda un modelo en una malla esculpible. No calcula la adyacencia (eso lo hace
    /// `Sculpt::new` tras reordenar los vértices por hojas del BVH).
    pub fn from_model(data: &ModelData) -> Result<SculptMesh, String> {
        if data.skinned() {
            return Err("los modelos con esqueleto no se pueden esculpir todavía".into());
        }
        let m = &data.mesh;
        let n = m.positions.len();
        if n == 0 || m.indices.len() < 3 {
            return Err("el modelo no tiene triángulos".into());
        }
        if m.positions.iter().any(|p| !p.is_finite()) {
            return Err("el modelo tiene posiciones no válidas".into());
        }
        // Soldar por posición cuantizada (las costuras repiten exactamente la misma posición;
        // la tolerancia relativa absorbe redondeos de los exportadores).
        let aabb = m.aabb();
        let eps = (aabb.size().length() * 1e-6).max(1e-9);
        let inv = 1.0 / eps;
        let key = |p: Vec3| {
            let q = (p * inv).round();
            (q.x as i64, q.y as i64, q.z as i64)
        };
        let mut map: HashMap<(i64, i64, i64), u32> = HashMap::with_capacity(n);
        let mut weld = Vec::with_capacity(n);
        let mut rep: Vec<u32> = Vec::new();
        for (r, p) in m.positions.iter().enumerate() {
            let s = *map.entry(key(*p)).or_insert_with(|| {
                rep.push(r as u32);
                (rep.len() - 1) as u32
            });
            weld.push(s);
        }
        let s_count = rep.len();
        // Duplicados de costura, ordenados por su vértice de esculpido.
        let mut extras: Vec<(u32, u32)> = (0..n as u32).filter(|&r| rep[weld[r as usize] as usize] != r).map(|r| (weld[r as usize], r)).collect();
        extras.sort_unstable();
        let mut new_r = vec![0u32; n];
        for (s, &r) in rep.iter().enumerate() {
            new_r[r as usize] = s as u32;
        }
        for (k, &(_, r)) in extras.iter().enumerate() {
            new_r[r as usize] = (s_count + k) as u32;
        }
        let order: Vec<u32> = rep.iter().copied().chain(extras.iter().map(|e| e.1)).collect();
        let pick = |v: &Vec<Vec2>| order.iter().map(|&r| v.get(r as usize).copied().unwrap_or_default()).collect::<Vec<_>>();
        let render = RenderMap {
            uvs: pick(&m.uvs),
            colors: if m.colors.len() == n { order.iter().map(|&r| m.colors[r as usize]).collect() } else { Vec::new() },
            tangents: if m.tangents.len() == n { order.iter().map(|&r| m.tangents[r as usize]).collect() } else { Vec::new() },
            extra_src: extras.iter().map(|e| e.0).collect(),
            indices: m.indices.iter().map(|&i| new_r[i as usize]).collect(),
            submeshes: if data.submeshes.is_empty() { vec![SubMesh { first_index: 0, index_count: m.indices.len() as u32, material: 0 }] } else { data.submeshes.clone() },
            materials: if data.materials.is_empty() { vec![ModelMaterial::default()] } else { data.materials.clone() },
        };
        let tris = m.indices.chunks_exact(3).map(|t| [weld[t[0] as usize], weld[t[1] as usize], weld[t[2] as usize]]).collect();
        let pos = rep.iter().map(|&r| m.positions[r as usize]).collect();
        Ok(SculptMesh { pos, nrm: vec![Vec3::Y; s_count], tris, render, ..Default::default() })
    }

    /// Renumera los vértices: `new_of_old[v]` es el nuevo índice de `v`.
    pub fn permute(&mut self, new_of_old: &[u32]) {
        let n = self.pos.len();
        debug_assert_eq!(new_of_old.len(), n);
        let perm = |v: &mut Vec<Vec3>| {
            let mut out = vec![Vec3::ZERO; n];
            for (o, &k) in new_of_old.iter().enumerate() {
                out[k as usize] = v[o];
            }
            *v = out;
        };
        perm(&mut self.pos);
        perm(&mut self.nrm);
        if self.mask.len() == n {
            let mut out = vec![0.0; n];
            for (o, &k) in new_of_old.iter().enumerate() {
                out[k as usize] = self.mask[o];
            }
            self.mask = out;
        }
        for t in &mut self.tris {
            for i in t.iter_mut() {
                *i = new_of_old[*i as usize];
            }
        }
        // Render: los S primeros siguen a los de esculpido; los duplicados se reordenan por su
        // nuevo origen.
        let rm = &mut self.render;
        let r_count = rm.uvs.len();
        let mut extras: Vec<(u32, u32)> = rm.extra_src.iter().enumerate().map(|(k, &s)| (new_of_old[s as usize], k as u32)).collect();
        extras.sort_unstable();
        let mut new_r = vec![0u32; r_count];
        for (o, &k) in new_of_old.iter().enumerate() {
            new_r[o] = k;
        }
        for (k, &(_, old_k)) in extras.iter().enumerate() {
            new_r[n + old_k as usize] = (n + k) as u32;
        }
        let mut old_of_new = vec![0u32; r_count];
        for (o, &k) in new_r.iter().enumerate() {
            old_of_new[k as usize] = o as u32;
        }
        fn reorder<T: Copy>(v: &mut Vec<T>, old_of_new: &[u32]) {
            if v.len() == old_of_new.len() {
                *v = old_of_new.iter().map(|&o| v[o as usize]).collect();
            }
        }
        reorder(&mut rm.uvs, &old_of_new);
        reorder(&mut rm.colors, &old_of_new);
        reorder(&mut rm.tangents, &old_of_new);
        rm.extra_src = extras.iter().map(|e| e.0).collect();
        for i in &mut rm.indices {
            *i = new_r[*i as usize];
        }
    }

    /// Adyacencia vértice→triángulos, anillos de vecinos y bordes.
    pub fn build_adjacency(&mut self) {
        let n = self.pos.len();
        let mut off = vec![0u32; n + 1];
        for t in &self.tris {
            for &v in t {
                off[v as usize + 1] += 1;
            }
        }
        for i in 0..n {
            off[i + 1] += off[i];
        }
        let mut fill = off.clone();
        let mut vt = vec![0u32; off[n] as usize];
        for (ti, t) in self.tris.iter().enumerate() {
            for &v in t {
                vt[fill[v as usize] as usize] = ti as u32;
                fill[v as usize] += 1;
            }
        }
        // Anillos: los otros dos vértices de cada triángulo, ordenados y sin repetir. Cada
        // vértice escribe en su propio tramo (cota 2·grado), luego se compacta.
        let mut ring = vec![NONE; off[n] as usize * 2];
        let mut counts = vec![0u32; n];
        {
            let mut slices: Vec<&mut [u32]> = Vec::with_capacity(n);
            let mut rest: &mut [u32] = &mut ring;
            for v in 0..n {
                let len = (off[v + 1] - off[v]) as usize * 2;
                let (a, b) = rest.split_at_mut(len);
                slices.push(a);
                rest = b;
            }
            let tris = &self.tris;
            slices.into_par_iter().zip(counts.par_iter_mut()).enumerate().for_each(|(v, (dst, count))| {
                let mut k = 0;
                for &ti in &vt[off[v] as usize..off[v + 1] as usize] {
                    for &u in &tris[ti as usize] {
                        if u as usize != v {
                            dst[k] = u;
                            k += 1;
                        }
                    }
                }
                let s = &mut dst[..k];
                s.sort_unstable();
                let mut w = 0;
                for i in 0..k {
                    if i == 0 || s[i] != s[w - 1] {
                        s[w] = s[i];
                        w += 1;
                    }
                }
                *count = w as u32;
            });
        }
        let mut vv_off = vec![0u32; n + 1];
        for v in 0..n {
            vv_off[v + 1] = vv_off[v] + counts[v];
        }
        let mut vv = Vec::with_capacity(vv_off[n] as usize);
        for v in 0..n {
            let s = off[v] as usize * 2;
            vv.extend_from_slice(&ring[s..s + counts[v] as usize]);
        }
        // Borde: alguna arista del vértice la usa un solo triángulo.
        let tris = &self.tris;
        let boundary = (0..n)
            .into_par_iter()
            .map(|v| {
                let my = &vt[off[v] as usize..off[v + 1] as usize];
                vv[vv_off[v] as usize..vv_off[v + 1] as usize].iter().any(|&u| my.iter().filter(|&&ti| tris[ti as usize].contains(&u)).count() == 1)
            })
            .collect();
        self.vt_off = off;
        self.vt = vt;
        self.vv_off = vv_off;
        self.vv = vv;
        self.boundary = boundary;
    }

    #[inline]
    pub fn tris_of(&self, v: u32) -> &[u32] {
        &self.vt[self.vt_off[v as usize] as usize..self.vt_off[v as usize + 1] as usize]
    }
    #[inline]
    pub fn ring(&self, v: u32) -> &[u32] {
        &self.vv[self.vv_off[v as usize] as usize..self.vv_off[v as usize + 1] as usize]
    }

    /// Normal (sin normalizar, con el doble del área) de un triángulo.
    #[inline]
    pub fn face_area_normal(&self, t: u32) -> Vec3 {
        let [a, b, c] = self.tris[t as usize];
        let (a, b, c) = (self.pos[a as usize], self.pos[b as usize], self.pos[c as usize]);
        (b - a).cross(c - a)
    }

    /// Normal de vértice: media de las de sus triángulos ponderada por área.
    #[inline]
    pub fn vertex_normal(&self, v: u32) -> Vec3 {
        let mut s = Vec3::ZERO;
        for &t in self.tris_of(v) {
            s += self.face_area_normal(t);
        }
        s.normalize_or(Vec3::Y)
    }

    /// Recalcula todas las normales.
    pub fn compute_normals(&mut self) {
        let mut nrm = std::mem::take(&mut self.nrm);
        nrm.resize(self.pos.len(), Vec3::Y);
        nrm.par_iter_mut().enumerate().for_each(|(v, n)| *n = self.vertex_normal(v as u32));
        self.nrm = nrm;
    }

    /// Crea la máscara (a cero) si no existe.
    pub fn ensure_mask(&mut self) {
        if self.mask.len() != self.pos.len() {
            self.mask = vec![0.0; self.pos.len()];
        }
    }

    #[inline]
    pub fn mask_at(&self, v: usize) -> f32 {
        self.mask.get(v).copied().unwrap_or(0.0)
    }

    /// Caja de la malla.
    pub fn bounds(&self) -> (Vec3, Vec3) {
        self.pos.par_iter().fold(|| (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(a, b), p| (a.min(*p), b.max(*p))).reduce(|| (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |x, y| (x.0.min(y.0), x.1.max(y.1)))
    }

    /// Volumen encerrado (teorema de la divergencia; exacto para mallas cerradas).
    pub fn volume(&self) -> f32 {
        self.tris
            .par_iter()
            .map(|t| {
                let (a, b, c) = (self.pos[t[0] as usize], self.pos[t[1] as usize], self.pos[t[2] as usize]);
                a.dot(b.cross(c)) as f64
            })
            .sum::<f64>() as f32
            / 6.0
    }

    /// Longitud media de arista (escala para las operaciones de malla completa).
    pub fn mean_edge(&self) -> f32 {
        if self.tris.is_empty() {
            return 0.0;
        }
        let s: f64 = self
            .tris
            .par_iter()
            .map(|t| {
                let (a, b, c) = (self.pos[t[0] as usize], self.pos[t[1] as usize], self.pos[t[2] as usize]);
                (a.distance(b) + b.distance(c) + c.distance(a)) as f64
            })
            .sum();
        (s / (self.tris.len() * 3) as f64) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use maker_engine::components::PrimitiveShape;
    use maker_engine::mesh::MeshData;

    #[test]
    fn welds_seams_and_keeps_render_vertices() {
        // El cubo de las primitivas tiene 24 vértices (4 por cara): 8 posiciones distintas.
        let data = ModelData::from_mesh(MeshData::primitive(PrimitiveShape::Cube));
        let m = SculptMesh::from_model(&data).unwrap();
        assert_eq!(m.pos.len(), 8);
        assert_eq!(m.render.len(), data.mesh.positions.len());
        assert_eq!(m.render.extra_src.len(), m.render.len() - 8);
        assert!(m.render.extra_src.windows(2).all(|w| w[0] <= w[1]));
        // Cada triángulo de render cae sobre las mismas posiciones que su triángulo soldado.
        for (k, t) in m.tris.iter().enumerate() {
            for c in 0..3 {
                let r = m.render.indices[k * 3 + c] as usize;
                let s = if r < 8 { r as u32 } else { m.render.extra_src[r - 8] };
                assert_eq!(s, t[c]);
                assert_eq!(m.pos[s as usize], data.mesh.positions[data.mesh.indices[k * 3 + c] as usize]);
            }
        }
    }

    #[test]
    fn permutation_keeps_geometry_and_render_map() {
        let data = ModelData::from_mesh(MeshData::primitive(PrimitiveShape::Sphere));
        let mut m = SculptMesh::from_model(&data).unwrap();
        let before: Vec<[Vec3; 3]> = m.tris.iter().map(|t| t.map(|i| m.pos[i as usize])).collect();
        let uv_before: Vec<Vec2> = m.render.indices.iter().map(|&r| m.render.uvs[r as usize]).collect();
        let n = m.pos.len() as u32;
        let perm: Vec<u32> = (0..n).map(|v| (v * 7919) % n).collect();
        let mut seen = vec![false; n as usize];
        for &p in &perm {
            assert!(!seen[p as usize]);
            seen[p as usize] = true;
        }
        m.permute(&perm);
        let after: Vec<[Vec3; 3]> = m.tris.iter().map(|t| t.map(|i| m.pos[i as usize])).collect();
        assert_eq!(before, after);
        let uv_after: Vec<Vec2> = m.render.indices.iter().map(|&r| m.render.uvs[r as usize]).collect();
        assert_eq!(uv_before, uv_after);
        assert!(m.render.extra_src.windows(2).all(|w| w[0] <= w[1]));
        for (k, t) in m.tris.iter().enumerate() {
            for c in 0..3 {
                let r = m.render.indices[k * 3 + c];
                let s = if r < n { r } else { m.render.extra_src[(r - n) as usize] };
                assert_eq!(s, t[c]);
            }
        }
    }

    #[test]
    fn adjacency_rings_and_boundary() {
        let data = ModelData::from_mesh(MeshData::primitive(PrimitiveShape::Plane));
        let mut m = SculptMesh::from_model(&data).unwrap();
        m.build_adjacency();
        // Dos triángulos: todos los vértices son de borde.
        assert!(m.boundary.iter().all(|&b| b));
        let data = ModelData::from_mesh(MeshData::primitive(PrimitiveShape::Cube));
        let mut m = SculptMesh::from_model(&data).unwrap();
        m.build_adjacency();
        assert!(m.boundary.iter().all(|&b| !b), "el cubo soldado es cerrado");
        for v in 0..8 {
            // Cada esquina del cubo triangulado tiene 3 aristas de cubo y 1 o 2 diagonales.
            let r = m.ring(v).len();
            assert!((4..=6).contains(&r), "{r}");
            assert!(m.ring(v).windows(2).all(|w| w[0] < w[1]));
        }
        m.compute_normals();
        for v in 0..8 {
            assert!(m.nrm[v].dot(m.pos[v].normalize()) > 0.5, "normales hacia fuera");
        }
        assert!((m.volume() - 1.0).abs() < 1e-5);
    }
}
