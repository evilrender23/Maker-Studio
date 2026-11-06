//! Bases para esculpir desde cero: esfera de cubo (*quad sphere*), icosfera, plano y cubo
//! subdivididos, con la resolución que se pida. Las mallas salen ya soldadas, con normales y
//! UV, en el formato de `MeshData` (simétricas respecto a los planos de los ejes).

use glam::{Vec2, Vec3};
use maker_engine::mesh::MeshData;
use std::collections::HashMap;

/// Rejilla de `res × res` celdas sobre una cara: `origin + u·i + v·j`.
fn grid(m: &mut MeshData, res: u32, origin: Vec3, u: Vec3, v: Vec3, map: &mut HashMap<[i64; 3], u32>, mut f: impl FnMut(Vec3) -> Vec3) {
    let res = res.max(1);
    let mut ids = Vec::with_capacity(((res + 1) * (res + 1)) as usize);
    for j in 0..=res {
        for i in 0..=res {
            let (a, b) = (i as f32 / res as f32, j as f32 / res as f32);
            let p = origin + u * a + v * b;
            // Clave exacta sobre la rejilla del cubo (antes de deformar) para soldar aristas.
            let key = [(p.x * res as f32 * 2.0).round() as i64, (p.y * res as f32 * 2.0).round() as i64, (p.z * res as f32 * 2.0).round() as i64];
            let id = *map.entry(key).or_insert_with(|| {
                m.positions.push(f(p));
                m.uvs.push(Vec2::new(a, 1.0 - b));
                (m.positions.len() - 1) as u32
            });
            ids.push(id);
        }
    }
    let row = res + 1;
    for j in 0..res {
        for i in 0..res {
            let a = ids[(j * row + i) as usize];
            let b = ids[(j * row + i + 1) as usize];
            let c = ids[((j + 1) * row + i + 1) as usize];
            let d = ids[((j + 1) * row + i) as usize];
            // Diagonal alterna en damero: la malla queda simétrica respecto a los ejes.
            if (i + j) % 2 == 0 {
                m.indices.extend_from_slice(&[a, b, c, a, c, d]);
            } else {
                m.indices.extend_from_slice(&[a, b, d, b, c, d]);
            }
        }
    }
}

/// Las seis caras de un cubo de lado 1 centrado, cada una vista desde fuera (CCW).
fn cube_faces() -> [(Vec3, Vec3, Vec3); 6] {
    [
        (Vec3::new(0.5, -0.5, 0.5), Vec3::NEG_Z, Vec3::Y),
        (Vec3::new(-0.5, -0.5, -0.5), Vec3::Z, Vec3::Y),
        (Vec3::new(-0.5, 0.5, 0.5), Vec3::X, Vec3::NEG_Z),
        (Vec3::new(-0.5, -0.5, -0.5), Vec3::X, Vec3::Z),
        (Vec3::new(-0.5, -0.5, 0.5), Vec3::X, Vec3::Y),
        (Vec3::new(0.5, -0.5, -0.5), Vec3::NEG_X, Vec3::Y),
    ]
}

fn finish(mut m: MeshData) -> MeshData {
    m.compute_normals();
    m
}

/// Esfera hecha de un cubo subdividido proyectado (celdas casi cuadradas, sin polos).
/// `res` celdas por arista: `6·res² + 2` vértices.
pub fn quad_sphere(res: u32, radius: f32) -> MeshData {
    let mut m = MeshData::default();
    let mut map = HashMap::new();
    for (o, u, v) in cube_faces() {
        // Proyección equiangular: celdas de tamaño más parecido que la normalización directa.
        grid(&mut m, res, o, u, v, &mut map, |p| {
            let q = p * 2.0;
            let t = Vec3::new((q.x * std::f32::consts::FRAC_PI_4).tan(), (q.y * std::f32::consts::FRAC_PI_4).tan(), (q.z * std::f32::consts::FRAC_PI_4).tan());
            t.normalize() * radius
        });
    }
    finish(m)
}

/// Cubo de lado 1 con `res` celdas por arista.
pub fn cube(res: u32) -> MeshData {
    let mut m = MeshData::default();
    let mut map = HashMap::new();
    for (o, u, v) in cube_faces() {
        grid(&mut m, res, o, u, v, &mut map, |p| p);
    }
    finish(m)
}

/// Plano de lado 1 en XZ (normal +Y) con `res` celdas por lado.
pub fn plane(res: u32) -> MeshData {
    let mut m = MeshData::default();
    let mut map = HashMap::new();
    grid(&mut m, res, Vec3::new(-0.5, 0.0, 0.5), Vec3::X, Vec3::NEG_Z, &mut map, |p| p);
    finish(m)
}

/// Icosfera: icosaedro subdividido `level` veces (`10·4^level + 2` vértices).
pub fn icosphere(level: u32, radius: f32) -> MeshData {
    let t = (1.0 + 5f32.sqrt()) / 2.0;
    let mut pos: Vec<Vec3> = [[-1.0, t, 0.0], [1.0, t, 0.0], [-1.0, -t, 0.0], [1.0, -t, 0.0], [0.0, -1.0, t], [0.0, 1.0, t], [0.0, -1.0, -t], [0.0, 1.0, -t], [t, 0.0, -1.0], [t, 0.0, 1.0], [-t, 0.0, -1.0], [-t, 0.0, 1.0]]
        .iter()
        .map(|p| Vec3::from(*p).normalize())
        .collect();
    let mut tris: Vec<[u32; 3]> = vec![
        [0, 11, 5], [0, 5, 1], [0, 1, 7], [0, 7, 10], [0, 10, 11], [1, 5, 9], [5, 11, 4], [11, 10, 2], [10, 7, 6], [7, 1, 8],
        [3, 9, 4], [3, 4, 2], [3, 2, 6], [3, 6, 8], [3, 8, 9], [4, 9, 5], [2, 4, 11], [6, 2, 10], [8, 6, 7], [9, 8, 1],
    ];
    for _ in 0..level {
        let mut mid: HashMap<(u32, u32), u32> = HashMap::new();
        let mut get = |a: u32, b: u32, pos: &mut Vec<Vec3>| -> u32 {
            let k = (a.min(b), a.max(b));
            *mid.entry(k).or_insert_with(|| {
                pos.push(((pos[a as usize] + pos[b as usize]) * 0.5).normalize());
                (pos.len() - 1) as u32
            })
        };
        let mut next = Vec::with_capacity(tris.len() * 4);
        for [a, b, c] in tris {
            let ab = get(a, b, &mut pos);
            let bc = get(b, c, &mut pos);
            let ca = get(c, a, &mut pos);
            next.extend_from_slice(&[[a, ab, ca], [b, bc, ab], [c, ca, bc], [ab, bc, ca]]);
        }
        tris = next;
    }
    let mut m = MeshData::default();
    m.uvs = pos.iter().map(|p| Vec2::new(0.5 + p.z.atan2(p.x) / std::f32::consts::TAU, 0.5 - p.y.asin() / std::f32::consts::PI)).collect();
    m.positions = pos.into_iter().map(|p| p * radius).collect();
    m.indices = tris.into_iter().flatten().collect();
    finish(m)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn closed_and_outward(m: &MeshData) {
        let s = crate::mesh::SculptMesh::from_model(&maker_engine::model::ModelData::from_mesh(m.clone())).unwrap();
        assert_eq!(s.pos.len(), m.positions.len(), "ya viene soldada");
        let mut s = s;
        s.build_adjacency();
        assert!(s.boundary.iter().all(|&b| !b), "cerrada");
        assert!(s.volume() > 0.0, "triángulos hacia fuera");
    }

    #[test]
    fn quad_sphere_counts_and_radius() {
        let m = quad_sphere(16, 0.5);
        assert_eq!(m.positions.len(), 6 * 16 * 16 + 2);
        assert!(m.positions.iter().all(|p| (p.length() - 0.5).abs() < 1e-5));
        closed_and_outward(&m);
        let v = crate::mesh::SculptMesh::from_model(&maker_engine::model::ModelData::from_mesh(quad_sphere(48, 1.0))).unwrap().volume();
        assert!((v - 4.0 / 3.0 * std::f32::consts::PI).abs() < 0.02, "{v}");
    }

    #[test]
    fn primitives_are_mirror_symmetric_in_x() {
        for m in [quad_sphere(12, 1.0), cube(6), plane(8), icosphere(2, 1.0)] {
            // Cada vértice tiene su espejo en X.
            for p in &m.positions {
                let q = Vec3::new(-p.x, p.y, p.z);
                assert!(m.positions.iter().any(|r| r.distance(q) < 1e-5), "{p}");
            }
        }
    }

    #[test]
    fn cube_plane_icosphere() {
        let c = cube(4);
        assert_eq!(c.positions.len(), 6 * 16 + 2);
        closed_and_outward(&c);
        let p = plane(10);
        assert_eq!(p.positions.len(), 121);
        assert!(p.normals.iter().all(|n| (*n - Vec3::Y).length() < 1e-5));
        let i = icosphere(3, 1.0);
        assert_eq!(i.positions.len(), 10 * 64 + 2);
        closed_and_outward(&i);
    }
}
