//! Componentes de datos (ECS). Solo datos, sin lógica: la lógica vive en
//! sistemas (`transform`, `physics`, `scripting`) — la práctica de Unity DOTS,
//! Bevy y Unreal Mass; el árbol de entidades con padre/hijo se toma de Godot.

use glam::{EulerRot, Mat4, Quat, Vec3};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Identificador estable de entidad (persistido en la escena, usado por scripts).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct EntityId(pub u64);

/// Nombre visible en la jerarquía.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Name(pub String);

/// Padre en la jerarquía (por id estable).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Parent(pub EntityId);

/// Orden entre hermanos en la jerarquía.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
pub struct SiblingIndex(pub i64);

/// Entidad desactivada (no se dibuja ni ejecuta scripts; como `SetActive(false)`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Inactive;

/// Transform local respecto al padre.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Transform {
    pub translation: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
}

impl Default for Transform {
    fn default() -> Self {
        Self { translation: Vec3::ZERO, rotation: Quat::IDENTITY, scale: Vec3::ONE }
    }
}

impl Transform {
    pub fn from_translation(t: Vec3) -> Self {
        Self { translation: t, ..Default::default() }
    }
    pub fn with_scale(mut self, s: Vec3) -> Self {
        self.scale = s;
        self
    }
    pub fn with_rotation(mut self, r: Quat) -> Self {
        self.rotation = r;
        self
    }
    pub fn matrix(&self) -> Mat4 {
        Mat4::from_scale_rotation_translation(self.scale, self.rotation, self.translation)
    }
    pub fn from_matrix(m: Mat4) -> Self {
        let (scale, rotation, translation) = m.to_scale_rotation_translation();
        Self { translation, rotation, scale }
    }
    /// Euler en grados (orden YXZ, como el inspector de Unity).
    pub fn euler_degrees(&self) -> Vec3 {
        let (y, x, z) = self.rotation.to_euler(EulerRot::YXZ);
        // `+ 0.0` convierte -0.0 en 0.0 para el inspector.
        Vec3::new(x.to_degrees() + 0.0, y.to_degrees() + 0.0, z.to_degrees() + 0.0)
    }
    pub fn set_euler_degrees(&mut self, e: Vec3) {
        self.rotation = Quat::from_euler(EulerRot::YXZ, e.y.to_radians(), e.x.to_radians(), e.z.to_radians());
    }
    pub fn forward(&self) -> Vec3 {
        self.rotation * Vec3::NEG_Z
    }
}

/// Matriz de mundo calculada por el sistema de transform. No se guarda.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlobalTransform(pub Mat4);

impl Default for GlobalTransform {
    fn default() -> Self {
        Self(Mat4::IDENTITY)
    }
}

impl GlobalTransform {
    pub fn translation(&self) -> Vec3 {
        self.0.w_axis.truncate()
    }
}

/// Forma primitiva generada por el motor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PrimitiveShape {
    Cube,
    Sphere,
    Plane,
    Cylinder,
    Cone,
    Capsule,
    Torus,
}

impl PrimitiveShape {
    pub const ALL: [PrimitiveShape; 7] = [
        PrimitiveShape::Cube,
        PrimitiveShape::Sphere,
        PrimitiveShape::Plane,
        PrimitiveShape::Cylinder,
        PrimitiveShape::Cone,
        PrimitiveShape::Capsule,
        PrimitiveShape::Torus,
    ];
    pub fn label(self) -> &'static str {
        match self {
            PrimitiveShape::Cube => "Cubo",
            PrimitiveShape::Sphere => "Esfera",
            PrimitiveShape::Plane => "Plano",
            PrimitiveShape::Cylinder => "Cilindro",
            PrimitiveShape::Cone => "Cono",
            PrimitiveShape::Capsule => "Cápsula",
            PrimitiveShape::Torus => "Toro",
        }
    }
    pub fn from_api(code: u32) -> Option<Self> {
        Some(match code {
            0 => PrimitiveShape::Cube,
            1 => PrimitiveShape::Sphere,
            2 => PrimitiveShape::Plane,
            3 => PrimitiveShape::Cylinder,
            4 => PrimitiveShape::Cone,
            5 => PrimitiveShape::Capsule,
            _ => return None,
        })
    }
}

/// De dónde sale la malla.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MeshSource {
    Primitive(PrimitiveShape),
    /// Ruta relativa a la raíz del proyecto (glTF / GLB).
    Model(String),
}

/// Material PBR metal-rugosidad (el modelo de Unreal, Unity HDRP y Godot).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Material {
    /// Color base en sRGB (como lo ve el usuario) + alfa.
    pub base_color: [f32; 4],
    pub metallic: f32,
    pub roughness: f32,
    /// Emisión en sRGB.
    pub emissive: [f32; 3],
    pub emissive_strength: f32,
    /// Sin iluminación.
    #[serde(default)]
    pub unlit: bool,
}

impl Default for Material {
    fn default() -> Self {
        Self { base_color: [0.8, 0.8, 0.82, 1.0], metallic: 0.0, roughness: 0.55, emissive: [0.0; 3], emissive_strength: 0.0, unlit: false }
    }
}

impl Material {
    pub fn color(c: [f32; 3]) -> Self {
        Self { base_color: [c[0], c[1], c[2], 1.0], ..Default::default() }
    }
}

/// Dibuja una malla con un material.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MeshRenderer {
    pub mesh: MeshSource,
    pub material: Material,
    pub cast_shadows: bool,
}

impl MeshRenderer {
    pub fn primitive(shape: PrimitiveShape) -> Self {
        Self { mesh: MeshSource::Primitive(shape), material: Material::default(), cast_shadows: true }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LightKind {
    /// Luz direccional (sol). La primera proyecta sombras.
    Directional,
    /// Luz puntual con alcance.
    Point,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Light {
    pub kind: LightKind,
    /// sRGB.
    pub color: [f32; 3],
    pub intensity: f32,
    /// Solo puntual.
    pub range: f32,
    pub cast_shadows: bool,
}

impl Light {
    pub fn directional() -> Self {
        Self { kind: LightKind::Directional, color: [1.0, 0.96, 0.9], intensity: 3.0, range: 0.0, cast_shadows: true }
    }
    pub fn point() -> Self {
        Self { kind: LightKind::Point, color: [1.0, 0.85, 0.6], intensity: 6.0, range: 8.0, cast_shadows: false }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Camera {
    /// Campo de visión vertical en grados.
    pub fov_y: f32,
    pub near: f32,
    pub far: f32,
    /// La cámara principal es la que usa el juego.
    pub primary: bool,
}

impl Default for Camera {
    fn default() -> Self {
        Self { fov_y: 60.0, near: 0.1, far: 500.0, primary: true }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BodyKind {
    Dynamic,
    Fixed,
    Kinematic,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct RigidBody {
    pub kind: BodyKind,
    pub mass: f32,
    pub linear_damping: f32,
    pub angular_damping: f32,
    pub gravity_scale: f32,
    pub lock_rotation: bool,
    pub ccd: bool,
}

impl RigidBody {
    pub fn dynamic() -> Self {
        Self { kind: BodyKind::Dynamic, mass: 1.0, linear_damping: 0.05, angular_damping: 0.05, gravity_scale: 1.0, lock_rotation: false, ccd: false }
    }
    pub fn fixed() -> Self {
        Self { kind: BodyKind::Fixed, ..Self::dynamic() }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum ColliderShape {
    /// Medias extensiones (se multiplican por la escala).
    Box { half_extents: Vec3 },
    Sphere { radius: f32 },
    Capsule { half_height: f32, radius: f32 },
    Cylinder { half_height: f32, radius: f32 },
}

impl ColliderShape {
    pub fn label(&self) -> &'static str {
        match self {
            ColliderShape::Box { .. } => "Caja",
            ColliderShape::Sphere { .. } => "Esfera",
            ColliderShape::Capsule { .. } => "Cápsula",
            ColliderShape::Cylinder { .. } => "Cilindro",
        }
    }
    /// Forma que encaja con una primitiva de tamaño unidad.
    pub fn fit(shape: PrimitiveShape) -> Self {
        match shape {
            PrimitiveShape::Sphere => ColliderShape::Sphere { radius: 0.5 },
            PrimitiveShape::Plane => ColliderShape::Box { half_extents: Vec3::new(0.5, 0.005, 0.5) },
            PrimitiveShape::Cylinder | PrimitiveShape::Cone => ColliderShape::Cylinder { half_height: 0.5, radius: 0.5 },
            PrimitiveShape::Capsule => ColliderShape::Capsule { half_height: 0.5, radius: 0.5 },
            PrimitiveShape::Cube | PrimitiveShape::Torus => ColliderShape::Box { half_extents: Vec3::splat(0.5) },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Collider {
    pub shape: ColliderShape,
    pub friction: f32,
    pub restitution: f32,
    pub is_trigger: bool,
}

impl Collider {
    pub fn new(shape: ColliderShape) -> Self {
        Self { shape, friction: 0.6, restitution: 0.1, is_trigger: false }
    }
}

/// Valor de una propiedad exportada por un script, guardado en la escena.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum PropVal {
    Float(f32),
    Int(i32),
    Bool(bool),
    Vec3([f32; 3]),
    Color([f32; 4]),
}

impl PropVal {
    pub fn to_api(self) -> maker_api::PropValue {
        use maker_api::prop_kind::*;
        match self {
            PropVal::Float(f) => maker_api::PropValue { kind: FLOAT, v: [f, 0.0, 0.0, 0.0] },
            PropVal::Int(i) => maker_api::PropValue { kind: INT, v: [i as f32, 0.0, 0.0, 0.0] },
            PropVal::Bool(b) => maker_api::PropValue { kind: BOOL, v: [b as i32 as f32, 0.0, 0.0, 0.0] },
            PropVal::Vec3(v) => maker_api::PropValue { kind: VEC3, v: [v[0], v[1], v[2], 0.0] },
            PropVal::Color(c) => maker_api::PropValue { kind: COLOR, v: c },
        }
    }
    pub fn from_api(p: maker_api::PropValue) -> Option<Self> {
        use maker_api::prop_kind::*;
        Some(match p.kind {
            FLOAT => PropVal::Float(p.v[0]),
            INT => PropVal::Int(p.v[0].round() as i32),
            BOOL => PropVal::Bool(p.v[0] > 0.5),
            VEC3 => PropVal::Vec3([p.v[0], p.v[1], p.v[2]]),
            COLOR => PropVal::Color(p.v),
            _ => return None,
        })
    }
}

/// Un script adjunto: nombre del tipo y valores de propiedades sobrescritos.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct ScriptSlot {
    pub name: String,
    #[serde(default)]
    pub props: BTreeMap<String, PropVal>,
    #[serde(default = "yes")]
    pub enabled: bool,
}

fn yes() -> bool {
    true
}

/// Lista de scripts de la entidad (varios por entidad, como en Unity).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct Scripts(pub Vec<ScriptSlot>);
