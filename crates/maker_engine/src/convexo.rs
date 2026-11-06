//! Colisionador convexo: el casco convexo de la malla de la entidad, simplificado según la
//! complejidad (0 = casi una caja, 1 = todos los vértices). Para simplificar se agrupan los
//! vértices en una rejilla de celdas (más celdas cuanta más complejidad) y se toma la media de
//! cada celda. Los cascos se guardan en caché por malla y complejidad.

use crate::components::MeshSource;
use crate::mesh::MeshData;
use glam::Vec3;
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

/// Celdas en el eje más largo para una complejidad (0..=1). `None` = sin simplificar.
pub fn celdas(complejidad: f32) -> Option<u32> {
    let c = complejidad.clamp(0.0, 1.0);
    (c < 0.999).then(|| 2 + (c * c * 62.0).round() as u32)
}

/// Agrupa los puntos en una rejilla (`celdas` en el eje más largo) y devuelve la media de cada celda.
pub fn simplificar(puntos: &[Vec3], complejidad: f32) -> Vec<Vec3> {
    let Some(n) = celdas(complejidad) else { return puntos.to_vec() };
    if puntos.is_empty() {
        return Vec::new();
    }
    let (mut min, mut max) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    for p in puntos {
        min = min.min(*p);
        max = max.max(*p);
    }
    let lado = ((max - min).max_element() / n as f32).max(1e-6);
    let mut celdas: HashMap<(i32, i32, i32), (Vec3, u32)> = HashMap::new();
    for p in puntos {
        // El extremo máximo cae justo en la celda `n`: va a la última.
        let k = ((*p - min) / lado).floor().min(Vec3::splat(n as f32 - 1.0));
        let e = celdas.entry((k.x as i32, k.y as i32, k.z as i32)).or_insert((Vec3::ZERO, 0));
        e.0 += *p;
        e.1 += 1;
    }
    let mut v: Vec<Vec3> = celdas.into_values().map(|(s, c)| s / c as f32).collect();
    v.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)).then(a.z.total_cmp(&b.z)));
    v
}

/// Vértices de la malla en espacio del modelo (en caché: el slider pide muchas complejidades).
fn vertices(src: &MeshSource, raiz: Option<&Path>, clave: &str) -> Option<Arc<Vec<Vec3>>> {
    static C: OnceLock<Mutex<HashMap<String, Option<Arc<Vec<Vec3>>>>>> = OnceLock::new();
    let c = C.get_or_init(Default::default);
    if let Some(v) = c.lock().unwrap().get(clave) {
        return v.clone();
    }
    let v = match src {
        MeshSource::Primitive(p) => Some(MeshData::primitive(*p).positions),
        MeshSource::Model(m) => MeshData::load_gltf(&crate::biblioteca::resolve(m, raiz)).ok().map(|d| d.positions),
    }
    .map(Arc::new);
    c.lock().unwrap().insert(clave.to_string(), v.clone());
    v
}

type Clave = (String, u32);

fn cache() -> &'static Mutex<HashMap<Clave, Option<Arc<Vec<Vec3>>>>> {
    static C: OnceLock<Mutex<HashMap<Clave, Option<Arc<Vec<Vec3>>>>>> = OnceLock::new();
    C.get_or_init(Default::default)
}

/// Vértices del casco convexo (sin escala) de la malla con esa complejidad. `None` si la malla no
/// carga o es plana del todo.
pub fn casco(src: &MeshSource, raiz: Option<&Path>, complejidad: f32) -> Option<Arc<Vec<Vec3>>> {
    let clave = (format!("{src:?}|{}", raiz.map(|r| r.display().to_string()).unwrap_or_default()), (complejidad.clamp(0.0, 1.0) * 1000.0).round() as u32);
    if let Some(c) = cache().lock().unwrap().get(&clave) {
        return c.clone();
    }
    let r = vertices(src, raiz, &clave.0).and_then(|v| {
        let puntos = simplificar(&v, complejidad);
        let forma = rapier3d::prelude::SharedShape::convex_hull(&puntos)?;
        let poli = forma.as_convex_polyhedron()?;
        Some(Arc::new(poli.points().iter().map(|p| Vec3::new(p.x, p.y, p.z)).collect::<Vec<Vec3>>()))
    });
    cache().lock().unwrap().insert(clave, r.clone());
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::PrimitiveShape;

    #[test]
    fn la_complejidad_reduce_los_vertices() {
        let esfera = MeshSource::Primitive(PrimitiveShape::Sphere);
        let poco = casco(&esfera, None, 0.0).unwrap().len();
        let medio = casco(&esfera, None, 0.5).unwrap().len();
        let todo = casco(&esfera, None, 1.0).unwrap().len();
        assert!(poco <= 8 && poco >= 4, "{poco}");
        assert!(poco < medio && medio < todo, "{poco} {medio} {todo}");
        // Todos los puntos del casco caben en la esfera unidad.
        assert!(casco(&esfera, None, 1.0).unwrap().iter().all(|p| p.length() <= 0.5001));
    }
}
