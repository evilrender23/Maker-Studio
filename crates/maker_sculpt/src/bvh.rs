//! Árbol de cajas (BVH) sobre los triángulos, con hojas de 128 a 256 triángulos.
//!
//! Cada vértice pertenece a una sola hoja (la del primer triángulo suyo que se recorre) y los
//! vértices se renumeran para que los de cada hoja sean contiguos: una hoja es a la vez un grupo
//! de triángulos, un rango de vértices y un rango de subida a la GPU. Cada hoja guarda dos
//! cajas: la de sus vértices (que se ajusta en el mismo pase que los mueve) y la de sus
//! triángulos (para los rayos, que se ajusta al vaciar los cambios).

use crate::mesh::NONE;
use glam::Vec3;

/// Máximo de triángulos por hoja (el reparto por la mediana deja entre la mitad y esto).
pub const LEAF_TRIS: usize = 256;

#[derive(Clone, Copy, Debug)]
pub struct Node {
    pub min: Vec3,
    pub max: Vec3,
    pub left: u32,
    pub right: u32,
    /// Índice de hoja, o `NONE` en los nodos internos.
    pub leaf: u32,
    pub parent: u32,
}

#[derive(Clone, Copy, Debug)]
pub struct Leaf {
    /// Rango dentro de `Bvh::tri_order`.
    pub t0: u32,
    pub t1: u32,
    /// Rango de vértices propios.
    pub v0: u32,
    pub v1: u32,
    pub node: u32,
    pub vmin: Vec3,
    pub vmax: Vec3,
    pub tmin: Vec3,
    pub tmax: Vec3,
}

/// Impacto de un rayo.
#[derive(Clone, Copy, Debug)]
pub struct RayHit {
    pub t: f32,
    pub tri: u32,
    /// Baricéntricas de los vértices 1 y 2.
    pub u: f32,
    pub v: f32,
}

#[derive(Clone, Debug, Default)]
pub struct Bvh {
    pub nodes: Vec<Node>,
    pub leaves: Vec<Leaf>,
    pub tri_order: Vec<u32>,
    /// Hojas vecinas (comparten algún triángulo) en CSR.
    pub adj_off: Vec<u32>,
    pub adj: Vec<u32>,
    /// Hoja de cada vértice (tras renumerar), `NONE` si no está en ningún triángulo.
    pub vert_leaf: Vec<u32>,
}

fn union(a: (Vec3, Vec3), b: (Vec3, Vec3)) -> (Vec3, Vec3) {
    (a.0.min(b.0), a.1.max(b.1))
}

const EMPTY: (Vec3, Vec3) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));

impl Bvh {
    /// Construye el árbol. Devuelve también la renumeración de vértices `new_of_old`.
    pub fn build(pos: &[Vec3], tris: &[[u32; 3]]) -> (Bvh, Vec<u32>) {
        let nt = tris.len();
        let cent: Vec<Vec3> = tris.iter().map(|t| (pos[t[0] as usize] + pos[t[1] as usize] + pos[t[2] as usize]) / 3.0).collect();
        let mut order: Vec<u32> = (0..nt as u32).collect();
        let mut nodes = vec![Node { min: Vec3::ZERO, max: Vec3::ZERO, left: NONE, right: NONE, leaf: NONE, parent: NONE }];
        let mut leaves: Vec<Leaf> = Vec::new();
        let mut stack = vec![(0u32, 0usize, nt)];
        while let Some((ni, s, e)) = stack.pop() {
            if e - s <= LEAF_TRIS {
                nodes[ni as usize].leaf = leaves.len() as u32;
                leaves.push(Leaf { t0: s as u32, t1: e as u32, v0: 0, v1: 0, node: ni, vmin: Vec3::ZERO, vmax: Vec3::ZERO, tmin: Vec3::ZERO, tmax: Vec3::ZERO });
                continue;
            }
            let (mut lo, mut hi) = EMPTY;
            for &t in &order[s..e] {
                lo = lo.min(cent[t as usize]);
                hi = hi.max(cent[t as usize]);
            }
            let ext = hi - lo;
            let axis = if ext.x >= ext.y && ext.x >= ext.z { 0 } else if ext.y >= ext.z { 1 } else { 2 };
            let mid = (s + e) / 2;
            order[s..e].select_nth_unstable_by(mid - s, |a, b| cent[*a as usize][axis].total_cmp(&cent[*b as usize][axis]));
            let l = nodes.len() as u32;
            nodes.push(Node { min: Vec3::ZERO, max: Vec3::ZERO, left: NONE, right: NONE, leaf: NONE, parent: ni });
            nodes.push(Node { min: Vec3::ZERO, max: Vec3::ZERO, left: NONE, right: NONE, leaf: NONE, parent: ni });
            nodes[ni as usize].left = l;
            nodes[ni as usize].right = l + 1;
            // La derecha se apila antes: las hojas salen en orden espacial de izquierda a derecha.
            stack.push((l + 1, mid, e));
            stack.push((l, s, mid));
        }
        // Las hojas se crearon en orden de recorrido; los rangos de triángulos son contiguos.
        // Vértices: dueño = primera hoja que los usa; numeración contigua por hoja.
        let nv = pos.len();
        let mut new_of_old = vec![NONE; nv];
        let mut next = 0u32;
        for leaf in leaves.iter_mut() {
            leaf.v0 = next;
            for &t in &order[leaf.t0 as usize..leaf.t1 as usize] {
                for &v in &tris[t as usize] {
                    if new_of_old[v as usize] == NONE {
                        new_of_old[v as usize] = next;
                        next += 1;
                    }
                }
            }
            leaf.v1 = next;
        }
        let owned = next;
        for k in new_of_old.iter_mut() {
            if *k == NONE {
                *k = next;
                next += 1;
            }
        }
        let mut vert_leaf = vec![NONE; nv];
        for (li, leaf) in leaves.iter().enumerate() {
            vert_leaf[leaf.v0 as usize..leaf.v1 as usize].fill(li as u32);
        }
        debug_assert!(vert_leaf[..owned as usize].iter().all(|&l| l != NONE));
        // Vecindad entre hojas: las de los vértices de cada triángulo y la del triángulo.
        let mut tri_leaf = vec![0u32; nt];
        for (li, leaf) in leaves.iter().enumerate() {
            for &t in &order[leaf.t0 as usize..leaf.t1 as usize] {
                tri_leaf[t as usize] = li as u32;
            }
        }
        let mut pairs: Vec<u64> = Vec::new();
        for (t, tri) in tris.iter().enumerate() {
            let ls = [tri_leaf[t], vert_leaf[new_of_old[tri[0] as usize] as usize], vert_leaf[new_of_old[tri[1] as usize] as usize], vert_leaf[new_of_old[tri[2] as usize] as usize]];
            for i in 0..4 {
                for j in 0..4 {
                    if ls[i] != ls[j] {
                        pairs.push((ls[i] as u64) << 32 | ls[j] as u64);
                    }
                }
            }
        }
        pairs.sort_unstable();
        pairs.dedup();
        let mut adj_off = vec![0u32; leaves.len() + 1];
        for p in &pairs {
            adj_off[(p >> 32) as usize + 1] += 1;
        }
        for i in 0..leaves.len() {
            adj_off[i + 1] += adj_off[i];
        }
        let adj = pairs.iter().map(|p| *p as u32).collect();
        (Bvh { nodes, leaves, tri_order: order, adj_off, adj, vert_leaf }, new_of_old)
    }

    pub fn neighbours(&self, leaf: u32) -> &[u32] {
        &self.adj[self.adj_off[leaf as usize] as usize..self.adj_off[leaf as usize + 1] as usize]
    }

    pub fn leaf_verts_box(leaf: &Leaf, pos: &[Vec3]) -> (Vec3, Vec3) {
        pos[leaf.v0 as usize..leaf.v1 as usize].iter().fold(EMPTY, |(a, b), p| (a.min(*p), b.max(*p)))
    }

    pub fn leaf_tris_box(&self, leaf: &Leaf, pos: &[Vec3], tris: &[[u32; 3]]) -> (Vec3, Vec3) {
        let mut bx = EMPTY;
        for &t in &self.tri_order[leaf.t0 as usize..leaf.t1 as usize] {
            for &v in &tris[t as usize] {
                bx.0 = bx.0.min(pos[v as usize]);
                bx.1 = bx.1.max(pos[v as usize]);
            }
        }
        bx
    }

    /// Ajusta todas las cajas (tras construir o cambiar toda la malla).
    pub fn refit_all(&mut self, pos: &[Vec3], tris: &[[u32; 3]]) {
        for i in 0..self.leaves.len() {
            let l = self.leaves[i];
            let (vmin, vmax) = Self::leaf_verts_box(&l, pos);
            let (tmin, tmax) = self.leaf_tris_box(&l, pos, tris);
            let leaf = &mut self.leaves[i];
            (leaf.vmin, leaf.vmax, leaf.tmin, leaf.tmax) = (vmin, vmax, tmin, tmax);
        }
        // Los hijos siempre van detrás de su padre: basta recorrer al revés.
        for i in (0..self.nodes.len()).rev() {
            let n = self.nodes[i];
            let b = if n.leaf != NONE {
                let l = &self.leaves[n.leaf as usize];
                union((l.vmin, l.vmax), (l.tmin, l.tmax))
            } else {
                let (a, c) = (&self.nodes[n.left as usize], &self.nodes[n.right as usize]);
                union((a.min, a.max), (c.min, c.max))
            };
            self.nodes[i].min = b.0;
            self.nodes[i].max = b.1;
        }
    }

    /// Recalcula la caja de los nodos de estas hojas y de sus antecesores (las cajas de hoja
    /// ya deben estar al día).
    pub fn refit_up(&mut self, leaves: &[u32]) {
        for &l in leaves {
            let lf = self.leaves[l as usize];
            let b = union((lf.vmin, lf.vmax), (lf.tmin, lf.tmax));
            let mut ni = lf.node;
            self.nodes[ni as usize].min = b.0;
            self.nodes[ni as usize].max = b.1;
            ni = self.nodes[ni as usize].parent;
            while ni != NONE {
                let n = self.nodes[ni as usize];
                let (a, c) = (&self.nodes[n.left as usize], &self.nodes[n.right as usize]);
                let b = union((a.min, a.max), (c.min, c.max));
                self.nodes[ni as usize].min = b.0;
                self.nodes[ni as usize].max = b.1;
                ni = n.parent;
            }
        }
    }

    /// Hojas cuya caja toca la esfera.
    pub fn leaves_in_sphere(&self, c: Vec3, r: f32, out: &mut Vec<u32>) {
        out.clear();
        if self.nodes.is_empty() || self.leaves.is_empty() {
            return;
        }
        let r2 = r * r;
        let mut stack = [0u32; 64];
        let mut sp = 1;
        while sp > 0 {
            sp -= 1;
            let n = &self.nodes[stack[sp] as usize];
            let d = (c.clamp(n.min, n.max) - c).length_squared();
            if d > r2 {
                continue;
            }
            if n.leaf != NONE {
                out.push(n.leaf);
            } else {
                stack[sp] = n.left;
                stack[sp + 1] = n.right;
                sp += 2;
            }
        }
    }

    /// Hojas cuya caja toca el cilindro infinito de radio `r` alrededor de la recta `o + t·d`.
    pub fn leaves_in_cylinder(&self, o: Vec3, d: Vec3, r: f32, out: &mut Vec<u32>) {
        out.clear();
        if self.nodes.is_empty() || self.leaves.is_empty() {
            return;
        }
        let inv = safe_inv(d);
        let mut stack = [0u32; 64];
        let mut sp = 1;
        while sp > 0 {
            sp -= 1;
            let n = &self.nodes[stack[sp] as usize];
            // Caja engordada por el radio contra la recta (conservador).
            let t0 = (n.min - Vec3::splat(r) - o) * inv;
            let t1 = (n.max + Vec3::splat(r) - o) * inv;
            let tmin = t0.min(t1).max_element();
            let tmax = t0.max(t1).min_element();
            if !(tmax >= tmin) {
                continue;
            }
            if n.leaf != NONE {
                out.push(n.leaf);
            } else {
                stack[sp] = n.left;
                stack[sp + 1] = n.right;
                sp += 2;
            }
        }
    }

    /// Impacto más cercano del rayo (por las dos caras).
    pub fn ray(&self, o: Vec3, d: Vec3, pos: &[Vec3], tris: &[[u32; 3]]) -> Option<RayHit> {
        if self.nodes.is_empty() || self.leaves.is_empty() {
            return None;
        }
        let inv = safe_inv(d);
        let slab = |n: &Node, best: f32| -> Option<f32> {
            let t0 = (n.min - o) * inv;
            let t1 = (n.max - o) * inv;
            let tmin = t0.min(t1).max_element().max(0.0);
            let tmax = t0.max(t1).min_element().min(best);
            (tmax >= tmin).then_some(tmin)
        };
        let mut best: Option<RayHit> = None;
        let mut best_t = f32::INFINITY;
        let mut stack = [0u32; 64];
        let mut sp = 0;
        if slab(&self.nodes[0], best_t).is_some() {
            stack[0] = 0;
            sp = 1;
        }
        while sp > 0 {
            sp -= 1;
            let n = &self.nodes[stack[sp] as usize];
            if slab(n, best_t).is_none() {
                continue;
            }
            if n.leaf != NONE {
                let l = &self.leaves[n.leaf as usize];
                for &t in &self.tri_order[l.t0 as usize..l.t1 as usize] {
                    let [a, b, c] = tris[t as usize];
                    if let Some((tt, u, v)) = ray_triangle(o, d, pos[a as usize], pos[b as usize], pos[c as usize])
                        && tt < best_t
                    {
                        best_t = tt;
                        best = Some(RayHit { t: tt, tri: t, u, v });
                    }
                }
            } else {
                let (a, b) = (&self.nodes[n.left as usize], &self.nodes[n.right as usize]);
                let (ta, tb) = (slab(a, best_t), slab(b, best_t));
                // El más cercano se apila el último (sale primero).
                match (ta, tb) {
                    (Some(x), Some(y)) => {
                        if x <= y {
                            stack[sp] = n.right;
                            stack[sp + 1] = n.left;
                        } else {
                            stack[sp] = n.left;
                            stack[sp + 1] = n.right;
                        }
                        sp += 2;
                    }
                    (Some(_), None) => {
                        stack[sp] = n.left;
                        sp += 1;
                    }
                    (None, Some(_)) => {
                        stack[sp] = n.right;
                        sp += 1;
                    }
                    _ => {}
                }
            }
        }
        best
    }
}

/// Inverso por componentes sin infinitos (los ejes paralelos dan pendientes enormes, no NaN).
#[inline]
fn safe_inv(d: Vec3) -> Vec3 {
    Vec3::select(d.abs().cmplt(Vec3::splat(1e-30)), Vec3::splat(1e30), Vec3::ONE / d)
}

/// Möller–Trumbore por las dos caras: (t, u, v).
#[inline]
pub fn ray_triangle(o: Vec3, d: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Option<(f32, f32, f32)> {
    let e1 = b - a;
    let e2 = c - a;
    let p = d.cross(e2);
    let det = e1.dot(p);
    if det.abs() < 1e-20 {
        return None;
    }
    let inv = 1.0 / det;
    let s = o - a;
    let u = s.dot(p) * inv;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = s.cross(e1);
    let v = d.dot(q) * inv;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let t = e2.dot(q) * inv;
    (t >= 0.0).then_some((t, u, v))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives;

    fn sphere() -> (Vec<Vec3>, Vec<[u32; 3]>) {
        let m = primitives::quad_sphere(32, 1.0);
        let s = crate::mesh::SculptMesh::from_model(&maker_engine::model::ModelData::from_mesh(m)).unwrap();
        (s.pos, s.tris)
    }

    #[test]
    fn leaves_partition_triangles_and_vertices() {
        let (pos, tris) = sphere();
        let (b, perm) = Bvh::build(&pos, &tris);
        let mut seen = vec![false; tris.len()];
        for l in &b.leaves {
            let n = (l.t1 - l.t0) as usize;
            assert!(n <= LEAF_TRIS && n >= LEAF_TRIS / 4, "{n}");
            for &t in &b.tri_order[l.t0 as usize..l.t1 as usize] {
                assert!(!seen[t as usize]);
                seen[t as usize] = true;
            }
        }
        assert!(seen.iter().all(|&s| s));
        // Rangos de vértices contiguos que cubren todos.
        let mut next = 0;
        for l in &b.leaves {
            assert_eq!(l.v0, next);
            next = l.v1;
        }
        assert_eq!(next as usize, pos.len());
        let mut p = perm.clone();
        p.sort_unstable();
        assert!(p.iter().enumerate().all(|(i, &k)| i as u32 == k));
    }

    fn brute_ray(o: Vec3, d: Vec3, pos: &[Vec3], tris: &[[u32; 3]]) -> Option<f32> {
        tris.iter().filter_map(|t| ray_triangle(o, d, pos[t[0] as usize], pos[t[1] as usize], pos[t[2] as usize]).map(|h| h.0)).min_by(|a, b| a.total_cmp(b))
    }

    #[test]
    fn ray_matches_brute_force() {
        let (pos, tris) = sphere();
        let (mut b, _) = Bvh::build(&pos, &tris);
        b.refit_all(&pos, &tris);
        let mut k = 0u32;
        let mut rnd = || {
            k = k.wrapping_mul(1664525).wrapping_add(1013904223);
            (k >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0
        };
        for _ in 0..200 {
            let o = Vec3::new(rnd(), rnd(), rnd()).normalize() * 3.0;
            let target = Vec3::new(rnd(), rnd(), rnd()) * 1.2;
            let d = (target - o).normalize();
            let got = b.ray(o, d, &pos, &tris).map(|h| h.t);
            let want = brute_ray(o, d, &pos, &tris);
            match (got, want) {
                (Some(g), Some(w)) => assert!((g - w).abs() < 1e-4, "{g} {w}"),
                (None, None) => {}
                x => panic!("{x:?}"),
            }
        }
        // Desde el centro hacia fuera: radio 1.
        let h = b.ray(Vec3::ZERO, Vec3::X, &pos, &tris).unwrap();
        assert!((h.t - 1.0).abs() < 0.01);
    }

    #[test]
    fn sphere_and_cylinder_queries_find_all_inside() {
        let (pos, tris) = sphere();
        let (mut b, perm) = Bvh::build(&pos, &tris);
        let mut p2 = vec![Vec3::ZERO; pos.len()];
        for (o, &k) in perm.iter().enumerate() {
            p2[k as usize] = pos[o];
        }
        let t2: Vec<[u32; 3]> = tris.iter().map(|t| t.map(|i| perm[i as usize])).collect();
        b.refit_all(&p2, &t2);
        let mut out = Vec::new();
        let c = Vec3::new(0.0, 0.0, 1.0);
        b.leaves_in_sphere(c, 0.3, &mut out);
        let inside: Vec<u32> = (0..p2.len() as u32).filter(|&v| p2[v as usize].distance(c) < 0.3).collect();
        assert!(!inside.is_empty());
        for v in &inside {
            assert!(out.contains(&b.vert_leaf[*v as usize]));
        }
        assert!(out.len() < b.leaves.len() / 2, "consulta local: {} de {}", out.len(), b.leaves.len());
        // Cilindro a lo largo de Z: atraviesa delante y detrás.
        b.leaves_in_cylinder(Vec3::new(0.0, 0.0, 5.0), Vec3::NEG_Z, 0.2, &mut out);
        let inside: Vec<u32> = (0..p2.len() as u32).filter(|&v| Vec3::new(p2[v as usize].x, p2[v as usize].y, 0.0).length() < 0.2).collect();
        assert!(inside.iter().any(|&v| p2[v as usize].z > 0.5) && inside.iter().any(|&v| p2[v as usize].z < -0.5));
        for v in &inside {
            assert!(out.contains(&b.vert_leaf[*v as usize]));
        }
    }

    #[test]
    fn refit_up_follows_moved_vertices() {
        let (pos, tris) = sphere();
        let (mut b, perm) = Bvh::build(&pos, &tris);
        let mut p2 = vec![Vec3::ZERO; pos.len()];
        for (o, &k) in perm.iter().enumerate() {
            p2[k as usize] = pos[o];
        }
        let t2: Vec<[u32; 3]> = tris.iter().map(|t| t.map(|i| perm[i as usize])).collect();
        b.refit_all(&p2, &t2);
        // Sacar un vértice lejos: tras reajustar su hoja y la raíz lo contienen.
        let v = 10usize;
        p2[v] = Vec3::new(0.0, 9.0, 0.0);
        let l = b.vert_leaf[v];
        let lf = b.leaves[l as usize];
        let (a, c) = Bvh::leaf_verts_box(&lf, &p2);
        b.leaves[l as usize].vmin = a;
        b.leaves[l as usize].vmax = c;
        b.refit_up(&[l]);
        assert!(b.nodes[0].max.y >= 9.0);
        let mut out = Vec::new();
        b.leaves_in_sphere(Vec3::new(0.0, 9.0, 0.0), 0.01, &mut out);
        assert_eq!(out, vec![l]);
    }
}
