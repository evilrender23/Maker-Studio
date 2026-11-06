//! Modelos glTF/GLB completos: submallas con su material (color base +
//! textura, alfa recortado o mezclado, doble cara), colores de vértice,
//! esqueleto (pieles) y clips de animación.
//!
//! Las mallas estáticas se hornean en el espacio del modelo (transformaciones
//! de los nodos aplicadas). Las mallas con piel quedan en el espacio de bind:
//! el sombreador de vértices las deforma con la paleta de huesos.

use glam::{Mat4, Quat, Vec2, Vec3, Vec4};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::mesh::{Aabb, MeshData};

/// Cómo trata el alfa un material (como `alphaMode` de glTF).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AlphaMode {
    Opaque,
    /// Recorte: se descartan los píxeles con alfa por debajo del umbral.
    Mask(f32),
    /// Mezcla (cristales): se dibuja tras los opacos, de atrás adelante.
    Blend,
}

/// Imagen de una textura.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum TextureSource {
    /// Archivo en disco (compartido entre modelos: la caché es por ruta).
    File(PathBuf),
    /// Imagen incrustada en el modelo. `key` identifica la imagen en la caché.
    Embedded { key: String, bytes: Arc<Vec<u8>> },
}

impl TextureSource {
    pub fn key(&self) -> String {
        match self {
            TextureSource::File(p) => p.to_string_lossy().into_owned(),
            TextureSource::Embedded { key, .. } => key.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ModelMaterial {
    pub name: String,
    /// Factor de color base en lineal + alfa.
    pub base_color: [f32; 4],
    pub texture: Option<TextureSource>,
    pub alpha: AlphaMode,
    pub double_sided: bool,
}

impl Default for ModelMaterial {
    fn default() -> Self {
        Self { name: String::new(), base_color: [1.0; 4], texture: None, alpha: AlphaMode::Opaque, double_sided: false }
    }
}

/// Rango de índices dibujado con un material.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SubMesh {
    pub first_index: u32,
    pub index_count: u32,
    pub material: usize,
}

/// Traslación, rotación y escala locales.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Trs {
    pub t: Vec3,
    pub r: Quat,
    pub s: Vec3,
}

impl Default for Trs {
    fn default() -> Self {
        Self { t: Vec3::ZERO, r: Quat::IDENTITY, s: Vec3::ONE }
    }
}

impl Trs {
    pub fn matrix(&self) -> Mat4 {
        Mat4::from_scale_rotation_translation(self.s, self.r, self.t)
    }
    pub fn lerp(&self, o: &Trs, k: f32) -> Trs {
        let r = if self.r.dot(o.r) < 0.0 { -o.r } else { o.r };
        Trs { t: self.t.lerp(o.t, k), r: self.r.lerp(r, k).normalize(), s: self.s.lerp(o.s, k) }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub name: String,
    /// Índice del padre; siempre menor que el del hijo.
    pub parent: Option<usize>,
    pub rest: Trs,
}

/// Nodos ordenados (padres antes que hijos) y la piel.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Skeleton {
    pub nodes: Vec<Node>,
    /// Paleta: índice de nodo de cada hueso.
    pub joints: Vec<usize>,
    pub inverse_bind: Vec<Mat4>,
}

impl Skeleton {
    pub fn find(&self, name: &str) -> Option<usize> {
        self.nodes.iter().position(|n| n.name == name)
    }
    /// Matrices de mundo (del modelo) de cada nodo para unas locales dadas.
    pub fn globals(&self, locals: &[Trs], out: &mut Vec<Mat4>) {
        out.clear();
        for (i, n) in self.nodes.iter().enumerate() {
            let l = locals.get(i).unwrap_or(&n.rest).matrix();
            let g = match n.parent {
                Some(p) => out[p] * l,
                None => l,
            };
            out.push(g);
        }
    }
    /// Paleta de huesos (global × bind inverso) a partir de matrices globales.
    pub fn palette(&self, globals: &[Mat4], out: &mut Vec<Mat4>) {
        out.clear();
        out.extend(self.joints.iter().zip(&self.inverse_bind).map(|(&j, ibm)| globals[j] * *ibm));
    }
    pub fn rest_locals(&self) -> Vec<Trs> {
        self.nodes.iter().map(|n| n.rest).collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChannelPath {
    Translation,
    Rotation,
    Scale,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Interpolation {
    Linear,
    Step,
    /// Se muestrea como lineal entre los valores de las claves.
    CubicSpline,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Channel {
    pub node: usize,
    pub path: ChannelPath,
    pub interpolation: Interpolation,
    pub times: Vec<f32>,
    /// xyz (+ w en rotaciones).
    pub values: Vec<Vec4>,
}

impl Channel {
    /// Valor en el instante `t` (fuera de rango se mantiene la clave extrema).
    pub fn sample(&self, t: f32) -> Vec4 {
        let n = self.times.len();
        if n == 0 {
            return Vec4::ZERO;
        }
        if n == 1 || t <= self.times[0] {
            return self.values[0];
        }
        if t >= self.times[n - 1] {
            return self.values[n - 1];
        }
        // Primera clave con tiempo > t.
        let hi = self.times.partition_point(|&k| k <= t);
        let lo = hi - 1;
        match self.interpolation {
            Interpolation::Step => self.values[lo],
            Interpolation::Linear | Interpolation::CubicSpline => {
                let (a, b) = (self.values[lo], self.values[hi]);
                let k = (t - self.times[lo]) / (self.times[hi] - self.times[lo]).max(1e-6);
                if self.path == ChannelPath::Rotation {
                    let qa = Quat::from_vec4(a);
                    let mut qb = Quat::from_vec4(b);
                    if qa.dot(qb) < 0.0 {
                        qb = -qb;
                    }
                    Vec4::from(qa.slerp(qb, k))
                } else {
                    a.lerp(b, k)
                }
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Clip {
    pub name: String,
    pub duration: f32,
    pub channels: Vec<Channel>,
}

impl Clip {
    /// Aplica el clip en `t` sobre unas locales (normalmente la pose de reposo).
    pub fn sample_into(&self, t: f32, locals: &mut [Trs]) {
        for c in &self.channels {
            let Some(l) = locals.get_mut(c.node) else { continue };
            let v = c.sample(t);
            match c.path {
                ChannelPath::Translation => l.t = v.truncate(),
                ChannelPath::Rotation => l.r = Quat::from_vec4(v).normalize(),
                ChannelPath::Scale => l.s = v.truncate(),
            }
        }
    }
}

/// Esqueleto y clips de un archivo: lo que necesita el sistema de animación.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AnimAsset {
    pub skeleton: Skeleton,
    pub clips: Vec<Clip>,
}

impl AnimAsset {
    pub fn clip(&self, name: &str) -> Option<usize> {
        self.clips.iter().position(|c| c.name == name)
    }
    pub fn has_skin(&self) -> bool {
        !self.skeleton.joints.is_empty()
    }
}

/// Un modelo listo para subir a GPU.
#[derive(Clone, Debug, Default)]
pub struct ModelData {
    /// Geometría combinada de todas las submallas (`colors`, `joints` y
    /// `weights` solo si alguna primitiva los trae).
    pub mesh: MeshData,
    pub submeshes: Vec<SubMesh>,
    pub materials: Vec<ModelMaterial>,
    pub anim: Arc<AnimAsset>,
}

impl ModelData {
    pub fn skinned(&self) -> bool {
        !self.mesh.joints.is_empty()
    }
    pub fn aabb(&self) -> Aabb {
        self.mesh.aabb()
    }
    pub fn triangles(&self) -> usize {
        self.mesh.indices.len() / 3
    }

    /// Una malla con un solo material blanco (primitivas).
    pub fn from_mesh(mesh: MeshData) -> ModelData {
        let n = mesh.indices.len() as u32;
        ModelData { mesh, submeshes: vec![SubMesh { first_index: 0, index_count: n, material: 0 }], materials: vec![ModelMaterial::default()], anim: Arc::default() }
    }

    /// Carga un glTF/GLB.
    pub fn load(path: &Path) -> Result<ModelData, String> {
        let err = |e: gltf::Error| format!("{}: {e}", path.display());
        let gltf = gltf::Gltf::open(path).map_err(err)?;
        let base = path.parent();
        let doc = gltf.document;
        let buffers = gltf::import_buffers(&doc, base, gltf.blob).map_err(err)?;
        let get = |b: gltf::Buffer| Some(&buffers[b.index()].0[..]);
        let scene = doc.default_scene().or_else(|| doc.scenes().next()).ok_or("el glTF no tiene escenas")?;

        // Orden de nodos: recorrido en profundidad desde la escena (padres antes).
        let mut order: Vec<(usize, Option<usize>)> = Vec::new();
        let mut stack: Vec<(gltf::Node, Option<usize>)> = scene.nodes().collect::<Vec<_>>().into_iter().rev().map(|n| (n, None)).collect();
        let mut remap: HashMap<usize, usize> = HashMap::new();
        while let Some((n, parent)) = stack.pop() {
            if remap.contains_key(&n.index()) {
                continue;
            }
            let idx = order.len();
            remap.insert(n.index(), idx);
            order.push((n.index(), parent));
            let children: Vec<_> = n.children().collect();
            for c in children.into_iter().rev() {
                stack.push((c, Some(idx)));
            }
        }
        let gnodes: Vec<gltf::Node> = doc.nodes().collect();
        let mut skeleton = Skeleton::default();
        for &(gi, parent) in &order {
            let n = &gnodes[gi];
            let (t, r, s) = n.transform().decomposed();
            skeleton.nodes.push(Node {
                name: n.name().unwrap_or("").to_string(),
                parent,
                rest: Trs { t: Vec3::from(t), r: Quat::from_array(r).normalize(), s: Vec3::from(s) },
            });
        }
        let mut rest_globals = Vec::new();
        skeleton.globals(&skeleton.rest_locals(), &mut rest_globals);

        // Pieles: todas en una paleta; cada una empieza en su desplazamiento.
        let mut skin_base: HashMap<usize, u16> = HashMap::new();
        for skin in doc.skins() {
            let base_idx = skeleton.joints.len() as u16;
            skin_base.insert(skin.index(), base_idx);
            let reader = skin.reader(get);
            let ibms: Vec<Mat4> = match reader.read_inverse_bind_matrices() {
                Some(m) => m.map(|c| Mat4::from_cols_array_2d(&c)).collect(),
                None => vec![Mat4::IDENTITY; skin.joints().count()],
            };
            for (j, ibm) in skin.joints().zip(ibms) {
                skeleton.joints.push(remap.get(&j.index()).copied().unwrap_or(0));
                skeleton.inverse_bind.push(ibm);
            }
        }

        // Materiales.
        let model_dir = base.map(Path::to_path_buf).unwrap_or_default();
        let mut materials: Vec<ModelMaterial> = Vec::new();
        for m in doc.materials() {
            let pbr = m.pbr_metallic_roughness();
            let texture = pbr.base_color_texture().and_then(|info| {
                let img = info.texture().source();
                match img.source() {
                    gltf::image::Source::Uri { uri, .. } => {
                        if let Some(data) = uri.strip_prefix("data:") {
                            let b64 = data.split_once(',').map(|(_, b)| b).unwrap_or("");
                            Some(TextureSource::Embedded { key: format!("{}#{}", path.display(), img.index()), bytes: Arc::new(base64_decode(b64)) })
                        } else {
                            Some(TextureSource::File(model_dir.join(percent_decode(uri))))
                        }
                    }
                    gltf::image::Source::View { view, .. } => {
                        let buf = &buffers[view.buffer().index()].0;
                        let bytes = buf.get(view.offset()..view.offset() + view.length())?.to_vec();
                        Some(TextureSource::Embedded { key: format!("{}#{}", path.display(), img.index()), bytes: Arc::new(bytes) })
                    }
                }
            });
            let alpha = match m.alpha_mode() {
                gltf::material::AlphaMode::Opaque => AlphaMode::Opaque,
                gltf::material::AlphaMode::Mask => AlphaMode::Mask(m.alpha_cutoff().unwrap_or(0.5)),
                gltf::material::AlphaMode::Blend => AlphaMode::Blend,
            };
            materials.push(ModelMaterial { name: m.name().unwrap_or("").to_string(), base_color: pbr.base_color_factor(), texture, alpha, double_sided: m.double_sided() });
        }
        let default_mat = materials.len();
        materials.push(ModelMaterial::default());

        // Primitivas agrupadas por material (menos dibujos).
        struct Part {
            mesh: MeshData,
            material: usize,
        }
        let mut parts: Vec<Part> = Vec::new();
        let (mut any_color, mut any_skin) = (false, false);
        for (ni, &(gi, _)) in order.iter().enumerate() {
            let node = &gnodes[gi];
            let Some(mesh) = node.mesh() else { continue };
            let skin = node.skin().map(|s| skin_base.get(&s.index()).copied().unwrap_or(0));
            for prim in mesh.primitives() {
                if prim.mode() != gltf::mesh::Mode::Triangles {
                    continue;
                }
                let reader = prim.reader(get);
                let Some(pos) = reader.read_positions() else { continue };
                let mut part = MeshData { positions: pos.map(Vec3::from).collect(), ..Default::default() };
                let n = part.positions.len();
                part.indices = match reader.read_indices() {
                    Some(i) => i.into_u32().collect(),
                    None => (0..n as u32).collect(),
                };
                match reader.read_normals() {
                    Some(nr) => part.normals = nr.map(Vec3::from).collect(),
                    None => part.compute_normals(),
                }
                part.uvs = match reader.read_tex_coords(0) {
                    Some(t) => t.into_f32().map(Vec2::from).collect(),
                    None => vec![Vec2::ZERO; n],
                };
                if let Some(c) = reader.read_colors(0) {
                    part.colors = c.into_rgba_u8().collect();
                    any_color = true;
                }
                match (skin, reader.read_joints(0), reader.read_weights(0)) {
                    (Some(base_j), Some(j), Some(w)) => {
                        part.joints = j.into_u16().map(|j| j.map(|x| x + base_j)).collect();
                        part.weights = w.into_f32().collect();
                        any_skin = true;
                    }
                    _ => {
                        // Estática: se hornea la transformación del nodo.
                        let m = rest_globals[ni];
                        if m != Mat4::IDENTITY {
                            let mut baked = MeshData::default();
                            baked.append(&part, m);
                            baked.colors = std::mem::take(&mut part.colors);
                            part = baked;
                        }
                    }
                }
                let material = prim.material().index().unwrap_or(default_mat);
                let fits = |p: &Part| p.material == material && p.mesh.joints.is_empty() == part.joints.is_empty();
                match parts.iter_mut().find(|p| fits(p)) {
                    Some(p) => merge(&mut p.mesh, part),
                    None => parts.push(Part { mesh: part, material }),
                }
            }
        }
        if parts.is_empty() {
            return Err(format!("{}: no hay triángulos", path.display()));
        }
        // Combinar todo en una malla con rangos por submalla.
        let mut out = ModelData { materials, ..Default::default() };
        for p in parts {
            let mut m = p.mesh;
            let n = m.positions.len();
            if any_color && m.colors.len() != n {
                m.colors = vec![[255; 4]; n];
            }
            if any_skin && m.joints.len() != n {
                // Malla estática dentro de un modelo con piel: sin huesos
                // influye el hueso 0 con peso 0 → se queda en bind (identidad).
                m.joints = vec![[0; 4]; n];
                m.weights = vec![[0.0; 4]; n];
            }
            let first = out.mesh.indices.len() as u32;
            let count = m.indices.len() as u32;
            merge(&mut out.mesh, m);
            out.submeshes.push(SubMesh { first_index: first, index_count: count, material: p.material });
        }

        // Animaciones.
        let mut clips = Vec::new();
        for anim in doc.animations() {
            let mut channels = Vec::new();
            let mut duration = 0.0f32;
            for ch in anim.channels() {
                let Some(&node) = remap.get(&ch.target().node().index()) else { continue };
                let path_ = match ch.target().property() {
                    gltf::animation::Property::Translation => ChannelPath::Translation,
                    gltf::animation::Property::Rotation => ChannelPath::Rotation,
                    gltf::animation::Property::Scale => ChannelPath::Scale,
                    gltf::animation::Property::MorphTargetWeights => continue,
                };
                let interpolation = match ch.sampler().interpolation() {
                    gltf::animation::Interpolation::Linear => Interpolation::Linear,
                    gltf::animation::Interpolation::Step => Interpolation::Step,
                    gltf::animation::Interpolation::CubicSpline => Interpolation::CubicSpline,
                };
                let r = ch.reader(get);
                let Some(times) = r.read_inputs() else { continue };
                let times: Vec<f32> = times.collect();
                let mut values: Vec<Vec4> = match r.read_outputs() {
                    Some(gltf::animation::util::ReadOutputs::Translations(t)) => t.map(|v| Vec3::from(v).extend(0.0)).collect(),
                    Some(gltf::animation::util::ReadOutputs::Scales(t)) => t.map(|v| Vec3::from(v).extend(0.0)).collect(),
                    Some(gltf::animation::util::ReadOutputs::Rotations(q)) => q.into_f32().map(Vec4::from).collect(),
                    _ => continue,
                };
                if interpolation == Interpolation::CubicSpline {
                    // [tangente de entrada, valor, tangente de salida] por clave.
                    values = values.chunks_exact(3).map(|c| c[1]).collect();
                }
                if values.len() != times.len() || times.is_empty() {
                    continue;
                }
                duration = duration.max(*times.last().unwrap());
                channels.push(Channel { node, path: path_, interpolation, times, values });
            }
            clips.push(Clip { name: anim.name().map(str::to_string).unwrap_or_else(|| format!("Clip {}", anim.index())), duration, channels });
        }
        out.anim = Arc::new(AnimAsset { skeleton, clips });
        Ok(out)
    }
}

fn merge(dst: &mut MeshData, src: MeshData) {
    let base = dst.positions.len() as u32;
    dst.positions.extend(src.positions);
    dst.normals.extend(src.normals);
    dst.uvs.extend(src.uvs);
    dst.colors.extend(src.colors);
    dst.joints.extend(src.joints);
    dst.weights.extend(src.weights);
    dst.indices.extend(src.indices.iter().map(|i| i + base));
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16)
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn base64_decode(s: &str) -> Vec<u8> {
    let val = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return None,
        } as u32)
    };
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0);
    for v in s.bytes().filter_map(val) {
        acc = acc << 6 | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ch(path: ChannelPath, interpolation: Interpolation, times: &[f32], values: &[Vec4]) -> Channel {
        Channel { node: 0, path, interpolation, times: times.to_vec(), values: values.to_vec() }
    }

    #[test]
    fn linear_and_step_sampling() {
        let c = ch(ChannelPath::Translation, Interpolation::Linear, &[0.0, 1.0, 3.0], &[Vec4::ZERO, Vec4::new(2.0, 0.0, 0.0, 0.0), Vec4::new(2.0, 4.0, 0.0, 0.0)]);
        assert_eq!(c.sample(-1.0), Vec4::ZERO);
        assert!((c.sample(0.5).x - 1.0).abs() < 1e-6);
        assert!((c.sample(2.0) - Vec4::new(2.0, 2.0, 0.0, 0.0)).length() < 1e-6);
        assert_eq!(c.sample(10.0), Vec4::new(2.0, 4.0, 0.0, 0.0));
        let s = Channel { interpolation: Interpolation::Step, ..c.clone() };
        assert_eq!(s.sample(0.99), Vec4::ZERO);
        assert_eq!(s.sample(1.0).x, 2.0);
        assert_eq!(s.sample(2.9).y, 0.0);
    }

    #[test]
    fn rotation_sampling_slerps_shortest_path() {
        let a = Quat::IDENTITY;
        let b = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
        // La segunda clave con el signo cambiado: misma rotación, otro hemisferio.
        let c = ch(ChannelPath::Rotation, Interpolation::Linear, &[0.0, 1.0], &[Vec4::from(a), -Vec4::from(b)]);
        let q = Quat::from_vec4(c.sample(0.5)).normalize();
        let want = Quat::from_rotation_y(std::f32::consts::FRAC_PI_4);
        assert!(q.dot(want).abs() > 0.9999, "{q:?}");
    }

    #[test]
    fn skin_palette_follows_animated_joint() {
        // Raíz → hueso a 1 m de altura. Bind inverso = inversa del reposo.
        let mut sk = Skeleton::default();
        sk.nodes.push(Node { name: "root".into(), parent: None, rest: Trs::default() });
        sk.nodes.push(Node { name: "bone".into(), parent: Some(0), rest: Trs { t: Vec3::Y, ..Default::default() } });
        sk.joints = vec![0, 1];
        let mut g = Vec::new();
        sk.globals(&sk.rest_locals(), &mut g);
        sk.inverse_bind = sk.joints.iter().map(|&j| g[j].inverse()).collect();
        // En reposo la paleta es la identidad.
        let mut pal = Vec::new();
        sk.palette(&g, &mut pal);
        for m in &pal {
            assert!(m.abs_diff_eq(Mat4::IDENTITY, 1e-5));
        }
        // Girar la raíz 90° en Z lleva un vértice en la punta del hueso (0,2,0) a (-2,0,0).
        let mut locals = sk.rest_locals();
        locals[0].r = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
        sk.globals(&locals, &mut g);
        sk.palette(&g, &mut pal);
        let p = pal[1].transform_point3(Vec3::new(0.0, 2.0, 0.0));
        assert!((p - Vec3::new(-2.0, 0.0, 0.0)).length() < 1e-5, "{p}");
    }

    #[test]
    fn decodes_uris() {
        assert_eq!(percent_decode("Mi%20Textura.png"), "Mi Textura.png");
        assert_eq!(base64_decode("aGVsbG8="), b"hello");
    }
}
