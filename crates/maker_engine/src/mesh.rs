//! Mallas en CPU: primitivas procedurales e importación glTF/GLB.
//! El renderizador las sube a GPU una sola vez y las cachea por `MeshSource`.

use glam::{Mat4, Vec2, Vec3};
use std::f32::consts::{PI, TAU};
use std::path::Path;

use crate::components::PrimitiveShape;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aabb {
    pub min: Vec3,
    pub max: Vec3,
}

impl Aabb {
    pub const EMPTY: Aabb = Aabb { min: Vec3::splat(f32::MAX), max: Vec3::splat(f32::MIN) };
    pub fn grow(&mut self, p: Vec3) {
        self.min = self.min.min(p);
        self.max = self.max.max(p);
    }
    pub fn center(&self) -> Vec3 {
        (self.min + self.max) * 0.5
    }
    pub fn size(&self) -> Vec3 {
        self.max - self.min
    }
    pub fn is_valid(&self) -> bool {
        self.min.x <= self.max.x
    }
    /// AABB del AABB transformado (las 8 esquinas).
    pub fn transformed(&self, m: &Mat4) -> Aabb {
        let mut out = Aabb::EMPTY;
        for i in 0..8 {
            let c = Vec3::new(
                if i & 1 == 0 { self.min.x } else { self.max.x },
                if i & 2 == 0 { self.min.y } else { self.max.y },
                if i & 4 == 0 { self.min.z } else { self.max.z },
            );
            out.grow(m.transform_point3(c));
        }
        out
    }
}

#[derive(Clone, Debug, Default)]
pub struct MeshData {
    pub positions: Vec<Vec3>,
    pub normals: Vec<Vec3>,
    pub uvs: Vec<Vec2>,
    pub indices: Vec<u32>,
}

impl MeshData {
    pub fn aabb(&self) -> Aabb {
        let mut b = Aabb::EMPTY;
        for p in &self.positions {
            b.grow(*p);
        }
        if !b.is_valid() {
            return Aabb { min: Vec3::splat(-0.5), max: Vec3::splat(0.5) };
        }
        b
    }

    fn push(&mut self, p: Vec3, n: Vec3, uv: Vec2) -> u32 {
        self.positions.push(p);
        self.normals.push(n);
        self.uvs.push(uv);
        (self.positions.len() - 1) as u32
    }

    /// Añade otra malla transformada.
    pub fn append(&mut self, other: &MeshData, m: Mat4) {
        let base = self.positions.len() as u32;
        let nm = m.inverse().transpose();
        self.positions.extend(other.positions.iter().map(|p| m.transform_point3(*p)));
        self.normals.extend(other.normals.iter().map(|n| nm.transform_vector3(*n).normalize_or_zero()));
        self.uvs.extend(other.uvs.iter().copied());
        self.indices.extend(other.indices.iter().map(|i| i + base));
    }

    /// Normales planas/suaves calculadas si el archivo no las trae.
    pub fn compute_normals(&mut self) {
        let mut n = vec![Vec3::ZERO; self.positions.len()];
        for t in self.indices.chunks_exact(3) {
            let (a, b, c) = (self.positions[t[0] as usize], self.positions[t[1] as usize], self.positions[t[2] as usize]);
            let f = (b - a).cross(c - a);
            for &i in t {
                n[i as usize] += f;
            }
        }
        self.normals = n.into_iter().map(|v| v.normalize_or(Vec3::Y)).collect();
    }

    pub fn primitive(shape: PrimitiveShape) -> MeshData {
        match shape {
            PrimitiveShape::Cube => cube(),
            PrimitiveShape::Sphere => uv_sphere(0.5, 48, 24),
            PrimitiveShape::Plane => plane(),
            PrimitiveShape::Cylinder => cylinder(0.5, 0.5, 1.0, 48),
            PrimitiveShape::Cone => cylinder(0.5, 0.0, 1.0, 48),
            PrimitiveShape::Capsule => capsule(0.5, 0.5, 48, 12),
            PrimitiveShape::Torus => torus(0.375, 0.125, 48, 24),
        }
    }

    /// Importa todas las mallas de un glTF/GLB, aplicando las transformaciones
    /// de los nodos de la escena por defecto, en una sola malla.
    pub fn load_gltf(path: &Path) -> Result<MeshData, String> {
        let (doc, buffers, _images) = gltf::import(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut out = MeshData::default();
        let scene = doc.default_scene().or_else(|| doc.scenes().next()).ok_or("el glTF no tiene escenas")?;
        let mut stack: Vec<(gltf::Node, Mat4)> = scene.nodes().map(|n| (n, Mat4::IDENTITY)).collect();
        while let Some((node, parent)) = stack.pop() {
            let m = parent * Mat4::from_cols_array_2d(&node.transform().matrix());
            if let Some(mesh) = node.mesh() {
                for prim in mesh.primitives() {
                    if prim.mode() != gltf::mesh::Mode::Triangles {
                        continue;
                    }
                    let reader = prim.reader(|b| Some(&buffers[b.index()]));
                    let Some(pos) = reader.read_positions() else { continue };
                    let mut part = MeshData { positions: pos.map(Vec3::from).collect(), ..Default::default() };
                    part.indices = match reader.read_indices() {
                        Some(i) => i.into_u32().collect(),
                        None => (0..part.positions.len() as u32).collect(),
                    };
                    match reader.read_normals() {
                        Some(n) => part.normals = n.map(Vec3::from).collect(),
                        None => part.compute_normals(),
                    }
                    part.uvs = match reader.read_tex_coords(0) {
                        Some(t) => t.into_f32().map(Vec2::from).collect(),
                        None => vec![Vec2::ZERO; part.positions.len()],
                    };
                    out.append(&part, m);
                }
            }
            for c in node.children() {
                stack.push((c, m));
            }
        }
        if out.indices.is_empty() {
            return Err(format!("{}: no hay triángulos", path.display()));
        }
        Ok(out)
    }
}

fn cube() -> MeshData {
    let mut m = MeshData::default();
    let faces = [
        (Vec3::X, Vec3::NEG_Z, Vec3::Y),
        (Vec3::NEG_X, Vec3::Z, Vec3::Y),
        (Vec3::Y, Vec3::X, Vec3::NEG_Z),
        (Vec3::NEG_Y, Vec3::X, Vec3::Z),
        (Vec3::Z, Vec3::X, Vec3::Y),
        (Vec3::NEG_Z, Vec3::NEG_X, Vec3::Y),
    ];
    for (n, u, v) in faces {
        let c = n * 0.5;
        let a = m.push(c - u * 0.5 - v * 0.5, n, Vec2::new(0.0, 1.0));
        let b = m.push(c + u * 0.5 - v * 0.5, n, Vec2::new(1.0, 1.0));
        let cc = m.push(c + u * 0.5 + v * 0.5, n, Vec2::new(1.0, 0.0));
        let d = m.push(c - u * 0.5 + v * 0.5, n, Vec2::new(0.0, 0.0));
        m.indices.extend_from_slice(&[a, b, cc, a, cc, d]);
    }
    m
}

fn plane() -> MeshData {
    let mut m = MeshData::default();
    let n = Vec3::Y;
    let a = m.push(Vec3::new(-0.5, 0.0, 0.5), n, Vec2::new(0.0, 1.0));
    let b = m.push(Vec3::new(0.5, 0.0, 0.5), n, Vec2::new(1.0, 1.0));
    let c = m.push(Vec3::new(0.5, 0.0, -0.5), n, Vec2::new(1.0, 0.0));
    let d = m.push(Vec3::new(-0.5, 0.0, -0.5), n, Vec2::new(0.0, 0.0));
    m.indices.extend_from_slice(&[a, b, c, a, c, d]);
    m
}

fn uv_sphere(r: f32, seg: u32, rings: u32) -> MeshData {
    let mut m = MeshData::default();
    for y in 0..=rings {
        let v = y as f32 / rings as f32;
        let phi = v * PI;
        for x in 0..=seg {
            let u = x as f32 / seg as f32;
            let th = u * TAU;
            let n = Vec3::new(phi.sin() * th.cos(), phi.cos(), -phi.sin() * th.sin());
            m.push(n * r, n, Vec2::new(u, v));
        }
    }
    let row = seg + 1;
    for y in 0..rings {
        for x in 0..seg {
            let a = y * row + x;
            let b = a + row;
            m.indices.extend_from_slice(&[a, b, a + 1, a + 1, b, b + 1]);
        }
    }
    m
}

/// Tronco de cono (r_top = 0 → cono). Centrado en el origen, eje Y.
fn cylinder(r_bottom: f32, r_top: f32, h: f32, seg: u32) -> MeshData {
    let mut m = MeshData::default();
    let hy = h * 0.5;
    let slope = (r_bottom - r_top) / h;
    for x in 0..=seg {
        let u = x as f32 / seg as f32;
        let th = u * TAU;
        let (s, c) = th.sin_cos();
        let n = Vec3::new(c, slope, -s).normalize();
        m.push(Vec3::new(c * r_bottom, -hy, -s * r_bottom), n, Vec2::new(u, 1.0));
        m.push(Vec3::new(c * r_top, hy, -s * r_top), n, Vec2::new(u, 0.0));
    }
    for x in 0..seg {
        let a = x * 2;
        m.indices.extend_from_slice(&[a, a + 2, a + 1, a + 1, a + 2, a + 3]);
    }
    for (y, r, n) in [(-hy, r_bottom, Vec3::NEG_Y), (hy, r_top, Vec3::Y)] {
        if r <= 0.0 {
            continue;
        }
        let center = m.push(Vec3::new(0.0, y, 0.0), n, Vec2::splat(0.5));
        let start = m.positions.len() as u32;
        for x in 0..=seg {
            let th = x as f32 / seg as f32 * TAU;
            let (s, c) = th.sin_cos();
            m.push(Vec3::new(c * r, y, -s * r), n, Vec2::new(0.5 + c * 0.5, 0.5 + s * 0.5));
        }
        for x in 0..seg {
            let (a, b) = (start + x, start + x + 1);
            if n.y > 0.0 {
                m.indices.extend_from_slice(&[center, a, b]);
            } else {
                m.indices.extend_from_slice(&[center, b, a]);
            }
        }
    }
    m
}

fn capsule(half_h: f32, r: f32, seg: u32, rings: u32) -> MeshData {
    // Esfera partida por el ecuador y estirada.
    let mut m = MeshData::default();
    let total_rings = rings * 2 + 1;
    for y in 0..=total_rings {
        let (phi, off) = if y <= rings {
            (y as f32 / rings as f32 * PI * 0.5, half_h)
        } else {
            (PI * 0.5 + (y - rings - 1) as f32 / rings as f32 * PI * 0.5, -half_h)
        };
        for x in 0..=seg {
            let u = x as f32 / seg as f32;
            let th = u * TAU;
            let n = Vec3::new(phi.sin() * th.cos(), phi.cos(), -phi.sin() * th.sin());
            m.push(n * r + Vec3::Y * off, n, Vec2::new(u, y as f32 / total_rings as f32));
        }
    }
    let row = seg + 1;
    for y in 0..total_rings {
        for x in 0..seg {
            let a = y * row + x;
            let b = a + row;
            m.indices.extend_from_slice(&[a, b, a + 1, a + 1, b, b + 1]);
        }
    }
    m
}

fn torus(big_r: f32, small_r: f32, seg: u32, sides: u32) -> MeshData {
    let mut m = MeshData::default();
    for i in 0..=seg {
        let u = i as f32 / seg as f32;
        let th = u * TAU;
        let dir = Vec3::new(th.cos(), 0.0, -th.sin());
        for j in 0..=sides {
            let v = j as f32 / sides as f32;
            let ph = v * TAU;
            let n = dir * ph.cos() + Vec3::Y * ph.sin();
            m.push(dir * big_r + n * small_r, n, Vec2::new(u, v));
        }
    }
    let row = sides + 1;
    for i in 0..seg {
        for j in 0..sides {
            let a = i * row + j;
            let b = a + row;
            m.indices.extend_from_slice(&[a, b, a + 1, a + 1, b, b + 1]);
        }
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base64(data: &[u8]) -> String {
        const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for c in data.chunks(3) {
            let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
            for i in 0..4 {
                if i <= c.len() {
                    out.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
                } else {
                    out.push('=');
                }
            }
        }
        out
    }

    /// glTF mínimo (un triángulo, sin normales) con un nodo trasladado.
    #[test]
    fn loads_gltf_with_node_transform_and_computed_normals() {
        let pos: [f32; 9] = [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
        let idx: [u16; 3] = [0, 1, 2];
        let mut buf: Vec<u8> = pos.iter().flat_map(|f| f.to_le_bytes()).collect();
        buf.extend(idx.iter().flat_map(|i| i.to_le_bytes()));
        buf.extend([0u8, 0]);
        let json = format!(
            r#"{{"asset":{{"version":"2.0"}},"scene":0,"scenes":[{{"nodes":[0]}}],
            "nodes":[{{"mesh":0,"translation":[5,0,0]}}],
            "meshes":[{{"primitives":[{{"attributes":{{"POSITION":0}},"indices":1}}]}}],
            "buffers":[{{"byteLength":{},"uri":"data:application/octet-stream;base64,{}"}}],
            "bufferViews":[{{"buffer":0,"byteOffset":0,"byteLength":36}},{{"buffer":0,"byteOffset":36,"byteLength":6}}],
            "accessors":[{{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[1,1,0]}},
                         {{"bufferView":1,"componentType":5123,"count":3,"type":"SCALAR"}}]}}"#,
            buf.len(),
            base64(&buf)
        );
        let path = std::env::temp_dir().join(format!("maker-tri-{}.gltf", std::process::id()));
        std::fs::write(&path, json).unwrap();
        let m = MeshData::load_gltf(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(m.indices, vec![0, 1, 2]);
        assert!((m.positions[1] - Vec3::new(6.0, 0.0, 0.0)).length() < 1e-5, "{:?}", m.positions);
        assert!((m.normals[0] - Vec3::Z).length() < 1e-5);
        let b = m.aabb();
        assert_eq!(b.min, Vec3::new(5.0, 0.0, 0.0));
    }

    /// El winding debe ser CCW visto desde fuera (normal geométrica ≈ normal).
    #[test]
    fn primitives_face_outward_and_fit_unit_box() {
        for shape in PrimitiveShape::ALL {
            let m = MeshData::primitive(shape);
            assert!(!m.indices.is_empty());
            let mut bad = 0;
            for t in m.indices.chunks_exact(3) {
                let (a, b, c) = (m.positions[t[0] as usize], m.positions[t[1] as usize], m.positions[t[2] as usize]);
                let f = (b - a).cross(c - a);
                if f.length_squared() < 1e-12 {
                    continue;
                }
                let n = m.normals[t[0] as usize] + m.normals[t[1] as usize] + m.normals[t[2] as usize];
                if f.dot(n) < 0.0 {
                    bad += 1;
                }
            }
            assert_eq!(bad, 0, "{shape:?} tiene triángulos invertidos");
            let b = m.aabb();
            let lim = if shape == PrimitiveShape::Capsule { 1.0 } else { 0.5 };
            assert!(b.max.max_element() <= lim + 1e-4 && b.min.min_element() >= -lim - 1e-4, "{shape:?} {b:?}");
        }
    }
}
