//! Escritura de modelos a glTF binario (`.glb`): una malla con una primitiva por submalla,
//! materiales con su color, alfa, doble cara y texturas (incrustadas, para que el archivo sea
//! autónomo).

use glam::Vec3;
use maker_engine::model::{AlphaMode, ModelData, TextureSource};
use serde_json::{Value, json};
use std::path::Path;

fn align4(b: &mut Vec<u8>) {
    while b.len() % 4 != 0 {
        b.push(0);
    }
}

fn mime(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG") {
        Some("image/png")
    } else if bytes.starts_with(&[0xFF, 0xD8]) {
        Some("image/jpeg")
    } else {
        None
    }
}

struct Writer {
    bin: Vec<u8>,
    views: Vec<Value>,
    accessors: Vec<Value>,
}

impl Writer {
    fn view(&mut self, data: &[u8], target: Option<u32>) -> usize {
        align4(&mut self.bin);
        let off = self.bin.len();
        self.bin.extend_from_slice(data);
        let mut v = json!({"buffer": 0, "byteOffset": off, "byteLength": data.len()});
        if let Some(t) = target {
            v["target"] = json!(t);
        }
        self.views.push(v);
        self.views.len() - 1
    }
    fn accessor(&mut self, view: usize, offset: usize, ctype: u32, count: usize, ty: &str, normalized: bool, minmax: Option<(Vec3, Vec3)>) -> usize {
        let mut a = json!({"bufferView": view, "byteOffset": offset, "componentType": ctype, "count": count, "type": ty});
        if normalized {
            a["normalized"] = json!(true);
        }
        if let Some((lo, hi)) = minmax {
            a["min"] = json!([lo.x, lo.y, lo.z]);
            a["max"] = json!([hi.x, hi.y, hi.z]);
        }
        self.accessors.push(a);
        self.accessors.len() - 1
    }
}

const FLOAT: u32 = 5126;
const UBYTE: u32 = 5121;
const UINT: u32 = 5125;
const ARRAY: u32 = 34962;
const ELEMENTS: u32 = 34963;

/// El `.glb` del modelo en memoria. `name` nombra la malla y el nodo.
pub fn glb_bytes(model: &ModelData, name: &str) -> Vec<u8> {
    let m = &model.mesh;
    let n = m.positions.len();
    let mut w = Writer { bin: Vec::new(), views: Vec::new(), accessors: Vec::new() };
    let f32s = |v: &mut Vec<u8>, xs: &[f32]| v.extend(xs.iter().flat_map(|f| f.to_le_bytes()));

    let mut bytes = Vec::with_capacity(n * 12);
    let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    for p in &m.positions {
        lo = lo.min(*p);
        hi = hi.max(*p);
        f32s(&mut bytes, &p.to_array());
    }
    let pv = w.view(&bytes, Some(ARRAY));
    let mut attrs = serde_json::Map::new();
    attrs.insert("POSITION".into(), json!(w.accessor(pv, 0, FLOAT, n, "VEC3", false, Some((lo, hi)))));
    if m.normals.len() == n {
        bytes.clear();
        for v in &m.normals {
            f32s(&mut bytes, &v.to_array());
        }
        let v = w.view(&bytes, Some(ARRAY));
        attrs.insert("NORMAL".into(), json!(w.accessor(v, 0, FLOAT, n, "VEC3", false, None)));
    }
    if m.uvs.len() == n {
        bytes.clear();
        for v in &m.uvs {
            f32s(&mut bytes, &v.to_array());
        }
        let v = w.view(&bytes, Some(ARRAY));
        attrs.insert("TEXCOORD_0".into(), json!(w.accessor(v, 0, FLOAT, n, "VEC2", false, None)));
    }
    if m.tangents.len() == n {
        bytes.clear();
        for v in &m.tangents {
            f32s(&mut bytes, &v.to_array());
        }
        let v = w.view(&bytes, Some(ARRAY));
        attrs.insert("TANGENT".into(), json!(w.accessor(v, 0, FLOAT, n, "VEC4", false, None)));
    }
    if m.colors.len() == n {
        bytes.clear();
        for c in &m.colors {
            bytes.extend_from_slice(c);
        }
        let v = w.view(&bytes, Some(ARRAY));
        attrs.insert("COLOR_0".into(), json!(w.accessor(v, 0, UBYTE, n, "VEC4", true, None)));
    }
    bytes.clear();
    for i in &m.indices {
        bytes.extend_from_slice(&i.to_le_bytes());
    }
    let iv = w.view(&bytes, Some(ELEMENTS));

    // Materiales y texturas.
    let mut images: Vec<Value> = Vec::new();
    let mut textures: Vec<Value> = Vec::new();
    let mut tex_index: Vec<(String, usize)> = Vec::new();
    let mut texture = |w: &mut Writer, src: &TextureSource| -> Option<usize> {
        let key = src.key();
        if let Some((_, i)) = tex_index.iter().find(|(k, _)| *k == key) {
            return Some(*i);
        }
        let data = match src {
            TextureSource::File(p) => std::fs::read(p).ok()?,
            TextureSource::Embedded { bytes, .. } => bytes.as_ref().clone(),
        };
        let mt = mime(&data)?;
        let v = w.view(&data, None);
        images.push(json!({"bufferView": v, "mimeType": mt}));
        textures.push(json!({"source": images.len() - 1, "sampler": 0}));
        tex_index.push((key, textures.len() - 1));
        Some(textures.len() - 1)
    };
    let mut materials = Vec::new();
    for mm in &model.materials {
        let mut pbr = json!({"baseColorFactor": mm.base_color, "metallicFactor": 0.0, "roughnessFactor": 1.0});
        if let Some(t) = mm.texture.as_ref().and_then(|t| texture(&mut w, t)) {
            pbr["baseColorTexture"] = json!({"index": t});
        }
        let mut mat = json!({"name": mm.name, "pbrMetallicRoughness": pbr, "doubleSided": mm.double_sided});
        match mm.alpha {
            AlphaMode::Opaque => {}
            AlphaMode::Mask(c) => {
                mat["alphaMode"] = json!("MASK");
                mat["alphaCutoff"] = json!(c);
            }
            AlphaMode::Blend => mat["alphaMode"] = json!("BLEND"),
        }
        if let Some(t) = mm.normal.as_ref().and_then(|t| texture(&mut w, t)) {
            mat["normalTexture"] = json!({"index": t, "scale": mm.normal_scale});
        }
        materials.push(mat);
    }

    let mut prims = Vec::new();
    for s in &model.submeshes {
        let a = w.accessor(iv, s.first_index as usize * 4, UINT, s.index_count as usize, "SCALAR", false, None);
        let mut p = json!({"attributes": Value::Object(attrs.clone()), "indices": a, "mode": 4});
        if s.material < materials.len() {
            p["material"] = json!(s.material);
        }
        prims.push(p);
    }
    align4(&mut w.bin);
    let mut doc = json!({
        "asset": {"version": "2.0", "generator": "Maker Studio (esculpir)"},
        "scene": 0,
        "scenes": [{"nodes": [0]}],
        "nodes": [{"mesh": 0, "name": name}],
        "meshes": [{"name": name, "primitives": prims}],
        "accessors": w.accessors,
        "bufferViews": w.views,
        "buffers": [{"byteLength": w.bin.len()}],
    });
    if !materials.is_empty() {
        doc["materials"] = json!(materials);
    }
    if !textures.is_empty() {
        doc["textures"] = json!(textures);
        doc["images"] = json!(images);
        doc["samplers"] = json!([{"magFilter": 9729, "minFilter": 9987, "wrapS": 10497, "wrapT": 10497}]);
    }
    let mut js = serde_json::to_vec(&doc).unwrap_or_default();
    while js.len() % 4 != 0 {
        js.push(b' ');
    }
    let total = 12 + 8 + js.len() + 8 + w.bin.len();
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(js.len() as u32).to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(&js);
    out.extend_from_slice(&(w.bin.len() as u32).to_le_bytes());
    out.extend_from_slice(b"BIN\0");
    out.extend_from_slice(&w.bin);
    out
}

/// Escribe el `.glb` (crea la carpeta; escribe a un temporal y lo renombra).
pub fn write_glb(model: &ModelData, path: &Path, name: &str) -> Result<(), String> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    let tmp = path.with_extension("glb.tmp");
    std::fs::write(&tmp, glb_bytes(model, name)).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Action, Brush, Dab, DabOptions, Sculpt, primitives};
    use maker_engine::model::ModelMaterial;
    use std::sync::Arc;

    fn temp(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("maker-sculpt-{}-{name}", std::process::id()));
        let _ = std::fs::create_dir_all(&d);
        d.join(format!("{name}.glb"))
    }

    #[test]
    fn sculpt_roundtrips_through_glb() {
        let mut sc = Sculpt::from_mesh(primitives::quad_sphere(20, 0.5));
        sc.dab(Action::Brush { brush: Brush::Standard, invert: false }, &Dab::at(Vec3::new(0.0, 0.0, 0.5), Vec3::Z, 0.2, 1.0), &DabOptions::default());
        sc.end_stroke();
        let model = sc.to_model();
        let path = temp("ida");
        write_glb(&model, &path, "Esfera").unwrap();
        let back = ModelData::load(&path).unwrap();
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
        assert_eq!(back.mesh.indices, model.mesh.indices);
        assert_eq!(back.mesh.positions.len(), model.mesh.positions.len());
        for (a, b) in back.mesh.positions.iter().zip(&model.mesh.positions) {
            assert_eq!(a, b, "posiciones exactas (f32)");
        }
        for (a, b) in back.mesh.uvs.iter().zip(&model.mesh.uvs) {
            assert_eq!(a, b);
        }
        // Y vuelve a ser la misma escultura al reabrirla.
        let again = Sculpt::from_model(&back).unwrap();
        assert_eq!(again.vertex_count(), sc.vertex_count());
        assert!((again.mesh.volume() - sc.mesh.volume()).abs() < 1e-6);
    }

    #[test]
    fn materials_seams_and_embedded_textures_survive() {
        // Un cubo con costuras (24 vértices de render, 8 de esculpido), dos materiales y una
        // textura PNG incrustada.
        let mut data = ModelData::from_mesh(maker_engine::mesh::MeshData::primitive(maker_engine::components::PrimitiveShape::Cube));
        let half = data.mesh.indices.len() as u32 / 2;
        data.submeshes = vec![
            maker_engine::model::SubMesh { first_index: 0, index_count: half, material: 0 },
            maker_engine::model::SubMesh { first_index: half, index_count: half, material: 1 },
        ];
        let mut png = Vec::new();
        {
            // PNG 1×1 mínimo hecho con el codificador del motor.
            let img = maker_engine_png();
            png.extend_from_slice(&img);
        }
        data.materials = vec![
            ModelMaterial { name: "rojo".into(), base_color: [1.0, 0.0, 0.0, 1.0], ..Default::default() },
            ModelMaterial { name: "tex".into(), texture: Some(TextureSource::Embedded { key: "k".into(), bytes: Arc::new(png) }), alpha: AlphaMode::Mask(0.4), double_sided: true, ..Default::default() },
        ];
        let sc = Sculpt::from_model(&data).unwrap();
        assert_eq!(sc.vertex_count(), 8);
        let model = sc.to_model();
        assert_eq!(model.mesh.positions.len(), 24, "las costuras vuelven");
        let path = temp("mat");
        write_glb(&model, &path, "Cubo").unwrap();
        let back = ModelData::load(&path).unwrap();
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
        assert_eq!(back.submeshes.len(), 2);
        let m1 = &back.materials[back.submeshes.iter().find(|s| back.materials[s.material].name == "tex").unwrap().material];
        assert!(matches!(m1.texture, Some(TextureSource::Embedded { .. })));
        assert_eq!(m1.alpha, AlphaMode::Mask(0.4));
        assert!(m1.double_sided);
        assert!(back.materials.iter().any(|m| m.name == "rojo" && m.base_color == [1.0, 0.0, 0.0, 1.0]));
    }

    /// PNG gris de 2×2 escrito a mano (cabecera + IHDR + IDAT sin comprimir + IEND).
    fn maker_engine_png() -> Vec<u8> {
        fn crc(data: &[u8]) -> u32 {
            let mut c = 0xFFFF_FFFFu32;
            for &b in data {
                c ^= b as u32;
                for _ in 0..8 {
                    c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
                }
            }
            !c
        }
        fn adler(data: &[u8]) -> u32 {
            let (mut a, mut b) = (1u32, 0u32);
            for &x in data {
                a = (a + x as u32) % 65521;
                b = (b + a) % 65521;
            }
            b << 16 | a
        }
        let chunk = |out: &mut Vec<u8>, ty: &[u8], data: &[u8]| {
            out.extend_from_slice(&(data.len() as u32).to_be_bytes());
            let mut c = ty.to_vec();
            c.extend_from_slice(data);
            out.extend_from_slice(&c);
            out.extend_from_slice(&crc(&c).to_be_bytes());
        };
        let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
        let ihdr = [0, 0, 0, 2, 0, 0, 0, 2, 8, 0, 0, 0, 0];
        chunk(&mut out, b"IHDR", &ihdr);
        let raw = [0u8, 128, 128, 0, 128, 128];
        let mut z = vec![0x78, 0x01, 1];
        z.extend_from_slice(&(raw.len() as u16).to_le_bytes());
        z.extend_from_slice(&(!(raw.len() as u16)).to_le_bytes());
        z.extend_from_slice(&raw);
        z.extend_from_slice(&adler(&raw).to_be_bytes());
        chunk(&mut out, b"IDAT", &z);
        chunk(&mut out, b"IEND", &[]);
        out
    }
}
