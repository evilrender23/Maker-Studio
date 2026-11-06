//! Renderizador de escena sobre wgpu (Vulkan / Metal / DX12 / GL).
//!
//! Dibuja a una textura fuera de pantalla (MSAA 4× resuelta a sRGB) que el
//! editor muestra en el viewport y el reproductor a pantalla completa.
//!
//! Cada modelo es un búfer de vértices con submallas; cada submalla tiene su
//! material (textura de color base con mipmaps, factor, recorte o mezcla alfa,
//! doble cara) y su normal map (del glTF, elegido en el material, generado a
//! un PNG o generado en memoria en un hilo aparte). Los pipelines se crean bajo demanda por combinación de modelo
//! de sombreado × piel × modo alfa × doble cara y los dibujos se ordenan por
//! pipeline y material. Las mallas con piel leen su paleta de huesos de un
//! búfer de almacenamiento que se sube una vez por fotograma.

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3, Vec4};
use maker_engine::biblioteca;
use maker_engine::components::*;
use maker_engine::math::{directional_light_view_proj, look_at};
use maker_engine::mesh::{Aabb, MeshData};
use maker_engine::model::{AlphaMode, ModelData, TextureSource};
use maker_engine::normal_gen;
use maker_engine::postproceso::Shade;
use maker_engine::scene::Scene;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender};
use wgpu::util::DeviceExt;

use crate::gi::{self, GiDebug, GiDraw, GiSystem};
use crate::postproceso::PostFx;
use crate::texture;

mod escultura;

pub const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
/// Los normal maps son datos, no color: sin conversión sRGB.
const NORMAL_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
pub const SAMPLES: u32 = 4;
const SHADOW_SIZE: u32 = 2048;
const OBJ_STRIDE: u64 = 256;
const MAX_POINTS: usize = 16;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Vertex {
    pos: [f32; 3],
    normal: [f32; 3],
    uv: [f32; 2],
    color: [u8; 4],
    /// Tangente xyz + signo de la bitangente, en snorm16.
    tangent: [i16; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SkinVertex {
    joints: [u16; 4],
    weights: [u16; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Default)]
struct PointU {
    pos_range: [f32; 4],
    color: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GlobalsU {
    view_proj: [[f32; 4]; 4],
    inv_view_proj: [[f32; 4]; 4],
    light_vp: [[f32; 4]; 4],
    cam_pos: [f32; 4],
    sun_dir: [f32; 4],
    sun_color: [f32; 4],
    sky_top: [f32; 4],
    sky_horizon: [f32; 4],
    ground: [f32; 4],
    fog: [f32; 4],
    params: [f32; 4],
    points: [PointU; MAX_POINTS],
    viewport: [f32; 4],
    /// x: GI activa, y: vista de depuración, z: intensidad indirecta, w: de reflejos.
    gi: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ObjectU {
    model: [[f32; 4]; 4],
    /// mat3x3 en WGSL: tres columnas de 16 B.
    normal_m: [[f32; 4]; 3],
    base_color: [f32; 4],
    emissive: [f32; 4],
    params: [f32; 4],
    // Modelos estilizados. Caben en el mismo paso de 256 B que ya usaba el
    // PBR (alineación del offset dinámico), así que no le cuestan nada.
    shadow_tint: [f32; 4],
    rim: [f32; 4],
    style: [f32; 4],
    outline: [f32; 4],
    /// x: intensidad del normal map, y: signo del verde, z: signo del determinante.
    nrm: [f32; 4],
    /// x: primer hueso de esta entidad en el búfer de huesos.
    skin: [u32; 4],
}

const _: () = assert!(std::mem::size_of::<ObjectU>() as u64 <= OBJ_STRIDE);

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct MaterialU {
    factor: [f32; 4],
    params: [f32; 4],
}

/// Grupo 3: x = escala del normal map (0 = no hay).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct NormalMapU {
    params: [f32; 4],
}

/// Normal map plano (sin relieve): el grupo 3 de quien no tiene.
const FLAT_NORMAL: usize = 0;

/// Normal map subido: vista + uniforme con su escala.
type NormalRes = (wgpu::TextureView, wgpu::Buffer);

/// Trabajo del modo automático: generar el normal map de una textura de color.
struct AutoJob {
    key: u64,
    src: TextureSource,
    params: NormalGen,
}

type AutoResult = (u64, Result<texture::MipChain, String>);

/// Qué normal maps tiene y le faltan a un modelo (para el Inspector).
#[derive(Clone, Debug, Default)]
pub struct NormalInfo {
    /// Submallas con textura de color.
    pub textured: usize,
    /// Submallas con normal map propio (glTF).
    pub with_normal: usize,
    /// Texturas de color sin normal map y dónde se escribiría el generado.
    pub missing: Vec<(TextureSource, PathBuf)>,
}

/// Modelo de sombreado de un dibujo (familia de pipelines).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Pipe {
    /// PBR y sin luz (rama uniforme dentro de `fs_main`).
    Standard,
    Toon,
    Hatching,
}

fn pipe_of(model: ShadingModel) -> Pipe {
    match model {
        ShadingModel::Pbr | ShadingModel::Unlit => Pipe::Standard,
        ShadingModel::Toon => Pipe::Toon,
        ShadingModel::Hatching => Pipe::Hatching,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Alpha {
    Opaque,
    Mask,
    Blend,
}

/// Variante de pipeline de color. El orden de los campos es el de dibujo.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct PipeKey {
    alpha: Alpha,
    pipe: Pipe,
    skinned: bool,
    double_sided: bool,
    /// Salida HDR (posprocesado) en vez de sRGB.
    hdr: bool,
}

/// Pipelines fijos (sin variantes de material); cada uno en sRGB y en HDR.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Fixed {
    Ink { skinned: bool },
    Outline { skinned: bool },
    Sky,
    Grid,
    /// Alambre de la malla esculpida (líneas).
    Wire,
}

fn target_format(hdr: bool) -> wgpu::TextureFormat {
    if hdr { crate::postproceso::HDR_FORMAT } else { COLOR_FORMAT }
}

/// Variante del mapa de sombras.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct ShadowKey {
    skinned: bool,
    mask: bool,
    double_sided: bool,
}

fn object_uniform(m: &Mat4, mat: &Material, sh: &Shade, skin_base: u32) -> ObjectU {
    let bc = lin3([mat.base_color[0], mat.base_color[1], mat.base_color[2]]);
    let st = sh.style;
    let outline = if sh.shading.has_outline() { st.outline_width.max(0.0) } else { 0.0 };
    let nm = glam::Mat3::from_mat4(*m).inverse().transpose();
    let col = |c: Vec3| [c.x, c.y, c.z, 0.0];
    let unlit = sh.shading == ShadingModel::Unlit;
    let n = &mat.normal;
    ObjectU {
        model: m.to_cols_array_2d(),
        normal_m: [col(nm.x_axis), col(nm.y_axis), col(nm.z_axis)],
        base_color: v4(bc, mat.base_color[3]),
        emissive: v4(lin3(mat.emissive), mat.emissive_strength),
        params: [sh.metallic, sh.roughness, if unlit { 1.0 } else { 0.0 }, st.hatch_scale],
        shadow_tint: v4(lin3(st.shadow_color), st.bands.clamp(2, 4) as f32),
        rim: v4(lin3(st.rim_color), st.rim_strength),
        style: [st.softness, st.specular, st.specular_size, st.rim_width],
        outline: v4(lin3(st.outline_color), outline),
        nrm: [
            if unlit { 0.0 } else { n.strength.clamp(0.0, 2.0) },
            if n.flip_green { -1.0 } else { 1.0 },
            if m.determinant() < 0.0 { -1.0 } else { 1.0 },
            0.0,
        ],
        skin: [skin_base, 0, 0, 0],
    }
}

/// Cámara para un fotograma.
#[derive(Clone, Copy, Debug)]
pub struct RenderView {
    pub view: Mat4,
    pub proj: Mat4,
    pub eye: Vec3,
}

impl RenderView {
    pub fn view_proj(&self) -> Mat4 {
        self.proj * self.view
    }
    /// Vista desde una matriz de mundo de cámara.
    pub fn from_camera(world: Mat4, cam: &Camera, aspect: f32) -> Self {
        let (_, rot, pos) = world.to_scale_rotation_translation();
        let view = look_at(pos, pos + rot * Vec3::NEG_Z, rot * Vec3::Y);
        let proj = maker_engine::math::perspective(cam.fov_y, aspect, cam.near, cam.far);
        Self { view, proj, eye: pos }
    }
}

#[derive(Clone, Debug, Default)]
pub struct RenderOptions {
    pub show_grid: bool,
    pub selected: HashSet<EntityId>,
    pub shadows: bool,
    /// Aplica la pila de posprocesado de la escena (las miniaturas no).
    pub post: bool,
    /// Permite la iluminación global de la escena (las miniaturas no).
    pub gi: bool,
    pub gi_debug: GiDebug,
    /// Alambre sobre la malla esculpida.
    pub sculpt_wire: bool,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct FrameStats {
    pub draw_calls: u32,
    pub triangles: u64,
    pub culled: u32,
    pub shadow_casters: u32,
    /// Cambios de pipeline en el pase de color (sin cielo, rejilla ni selección).
    pub pipeline_switches: u32,
    /// Contornos de casco invertido dibujados.
    pub outlines: u32,
    /// Pases de posprocesado (0 sin efectos: el tonemap va en la escena).
    pub post_passes: u32,
    /// La iluminación global se ejecutó en este fotograma.
    pub gi: bool,
    /// Cascadas del volumen de la GI revoxelizadas en este fotograma.
    pub gi_revoxelized: u32,
    /// Tiempo de GPU de la GI (ms; última medida, si hay consultas de tiempo).
    pub gi_ms: Option<f32>,
    /// Bytes de la malla esculpida subidos desde el fotograma anterior.
    pub sculpt_upload: u64,
}

pub(crate) struct GpuSub {
    pub(crate) first: u32,
    pub(crate) count: u32,
    mat: usize,
    alpha: Alpha,
    double_sided: bool,
    /// Normal map del glTF o `FLAT_NORMAL`.
    normal: usize,
    /// Textura de color sin normal map propio: de ella se generan.
    base: Option<TextureSource>,
    /// Dónde está su normal generado (`NormalSource::Generated`).
    gen_path: Option<String>,
    /// Hash de la clave de `base` (modo automático, sin reservas por fotograma).
    base_hash: u64,
}

pub(crate) struct GpuModel {
    pub(crate) vbuf: wgpu::Buffer,
    /// Normales suavizadas para el contorno de casco invertido (búfer aparte:
    /// el vértice del PBR no crece).
    smooth: wgpu::Buffer,
    /// Huesos y pesos (solo modelos con piel).
    pub(crate) skin: Option<wgpu::Buffer>,
    pub(crate) ibuf: wgpu::Buffer,
    pub(crate) subs: Vec<GpuSub>,
    aabb: Aabb,
    /// Huesos de la paleta (0 = estático).
    joints: u32,
    textured: usize,
    with_normal: usize,
}

struct Targets {
    size: (u32, u32),
    msaa: wgpu::TextureView,
    depth: wgpu::TextureView,
    resolve: wgpu::Texture,
    resolve_view: wgpu::TextureView,
}

#[derive(Clone, Copy)]
struct Draw {
    model: usize,
    sub: usize,
    key: PipeKey,
    mat: usize,
    /// Grupo 3 (normal map).
    nrm: usize,
    slot: u32,
    /// Distancia² a la cámara (orden de los transparentes).
    depth: f32,
    ink: bool,
    shadow: bool,
    visible: bool,
    selected: bool,
}

pub struct SceneRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    globals_buf: wgpu::Buffer,
    globals_bg: wgpu::BindGroup,
    shadow_globals_buf: wgpu::Buffer,
    shadow_globals_bg: wgpu::BindGroup,
    object_bgl: wgpu::BindGroupLayout,
    material_bgl: wgpu::BindGroupLayout,
    normal_bgl: wgpu::BindGroupLayout,
    object_buf: wgpu::Buffer,
    joint_buf: wgpu::Buffer,
    object_bg: wgpu::BindGroup,
    object_capacity: u64,
    joint_capacity: u64,
    module: wgpu::ShaderModule,
    main_layout: wgpu::PipelineLayout,
    /// Sin el grupo 3: contornos (casco invertido y selección).
    ink_layout: wgpu::PipelineLayout,
    shadow_layout: wgpu::PipelineLayout,
    pipelines: HashMap<PipeKey, wgpu::RenderPipeline>,
    shadow_pipelines: HashMap<ShadowKey, wgpu::RenderPipeline>,
    globals_only: wgpu::PipelineLayout,
    fixed: HashMap<(Fixed, bool), wgpu::RenderPipeline>,
    /// Posprocesado; se crea la primera vez que una escena lo usa.
    post: Option<PostFx>,
    /// Iluminación global; existe solo mientras está activa.
    pub(crate) gi: Option<GiSystem>,
    gi_supported: bool,
    gi_bgl: wgpu::BindGroupLayout,
    /// Grupo 3 vacío del sombreado cuando la GI está apagada.
    gi_dummy: wgpu::BindGroup,
    gi_draws: Vec<GiDraw>,
    shadow_view: wgpu::TextureView,
    sampler: wgpu::Sampler,
    targets: Option<Targets>,
    models: Vec<GpuModel>,
    model_index: HashMap<MeshSource, usize>,
    materials: Vec<wgpu::BindGroup>,
    /// Recursos de cada material (para combinarlo con su normal map en el grupo 2).
    material_res: Vec<(wgpu::TextureView, wgpu::Buffer)>,
    /// Grupo 2 de las pipelines principales por (material, normal map).
    matn: HashMap<(usize, usize), wgpu::BindGroup>,
    textures: HashMap<String, Option<Arc<wgpu::TextureView>>>,
    white: Arc<wgpu::TextureView>,
    /// Normal maps (vista + uniforme); el 0 es el plano.
    normal_bgs: Vec<NormalRes>,
    /// Normal maps por archivo (clave de la textura) → grupo 3 con escala 1.
    normal_index: HashMap<String, usize>,
    /// Modo automático: clave (textura + parámetros) → grupo 3 (plano hasta que esté).
    auto_normals: HashMap<u64, usize>,
    auto_tx: Option<Sender<AutoJob>>,
    auto_rx: Option<Receiver<AutoResult>>,
    auto_pending: usize,
    failed_models: HashSet<String>,
    /// Malla esculpida (dinámica) y el hueco de `models` que usa.
    sculpt: Option<escultura::SculptGpu>,
    sculpt_slot: Option<usize>,
    // Memoria reutilizada entre fotogramas (sin reservas por fotograma).
    draws: Vec<Draw>,
    order: Vec<usize>,
    object_bytes: Vec<u8>,
    joint_data: Vec<Mat4>,
    pub project_root: Option<PathBuf>,
    /// Incrementa cada vez que cambia la textura de salida (para re-registrarla).
    pub target_generation: u64,
    pub stats: FrameStats,
}

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}
pub fn lin3(c: [f32; 3]) -> [f32; 3] {
    [srgb_to_linear(c[0]), srgb_to_linear(c[1]), srgb_to_linear(c[2])]
}
fn v4(c: [f32; 3], w: f32) -> [f32; 4] {
    [c[0], c[1], c[2], w]
}

const VERTEX_ATTRS: [wgpu::VertexAttribute; 5] = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x2, 3 => Unorm8x4, 6 => Snorm16x4];
const SKIN_ATTRS: [wgpu::VertexAttribute; 2] = wgpu::vertex_attr_array![4 => Uint16x4, 5 => Unorm16x4];
const INK_POS_ATTRS: [wgpu::VertexAttribute; 1] = wgpu::vertex_attr_array![0 => Float32x3];
const INK_NORMAL_ATTRS: [wgpu::VertexAttribute; 1] = wgpu::vertex_attr_array![1 => Float32x3];

pub(crate) fn vertex_layout() -> wgpu::VertexBufferLayout<'static> {
    wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<Vertex>() as u64, step_mode: wgpu::VertexStepMode::Vertex, attributes: &VERTEX_ATTRS }
}
pub(crate) fn skin_layout() -> wgpu::VertexBufferLayout<'static> {
    wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<SkinVertex>() as u64, step_mode: wgpu::VertexStepMode::Vertex, attributes: &SKIN_ATTRS }
}

struct PipeDesc<'a> {
    label: &'a str,
    layout: &'a wgpu::PipelineLayout,
    vs: &'a str,
    fs: Option<&'a str>,
    constants: &'a [(&'a str, f64)],
    buffers: &'a [Option<wgpu::VertexBufferLayout<'a>>],
    cull: Option<wgpu::Face>,
    depth_write: bool,
    depth_cmp: wgpu::CompareFunction,
    bias: wgpu::DepthBiasState,
    samples: u32,
    format: wgpu::TextureFormat,
    color: Option<Option<wgpu::BlendState>>,
}

fn make_pipeline(device: &wgpu::Device, module: &wgpu::ShaderModule, d: PipeDesc) -> wgpu::RenderPipeline {
    let targets = [d.color.map(|blend| wgpu::ColorTargetState { format: d.format, blend, write_mask: wgpu::ColorWrites::ALL })];
    let targets: &[Option<wgpu::ColorTargetState>] = if d.color.is_some() { &targets } else { &[] };
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(d.label),
        layout: Some(d.layout),
        vertex: wgpu::VertexState { module, entry_point: Some(d.vs), compilation_options: Default::default(), buffers: d.buffers },
        primitive: wgpu::PrimitiveState { cull_mode: d.cull, ..Default::default() },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(d.depth_write),
            depth_compare: Some(d.depth_cmp),
            stencil: Default::default(),
            bias: d.bias,
        }),
        multisample: wgpu::MultisampleState { count: d.samples, mask: !0, alpha_to_coverage_enabled: false },
        fragment: d.fs.map(|fs| wgpu::FragmentState {
            module,
            entry_point: Some(fs),
            compilation_options: wgpu::PipelineCompilationOptions { constants: d.constants, ..Default::default() },
            targets,
        }),
        multiview_mask: None,
        cache: None,
    })
}

impl SceneRenderer {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let device = device.clone();
        let queue = queue.clone();
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("maker main"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders.wgsl").into()),
        });

        let uniform_entry = |binding: u32, dynamic: bool| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: dynamic, min_binding_size: None },
            count: None,
        };
        let globals_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("globals"),
            entries: &[
                uniform_entry(0, false),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison), count: None },
            ],
        });
        let shadow_globals_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("shadow globals"), entries: &[uniform_entry(0, false)] });
        let object_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("object"),
            entries: &[
                uniform_entry(0, true),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
            ],
        });
        let material_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("material"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
                uniform_entry(2, false),
            ],
        });

        // Grupo 2 de las pipelines principales: material (0..2) + normal map (3, 4). El grupo 3 es
        // de la iluminación global; las de sombra y contorno usan solo `material_bgl`.
        let normal_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("material + normal map"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
                uniform_entry(2, false),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
            ],
        });

        let globals_size = std::mem::size_of::<GlobalsU>() as u64;
        let globals_buf = device.create_buffer(&wgpu::BufferDescriptor { label: Some("globals"), size: globals_size, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let shadow_globals_buf =
            device.create_buffer(&wgpu::BufferDescriptor { label: Some("shadow globals"), size: globals_size, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });

        let shadow_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("shadow map"),
            size: wgpu::Extent3d { width: SHADOW_SIZE, height: SHADOW_SIZE, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let shadow_view = shadow_tex.create_view(&Default::default());
        let shadow_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("shadow sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        });
        // Trilineal + anisótropo ×16 para las texturas de color.
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("base color"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp: 16,
            ..Default::default()
        });
        let globals_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("globals"),
            layout: &globals_bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: globals_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&shadow_view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&shadow_sampler) },
            ],
        });
        let shadow_globals_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("shadow globals"),
            layout: &shadow_globals_bgl,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: shadow_globals_buf.as_entire_binding() }],
        });
        let (object_capacity, joint_capacity) = (256, 1024);
        let (object_buf, joint_buf, object_bg) = Self::make_object_buffers(&device, &object_bgl, object_capacity, joint_capacity);

        let gi_bgl = gi::shading_bgl(&device);
        let gi_supported = gi::soportada(&device);
        let gi_dummy = gi::shading_dummy(&device, &gi_bgl);
        let main_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("main"),
            bind_group_layouts: &[Some(&globals_bgl), Some(&object_bgl), Some(&normal_bgl), Some(&gi_bgl)],
            immediate_size: 0,
        });
        let ink_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ink"),
            bind_group_layouts: &[Some(&globals_bgl), Some(&object_bgl), Some(&material_bgl)],
            immediate_size: 0,
        });
        let globals_only = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("globals only"), bind_group_layouts: &[Some(&globals_bgl)], immediate_size: 0 });
        let shadow_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("shadow"),
            bind_group_layouts: &[Some(&shadow_globals_bgl), Some(&object_bgl), Some(&material_bgl)],
            immediate_size: 0,
        });

        let white = Arc::new(Self::upload_texture(&device, &queue, "white", texture::build_mips(1, 1, vec![255; 4], None), COLOR_FORMAT));
        let flat = Self::upload_texture(&device, &queue, "flat normal", texture::build_normal_mips(1, 1, vec![128, 128, 255, 255]), NORMAL_FORMAT);
        let flat_bg = Self::make_normal_bg(&device, &normal_bgl, &flat, 0.0);
        let mut r = Self {
            device,
            queue,
            globals_buf,
            globals_bg,
            shadow_globals_buf,
            shadow_globals_bg,
            object_bgl,
            material_bgl,
            normal_bgl,
            object_buf,
            joint_buf,
            object_bg,
            object_capacity,
            joint_capacity,
            module,
            main_layout,
            ink_layout,
            shadow_layout,
            pipelines: HashMap::new(),
            shadow_pipelines: HashMap::new(),
            globals_only,
            fixed: HashMap::new(),
            post: None,
            gi: None,
            gi_supported,
            gi_bgl,
            gi_dummy,
            gi_draws: Vec::new(),
            shadow_view,
            sampler,
            targets: None,
            models: Vec::new(),
            model_index: HashMap::new(),
            materials: Vec::new(),
            material_res: Vec::new(),
            matn: HashMap::new(),
            textures: HashMap::new(),
            white,
            normal_bgs: vec![flat_bg],
            normal_index: HashMap::new(),
            auto_normals: HashMap::new(),
            auto_tx: None,
            auto_rx: None,
            auto_pending: 0,
            failed_models: HashSet::new(),
            sculpt: None,
            sculpt_slot: None,
            draws: Vec::new(),
            order: Vec::new(),
            object_bytes: Vec::new(),
            joint_data: Vec::new(),
            project_root: None,
            target_generation: 0,
            stats: FrameStats::default(),
        };
        // Los pipelines más comunes, ya listos para el primer fotograma.
        for pipe in [Pipe::Standard, Pipe::Toon] {
            r.ensure_pipeline(PipeKey { alpha: Alpha::Opaque, pipe, skinned: false, double_sided: false, hdr: false });
        }
        r.ensure_fixed(Fixed::Sky, false);
        r.ensure_shadow_pipeline(ShadowKey { skinned: false, mask: false, double_sided: false });
        r
    }

    fn make_object_buffers(device: &wgpu::Device, bgl: &wgpu::BindGroupLayout, objects: u64, joints: u64) -> (wgpu::Buffer, wgpu::Buffer, wgpu::BindGroup) {
        let buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("objects"),
            size: objects * OBJ_STRIDE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let jbuf = device.create_buffer(&wgpu::BufferDescriptor { label: Some("joints"), size: joints * 64, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("objects"),
            layout: bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: &buf, offset: 0, size: wgpu::BufferSize::new(std::mem::size_of::<ObjectU>() as u64) }),
                },
                wgpu::BindGroupEntry { binding: 1, resource: jbuf.as_entire_binding() },
            ],
        });
        (buf, jbuf, bg)
    }

    fn ensure_pipeline(&mut self, key: PipeKey) {
        if self.pipelines.contains_key(&key) {
            return;
        }
        let fs = match key.pipe {
            Pipe::Standard => "fs_main",
            Pipe::Toon => "fs_toon",
            Pipe::Hatching => "fs_hatch",
        };
        let vb = [Some(vertex_layout())];
        let vb_skin = [Some(vertex_layout()), Some(skin_layout())];
        let constants = [("ALPHA_TEST", if key.alpha == Alpha::Mask { 1.0 } else { 0.0 }), ("BLEND", if key.alpha == Alpha::Blend { 1.0 } else { 0.0 })];
        let blend = key.alpha == Alpha::Blend;
        let p = make_pipeline(
            &self.device,
            &self.module,
            PipeDesc {
                label: &format!("{key:?}"),
                layout: &self.main_layout,
                vs: if key.skinned { "vs_main_skinned" } else { "vs_main" },
                fs: Some(fs),
                constants: &constants,
                buffers: if key.skinned { &vb_skin } else { &vb },
                cull: if key.double_sided { None } else { Some(wgpu::Face::Back) },
                depth_write: !blend,
                depth_cmp: wgpu::CompareFunction::Less,
                bias: Default::default(),
                samples: SAMPLES,
                format: target_format(key.hdr),
                color: Some(blend.then_some(wgpu::BlendState::ALPHA_BLENDING)),
            },
        );
        self.pipelines.insert(key, p);
    }

    fn ensure_fixed(&mut self, f: Fixed, hdr: bool) {
        if self.fixed.contains_key(&(f, hdr)) {
            return;
        }
        if f == Fixed::Wire {
            let p = self.wire_pipeline(hdr);
            self.fixed.insert((f, hdr), p);
            return;
        }
        // Casco invertido: posición del búfer de vértices normal + normales suavizadas (+ piel).
        let ink_pos = wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<Vertex>() as u64, step_mode: wgpu::VertexStepMode::Vertex, attributes: &INK_POS_ATTRS };
        let ink_n = wgpu::VertexBufferLayout { array_stride: 12, step_mode: wgpu::VertexStepMode::Vertex, attributes: &INK_NORMAL_ATTRS };
        let ink_static = [Some(ink_pos.clone()), Some(ink_n.clone())];
        let ink_skinned = [Some(ink_pos), Some(ink_n), Some(skin_layout())];
        let vb = [Some(vertex_layout())];
        let vb_skin = [Some(vertex_layout()), Some(skin_layout())];
        let (label, layout, vs, fs, buffers, cull, write, cmp, blend): (_, _, _, _, &[Option<wgpu::VertexBufferLayout>], _, _, _, _) = match f {
            Fixed::Ink { skinned: false } => ("ink outline", &self.ink_layout, "vs_ink", "fs_ink", &ink_static, Some(wgpu::Face::Front), true, wgpu::CompareFunction::Less, None),
            Fixed::Ink { skinned: true } => ("ink outline skinned", &self.ink_layout, "vs_ink_skinned", "fs_ink", &ink_skinned, Some(wgpu::Face::Front), true, wgpu::CompareFunction::Less, None),
            Fixed::Outline { skinned: false } => ("outline", &self.ink_layout, "vs_outline", "fs_outline", &vb, Some(wgpu::Face::Front), true, wgpu::CompareFunction::LessEqual, None),
            Fixed::Outline { skinned: true } => ("outline skinned", &self.ink_layout, "vs_outline_skinned", "fs_outline", &vb_skin, Some(wgpu::Face::Front), true, wgpu::CompareFunction::LessEqual, None),
            Fixed::Sky => ("sky", &self.globals_only, "vs_sky", "fs_sky", &[], None, false, wgpu::CompareFunction::Always, None),
            Fixed::Grid => ("grid", &self.globals_only, "vs_grid", "fs_grid", &[], None, false, wgpu::CompareFunction::LessEqual, Some(wgpu::BlendState::ALPHA_BLENDING)),
            Fixed::Wire => unreachable!(),
        };
        let p = make_pipeline(
            &self.device,
            &self.module,
            PipeDesc {
                label,
                layout,
                vs,
                fs: Some(fs),
                constants: &[],
                buffers,
                cull,
                depth_write: write,
                depth_cmp: cmp,
                bias: Default::default(),
                samples: SAMPLES,
                format: target_format(hdr),
                color: Some(blend),
            },
        );
        self.fixed.insert((f, hdr), p);
    }

    fn ensure_shadow_pipeline(&mut self, key: ShadowKey) {
        if self.shadow_pipelines.contains_key(&key) {
            return;
        }
        let vb = [Some(vertex_layout())];
        let vb_skin = [Some(vertex_layout()), Some(skin_layout())];
        let p = make_pipeline(
            &self.device,
            &self.module,
            PipeDesc {
                label: &format!("shadow {key:?}"),
                layout: &self.shadow_layout,
                vs: if key.skinned { "vs_shadow_skinned" } else { "vs_shadow" },
                fs: key.mask.then_some("fs_shadow_mask"),
                constants: &[],
                buffers: if key.skinned { &vb_skin } else { &vb },
                cull: if key.double_sided { None } else { Some(wgpu::Face::Back) },
                depth_write: true,
                depth_cmp: wgpu::CompareFunction::LessEqual,
                bias: wgpu::DepthBiasState { constant: 2, slope_scale: 2.0, clamp: 0.0 },
                samples: 1,
                format: COLOR_FORMAT,
                color: None,
            },
        );
        self.shadow_pipelines.insert(key, p);
    }

    fn upload_texture(device: &wgpu::Device, queue: &wgpu::Queue, label: &str, mips: texture::MipChain, format: wgpu::TextureFormat) -> wgpu::TextureView {
        let (w, h, _) = &mips.levels[0];
        let tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d { width: *w, height: *h, depth_or_array_layers: 1 },
            mip_level_count: mips.levels.len() as u32,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        for (level, (lw, lh, data)) in mips.levels.iter().enumerate() {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo { texture: &tex, mip_level: level as u32, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                data,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(lw * 4), rows_per_image: Some(*lh) },
                wgpu::Extent3d { width: *lw, height: *lh, depth_or_array_layers: 1 },
            );
        }
        tex.create_view(&Default::default())
    }

    /// Textura cacheada por ruta (o clave de imagen incrustada).
    fn texture(&mut self, src: &TextureSource, cutoff: Option<f32>) -> Arc<wgpu::TextureView> {
        let key = src.key();
        if let Some(t) = self.textures.get(&key) {
            return t.clone().unwrap_or_else(|| self.white.clone());
        }
        let bytes = match src {
            TextureSource::File(p) => std::fs::read(p).map_err(|e| format!("{}: {e}", p.display())),
            TextureSource::Embedded { bytes, .. } => Ok(bytes.as_ref().clone()),
        };
        let view = match bytes.and_then(|b| texture::decode(&b)) {
            Ok((w, h, px)) => Some(Arc::new(Self::upload_texture(&self.device, &self.queue, &key, texture::build_mips(w, h, px, cutoff), COLOR_FORMAT))),
            Err(e) => {
                maker_engine::log::error("render", format!("textura {key}: {e}"));
                None
            }
        };
        self.textures.insert(key, view.clone());
        view.unwrap_or_else(|| self.white.clone())
    }

    fn make_normal_bg(device: &wgpu::Device, _layout: &wgpu::BindGroupLayout, view: &wgpu::TextureView, scale: f32) -> NormalRes {
        let u = NormalMapU { params: [scale, 0.0, 0.0, 0.0] };
        let buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("normal map"), contents: bytemuck::bytes_of(&u), usage: wgpu::BufferUsages::UNIFORM });
        (view.clone(), buf)
    }

    /// Grupo 2 de las pipelines principales para (material, normal map), cacheado.
    fn ensure_matn(&mut self, mat: usize, nrm: usize) {
        if self.matn.contains_key(&(mat, nrm)) {
            return;
        }
        let (tex, mbuf) = &self.material_res[mat];
        let (ntex, nbuf) = &self.normal_bgs[nrm];
        let bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("material + normal map"),
            layout: &self.normal_bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(tex) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                wgpu::BindGroupEntry { binding: 2, resource: mbuf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(ntex) },
                wgpu::BindGroupEntry { binding: 4, resource: nbuf.as_entire_binding() },
            ],
        });
        self.matn.insert((mat, nrm), bg);
    }

    /// Lee y sube un normal map (lineal, mips renormalizados). Sin él, plano.
    fn load_normal_view(&self, src: &TextureSource) -> Option<wgpu::TextureView> {
        let bytes = match src {
            TextureSource::File(p) => std::fs::read(p).map_err(|e| format!("{}: {e}", p.display())),
            TextureSource::Embedded { bytes, .. } => Ok(bytes.as_ref().clone()),
        };
        match bytes.and_then(|b| texture::decode(&b)) {
            Ok((w, h, px)) => Some(Self::upload_texture(&self.device, &self.queue, &src.key(), texture::build_normal_mips(w, h, px), NORMAL_FORMAT)),
            Err(e) => {
                maker_engine::log::error("render", format!("normal map {}: {e}", src.key()));
                None
            }
        }
    }

    /// Grupo 3 de un normal map cacheado por clave (`key` + escala). Si no se
    /// puede leer queda el plano (y no se reintenta hasta `reload_normal_map`).
    fn normal_bg(&mut self, src: &TextureSource, scale: f32) -> usize {
        let key = if scale == 1.0 { src.key() } else { format!("{}@{scale}", src.key()) };
        if let Some(&i) = self.normal_index.get(&key) {
            return i;
        }
        let i = match self.load_normal_view(src) {
            Some(v) => {
                self.normal_bgs.push(Self::make_normal_bg(&self.device, &self.normal_bgl, &v, scale));
                self.normal_bgs.len() - 1
            }
            None => FLAT_NORMAL,
        };
        self.normal_index.insert(key, i);
        i
    }

    /// Normal map de un archivo del proyecto o `biblioteca://…`.
    fn normal_file(&mut self, path: &str) -> usize {
        if let Some(&i) = self.normal_index.get(path) {
            return i;
        }
        let full = biblioteca::resolve(path, self.project_root.as_deref());
        let i = self.normal_bg(&TextureSource::File(full), 1.0);
        self.normal_index.insert(path.to_string(), i);
        i
    }

    /// Vuelve a leer un normal map que ha cambiado en disco (p. ej. al
    /// regenerarlo en el editor). Los dibujos que lo usan lo ven en el
    /// siguiente fotograma.
    pub fn reload_normal_map(&mut self, path: &Path) {
        let key = path.to_string_lossy().into_owned();
        let Some(&i) = self.normal_index.get(&key) else { return };
        match self.load_normal_view(&TextureSource::File(path.to_path_buf())) {
            Some(v) => {
                let bg = Self::make_normal_bg(&self.device, &self.normal_bgl, &v, 1.0);
                if i == FLAT_NORMAL {
                    self.normal_bgs.push(bg);
                    let n = self.normal_bgs.len() - 1;
                    self.normal_index.insert(key, n);
                } else {
                    self.normal_bgs[i] = bg;
                    self.matn.retain(|k, _| k.1 != i);
                }
            }
            None => {
                self.normal_index.insert(key, FLAT_NORMAL);
            }
        }
    }

    /// Normal map en memoria (modo automático): lo pide al hilo de generación
    /// la primera vez; hasta que llega se dibuja plano.
    fn auto_normal(&mut self, key: u64, src: &TextureSource, params: NormalGen) -> usize {
        if let Some(&i) = self.auto_normals.get(&key) {
            return i;
        }
        self.auto_normals.insert(key, FLAT_NORMAL);
        if self.auto_tx.is_none() {
            let (tx, rx) = std::sync::mpsc::channel::<AutoJob>();
            let (done_tx, done_rx) = std::sync::mpsc::channel::<AutoResult>();
            let spawned = std::thread::Builder::new().name("normal maps".into()).spawn(move || {
                for job in rx {
                    let r = normal_gen::generate_from(&job.src, &job.params).map(|(w, h, px)| texture::build_normal_mips(w, h, px));
                    if done_tx.send((job.key, r)).is_err() {
                        break;
                    }
                }
            });
            if spawned.is_err() {
                return FLAT_NORMAL;
            }
            self.auto_tx = Some(tx);
            self.auto_rx = Some(done_rx);
        }
        if let Some(tx) = &self.auto_tx
            && tx.send(AutoJob { key, src: src.clone(), params }).is_ok()
        {
            self.auto_pending += 1;
        }
        FLAT_NORMAL
    }

    /// Recoge los normal maps automáticos ya generados y los sube.
    fn poll_auto_normals(&mut self) {
        let Some(rx) = &self.auto_rx else { return };
        let mut done: Vec<AutoResult> = Vec::new();
        while let Ok(r) = rx.try_recv() {
            done.push(r);
        }
        for (key, r) in done {
            self.auto_pending = self.auto_pending.saturating_sub(1);
            match r {
                Ok(mips) => {
                    let v = Self::upload_texture(&self.device, &self.queue, "auto normal map", mips, NORMAL_FORMAT);
                    self.normal_bgs.push(Self::make_normal_bg(&self.device, &self.normal_bgl, &v, 1.0));
                    self.auto_normals.insert(key, self.normal_bgs.len() - 1);
                }
                Err(e) => maker_engine::log::error("render", format!("normal map automático: {e}")),
            }
        }
    }

    /// Normal maps automáticos aún en el hilo de generación.
    pub fn pending_normal_maps(&self) -> usize {
        self.auto_pending
    }

    /// Espera a que el hilo termine los normal maps automáticos (miniaturas,
    /// capturas y tests). Devuelve si terminaron antes de `timeout`.
    pub fn wait_normal_maps(&mut self, timeout: std::time::Duration) -> bool {
        let t0 = std::time::Instant::now();
        while self.auto_pending > 0 && t0.elapsed() < timeout {
            std::thread::sleep(std::time::Duration::from_millis(5));
            self.poll_auto_normals();
        }
        self.auto_pending == 0
    }

    /// Normal maps de un modelo: cuántas submallas tienen textura de color,
    /// cuántas normal map propio, y qué texturas de color no lo tienen.
    pub fn normal_info(&mut self, src: &MeshSource) -> Option<NormalInfo> {
        let i = self.ensure_model(src)?;
        let m = &self.models[i];
        let mut missing: Vec<(TextureSource, PathBuf)> = Vec::new();
        for s in &m.subs {
            if let (Some(b), Some(p)) = (&s.base, &s.gen_path)
                && !missing.iter().any(|(x, _)| x == b)
            {
                missing.push((b.clone(), PathBuf::from(p)));
            }
        }
        Some(NormalInfo { textured: m.textured, with_normal: m.with_normal, missing })
    }

    /// Grupo 3 de una submalla según el normal map del material.
    fn sub_normal(&mut self, mi: usize, si: usize, n: &NormalMap) -> usize {
        let s = &self.models[mi].subs[si];
        match n.source {
            NormalSource::Model => s.normal,
            NormalSource::Texture => {
                if n.texture.is_empty() {
                    s.normal
                } else {
                    self.normal_file(&n.texture)
                }
            }
            NormalSource::Generated => match &s.gen_path {
                None => s.normal,
                Some(p) => match self.normal_index.get(p.as_str()) {
                    Some(&i) => i,
                    None => {
                        let p = p.clone();
                        let i = if Path::new(&p).is_file() { self.normal_bg(&TextureSource::File(PathBuf::from(&p)), 1.0) } else { FLAT_NORMAL };
                        self.normal_index.insert(p, i);
                        i
                    }
                },
            },
            NormalSource::Auto => {
                if s.base.is_none() {
                    return s.normal;
                }
                let mut key = s.base_hash;
                for b in n.generate.bits() {
                    key = (key ^ b as u64).wrapping_mul(0x0000_0100_0000_01b3);
                }
                if let Some(&i) = self.auto_normals.get(&key) {
                    return i;
                }
                let src = s.base.clone().unwrap();
                self.auto_normal(key, &src, n.generate)
            }
        }
    }

    fn material_bind_group(&mut self, factor: [f32; 4], cutoff: f32, tex: &wgpu::TextureView) -> usize {
        let u = MaterialU { factor, params: [cutoff, 0.0, 0.0, 0.0] };
        let buf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("material"), contents: bytemuck::bytes_of(&u), usage: wgpu::BufferUsages::UNIFORM });
        let bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("material"),
            layout: &self.material_bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(tex) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                wgpu::BindGroupEntry { binding: 2, resource: buf.as_entire_binding() },
            ],
        });
        self.materials.push(bg);
        self.material_res.push((tex.clone(), buf));
        self.materials.len() - 1
    }

    /// Textura de salida (sRGB, 1 muestra) para mostrar o copiar.
    pub fn output_view(&self) -> Option<&wgpu::TextureView> {
        self.targets.as_ref().map(|t| &t.resolve_view)
    }
    pub fn output_texture(&self) -> Option<&wgpu::Texture> {
        self.targets.as_ref().map(|t| &t.resolve)
    }
    pub fn output_size(&self) -> (u32, u32) {
        self.targets.as_ref().map(|t| t.size).unwrap_or((0, 0))
    }

    /// Posprocesado (se crea la primera vez).
    pub fn post_fx(&mut self) -> &mut PostFx {
        let (d, q) = (&self.device, &self.queue);
        self.post.get_or_insert_with(|| PostFx::new(d, q))
    }

    /// Dispositivo y cola de la GPU (para crear otro renderizador que los comparta).
    pub fn gpu(&self) -> (&wgpu::Device, &wgpu::Queue) {
        (&self.device, &self.queue)
    }

    pub fn resize(&mut self, w: u32, h: u32) {
        let (w, h) = (w.clamp(1, 8192), h.clamp(1, 8192));
        if self.targets.as_ref().is_some_and(|t| t.size == (w, h)) {
            return;
        }
        let size = wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 };
        let tex = |label: &str, format, samples, usage| {
            self.device.create_texture(&wgpu::TextureDescriptor { label: Some(label), size, mip_level_count: 1, sample_count: samples, dimension: wgpu::TextureDimension::D2, format, usage, view_formats: &[] })
        };
        let msaa = tex("msaa color", COLOR_FORMAT, SAMPLES, wgpu::TextureUsages::RENDER_ATTACHMENT).create_view(&Default::default());
        // También se lee en el posprocesado (contorno, niebla, desenfoque, efectos propios).
        let depth = tex("depth", DEPTH_FORMAT, SAMPLES, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING).create_view(&Default::default());
        let resolve = tex("scene output", COLOR_FORMAT, 1, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC);
        let resolve_view = resolve.create_view(&Default::default());
        self.targets = Some(Targets { size: (w, h), msaa, depth, resolve, resolve_view });
        self.target_generation += 1;
    }

    /// AABB local de una malla (la carga si hace falta). Para selección.
    pub fn mesh_aabb(&mut self, src: &MeshSource) -> Option<Aabb> {
        self.ensure_model(src).map(|i| self.models[i].aabb)
    }

    /// Olvida modelos y texturas cacheados (tras reimportar archivos).
    pub fn clear_model_cache(&mut self) {
        // La malla esculpida vive en `models`: quien esculpe la vuelve a subir (`sculpt_entity`).
        self.sculpt = None;
        self.sculpt_slot = None;
        self.models.clear();
        self.model_index.clear();
        self.materials.clear();
        self.material_res.clear();
        self.matn.clear();
        self.textures.clear();
        self.normal_bgs.truncate(1);
        self.normal_index.clear();
        self.auto_normals.clear();
        self.failed_models.clear();
        maker_engine::animation::clear_cache();
    }

    /// Índice del modelo en GPU (lo carga y sube la primera vez).
    fn ensure_model(&mut self, src: &MeshSource) -> Option<usize> {
        if let Some(&i) = self.model_index.get(src) {
            return Some(i);
        }
        let data = match src {
            MeshSource::Primitive(p) => ModelData::from_mesh(MeshData::primitive(*p)),
            MeshSource::Model(path) => {
                if self.failed_models.contains(path) {
                    return None;
                }
                let full = biblioteca::resolve(path, self.project_root.as_deref());
                match ModelData::load(&full) {
                    Ok(d) => {
                        maker_engine::log::info("render", format!("modelo cargado: {path} ({} triángulos, {} materiales)", d.triangles(), d.submeshes.len()));
                        maker_engine::animation::cache_asset(&full, d.anim.clone());
                        d
                    }
                    Err(e) => {
                        maker_engine::log::error("render", e);
                        self.failed_models.insert(path.clone());
                        return None;
                    }
                }
            }
        };
        let i = self.upload_model(&data, false);
        self.model_index.insert(src.clone(), i);
        Some(i)
    }

    /// Vuelve a cargar un modelo desde su archivo (tras reescribirlo), en el mismo hueco.
    pub fn reload_model(&mut self, src: &MeshSource) {
        if let MeshSource::Model(p) = src {
            self.failed_models.remove(p);
        }
        let Some(old) = self.model_index.remove(src) else { return };
        if let Some(new) = self.ensure_model(src) {
            self.models.swap(old, new);
            self.models.pop();
            self.model_index.insert(src.clone(), old);
        }
    }

    /// `dynamic`: el búfer de vértices admite escrituras parciales (malla esculpida).
    fn upload_model(&mut self, data: &ModelData, dynamic: bool) -> usize {
        let extra = if dynamic { wgpu::BufferUsages::COPY_DST } else { wgpu::BufferUsages::empty() };
        let m = &data.mesh;
        let verts: Vec<Vertex> = (0..m.positions.len())
            .map(|i| Vertex {
                pos: m.positions[i].to_array(),
                normal: m.normals.get(i).copied().unwrap_or(Vec3::Y).to_array(),
                uv: m.uvs.get(i).copied().unwrap_or_default().to_array(),
                color: m.colors.get(i).copied().unwrap_or([255; 4]),
                tangent: m.tangents.get(i).copied().unwrap_or(Vec4::new(1.0, 0.0, 0.0, 1.0)).to_array().map(|c| (c.clamp(-1.0, 1.0) * 32767.0).round() as i16),
            })
            .collect();
        let vbuf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("mesh vertices"), contents: bytemuck::cast_slice(&verts), usage: wgpu::BufferUsages::VERTEX | extra });
        let smooth: Vec<[f32; 3]> = m.smooth_normals().iter().map(|n| n.to_array()).collect();
        let smooth = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("mesh smooth normals"), contents: bytemuck::cast_slice(&smooth), usage: wgpu::BufferUsages::VERTEX | extra });
        let ibuf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("mesh indices"), contents: bytemuck::cast_slice(&m.indices), usage: wgpu::BufferUsages::INDEX });
        let skinned = data.skinned() && data.anim.has_skin();
        let skin = skinned.then(|| {
            let sv: Vec<SkinVertex> = m
                .joints
                .iter()
                .zip(&m.weights)
                .map(|(j, w)| SkinVertex { joints: *j, weights: w.map(|x| (x.clamp(0.0, 1.0) * 65535.0).round() as u16) })
                .collect();
            self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("mesh skin"), contents: bytemuck::cast_slice(&sv), usage: wgpu::BufferUsages::VERTEX })
        });
        let mut mat_bg: Vec<usize> = Vec::with_capacity(data.materials.len());
        let mut nrm_bg: Vec<usize> = Vec::with_capacity(data.materials.len());
        for mm in &data.materials {
            let cutoff = if let AlphaMode::Mask(c) = mm.alpha { Some(c) } else { None };
            let tex = match &mm.texture {
                Some(t) => self.texture(t, cutoff),
                None => self.white.clone(),
            };
            mat_bg.push(self.material_bind_group(mm.base_color, cutoff.unwrap_or(0.0), &tex));
            nrm_bg.push(match &mm.normal {
                Some(n) => self.normal_bg(n, mm.normal_scale),
                None => FLAT_NORMAL,
            });
        }
        let root = self.project_root.clone().unwrap_or_else(|| PathBuf::from("."));
        let (mut textured, mut with_normal) = (0, 0);
        let subs = data
            .submeshes
            .iter()
            .map(|s| {
                let mm = &data.materials[s.material];
                textured += mm.texture.is_some() as usize;
                with_normal += mm.normal.is_some() as usize;
                let base = mm.texture.clone().filter(|_| mm.normal.is_none());
                GpuSub {
                    first: s.first_index,
                    count: s.index_count,
                    mat: mat_bg[s.material],
                    alpha: match mm.alpha {
                        AlphaMode::Opaque => Alpha::Opaque,
                        AlphaMode::Mask(_) => Alpha::Mask,
                        AlphaMode::Blend => Alpha::Blend,
                    },
                    double_sided: mm.double_sided,
                    normal: nrm_bg[s.material],
                    gen_path: base.as_ref().map(|b| normal_gen::output_path(b, &root).to_string_lossy().into_owned()),
                    base_hash: base.as_ref().map(|b| normal_gen::hash_bytes(b.key().as_bytes())).unwrap_or(0),
                    base,
                }
            })
            .collect();
        let mut aabb = data.aabb();
        if skinned {
            // La animación puede sacar la malla de su caja de reposo.
            let c = aabb.center();
            let h = aabb.size() * 0.75;
            aabb = Aabb { min: c - h, max: c + h };
        }
        self.models.push(GpuModel { vbuf, smooth, skin, ibuf, subs, aabb, joints: if skinned { data.anim.skeleton.joints.len() as u32 } else { 0 }, textured, with_normal });
        self.models.len() - 1
    }

    /// Dibuja la escena en la textura de salida y envía los comandos a la GPU.
    pub fn render(&mut self, scene: &Scene, view: &RenderView, opts: &RenderOptions) {
        let Some((w, h)) = self.targets.as_ref().map(|t| t.size) else { return };
        self.poll_auto_normals();
        let mut stats = FrameStats::default();
        let view_proj = view.view_proj();

        // Entidades inactivas (y sus descendientes).
        let mut hidden: HashSet<EntityId> = HashSet::new();
        let any_inactive = scene.world.query::<&Inactive>().iter().next().is_some();
        if any_inactive {
            let hier = scene.hierarchy();
            for (id, _) in hier.depth_first() {
                if !scene.is_active(id) || scene.parent(id).is_some_and(|p| hidden.contains(&p)) {
                    hidden.insert(id);
                }
            }
        }

        // Luces.
        let mut sun: Option<(Vec3, Vec3, bool)> = None;
        let mut points: Vec<(f32, PointU)> = Vec::new();
        for (id, light, gt) in scene.world.query::<(&EntityId, &Light, &GlobalTransform)>().iter() {
            if hidden.contains(id) {
                continue;
            }
            let c = lin3(light.color);
            let col = Vec3::from(c) * light.intensity;
            match light.kind {
                LightKind::Directional => {
                    if sun.is_none() {
                        let dir = gt.0.transform_vector3(Vec3::NEG_Z).normalize_or(Vec3::NEG_Y);
                        sun = Some((dir, col, light.cast_shadows && opts.shadows));
                    }
                }
                LightKind::Point => {
                    let p = gt.translation();
                    points.push((p.distance_squared(view.eye), PointU { pos_range: v4(p.to_array(), light.range), color: v4(col.to_array(), 0.0) }));
                }
            }
        }
        points.sort_by(|a, b| a.0.total_cmp(&b.0));
        points.truncate(MAX_POINTS);
        let shadows_on = sun.is_some_and(|s| s.2);

        // Mapa de sombras centrado delante de la cámara, ajustado a texel.
        let shadow_radius = 22.0;
        let light_vp = if let Some((dir, _, true)) = sun {
            let fwd = view.view.inverse().transform_vector3(Vec3::NEG_Z);
            let focus = view.eye + fwd * shadow_radius * 0.6;
            let texel = 2.0 * shadow_radius / SHADOW_SIZE as f32;
            let up = if dir.y.abs() > 0.99 { Vec3::Z } else { Vec3::Y };
            let lview = look_at(Vec3::ZERO, dir, up);
            let mut lf = lview.transform_point3(focus);
            lf.x = (lf.x / texel).floor() * texel;
            lf.y = (lf.y / texel).floor() * texel;
            directional_light_view_proj(dir, lview.inverse().transform_point3(lf), shadow_radius)
        } else {
            Mat4::IDENTITY
        };

        let env = &scene.env;
        // Posprocesado: sin efectos activos, el tonemap va en el shader de la
        // escena y no hay pases extra (lo mismo que sin posprocesado).
        let root = self.project_root.clone();
        let plan = if opts.post && !env.post.is_empty() { Some(self.post_fx().plan(env, root.as_deref())) } else { None };
        let hdr = plan.as_ref().is_some_and(|p| !p.steps.is_empty());
        let (tonemap, exposure_ev) = match plan.as_ref().map(|p| p.scene_tonemap) {
            Some(Some((op, ev))) => (op.code(), ev),
            Some(None) => (3, 0.0),
            None => (0, 0.0),
        };
        // Iluminación global: solo si la escena la pide, la vista la permite y la GPU puede.
        let gi_on = opts.gi && env.iluminacion_global.activa() && self.gi_supported;
        if !gi_on {
            // Sin GI no se guarda nada en la GPU.
            self.gi = None;
        }
        let gi_u = if gi_on {
            let g = &env.iluminacion_global;
            [1.0, opts.gi_debug.code(), g.intensidad_indirecta.max(0.0), g.intensidad_reflejos.max(0.0)]
        } else {
            [0.0; 4]
        };
        let mut pts = [PointU::default(); MAX_POINTS];
        for (i, (_, p)) in points.iter().enumerate() {
            pts[i] = *p;
        }
        let mut globals = GlobalsU {
            view_proj: view_proj.to_cols_array_2d(),
            inv_view_proj: view_proj.inverse().to_cols_array_2d(),
            light_vp: light_vp.to_cols_array_2d(),
            cam_pos: v4(view.eye.to_array(), 1.0),
            sun_dir: sun.map(|s| v4(s.0.to_array(), 1.0)).unwrap_or([0.0, -1.0, 0.0, 0.0]),
            sun_color: sun.map(|s| v4(s.1.to_array(), if s.2 { 1.0 } else { 0.0 })).unwrap_or([0.0; 4]),
            sky_top: v4(lin3(env.sky_top), 0.0),
            sky_horizon: v4(lin3(env.sky_horizon), 0.0),
            ground: v4(lin3(env.ground), env.ambient_intensity),
            fog: v4(lin3(env.fog_color), env.fog_density),
            params: [points.len() as f32, env.exposure * 2f32.powf(exposure_ev), 0.0, SHADOW_SIZE as f32],
            points: pts,
            viewport: [w as f32, h as f32, tonemap as f32, 0.0],
            gi: gi_u,
        };
        self.queue.write_buffer(&self.globals_buf, 0, bytemuck::bytes_of(&globals));
        // El pase de sombras usa el mismo Globals con la matriz de la luz como cámara.
        globals.view_proj = light_vp.to_cols_array_2d();
        self.queue.write_buffer(&self.shadow_globals_buf, 0, bytemuck::bytes_of(&globals));

        // Lista de dibujo + uniformes por objeto + paletas de huesos.
        let mut draws = std::mem::take(&mut self.draws);
        let mut order = std::mem::take(&mut self.order);
        let mut bytes = std::mem::take(&mut self.object_bytes);
        let mut joints = std::mem::take(&mut self.joint_data);
        draws.clear();
        order.clear();
        bytes.clear();
        joints.clear();
        let mut slot = 0u32;
        for (id, mr, gt, pose) in scene.world.query::<(&EntityId, &MeshRenderer, &GlobalTransform, Option<&SkinPose>)>().iter() {
            if hidden.contains(id) {
                continue;
            }
            let dynamic = self.sculpt.as_ref().filter(|s| s.entity == *id).map(|s| s.model);
            let Some(mi) = dynamic.or_else(|| self.ensure_model(&mr.mesh)) else { continue };
            let m = gt.0;
            let model = &self.models[mi];
            let wb = model.aabb.transformed(&m);
            let visible = !outside_frustum(&view_proj, &wb);
            if !visible {
                stats.culled += 1;
            }
            // Con GI también cuenta lo que queda fuera de la vista (rebota luz).
            if !visible && !(mr.cast_shadows && shadows_on) && !gi_on {
                continue;
            }
            // Paleta: una vez por entidad y fotograma, compartida por sus submallas.
            let skin_base = joints.len() as u32;
            if model.joints > 0 {
                match pose.filter(|p| p.palette.len() == model.joints as usize) {
                    Some(p) => joints.extend_from_slice(&p.palette),
                    None => joints.extend(std::iter::repeat_n(Mat4::IDENTITY, model.joints as usize)),
                }
            }
            let sh = env.global_shading.resolve(&mr.material);
            bytes.extend_from_slice(bytemuck::bytes_of(&object_uniform(&m, &mr.material, &sh, skin_base)));
            bytes.resize((slot as usize + 1) * OBJ_STRIDE as usize, 0);
            let ink = sh.shading.has_outline() && sh.style.outline_width > 0.0;
            let depth = wb.center().distance_squared(view.eye);
            let selected = opts.selected.contains(id);
            let (joints_n, n_subs) = (model.joints, model.subs.len());
            let lit = sh.shading != ShadingModel::Unlit && mr.material.normal.strength > 0.0;
            for si in 0..n_subs {
                // Sin luz no se usa: todos con el plano (menos cambios de grupo).
                let nrm = if lit && visible { self.sub_normal(mi, si, &mr.material.normal) } else { FLAT_NORMAL };
                let s = &self.models[mi].subs[si];
                let key = PipeKey { alpha: s.alpha, pipe: pipe_of(sh.shading), skinned: joints_n > 0, double_sided: s.double_sided, hdr };
                draws.push(Draw {
                    model: mi,
                    sub: si,
                    key,
                    mat: s.mat,
                    nrm,
                    slot,
                    depth,
                    ink: ink && s.alpha == Alpha::Opaque,
                    shadow: mr.cast_shadows && s.alpha != Alpha::Blend,
                    visible,
                    selected,
                });
            }
            slot += 1;
        }
        // Orden: opacos y recortes por pipeline y material; transparentes de atrás adelante.
        order.extend((0..draws.len()).filter(|&i| draws[i].visible));
        order.sort_by(|&a, &b| {
            let (da, db) = (&draws[a], &draws[b]);
            da.key.alpha.cmp(&db.key.alpha).then_with(|| {
                if da.key.alpha == Alpha::Blend {
                    db.depth.total_cmp(&da.depth)
                } else {
                    da.key.cmp(&db.key).then(da.mat.cmp(&db.mat)).then(da.nrm.cmp(&db.nrm)).then(da.model.cmp(&db.model))
                }
            })
        });
        self.ensure_fixed(Fixed::Sky, hdr);
        if opts.show_grid {
            self.ensure_fixed(Fixed::Grid, hdr);
        }
        let wire = if opts.sculpt_wire {
            self.sculpt.as_ref().and_then(|s| s.wire.as_ref().map(|_| s.model)).and_then(|m| draws.iter().find(|d| d.model == m && d.visible).map(|d| (d.slot, d.mat)))
        } else {
            None
        };
        if wire.is_some() {
            self.ensure_fixed(Fixed::Wire, hdr);
        }
        for d in &draws {
            if d.visible {
                self.ensure_matn(d.mat, d.nrm);
                self.ensure_pipeline(d.key);
                if d.ink {
                    self.ensure_fixed(Fixed::Ink { skinned: d.key.skinned }, hdr);
                }
                if d.selected {
                    self.ensure_fixed(Fixed::Outline { skinned: d.key.skinned }, hdr);
                }
            }
            if d.shadow && shadows_on {
                self.ensure_shadow_pipeline(ShadowKey { skinned: d.key.skinned, mask: d.key.alpha == Alpha::Mask, double_sided: d.key.double_sided });
            }
        }
        // Lo que ve la GI: opacos y recortes, con su caja y su huella.
        let mut gi_draws = std::mem::take(&mut self.gi_draws);
        gi_draws.clear();
        if gi_on {
            for d in draws.iter().filter(|d| d.key.alpha != Alpha::Blend) {
                let m = &self.models[d.model];
                let o = d.slot as usize * OBJ_STRIDE as usize;
                let obj: &ObjectU = bytemuck::from_bytes(&bytes[o..o + std::mem::size_of::<ObjectU>()]);
                let wb = m.aabb.transformed(&Mat4::from_cols_array_2d(&obj.model));
                // Con piel, la paleta de huesos también cuenta (se revoxeliza al animar).
                let mut semilla = ((d.model as u64) << 32) ^ ((d.sub as u64) << 16) ^ d.mat as u64;
                if let Some(s) = self.sculpt.as_ref().filter(|s| s.model == d.model) {
                    // La malla esculpida cambia sin cambiar de modelo: se revoxeliza al esculpir.
                    semilla = gi::huella(&s.version.to_le_bytes(), semilla);
                }
                if d.key.skinned {
                    let j0 = obj.skin[0] as usize;
                    let pal = joints.get(j0..j0 + m.joints as usize).unwrap_or(&[]);
                    semilla = gi::huella(bytemuck::cast_slice(pal), semilla);
                }
                gi_draws.push(GiDraw {
                    model: d.model,
                    sub: d.sub,
                    slot: d.slot,
                    mat: d.mat,
                    skinned: d.key.skinned,
                    mask: d.key.alpha == Alpha::Mask,
                    double_sided: d.key.double_sided,
                    visible: d.visible,
                    min: wb.min,
                    max: wb.max,
                    huella: gi::huella(&bytes[o..o + std::mem::size_of::<ObjectU>()], semilla),
                });
            }
        }
        let need_objects = slot as u64;
        let need_joints = joints.len() as u64;
        if need_objects > self.object_capacity || need_joints > self.joint_capacity {
            self.object_capacity = self.object_capacity.max(need_objects.next_power_of_two());
            self.joint_capacity = self.joint_capacity.max(need_joints.next_power_of_two());
            let (b, j, bg) = Self::make_object_buffers(&self.device, &self.object_bgl, self.object_capacity, self.joint_capacity);
            self.object_buf = b;
            self.joint_buf = j;
            self.object_bg = bg;
        }
        if !bytes.is_empty() {
            self.queue.write_buffer(&self.object_buf, 0, &bytes);
        }
        if !joints.is_empty() {
            self.queue.write_buffer(&self.joint_buf, 0, bytemuck::cast_slice(&joints));
        }

        let mut enc = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("scene") });
        // Sombras.
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("shadow pass"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.shadow_view,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if shadows_on {
                pass.set_bind_group(0, &self.shadow_globals_bg, &[]);
                let mut bound: Option<ShadowKey> = None;
                let mut last_slot = u32::MAX;
                for d in draws.iter().filter(|d| d.shadow) {
                    let key = ShadowKey { skinned: d.key.skinned, mask: d.key.alpha == Alpha::Mask, double_sided: d.key.double_sided };
                    if bound != Some(key) {
                        pass.set_pipeline(&self.shadow_pipelines[&key]);
                        bound = Some(key);
                    }
                    let m = &self.models[d.model];
                    let s = &m.subs[d.sub];
                    if d.slot != last_slot {
                        pass.set_bind_group(1, &self.object_bg, &[(d.slot as u64 * OBJ_STRIDE) as u32]);
                        last_slot = d.slot;
                    }
                    pass.set_bind_group(2, &self.materials[d.mat], &[]);
                    pass.set_vertex_buffer(0, m.vbuf.slice(..));
                    if let Some(sk) = &m.skin {
                        pass.set_vertex_buffer(1, sk.slice(..));
                    }
                    pass.set_index_buffer(m.ibuf.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(s.first..s.first + s.count, 0, 0..1);
                    stats.shadow_casters += 1;
                }
            }
        }
        // Iluminación global: tras las sombras (la inyección las usa), antes del color.
        if gi_on {
            if self.gi.is_none() {
                self.gi = Some(GiSystem::new(&self.device, &self.queue, &self.object_bgl, &self.material_bgl, &self.gi_bgl, &self.globals_buf, &self.shadow_view));
            }
            let frame = gi::Frame {
                draws: &gi_draws,
                models: &self.models,
                materials: &self.materials,
                object_bg: &self.object_bg,
                obj_stride: OBJ_STRIDE,
                view,
                env,
                size: (w, h),
                debug: opts.gi_debug,
            };
            if let Some(g) = self.gi.as_mut() {
                g.encode(&mut enc, &frame, &env.iluminacion_global);
                stats.gi = true;
                stats.gi_revoxelized = g.revoxelizadas;
            }
        }
        // Color: directo a sRGB, o a HDR si hay efectos de posprocesado.
        if hdr {
            let (size, depth) = self.targets.as_ref().map(|t| (t.size, t.depth.clone())).unwrap();
            self.post_fx().ensure_targets(size, SAMPLES, &depth);
        }
        {
            let t = self.targets.as_ref().unwrap();
            let (color, resolve) = match self.post.as_ref().filter(|_| hdr).and_then(|p| p.scene_attachment()) {
                Some(a) => a,
                None => (&t.msaa, &t.resolve_view),
            };
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("main pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: color,
                    depth_slice: None,
                    resolve_target: Some(resolve),
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Discard },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &t.depth,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: if hdr { wgpu::StoreOp::Store } else { wgpu::StoreOp::Discard } }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &self.globals_bg, &[]);
            pass.set_bind_group(3, self.gi.as_ref().and_then(|g| g.shading_bg(opts.gi_debug)).unwrap_or(&self.gi_dummy), &[]);
            pass.set_pipeline(&self.fixed[&(Fixed::Sky, hdr)]);
            pass.draw(0..3, 0..1);
            let mut bound: Option<PipeKey> = None;
            // Los transparentes van tras los contornos (que sí escriben profundidad).
            let split = order.iter().position(|&i| draws[i].key.alpha == Alpha::Blend).unwrap_or(order.len());
            let draw_list = |pass: &mut wgpu::RenderPass, list: &[usize], stats: &mut FrameStats, bound: &mut Option<PipeKey>, last: &mut (u32, usize, usize, usize)| {
                for &i in list {
                    let d = &draws[i];
                    if *bound != Some(d.key) {
                        pass.set_pipeline(&self.pipelines[&d.key]);
                        *bound = Some(d.key);
                        stats.pipeline_switches += 1;
                    }
                    let m = &self.models[d.model];
                    let s = &m.subs[d.sub];
                    if d.slot != last.0 {
                        pass.set_bind_group(1, &self.object_bg, &[(d.slot as u64 * OBJ_STRIDE) as u32]);
                        last.0 = d.slot;
                    }
                    if d.mat != last.1 || d.nrm != last.3 {
                        pass.set_bind_group(2, &self.matn[&(d.mat, d.nrm)], &[]);
                        last.1 = d.mat;
                        last.3 = d.nrm;
                    }
                    if d.model != last.2 {
                        pass.set_vertex_buffer(0, m.vbuf.slice(..));
                        if let Some(sk) = &m.skin {
                            pass.set_vertex_buffer(1, sk.slice(..));
                        }
                        pass.set_index_buffer(m.ibuf.slice(..), wgpu::IndexFormat::Uint32);
                        last.2 = d.model;
                    }
                    pass.draw_indexed(s.first..s.first + s.count, 0, 0..1);
                    stats.draw_calls += 1;
                    stats.triangles += s.count as u64 / 3;
                }
            };
            let mut last = (u32::MAX, usize::MAX, usize::MAX, usize::MAX);
            draw_list(&mut pass, &order[..split], &mut stats, &mut bound, &mut last);
            // Contornos de casco invertido, tras los opacos para aprovechar el early-z.
            if order.iter().any(|&i| draws[i].ink) {
                let mut ink_bound: Option<bool> = None;
                for &i in order.iter().filter(|&&i| draws[i].ink) {
                    let d = &draws[i];
                    let m = &self.models[d.model];
                    let s = &m.subs[d.sub];
                    if ink_bound != Some(d.key.skinned) {
                        pass.set_pipeline(&self.fixed[&(Fixed::Ink { skinned: d.key.skinned }, hdr)]);
                        ink_bound = Some(d.key.skinned);
                        stats.pipeline_switches += 1;
                    }
                    pass.set_bind_group(1, &self.object_bg, &[(d.slot as u64 * OBJ_STRIDE) as u32]);
                    pass.set_bind_group(2, &self.materials[d.mat], &[]);
                    pass.set_vertex_buffer(0, m.vbuf.slice(..));
                    pass.set_vertex_buffer(1, m.smooth.slice(..));
                    if let Some(sk) = &m.skin {
                        pass.set_vertex_buffer(2, sk.slice(..));
                    }
                    pass.set_index_buffer(m.ibuf.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(s.first..s.first + s.count, 0, 0..1);
                    stats.draw_calls += 1;
                    stats.outlines += 1;
                }
                bound = None;
                last = (u32::MAX, usize::MAX, usize::MAX, usize::MAX);
            }
            draw_list(&mut pass, &order[split..], &mut stats, &mut bound, &mut last);
            if draws.iter().any(|d| d.selected && d.visible) {
                let mut sel_bound: Option<bool> = None;
                for d in draws.iter().filter(|d| d.selected && d.visible) {
                    let m = &self.models[d.model];
                    let s = &m.subs[d.sub];
                    if sel_bound != Some(d.key.skinned) {
                        pass.set_pipeline(&self.fixed[&(Fixed::Outline { skinned: d.key.skinned }, hdr)]);
                        sel_bound = Some(d.key.skinned);
                    }
                    pass.set_bind_group(1, &self.object_bg, &[(d.slot as u64 * OBJ_STRIDE) as u32]);
                    pass.set_bind_group(2, &self.materials[d.mat], &[]);
                    pass.set_vertex_buffer(0, m.vbuf.slice(..));
                    if let Some(sk) = &m.skin {
                        pass.set_vertex_buffer(1, sk.slice(..));
                    }
                    pass.set_index_buffer(m.ibuf.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(s.first..s.first + s.count, 0, 0..1);
                    stats.draw_calls += 1;
                }
            }
            if let (Some((slot, mat)), Some(s)) = (wire, self.sculpt.as_ref())
                && let Some((ibuf, count)) = &s.wire
            {
                let m = &self.models[s.model];
                pass.set_pipeline(&self.fixed[&(Fixed::Wire, hdr)]);
                pass.set_bind_group(1, &self.object_bg, &[(slot as u64 * OBJ_STRIDE) as u32]);
                pass.set_bind_group(2, &self.materials[mat], &[]);
                pass.set_vertex_buffer(0, m.vbuf.slice(..));
                pass.set_index_buffer(ibuf.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..*count, 0, 0..1);
                stats.draw_calls += 1;
            }
            if opts.show_grid {
                pass.set_pipeline(&self.fixed[&(Fixed::Grid, hdr)]);
                pass.set_bind_group(0, &self.globals_bg, &[]);
                pass.draw(0..6, 0..1);
            }
        }
        // Posprocesado: de la escena HDR a la salida sRGB. (La interfaz del
        // juego, si la hay, se dibuja después y no se posprocesa.)
        stats.post_passes = 0;
        if hdr && let (Some(plan), Some(post), Some(t)) = (plan.as_ref(), self.post.as_mut(), self.targets.as_ref()) {
            post.run(&mut enc, &scene.env, plan, view, root.as_deref(), &t.resolve_view);
            stats.post_passes = post.passes;
        }
        self.queue.submit(Some(enc.finish()));
        if let Some(g) = self.gi.as_mut() {
            g.tras_enviar();
            stats.gi_ms = g.tiempos(false).map(|t| t.total());
        }
        self.gi_draws = gi_draws;
        self.draws = draws;
        self.order = order;
        self.object_bytes = bytes;
        self.joint_data = joints;
        if let Some(s) = self.sculpt.as_mut() {
            stats.sculpt_upload = std::mem::take(&mut s.uploaded);
        }
        self.stats = stats;
    }

    /// Tiempos de GPU de la GI del último fotograma medido (solo si el
    /// dispositivo tiene consultas de marca de tiempo). `esperar` bloquea
    /// hasta que llegue la medida del último fotograma.
    pub fn gi_tiempos(&mut self, esperar: bool) -> Option<gi::GiTiempos> {
        self.gi.as_mut()?.tiempos(esperar)
    }

    /// Memoria de GPU reservada por la GI (bytes; 0 si está apagada).
    pub fn gi_memoria(&self) -> u64 {
        self.gi.as_ref().map(|g| g.memoria()).unwrap_or(0)
    }

    /// ¿Puede este dispositivo ejecutar la GI por software?
    pub fn gi_soportada(&self) -> bool {
        self.gi_supported
    }

    /// Lee la textura de salida a RGBA8 (para capturas y tests). Bloquea.
    pub fn read_pixels(&self) -> Option<(u32, u32, Vec<u8>)> {
        let t = self.targets.as_ref()?;
        let (w, h) = t.size;
        let bpr = (w * 4).div_ceil(256) * 256;
        let buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: (bpr * h) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = self.device.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(
            t.resolve.as_image_copy(),
            wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(bpr), rows_per_image: Some(h) } },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        self.queue.submit(Some(enc.finish()));
        let slice = buf.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
        rx.recv().ok()?.ok()?;
        let data = slice.get_mapped_range().ok()?;
        let mut out = Vec::with_capacity((w * h * 4) as usize);
        for row in 0..h {
            let s = (row * bpr) as usize;
            out.extend_from_slice(&data[s..s + (w * 4) as usize]);
        }
        drop(data);
        buf.unmap();
        Some((w, h, out))
    }
}

/// ¿El AABB de mundo queda completamente fuera de algún plano del frustum?
fn outside_frustum(vp: &Mat4, b: &Aabb) -> bool {
    let mut out = [true; 6];
    for i in 0..8 {
        let c = Vec3::new(
            if i & 1 == 0 { b.min.x } else { b.max.x },
            if i & 2 == 0 { b.min.y } else { b.max.y },
            if i & 4 == 0 { b.min.z } else { b.max.z },
        );
        let p: Vec4 = *vp * c.extend(1.0);
        let tests = [p.x < -p.w, p.x > p.w, p.y < -p.w, p.y > p.w, p.z < 0.0, p.z > p.w];
        for k in 0..6 {
            out[k] &= tests[k];
        }
    }
    out.iter().any(|&o| o)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Dispositivo para las pruebas. Devuelve también un cerrojo: las pruebas de GPU van de una
    /// en una, porque en paralelo algunos drivers se quedan bloqueados.
    pub(crate) fn device() -> Option<(wgpu::Device, wgpu::Queue, std::sync::MutexGuard<'static, ()>)> {
        static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let cerrojo = GPU.lock().unwrap_or_else(|e| e.into_inner());
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).ok()?;
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok().map(|(d, q)| (d, q, cerrojo))
    }

    /// Renderiza la escena inicial sin ventana y comprueba que el cubo azul,
    /// el suelo y el cielo aparecen donde deben.
    #[test]
    fn renders_starter_scene_headless() {
        let Some((device, queue, _gpu)) = device() else {
            eprintln!("sin adaptador GPU: test omitido");
            return;
        };
        let scene = Scene::starter();
        let mut r = SceneRenderer::new(&device, &queue);
        r.resize(320, 180);
        let (cam_id, cam) = scene.primary_camera().unwrap();
        let view = RenderView::from_camera(scene.global(cam_id), &cam, 320.0 / 180.0);
        let mut opts = RenderOptions { show_grid: true, shadows: true, ..Default::default() };
        opts.selected.insert(scene.find_by_name("Cubo").unwrap());
        r.render(&scene, &view, &opts);
        let (w, h, px) = r.read_pixels().unwrap();
        let at = |x: u32, y: u32| {
            let i = ((y * w + x) * 4) as usize;
            [px[i], px[i + 1], px[i + 2]]
        };
        let sky = at(w / 2, 5);
        assert!(sky[2] > sky[0] && sky[2] > 150, "cielo azulado arriba: {sky:?}");
        let cube = maker_engine::math::world_to_screen(view.view_proj(), Vec3::new(0.0, 0.5, 0.0), glam::Vec2::new(w as f32, h as f32)).unwrap();
        let c = at(cube.x as u32, cube.y as u32);
        assert!(c[2] > c[0] + 40, "el cubo es azul: {c:?}");
        assert!(r.stats.draw_calls >= 2 && r.stats.shadow_casters >= 1, "{:?}", r.stats);
        assert_eq!(r.stats.pipeline_switches, 1, "todo PBR: un solo pipeline");
        assert_eq!(r.stats.outlines, 0);
    }

    fn starter_view(scene: &Scene, w: u32, h: u32) -> RenderView {
        let (cam_id, cam) = scene.primary_camera().unwrap();
        RenderView::from_camera(scene.global(cam_id), &cam, w as f32 / h as f32)
    }

    fn render_px(r: &mut SceneRenderer, scene: &Scene, post: bool) -> Vec<u8> {
        let view = starter_view(scene, 320, 180);
        r.render(scene, &view, &RenderOptions { shadows: true, post, ..Default::default() });
        r.read_pixels().unwrap().2
    }

    fn px_at(px: &[u8], x: u32, y: u32) -> [u8; 3] {
        let i = ((y * 320 + x) * 4) as usize;
        [px[i], px[i + 1], px[i + 2]]
    }

    fn max_diff(a: &[u8], b: &[u8]) -> u8 {
        a.iter().zip(b).map(|(x, y)| x.abs_diff(*y)).max().unwrap_or(0)
    }

    fn mean_diff(a: &[u8], b: &[u8]) -> f64 {
        a.iter().zip(b).map(|(x, y)| x.abs_diff(*y) as f64).sum::<f64>() / a.len() as f64
    }

    /// Sin efectos (o con todos apagados) la imagen es idéntica a la de
    /// siempre y no hay ningún pase extra.
    #[test]
    fn empty_post_stack_is_identical_and_free() {
        use maker_engine::postproceso::*;
        let Some((device, queue, _gpu)) = device() else { return };
        let mut scene = Scene::starter();
        let mut r = SceneRenderer::new(&device, &queue);
        r.resize(320, 180);
        let base = render_px(&mut r, &scene, false);
        assert_eq!(render_px(&mut r, &scene, true), base);
        assert_eq!(r.stats.post_passes, 0);
        assert!(r.post.is_none(), "una pila vacía ni siquiera crea el posprocesado");
        let mut off = PostEffect::new(PostKind::Vignette(Vignette::default()));
        off.enabled = false;
        scene.env.post = vec![off, PostEffect { intensity: 0.0, ..PostEffect::new(PostKind::Fxaa) }, PostEffect::new(PostKind::Tonemap(Tonemap::default()))];
        assert_eq!(render_px(&mut r, &scene, true), base);
        assert_eq!(r.stats.post_passes, 0);
    }

    /// Con un efecto HDR que no cambia nada (bloom con umbral inalcanzable),
    /// la cadena HDR → tonemap da la misma imagen que el tonemap en la escena.
    #[test]
    fn hdr_chain_matches_direct_output() {
        use maker_engine::postproceso::*;
        let Some((device, queue, _gpu)) = device() else { return };
        let mut scene = Scene::starter();
        let mut r = SceneRenderer::new(&device, &queue);
        r.resize(320, 180);
        let base = render_px(&mut r, &scene, false);
        scene.env.post = vec![PostEffect::new(PostKind::Bloom(Bloom { threshold: 1e6, radius: 0.0 }))];
        let hdr = render_px(&mut r, &scene, true);
        // umbral + mezcla + tonemap.
        assert_eq!(r.stats.post_passes, 3);
        // El MSAA se resuelve antes del tonemap: solo los bordes difieren algo.
        assert!(mean_diff(&base, &hdr) < 0.5, "media {}", mean_diff(&base, &hdr));
        let view = starter_view(&scene, 320, 180);
        let cube = maker_engine::math::world_to_screen(view.view_proj(), Vec3::new(0.0, 0.5, 0.0), glam::Vec2::new(320.0, 180.0)).unwrap();
        for (x, y) in [(160, 5), (cube.x as u32, cube.y as u32), (20, 170)] {
            let (a, b) = (px_at(&base, x, y), px_at(&hdr, x, y));
            assert!(a.iter().zip(b).all(|(p, q)| p.abs_diff(q) <= 1), "({x},{y}): {a:?} vs {b:?}");
        }
        // Mismo resultado con un tonemap explícito; AgX ya es distinto.
        scene.env.post.push(PostEffect::new(PostKind::Tonemap(Tonemap::default())));
        assert!(max_diff(&render_px(&mut r, &scene, true), &hdr) <= 1);
        scene.env.post[1] = PostEffect::new(PostKind::Tonemap(Tonemap { op: Tonemapper::Agx, exposure_ev: 0.0 }));
        assert!(mean_diff(&render_px(&mut r, &scene, true), &hdr) > 2.0);
    }

    /// Todos los efectos integrados compilan y se encadenan; los que
    /// cambian la imagen la cambian donde deben.
    #[test]
    fn builtin_effects_run_and_change_the_image() {
        use maker_engine::postproceso::*;
        let Some((device, queue, _gpu)) = device() else { return };
        let mut scene = Scene::starter();
        let mut r = SceneRenderer::new(&device, &queue);
        r.resize(320, 180);
        let base = render_px(&mut r, &scene, false);
        scene.env.post = PostKind::builtins().into_iter().map(PostEffect::new).collect();
        let all = render_px(&mut r, &scene, true);
        assert!(r.stats.post_passes >= 12, "{:?}", r.stats);
        assert!(mean_diff(&base, &all) > 1.0);
        // La viñeta oscurece las esquinas y deja el centro.
        scene.env.post = vec![PostEffect::new(PostKind::Vignette(Vignette { radius: 0.4, softness: 0.3, color: [0.0; 3] }))];
        let v = render_px(&mut r, &scene, true);
        assert_eq!(r.stats.post_passes, 1, "solo efectos en 0..1: el tonemap va en la escena");
        let lum = |p: [u8; 3]| p.iter().map(|&c| c as u32).sum::<u32>();
        assert!(lum(px_at(&v, 2, 2)) + 60 < lum(px_at(&base, 2, 2)), "{:?} vs {:?}", px_at(&v, 2, 2), px_at(&base, 2, 2));
        assert!(lum(px_at(&v, 160, 90)).abs_diff(lum(px_at(&base, 160, 90))) <= 3);
        // El bloom ilumina alrededor de lo muy brillante.
        let cube = scene.find_by_name("Cubo").unwrap();
        let mut mr = scene.get::<MeshRenderer>(cube).unwrap();
        mr.material.emissive = [1.0; 3];
        mr.material.emissive_strength = 20.0;
        scene.insert(cube, mr);
        let lit = render_px(&mut r, &scene, false);
        scene.env.post = vec![PostEffect { intensity: 1.0, ..PostEffect::new(PostKind::Bloom(Bloom::default())) }];
        let bloom = render_px(&mut r, &scene, true);
        assert!(mean_diff(&lit, &bloom) > 0.5, "{}", mean_diff(&lit, &bloom));
    }

    /// Efectos WGSL del proyecto: se aplican, un error se omite sin romper
    /// nada (y se informa), y al arreglar el archivo se recompila.
    #[test]
    fn custom_effects_compile_reload_and_fail_safely() {
        use maker_engine::postproceso::*;
        let Some((device, queue, _gpu)) = device() else { return };
        let root = std::env::temp_dir().join(format!("maker-post-render-{}", std::process::id()));
        let dir = root.join(CUSTOM_DIR);
        std::fs::create_dir_all(&dir).unwrap();
        let invert = "// @param fuerza float 1 0 1\n// @param tinte color 1 1 1\n@fragment\nfn efecto(in: PostIn) -> @location(0) vec4<f32> {\n    let c = muestra(in.uv).rgb;\n    return vec4<f32>(mix(c, (vec3<f32>(1.0) - c) * tinte(), fuerza() * post.intensidad), 1.0);\n}\n";
        std::fs::write(dir.join("invertir.wgsl"), invert).unwrap();
        for (name, src) in EXAMPLES {
            std::fs::write(dir.join(name), src).unwrap();
        }
        std::fs::write(dir.join("plantilla.wgsl"), CUSTOM_TEMPLATE).unwrap();
        let mut scene = Scene::starter();
        let mut r = SceneRenderer::new(&device, &queue);
        r.project_root = Some(root.clone());
        r.resize(320, 180);
        let base = render_px(&mut r, &scene, false);
        for name in ["crt.wgsl", "tramado.wgsl", "plantilla.wgsl"] {
            let path = format!("{CUSTOM_DIR}/{name}");
            let info = r.post_fx().custom_info(&path, Some(&root));
            assert!(info.error.is_none(), "{name}: {:?}", info.error);
            assert!(!info.params.is_empty());
            scene.env.post = vec![PostEffect::new(PostKind::Custom(CustomEffect { path, ..Default::default() }))];
            let out = render_px(&mut r, &scene, true);
            assert_eq!(r.stats.post_passes, 1, "{name}");
            assert!(mean_diff(&base, &out) > 0.5, "{name} cambia la imagen");
        }
        let path = format!("{CUSTOM_DIR}/invertir.wgsl");
        scene.env.post = vec![PostEffect::new(PostKind::Custom(CustomEffect { path: path.clone(), ..Default::default() }))];
        let inv = render_px(&mut r, &scene, true);
        let (a, b) = (px_at(&base, 160, 5), px_at(&inv, 160, 5));
        let inv_srgb = |x: u8| {
            let l = 1.0 - srgb_to_linear(x as f32 / 255.0);
            let s = if l <= 0.003_130_8 { l * 12.92 } else { 1.055 * l.powf(1.0 / 2.4) - 0.055 };
            (s * 255.0).round() as i32
        };
        assert!(a.iter().zip(b).all(|(x, y)| (inv_srgb(*x) - y as i32).abs() <= 3), "invertido en lineal: {a:?} → {b:?}");
        // Valores del Inspector: fuerza 0 = sin cambio.
        if let PostKind::Custom(c) = &mut scene.env.post[0].kind {
            c.values.insert("fuerza".into(), [0.0; 4]);
        }
        assert!(max_diff(&render_px(&mut r, &scene, true), &base) <= 1);
        // Roto: se omite (misma imagen que sin efectos) y el error dice la línea.
        std::fs::write(dir.join("invertir.wgsl"), invert.replace("muestra(in.uv).rgb", "muestra(in.uv).rgbx")).unwrap();
        r.post_fx().reload_custom(&path, Some(&root));
        assert_eq!(render_px(&mut r, &scene, true), base);
        assert_eq!(r.stats.post_passes, 0);
        let err = r.post_fx().custom_info(&path, Some(&root)).error.unwrap().to_string();
        assert!(err.contains("invertir.wgsl") && err.contains(":5:"), "{err}");
        // Sin el entry point.
        std::fs::write(dir.join("invertir.wgsl"), "fn otra() {}").unwrap();
        r.post_fx().reload_custom(&path, Some(&root));
        assert!(r.post_fx().custom_info(&path, Some(&root)).error.unwrap().contains("efecto"));
        // Arreglado: vuelve a aplicarse.
        std::fs::write(dir.join("invertir.wgsl"), invert).unwrap();
        r.post_fx().reload_custom(&path, Some(&root));
        scene.env.post[0] = PostEffect::new(PostKind::Custom(CustomEffect { path: path.clone(), ..Default::default() }));
        assert!(max_diff(&render_px(&mut r, &scene, true), &inv) <= 1);
        // Archivo que no existe: se omite.
        scene.env.post[0] = PostEffect::new(PostKind::Custom(CustomEffect { path: "shaders/post/no.wgsl".into(), ..Default::default() }));
        assert_eq!(render_px(&mut r, &scene, true), base);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// El sombreado global cambia el modelo de todo salvo de las excepciones.
    #[test]
    fn global_shading_overrides_materials() {
        let Some((device, queue, _gpu)) = device() else { return };
        let mut scene = Scene::starter();
        let mut r = SceneRenderer::new(&device, &queue);
        r.resize(320, 180);
        let base = render_px(&mut r, &scene, false);
        assert_eq!(r.stats.pipeline_switches, 1);
        scene.env.global_shading.model = Some(ShadingModel::Toon);
        scene.env.global_shading.style.outline_width = 0.0;
        let toon = render_px(&mut r, &scene, false);
        assert_eq!((r.stats.pipeline_switches, r.stats.outlines), (1, 0), "todo toon: {:?}", r.stats);
        assert!(mean_diff(&base, &toon) > 1.0);
        // El contorno global se dibuja en todos.
        scene.env.global_shading.style.outline_width = 3.0;
        render_px(&mut r, &scene, false);
        assert_eq!(r.stats.outlines, 2, "{:?}", r.stats);
        // Excepción: el cubo conserva su PBR.
        let cube = scene.find_by_name("Cubo").unwrap();
        let mut mr = scene.get::<MeshRenderer>(cube).unwrap();
        mr.material.ignore_global = true;
        scene.insert(cube, mr);
        scene.env.global_shading.style.outline_width = 0.0;
        render_px(&mut r, &scene, false);
        assert_eq!(r.stats.pipeline_switches, 2, "PBR + toon: {:?}", r.stats);
        // Conserva el color: el cubo sigue azul en toon.
        let mr = scene.get::<MeshRenderer>(cube).unwrap();
        scene.insert(cube, MeshRenderer { material: Material { ignore_global: false, ..mr.material }, ..mr });
        let px = render_px(&mut r, &scene, false);
        let view = starter_view(&scene, 320, 180);
        let c = maker_engine::math::world_to_screen(view.view_proj(), Vec3::new(0.0, 0.5, 0.0), glam::Vec2::new(320.0, 180.0)).unwrap();
        let c = px_at(&px, c.x as u32, c.y as u32);
        assert!(c[2] > c[0] + 40, "azul en toon: {c:?}");
    }

    /// Todos los modelos de sombreado compilan y se dibujan; los dibujos se
    /// agrupan por pipeline y el contorno toon sale con su color.
    #[test]
    fn renders_stylized_models_grouped_by_pipeline() {
        let Some((device, queue, _gpu)) = device() else {
            eprintln!("sin adaptador GPU: test omitido");
            return;
        };
        let mut scene = Scene::starter();
        let cube = scene.find_by_name("Cubo").unwrap();
        let mut mr = scene.get::<MeshRenderer>(cube).unwrap();
        mr.material = Material::toon([0.2, 0.7, 0.3]);
        mr.material.style.outline_width = 30.0; // ≈ 5 px a 180 p
        mr.material.style.outline_color = [1.0, 0.0, 0.0];
        scene.insert(cube, mr);
        let ground = scene.find_by_name("Suelo").unwrap();
        let mut mr = scene.get::<MeshRenderer>(ground).unwrap();
        mr.material.shading = ShadingModel::Hatching;
        mr.material.style.outline_width = 0.0;
        scene.insert(ground, mr);
        // Modelos intercalados: el orden de la escena no debe multiplicar los cambios de pipeline.
        for (i, model) in [ShadingModel::Unlit, ShadingModel::Toon, ShadingModel::Pbr, ShadingModel::Hatching, ShadingModel::Toon].into_iter().enumerate() {
            let e = scene.spawn(&format!("Extra {i}"));
            scene.set_transform(e, Transform::from_translation(Vec3::new(-3.0 + i as f32 * 1.5, 0.5, -3.0)).with_scale(Vec3::splat(0.6)));
            let mut mr = MeshRenderer::primitive(PrimitiveShape::Sphere);
            mr.material.shading = model;
            mr.material.style.outline_width = 0.0;
            scene.insert(e, mr);
        }
        scene.update_transforms();
        let mut r = SceneRenderer::new(&device, &queue);
        r.resize(320, 180);
        let (cam_id, cam) = scene.primary_camera().unwrap();
        let view = RenderView::from_camera(scene.global(cam_id), &cam, 320.0 / 180.0);
        r.render(&scene, &view, &RenderOptions { shadows: true, ..Default::default() });
        // Standard, toon, plumilla y el pase de contorno.
        assert_eq!(r.stats.pipeline_switches, 4, "{:?}", r.stats);
        assert_eq!(r.stats.outlines, 1, "{:?}", r.stats);
        let (_, _, px) = r.read_pixels().unwrap();
        let red = px.chunks(4).filter(|p| p[0] > 180 && p[1] < 60 && p[2] < 60).count();
        assert!(red > 30, "el contorno rojo debe verse: {red} píxeles");
    }

    // ───────────────────────────── normal maps ─────────────────────────────

    /// Escena de prueba: el suelo de la escena inicial (sin el cubo) y un sol
    /// que viaja hacia +Z: la luz llega desde −Z, que en el plano es «arriba»
    /// en la imagen (V = 0).
    fn normal_scene() -> Scene {
        let mut scene = Scene::starter();
        let cube = scene.find_by_name("Cubo").unwrap();
        scene.despawn_recursive(cube);
        let sun = scene.find_by_name("Sol").unwrap();
        let dir = Vec3::new(0.0, -1.0, 1.0).normalize();
        scene.set_transform(sun, Transform::from_translation(Vec3::Y * 10.0).with_rotation(glam::Quat::from_rotation_arc(Vec3::NEG_Z, dir)));
        // Luz moderada: el tonemap no satura el suelo y se ven las dos inclinaciones.
        let mut l = scene.get::<Light>(sun).unwrap();
        l.intensity = 1.2;
        scene.insert(sun, l);
        scene.update_transforms();
        scene
    }

    /// PNG 2×2 de una normal uniforme en espacio tangente (convención OpenGL).
    fn write_normal_png(path: &Path, n: Vec3) {
        let n = n.normalize();
        let px = [n.x, n.y, n.z].map(|c| ((c * 0.5 + 0.5) * 255.0).round() as u8);
        image::RgbImage::from_pixel(2, 2, image::Rgb(px)).save(path).unwrap();
    }

    fn set_ground(scene: &mut Scene, f: impl FnOnce(&mut MeshRenderer)) {
        let g = scene.find_by_name("Suelo").unwrap();
        let mut mr = scene.get::<MeshRenderer>(g).unwrap();
        f(&mut mr);
        scene.insert(g, mr);
    }

    /// Luminancia media de la franja de suelo de la parte baja de la imagen.
    fn ground_luma(px: &[u8]) -> f64 {
        let mut sum = 0.0;
        let mut n = 0.0;
        for y in 130..178 {
            for x in 20..300 {
                let p = px_at(px, x, y);
                sum += p.iter().map(|&c| c as f64).sum::<f64>() / 3.0;
                n += 1.0;
            }
        }
        sum / n
    }

    fn temp_project(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("maker-normal-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    /// Un normal map inclinado hacia «arriba» en la imagen (verde > 0,5)
    /// ilumina más el plano cuando la luz viene de ese lado; el verde
    /// invertido (DirectX) da lo contrario; intensidad 0 y sin luz lo ignoran;
    /// uno plano no cambia nada (se muestrea en lineal, no en sRGB).
    #[test]
    fn normal_map_tilts_lighting_in_gltf_convention() {
        let Some((device, queue, _gpu)) = device() else { return };
        let root = temp_project("tilt");
        write_normal_png(&root.join("arriba.png"), Vec3::new(0.0, 0.6, 0.8));
        write_normal_png(&root.join("abajo.png"), Vec3::new(0.0, -0.6, 0.8));
        write_normal_png(&root.join("izq.png"), Vec3::new(-0.6, 0.0, 0.8));
        write_normal_png(&root.join("der.png"), Vec3::new(0.6, 0.0, 0.8));
        image::RgbImage::from_pixel(2, 2, image::Rgb([128, 128, 255])).save(root.join("plano.png")).unwrap();
        let mut r = SceneRenderer::new(&device, &queue);
        r.project_root = Some(root.clone());
        r.resize(320, 180);
        let mut scene = normal_scene();
        let base = render_px(&mut r, &scene, false);
        let with = |scene: &mut Scene, r: &mut SceneRenderer, tex: &str, f: &dyn Fn(&mut NormalMap)| {
            set_ground(scene, |mr| {
                mr.material.normal = NormalMap { source: NormalSource::Texture, texture: tex.into(), ..Default::default() };
                f(&mut mr.material.normal);
            });
            render_px(r, scene, false)
        };
        let plano = with(&mut scene, &mut r, "plano.png", &|_| {});
        assert!(max_diff(&plano, &base) <= 2, "un normal map plano no cambia nada: {}", max_diff(&plano, &base));
        let up = with(&mut scene, &mut r, "arriba.png", &|_| {});
        let down = with(&mut scene, &mut r, "abajo.png", &|_| {});
        let (lb, lu, ld) = (ground_luma(&base), ground_luma(&up), ground_luma(&down));
        assert!(lu > lb + 8.0 && ld + 8.0 < lb, "verde arriba ilumina, abajo oscurece: {lu:.1} / {lb:.1} / {ld:.1}");
        // Verde invertido = la inclinación contraria.
        let flipped = with(&mut scene, &mut r, "arriba.png", &|n| n.flip_green = true);
        assert!(max_diff(&flipped, &down) <= 2, "DirectX: {}", max_diff(&flipped, &down));
        // La X es perpendicular a la luz: izquierda y derecha, iguales.
        let left = with(&mut scene, &mut r, "izq.png", &|_| {});
        let right = with(&mut scene, &mut r, "der.png", &|_| {});
        assert!((ground_luma(&left) - ground_luma(&right)).abs() < 1.0);
        // Con la luz llegando desde +X (+U, la tangente), el rojo > 0,5 ilumina.
        let sun = scene.find_by_name("Sol").unwrap();
        let t = scene.transform(sun);
        scene.set_transform(sun, t.with_rotation(glam::Quat::from_rotation_arc(Vec3::NEG_Z, Vec3::new(-1.0, -1.0, 0.0).normalize())));
        scene.update_transforms();
        let left = with(&mut scene, &mut r, "izq.png", &|_| {});
        let right = with(&mut scene, &mut r, "der.png", &|_| {});
        assert!(ground_luma(&right) > ground_luma(&left) + 20.0, "{} vs {}", ground_luma(&right), ground_luma(&left));
        scene.set_transform(sun, t);
        scene.update_transforms();
        // Intensidad: 0 = sin relieve; 2 = más que 1.
        let zero = with(&mut scene, &mut r, "arriba.png", &|n| n.strength = 0.0);
        assert_eq!(zero, base);
        let double = with(&mut scene, &mut r, "abajo.png", &|n| n.strength = 2.0);
        assert!(ground_luma(&double) + 2.0 < ld);
        // Toon y plumilla también lo usan.
        for model in [ShadingModel::Toon, ShadingModel::Hatching] {
            set_ground(&mut scene, |mr| {
                mr.material.shading = model;
                mr.material.style.outline_width = 0.0;
            });
            let a = with(&mut scene, &mut r, "arriba.png", &|_| {});
            let b = with(&mut scene, &mut r, "abajo.png", &|_| {});
            assert!(mean_diff(&a, &b) > 1.0, "{model:?}: {}", mean_diff(&a, &b));
        }
        // Sin luz lo ignora.
        set_ground(&mut scene, |mr| mr.material.shading = ShadingModel::Unlit);
        let a = with(&mut scene, &mut r, "arriba.png", &|_| {});
        set_ground(&mut scene, |mr| mr.material.normal = NormalMap::default());
        assert_eq!(a, render_px(&mut r, &scene, false));
        let _ = std::fs::remove_dir_all(&root);
    }

    fn b64(data: &[u8]) -> String {
        const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for c in data.chunks(3) {
            let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
            for i in 0..4 {
                out.push(if i <= c.len() { T[(n >> (18 - 6 * i) & 63) as usize] as char } else { '=' });
            }
        }
        out
    }

    /// glTF de un plano de 2×2 m (como la primitiva) con textura de color y,
    /// opcionalmente, `normalTexture`.
    fn write_plane_gltf(path: &Path, color: &str, normal: Option<(&str, f32)>) {
        let pos: [f32; 12] = [-1.0, 0.0, 1.0, 1.0, 0.0, 1.0, 1.0, 0.0, -1.0, -1.0, 0.0, -1.0];
        let nrm: [f32; 12] = [0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0];
        let uv: [f32; 8] = [0.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0];
        let idx: [u16; 6] = [0, 1, 2, 0, 2, 3];
        let mut buf: Vec<u8> = pos.iter().chain(&nrm).chain(&uv).flat_map(|f| f.to_le_bytes()).collect();
        buf.extend(idx.iter().flat_map(|i| i.to_le_bytes()));
        let (images, textures, ntex) = match normal {
            Some((n, scale)) => (format!(r#"[{{"uri":"{color}"}},{{"uri":"{n}"}}]"#), r#"[{"source":0},{"source":1}]"#, format!(r#","normalTexture":{{"index":1,"scale":{scale}}}"#)),
            None => (format!(r#"[{{"uri":"{color}"}}]"#), r#"[{"source":0}]"#, String::new()),
        };
        let json = format!(
            r#"{{"asset":{{"version":"2.0"}},"scene":0,"scenes":[{{"nodes":[0]}}],"nodes":[{{"mesh":0}}],
            "meshes":[{{"primitives":[{{"attributes":{{"POSITION":0,"NORMAL":1,"TEXCOORD_0":2}},"indices":3,"material":0}}]}}],
            "materials":[{{"pbrMetallicRoughness":{{"baseColorTexture":{{"index":0}}}}{ntex}}}],
            "images":{images},"textures":{textures},
            "buffers":[{{"byteLength":{},"uri":"data:application/octet-stream;base64,{}"}}],
            "bufferViews":[{{"buffer":0,"byteOffset":0,"byteLength":48}},{{"buffer":0,"byteOffset":48,"byteLength":48}},{{"buffer":0,"byteOffset":96,"byteLength":32}},{{"buffer":0,"byteOffset":128,"byteLength":12}}],
            "accessors":[{{"bufferView":0,"componentType":5126,"count":4,"type":"VEC3","min":[-1,0,-1],"max":[1,0,1]}},
                         {{"bufferView":1,"componentType":5126,"count":4,"type":"VEC3"}},
                         {{"bufferView":2,"componentType":5126,"count":4,"type":"VEC2"}},
                         {{"bufferView":3,"componentType":5123,"count":6,"type":"SCALAR"}}]}}"#,
            buf.len(),
            b64(&buf)
        );
        std::fs::write(path, json).unwrap();
    }

    /// Modelos glTF: el `normalTexture` del archivo se usa (con su escala);
    /// sin él, el normal generado en memoria (automático) y el guardado en
    /// PNG dan la misma imagen y cambian la luz respecto al plano.
    #[test]
    fn gltf_generated_and_auto_normal_maps() {
        let Some((device, queue, _gpu)) = device() else { return };
        let root = temp_project("gltf");
        // Color con relieve: franjas horizontales claras y oscuras.
        let mut color = image::RgbImage::new(64, 64);
        for (_, y, p) in color.enumerate_pixels_mut() {
            *p = if (y / 8) % 2 == 0 { image::Rgb([230, 230, 230]) } else { image::Rgb([60, 60, 60]) };
        }
        color.save(root.join("color.png")).unwrap();
        write_normal_png(&root.join("n.png"), Vec3::new(0.0, 0.6, 0.8));
        write_plane_gltf(&root.join("plano.gltf"), "color.png", None);
        write_plane_gltf(&root.join("plano_n.gltf"), "color.png", Some(("n.png", 1.0)));
        write_plane_gltf(&root.join("plano_n0.gltf"), "color.png", Some(("n.png", 0.0)));
        let mut r = SceneRenderer::new(&device, &queue);
        r.project_root = Some(root.clone());
        r.resize(320, 180);
        let mut scene = normal_scene();
        let model = |m: &str| MeshSource::Model(m.into());
        set_ground(&mut scene, |mr| {
            mr.mesh = model("plano.gltf");
            mr.material = Material { base_color: [1.0; 4], ..Default::default() };
        });
        let g = scene.find_by_name("Suelo").unwrap();
        scene.set_transform(g, Transform::default().with_scale(Vec3::new(10.0, 1.0, 10.0)));
        scene.update_transforms();
        let flat = render_px(&mut r, &scene, false);
        let info = r.normal_info(&model("plano.gltf")).unwrap();
        assert_eq!((info.textured, info.with_normal, info.missing.len()), (1, 0, 1));
        assert_eq!(info.missing[0].1, root.join("color_normal_gen.png"));
        // Normal map del glTF; escala 0 = plano.
        set_ground(&mut scene, |mr| mr.mesh = model("plano_n.gltf"));
        let own = render_px(&mut r, &scene, false);
        // Lo mismo que elegir esa textura a mano en el material.
        set_ground(&mut scene, |mr| {
            mr.mesh = model("plano.gltf");
            mr.material.normal = NormalMap { source: NormalSource::Texture, texture: "n.png".into(), ..Default::default() };
        });
        assert!(max_diff(&render_px(&mut r, &scene, false), &own) <= 1);
        set_ground(&mut scene, |mr| {
            mr.mesh = model("plano_n.gltf");
            mr.material.normal = NormalMap::default();
        });
        let info = r.normal_info(&model("plano_n.gltf")).unwrap();
        assert_eq!((info.with_normal, info.missing.len()), (1, 0));
        set_ground(&mut scene, |mr| mr.mesh = model("plano_n0.gltf"));
        assert!(max_diff(&render_px(&mut r, &scene, false), &flat) <= 2);
        // Automático: plano hasta que el hilo termina, luego con relieve.
        set_ground(&mut scene, |mr| {
            mr.mesh = model("plano.gltf");
            mr.material.normal.source = NormalSource::Auto;
            mr.material.normal.generate = NormalGen { strength: 6.0, radius: 0.0, invert: false };
        });
        let first = render_px(&mut r, &scene, false);
        assert_eq!(first, flat, "mientras se genera, plano");
        assert!(r.wait_normal_maps(std::time::Duration::from_secs(20)));
        let auto = render_px(&mut r, &scene, false);
        assert!(mean_diff(&auto, &flat) > 0.5, "{}", mean_diff(&auto, &flat));
        // Generado a PNG (sin archivo aún: plano).
        set_ground(&mut scene, |mr| mr.material.normal.source = NormalSource::Generated);
        assert_eq!(render_px(&mut r, &scene, false), flat);
        let gen_params = NormalGen { strength: 6.0, radius: 0.0, invert: false };
        assert!(normal_gen::generate_file(&info_base(&mut r), &root.join("color_normal_gen.png"), &gen_params).unwrap());
        r.reload_normal_map(&root.join("color_normal_gen.png"));
        let generated = render_px(&mut r, &scene, false);
        assert!(max_diff(&generated, &auto) <= 1, "PNG = memoria: {}", max_diff(&generated, &auto));
        let _ = std::fs::remove_dir_all(&root);
    }

    fn info_base(r: &mut SceneRenderer) -> TextureSource {
        r.normal_info(&MeshSource::Model("plano.gltf".into())).unwrap().missing[0].0.clone()
    }

    /// La malla esculpida se dibuja desde su búfer dinámico y solo cambia en pantalla cuando
    /// se suben sus rangos sucios (subida parcial). La máscara oscurece.
    #[test]
    fn sculpt_mesh_partial_updates_reach_the_gpu() {
        use maker_sculpt::{Action, Brush, Dab, DabOptions, Sculpt, primitives};
        let Some((device, queue, _gpu)) = device() else {
            eprintln!("sin adaptador GPU: test omitido");
            return;
        };
        let mut scene = Scene::new();
        scene.env.fog_density = 0.0;
        let sun = scene.spawn("Sol");
        let mut t = Transform::default();
        t.set_euler_degrees(Vec3::new(-30.0, 20.0, 0.0));
        scene.set_transform(sun, t);
        scene.insert(sun, Light { intensity: 3.0, ..Light::directional() });
        let id = scene.spawn("Escultura");
        scene.insert(id, MeshRenderer { material: Material::color([0.9, 0.2, 0.1]), ..MeshRenderer::primitive(PrimitiveShape::Cube) });
        scene.update_transforms();
        let eid = id;
        let mut r = SceneRenderer::new(&device, &queue);
        let (w, h) = (128u32, 128u32);
        r.resize(w, h);
        let eye = Vec3::new(0.0, 0.0, 3.0);
        let view = RenderView { view: look_at(eye, Vec3::ZERO, Vec3::Y), proj: maker_engine::math::perspective(40.0, 1.0, 0.05, 50.0), eye };
        let opts = RenderOptions { shadows: true, ..Default::default() };
        let render = |r: &mut SceneRenderer| {
            r.render(&scene, &view, &opts);
            r.read_pixels().unwrap().2
        };
        let px = |img: &[u8], x: u32, y: u32| {
            let i = ((y * w + x) * 4) as usize;
            [img[i], img[i + 1], img[i + 2]]
        };
        let diff = |a: [u8; 3], b: [u8; 3]| a.iter().zip(&b).map(|(x, y)| (*x as i32 - *y as i32).abs()).max().unwrap();

        let mut sc = Sculpt::from_mesh(primitives::quad_sphere(32, 0.5));
        r.sculpt_begin(eid, &sc.render_model());
        assert_eq!(r.sculpt_entity(), Some(eid));
        let a = render(&mut r);
        // Radio 0,5 a 3 m con 40°: ~29 px. Justo fuera del borde derecho e izquierdo.
        let (right, left, center) = ((w / 2 + 36, h / 2), (w / 2 - 36, h / 2), (w / 2, h / 2));
        assert!(diff(px(&a, center.0, center.1), px(&a, right.0, right.1)) > 20, "la esfera (no el cubo) se ve en el centro");

        // Hinchar el lado derecho en la CPU: sin subirlo, la imagen no cambia.
        let o = DabOptions::default();
        for _ in 0..25 {
            sc.dab(Action::Brush { brush: Brush::Inflate, invert: false }, &Dab::at(Vec3::new(0.5, 0.0, 0.0), Vec3::X, 0.3, 1.0), &o);
        }
        sc.end_stroke();
        let b = render(&mut r);
        assert_eq!(diff(px(&a, right.0, right.1), px(&b, right.0, right.1)), 0);

        let mut ranges = Vec::new();
        sc.take_gpu_ranges(&mut ranges);
        let total: u32 = ranges.iter().map(|x| x.end - x.start).sum();
        assert!(total > 0 && (total as usize) < sc.render_vertex_count(), "subida parcial: {total}");
        let bytes = r.sculpt_write(&ranges, |i| sc.render_vertex(i));
        assert!(bytes > 0);
        let (bmin, bmax) = sc.mesh.bounds();
        r.sculpt_set_bounds(Aabb { min: bmin, max: bmax });
        let c = render(&mut r);
        assert!(r.stats.sculpt_upload > 0);
        assert!(diff(px(&a, right.0, right.1), px(&c, right.0, right.1)) > 20, "el bulto derecho aparece");
        assert!(diff(px(&a, left.0, left.1), px(&c, left.0, left.1)) < 4, "el lado izquierdo sigue igual");

        // Máscara en el centro: más oscuro tras subirla.
        for _ in 0..10 {
            sc.dab(Action::Mask { erase: false }, &Dab::at(Vec3::new(0.0, 0.0, 0.5), Vec3::Z, 0.2, 1.0), &o);
        }
        sc.end_stroke();
        sc.take_gpu_ranges(&mut ranges);
        r.sculpt_write(&ranges, |i| sc.render_vertex(i));
        let d = render(&mut r);
        let luma = |p: [u8; 3]| p.iter().map(|&x| x as i32).sum::<i32>();
        assert!(luma(px(&d, center.0, center.1)) < luma(px(&c, center.0, center.1)) - 30, "enmascarado más oscuro");

        // Alambre: añade líneas oscuras sobre la superficie.
        r.sculpt_set_wire(Some(&sc.wire_edges()));
        r.render(&scene, &view, &RenderOptions { sculpt_wire: true, ..opts.clone() });
        let e = r.read_pixels().unwrap().2;
        assert!(mean_diff(&d, &e) > 0.5, "se ven las aristas");

        // Al terminar, la entidad vuelve a su malla (el cubo).
        r.sculpt_end();
        assert_eq!(r.sculpt_entity(), None);
        let f = render(&mut r);
        assert!(diff(px(&f, w / 2 + 31, h / 2 + 31), px(&a, w / 2 + 31, h / 2 + 31)) > 10, "la esquina del cubo vuelve");
    }
}
