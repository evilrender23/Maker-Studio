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

/// Divide cada triángulo en cuatro por los puntos medios de sus aristas (atributos
/// promediados). Las costuras dan puntos medios repetidos en la misma posición, que se sueldan
/// al esculpir.
pub fn subdivide(m: &MeshData) -> MeshData {
    let n = m.positions.len();
    let mut out = MeshData {
        positions: m.positions.clone(),
        normals: if m.normals.len() == n { m.normals.clone() } else { Vec::new() },
        uvs: if m.uvs.len() == n { m.uvs.clone() } else { vec![Vec2::ZERO; n] },
        colors: if m.colors.len() == n { m.colors.clone() } else { Vec::new() },
        ..Default::default()
    };
    let mut mid: HashMap<(u32, u32), u32> = HashMap::with_capacity(m.indices.len());
    let mut get = |a: u32, b: u32, out: &mut MeshData| -> u32 {
        *mid.entry((a.min(b), a.max(b))).or_insert_with(|| {
            let (a, b) = (a as usize, b as usize);
            out.positions.push((out.positions[a] + out.positions[b]) * 0.5);
            if !out.normals.is_empty() {
                out.normals.push((out.normals[a] + out.normals[b]).normalize_or(Vec3::Y));
            }
            out.uvs.push((out.uvs[a] + out.uvs[b]) * 0.5);
            if !out.colors.is_empty() {
                let (x, y) = (out.colors[a], out.colors[b]);
                out.colors.push(std::array::from_fn(|i| ((x[i] as u16 + y[i] as u16) / 2) as u8));
            }
            (out.positions.len() - 1) as u32
        })
    };
    let mut idx = Vec::with_capacity(m.indices.len() * 4);
    for t in m.indices.chunks_exact(3) {
        let (a, b, c) = (t[0], t[1], t[2]);
        let ab = get(a, b, &mut out);
        let bc = get(b, c, &mut out);
        let ca = get(c, a, &mut out);
        idx.extend_from_slice(&[a, ab, ca, b, bc, ab, c, ca, bc, ab, bc, ca]);
    }
    out.indices = idx;
    if out.normals.len() != out.positions.len() {
        out.compute_normals();
    }
    out
}

/// Base esculpible de una primitiva del motor: la esfera, el cubo y el plano se generan con
/// celdas regulares; el resto se subdivide hasta tener al menos `min_vertices` vértices.
pub fn sculptable(shape: maker_engine::components::PrimitiveShape, min_vertices: usize) -> MeshData {
    use maker_engine::components::PrimitiveShape as P;
    // Celdas por arista para llegar a los vértices pedidos.
    let per_face = |faces: f32| ((min_vertices as f32 / faces).sqrt().ceil() as u32).max(2);
    match shape {
        P::Sphere => quad_sphere(per_face(6.0), 0.5),
        P::Cube => cube(per_face(6.0)),
        P::Plane => plane(per_face(1.0)),
        other => {
            let mut m = MeshData::primitive(other);
            m.tangents.clear();
            // Cuenta los vértices ya soldados (las costuras repiten posiciones).
            let welded = |m: &MeshData| m.positions.iter().map(|p| p.to_array().map(f32::to_bits)).collect::<std::collections::HashSet<_>>().len();
            while welded(&m) < min_vertices {
                m = subdivide(&m);
            }
            m
        }
    }
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
    fn subdivide_and_sculptable_bases() {
        use maker_engine::components::PrimitiveShape;
        let c = cube(1);
        let s = subdivide(&c);
        assert_eq!(s.indices.len(), c.indices.len() * 4);
        closed_and_outward(&s);
        for shape in PrimitiveShape::ALL {
            let m = sculptable(shape, 5000);
            let sc = crate::Sculpt::from_mesh(m.clone());
            assert!(sc.vertex_count() >= 5000 * 9 / 10, "{shape:?}: {}", sc.vertex_count());
            assert!(sc.vertex_count() < 5000 * 5, "{shape:?}: {}", sc.vertex_count());
            // La forma no cambia de tamaño.
            let b = m.aabb();
            let o = MeshData::primitive(shape).aabb();
            assert!((b.size() - o.size()).length() < 0.05, "{shape:?}");
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
