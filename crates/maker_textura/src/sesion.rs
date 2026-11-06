//! Una sesión de texturizado sobre un asset de Maker: abrirlo (con UV únicos opcionales y
//! partiendo de sus texturas), guardarla como `.tcraft` y aplicarla (texturas glTF + una copia
//! del modelo que las usa).

use crate::glb::Glb;
use crate::tc::{self, Session};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Carpeta del proyecto donde viven los assets texturizados.
pub const CARPETA: &str = "texturizados";
/// Preset de exportación de TextureCraft que lee el renderer de Maker.
pub const PRESET: &str = "glTF PBR Metal Roughness";
/// Extensiones de malla que se pueden texturizar.
pub const MALLAS: [&str; 12] = ["glb", "gltf", "fbx", "dae", "usd", "usda", "usdc", "usdz", "abc", "obj", "ply", "stl"];

/// Cómo abrir un asset.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Opciones {
    /// Lado de las texturas (potencia de dos, 128..8192).
    pub resolucion: u32,
    /// Desplegar los UV de nuevo (sin solapes). No en modelos con piel.
    pub uv_unicos: bool,
    /// Empezar con el color base y el normal map que ya tiene el asset (con sus UV).
    pub desde_original: bool,
}

impl Default for Opciones {
    fn default() -> Self {
        Self { resolucion: 2048, uv_unicos: false, desde_original: true }
    }
}

/// Lo que conviene saber de un asset antes de abrirlo.
#[derive(Clone, Debug, PartialEq)]
pub struct Analisis {
    pub triangulos: usize,
    pub materiales: usize,
    /// Fracción de texels de UV que se pisan (0..1).
    pub solape: f32,
    pub fuera_de_rango: usize,
    pub sin_uv: bool,
    /// Personaje con piel: no admite UV únicos (perdería el esqueleto).
    pub con_piel: bool,
}

impl Analisis {
    /// Más de un 5 % de solape: pintar repetiría la pincelada en otros sitios.
    pub fn solapado(&self) -> bool {
        self.solape > 0.05
    }
}

/// Un asset abierto en el modo Texturizado.
pub struct Textura {
    pub s: Session,
    /// `texturizados/<nombre>/` del proyecto: el `.tcraft`, el `.glb` y `texturas/`.
    pub carpeta: PathBuf,
    pub nombre: String,
    /// Asset del que salió (vacío al abrir un `.tcraft`).
    pub origen: Option<PathBuf>,
}

impl Textura {
    pub fn ruta_proyecto(&self) -> PathBuf {
        self.carpeta.join(format!("{}.tcraft", self.nombre))
    }
    pub fn ruta_modelo(&self) -> PathBuf {
        self.carpeta.join(format!("{}.glb", self.nombre))
    }
}

/// Nombre de archivo seguro a partir del de un asset.
pub fn nombre_de(path: &Path) -> String {
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("asset");
    let n: String = stem.chars().map(|c| if c.is_alphanumeric() || c == '_' || c == '-' { c } else { '_' }).collect();
    if n.is_empty() { "asset".into() } else { n }
}

pub fn carpeta_de(raiz: &Path, nombre: &str) -> PathBuf {
    raiz.join(CARPETA).join(nombre)
}

fn ext_de(path: &Path) -> String {
    path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase()
}

pub fn es_malla(path: &Path) -> bool {
    MALLAS.contains(&ext_de(path).as_str())
}

/// Los bytes de malla que recibe TextureCraft (un `.gltf` se convierte en GLB autocontenido)
/// y su nombre de archivo.
fn bytes_de_malla(path: &Path, nombre: &str) -> Result<(String, Vec<u8>, Option<Glb>), String> {
    match ext_de(path).as_str() {
        "glb" | "gltf" => {
            let g = Glb::load(path)?;
            Ok((format!("{nombre}.glb"), g.to_bytes(), Some(g)))
        }
        _ => {
            let f = tc::formats::read_mesh_file(path).map_err(|e| format!("{}: {e}", path.display()))?;
            Ok((f.name, f.bytes, None))
        }
    }
}

pub fn analizar(path: &Path) -> Result<Analisis, String> {
    let (archivo, bytes, glb) = bytes_de_malla(path, &nombre_de(path))?;
    let m = tc::formats::import_mesh_bytes(&archivo, &bytes).map_err(|e| e.to_string())?;
    let r = m.report;
    Ok(Analisis {
        triangulos: r.triangles,
        materiales: r.materials,
        solape: r.overlap_ratio,
        fuera_de_rango: r.uv_out_of_range,
        sin_uv: r.uvs_generated,
        con_piel: glb.is_some_and(|g| g.skinned()),
    })
}

/// Abre un asset (`.glb`, `.gltf`, FBX…) o un `.tcraft` ya guardado. `raiz` es la del proyecto.
pub fn abrir(path: &Path, raiz: &Path, op: Opciones) -> Result<Textura, String> {
    let nombre = nombre_de(path);
    if ext_de(path) == "tcraft" {
        let mut s = tc::file_cmds::open_path(&path.to_string_lossy()).map_err(|e| e.to_string())?;
        s.path = Some(path.to_string_lossy().into_owned());
        let carpeta = path.parent().map(Path::to_path_buf).unwrap_or_else(|| carpeta_de(raiz, &nombre));
        return Ok(Textura { s, carpeta, nombre, origen: None });
    }
    let res = tc::session::check_resolution(op.resolucion).map_err(|e| e.to_string())?;
    let (archivo, mut bytes, glb) = bytes_de_malla(path, &nombre)?;
    let con_piel = glb.as_ref().is_some_and(Glb::skinned);
    let unicos = op.uv_unicos && !con_piel;
    if unicos {
        let mut mesh = tc::formats::import_mesh_bytes(&archivo, &bytes).map_err(|e| e.to_string())?.mesh;
        tc::geom::unwrap::auto_unwrap(&mut mesh, 4.0, res).map_err(|e| e.to_string())?;
        bytes = tc::formats::glb_bytes(&mesh, &nombre, &[]).map_err(|e| e.to_string())?;
    }
    let archivo = if unicos { format!("{nombre}.glb") } else { archivo };
    let mut s = Session::from_mesh_bytes_tiled(&archivo, &bytes, res, &nombre, Some(false)).map_err(|e| e.to_string())?;
    let carpeta = carpeta_de(raiz, &nombre);
    s.path = Some(carpeta.join(format!("{nombre}.tcraft")).to_string_lossy().into_owned());
    if op.desde_original && !unicos {
        capas_originales(&mut s, path)?;
    }
    s.modified = true;
    Ok(Textura { s, carpeta, nombre, origen: Some(path.to_path_buf()) })
}

/// Una capa de relleno por texture set con el color base (textura o color) y el normal map del
/// material original, proyectados por sus UV: se empieza viendo el asset como era.
fn capas_originales(s: &mut Session, path: &Path) -> Result<(), String> {
    let modelo = maker_engine::model::ModelData::load(path)?;
    let leer = |t: &maker_engine::model::TextureSource| -> Option<(String, Vec<u8>)> {
        match t {
            maker_engine::model::TextureSource::File(p) => Some((p.file_name()?.to_string_lossy().into_owned(), std::fs::read(p).ok()?)),
            maker_engine::model::TextureSource::Embedded { key, bytes } => Some((format!("{}.png", nombre_de(Path::new(key)).replace('#', "_")), bytes.as_ref().clone())),
        }
    };
    for ts in 0..s.set_count() {
        let set = s.doc.project.texture_sets[ts].name.clone();
        let mm = modelo.materials.iter().enumerate().find(|(i, m)| m.name == set || (m.name.is_empty() && set == format!("Material_{i}"))).map(|(_, m)| m.clone());
        let Some(mm) = mm else { continue };
        tc::execute(s, "textureSet.select", &json!({ "ts": ts })).map_err(|e| e.to_string())?;
        let base = mm.texture.as_ref().and_then(leer);
        let normal = mm.normal.as_ref().and_then(leer);
        let capa = match base {
            Some((n, bytes)) => {
                let r = s.import_resource_bytes(&n, bytes).map_err(|e| e.to_string())?;
                tc::execute(s, "layer.add.image", &json!({ "name": r, "channel": "baseColor" })).map_err(|e| e.to_string())?
            }
            None => {
                let c = mm.base_color;
                tc::execute(s, "layer.add.fill", &json!({ "name": "Color original", "color": [c[0], c[1], c[2]] })).map_err(|e| e.to_string())?
            }
        };
        let id = capa["layer"].clone();
        tc::execute(s, "layer.rename", &json!({ "layer": id, "name": "Original" })).map_err(|e| e.to_string())?;
        if let Some((n, bytes)) = normal {
            let r = s.import_resource_bytes(&n, bytes).map_err(|e| e.to_string())?;
            tc::execute(s, "layer.fill.set", &json!({ "layer": id, "channel": "normal", "image": r })).map_err(|e| e.to_string())?;
        }
        // La capa de pintura vacía del proyecto nuevo, encima: se pinta sobre el original.
        if let Some(&top) = s.doc.project.texture_sets[ts].stack.iter().find(|l| s.doc.project.layers.get(l).is_some_and(|l| matches!(l.kind, tc::core::LayerKind::Paint))) {
            tc::execute(s, "layer.move", &json!({ "layer": top.0, "index": s.doc.project.texture_sets[ts].stack.len() - 1 })).map_err(|e| e.to_string())?;
            tc::execute(s, "layer.select", &json!({ "layer": top.0 })).map_err(|e| e.to_string())?;
        }
    }
    if s.set_count() > 0 {
        tc::execute(s, "textureSet.select", &json!({ "ts": 0 })).map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub fn guardar(t: &mut Textura) -> Result<PathBuf, String> {
    std::fs::create_dir_all(&t.carpeta).map_err(|e| format!("{}: {e}", t.carpeta.display()))?;
    let ruta = t.ruta_proyecto();
    let bytes = t.s.to_project_bytes().map_err(|e| e.to_string())?;
    // Escritura atómica: un fallo a medias no rompe el proyecto anterior.
    let tmp = ruta.with_extension("tcraft.tmp");
    std::fs::write(&tmp, bytes).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, &ruta).map_err(|e| format!("{}: {e}", ruta.display()))?;
    t.s.path = Some(ruta.to_string_lossy().into_owned());
    t.s.modified = false;
    Ok(ruta)
}

/// Mapas de un material en el GLB aplicado (rutas relativas a la carpeta del modelo).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Mapas {
    pub base: Option<String>,
    pub orm: Option<String>,
    pub normal: Option<String>,
    pub emision: Option<String>,
}

/// Guarda el `.tcraft`, exporta las texturas glTF de cada texture set a `texturas/` y escribe
/// `<nombre>.glb`: el modelo de partida (con su jerarquía, piel y animaciones) con los
/// materiales apuntando a esas texturas. Devuelve la ruta del `.glb`.
pub fn aplicar(t: &mut Textura) -> Result<PathBuf, String> {
    guardar(t)?;
    let dir_tex = t.carpeta.join("texturas");
    std::fs::create_dir_all(&dir_tex).map_err(|e| format!("{}: {e}", dir_tex.display()))?;
    let preset = tc::formats::builtin_preset(PRESET).ok_or("falta el preset glTF de TextureCraft")?;
    let opts = tc::formats::ExportOptions::new(preset);
    let mut por_set: BTreeMap<String, Mapas> = BTreeMap::new();
    for ts in 0..t.s.set_count() {
        let set = t.s.doc.project.texture_sets[ts].name.clone();
        let files = t.s.export_files(&[ts], &opts).map_err(|e| e.to_string())?;
        let mut m = Mapas::default();
        for f in files {
            let archivo = format!("{}_{}", t.nombre, f.name.trim_start_matches("mesh_"));
            std::fs::write(dir_tex.join(&archivo), &f.bytes).map_err(|e| format!("{archivo}: {e}"))?;
            let rel = format!("texturas/{archivo}");
            let stem = Path::new(&f.name).file_stem().and_then(|s| s.to_str()).unwrap_or("");
            if stem.ends_with("_BaseColor") {
                m.base = Some(rel);
            } else if stem.ends_with("_OcclusionRoughnessMetallic") {
                m.orm = Some(rel);
            } else if stem.ends_with("_Normal") {
                m.normal = Some(rel);
            } else if stem.ends_with("_Emissive") {
                m.emision = Some(rel);
            }
        }
        por_set.insert(set, m);
    }
    let base = match &t.s.mesh_source {
        Some((ext, bytes)) if ext == "glb" => Glb::parse(bytes)?,
        _ => Glb::parse(&t.s.mesh_glb_bytes().map_err(|e| e.to_string())?)?,
    };
    let g = con_materiales(base, &por_set);
    let ruta = t.ruta_modelo();
    std::fs::write(&ruta, g.to_bytes()).map_err(|e| format!("{}: {e}", ruta.display()))?;
    t.s.log.push(format!("Aplicado en {}", ruta.display()));
    Ok(ruta)
}

/// El GLB con los materiales de cada texture set (por nombre, como los importa TextureCraft)
/// apuntando a sus mapas. Las imágenes y texturas anteriores se descartan.
pub fn con_materiales(mut g: Glb, por_set: &BTreeMap<String, Mapas>) -> Glb {
    let j = &mut g.json;
    let mut images: Vec<Value> = Vec::new();
    let mut textures: Vec<Value> = Vec::new();
    let mut tex = |uri: &str| -> usize {
        images.push(json!({ "uri": uri }));
        textures.push(json!({ "source": images.len() - 1, "sampler": 0 }));
        textures.len() - 1
    };
    // Primitivas sin material: el texture set "Default".
    if por_set.contains_key("Default") {
        let n = j["materials"].as_array().map_or(0, Vec::len);
        let mut usado = false;
        for mesh in j["meshes"].as_array_mut().into_iter().flatten() {
            for p in mesh["primitives"].as_array_mut().into_iter().flatten() {
                if p.get("material").is_none() {
                    p["material"] = n.into();
                    usado = true;
                }
            }
        }
        if usado {
            if !j["materials"].is_array() {
                j["materials"] = json!([]);
            }
            if let Some(a) = j["materials"].as_array_mut() {
                a.push(json!({ "name": "Default" }));
            }
        }
    }
    let n = j["materials"].as_array().map_or(0, Vec::len);
    for i in 0..n {
        let nombre = j["materials"][i]["name"].as_str().map(str::to_string).unwrap_or_else(|| format!("Material_{i}"));
        let mat = &mut j["materials"][i];
        let Some(o) = mat.as_object_mut() else { continue };
        for k in ["normalTexture", "occlusionTexture", "emissiveTexture", "extensions"] {
            o.remove(k);
        }
        let mut pbr = o.get("pbrMetallicRoughness").cloned().unwrap_or_else(|| json!({}));
        if let Some(p) = pbr.as_object_mut() {
            p.remove("baseColorTexture");
            p.remove("metallicRoughnessTexture");
        }
        if let Some(m) = por_set.get(&nombre) {
            if let Some(b) = &m.base {
                pbr["baseColorTexture"] = json!({ "index": tex(b) });
                pbr["baseColorFactor"] = json!([1.0, 1.0, 1.0, 1.0]);
            }
            if let Some(orm) = &m.orm {
                let t = tex(orm);
                pbr["metallicRoughnessTexture"] = json!({ "index": t });
                pbr["metallicFactor"] = json!(1.0);
                pbr["roughnessFactor"] = json!(1.0);
                o.insert("occlusionTexture".into(), json!({ "index": t }));
            }
            if let Some(nm) = &m.normal {
                o.insert("normalTexture".into(), json!({ "index": tex(nm) }));
            }
            if let Some(e) = &m.emision {
                o.insert("emissiveTexture".into(), json!({ "index": tex(e) }));
                o.insert("emissiveFactor".into(), json!([1.0, 1.0, 1.0]));
            }
        }
        o.insert("pbrMetallicRoughness".into(), pbr);
    }
    if let Some(o) = j.as_object_mut() {
        o.insert("images".into(), Value::Array(images));
        o.insert("textures".into(), Value::Array(textures));
        o.insert("samplers".into(), json!([{ "magFilter": 9729, "minFilter": 9987, "wrapS": 10497, "wrapT": 10497 }]));
        for k in ["extensionsUsed", "extensionsRequired"] {
            if let Some(Value::Array(a)) = o.get_mut(k) {
                a.retain(|e| e.as_str().is_some_and(|e| !e.starts_with("KHR_materials_") && !e.starts_with("KHR_texture_") && !e.starts_with("EXT_texture_")));
            }
        }
    }
    g
}
