//! Renderizador de escena sobre wgpu (Vulkan / Metal / DX12 / GL).
//!
//! Dibuja a una textura fuera de pantalla (MSAA 4× resuelta a sRGB) que el
//! editor muestra en el viewport y el reproductor a pantalla completa.
//!
//! Cada modelo es un búfer de vértices con submallas; cada submalla tiene su
//! material (textura de color base con mipmaps, factor, recorte o mezcla alfa,
//! doble cara). Los pipelines se crean bajo demanda por combinación de modelo
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
use maker_engine::scene::Scene;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use wgpu::util::DeviceExt;

use crate::gi::{self, GiDebug, GiDraw, GiSystem};
use crate::postproceso::PostFx;
use crate::texture;

pub const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
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
    normal_m: [[f32; 4]; 4],
    base_color: [f32; 4],
    emissive: [f32; 4],
    params: [f32; 4],
    // Modelos estilizados. Caben en el mismo paso de 256 B que ya usaba el
    // PBR (alineación del offset dinámico), así que no le cuestan nada.
    shadow_tint: [f32; 4],
    rim: [f32; 4],
    style: [f32; 4],
    outline: [f32; 4],
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

fn object_uniform(m: &Mat4, mat: &Material, skin_base: u32) -> ObjectU {
    let bc = lin3([mat.base_color[0], mat.base_color[1], mat.base_color[2]]);
    let st = &mat.style;
    let outline = if mat.shading.has_outline() { st.outline_width.max(0.0) } else { 0.0 };
    ObjectU {
        model: m.to_cols_array_2d(),
        normal_m: m.inverse().transpose().to_cols_array_2d(),
        base_color: v4(bc, mat.base_color[3]),
        emissive: v4(lin3(mat.emissive), mat.emissive_strength),
        params: [mat.metallic, mat.roughness, if mat.shading == ShadingModel::Unlit { 1.0 } else { 0.0 }, st.hatch_scale],
        shadow_tint: v4(lin3(st.shadow_color), st.bands.clamp(2, 4) as f32),
        rim: v4(lin3(st.rim_color), st.rim_strength),
        style: [st.softness, st.specular, st.specular_size, st.rim_width],
        outline: v4(lin3(st.outline_color), outline),
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
}

pub(crate) struct GpuSub {
    pub(crate) first: u32,
    pub(crate) count: u32,
    mat: usize,
    alpha: Alpha,
    double_sided: bool,
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
    object_buf: wgpu::Buffer,
    joint_buf: wgpu::Buffer,
    object_bg: wgpu::BindGroup,
    object_capacity: u64,
    joint_capacity: u64,
    module: wgpu::ShaderModule,
    main_layout: wgpu::PipelineLayout,
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
    textures: HashMap<String, Option<Arc<wgpu::TextureView>>>,
    white: Arc<wgpu::TextureView>,
    failed_models: HashSet<String>,
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

const VERTEX_ATTRS: [wgpu::VertexAttribute; 4] = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x2, 3 => Unorm8x4];
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
            bind_group_layouts: &[Some(&globals_bgl), Some(&object_bgl), Some(&material_bgl), Some(&gi_bgl)],
            immediate_size: 0,
        });
        let globals_only = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("globals only"), bind_group_layouts: &[Some(&globals_bgl)], immediate_size: 0 });
        let shadow_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("shadow"),
            bind_group_layouts: &[Some(&shadow_globals_bgl), Some(&object_bgl), Some(&material_bgl)],
            immediate_size: 0,
        });

        let white = Arc::new(Self::upload_texture(&device, &queue, "white", texture::build_mips(1, 1, vec![255; 4], None)));
        let mut r = Self {
            device,
            queue,
            globals_buf,
            globals_bg,
            shadow_globals_buf,
            shadow_globals_bg,
            object_bgl,
            material_bgl,
            object_buf,
            joint_buf,
            object_bg,
            object_capacity,
            joint_capacity,
            module,
            main_layout,
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
            textures: HashMap::new(),
            white,
            failed_models: HashSet::new(),
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
        // Casco invertido: posición del búfer de vértices normal + normales suavizadas (+ piel).
        let ink_pos = wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<Vertex>() as u64, step_mode: wgpu::VertexStepMode::Vertex, attributes: &INK_POS_ATTRS };
        let ink_n = wgpu::VertexBufferLayout { array_stride: 12, step_mode: wgpu::VertexStepMode::Vertex, attributes: &INK_NORMAL_ATTRS };
        let ink_static = [Some(ink_pos.clone()), Some(ink_n.clone())];
        let ink_skinned = [Some(ink_pos), Some(ink_n), Some(skin_layout())];
        let vb = [Some(vertex_layout())];
        let vb_skin = [Some(vertex_layout()), Some(skin_layout())];
        let (label, layout, vs, fs, buffers, cull, write, cmp, blend): (_, _, _, _, &[Option<wgpu::VertexBufferLayout>], _, _, _, _) = match f {
            Fixed::Ink { skinned: false } => ("ink outline", &self.main_layout, "vs_ink", "fs_ink", &ink_static, Some(wgpu::Face::Front), true, wgpu::CompareFunction::Less, None),
            Fixed::Ink { skinned: true } => ("ink outline skinned", &self.main_layout, "vs_ink_skinned", "fs_ink", &ink_skinned, Some(wgpu::Face::Front), true, wgpu::CompareFunction::Less, None),
            Fixed::Outline { skinned: false } => ("outline", &self.main_layout, "vs_outline", "fs_outline", &vb, Some(wgpu::Face::Front), true, wgpu::CompareFunction::LessEqual, None),
            Fixed::Outline { skinned: true } => ("outline skinned", &self.main_layout, "vs_outline_skinned", "fs_outline", &vb_skin, Some(wgpu::Face::Front), true, wgpu::CompareFunction::LessEqual, None),
            Fixed::Sky => ("sky", &self.globals_only, "vs_sky", "fs_sky", &[], None, false, wgpu::CompareFunction::Always, None),
            Fixed::Grid => ("grid", &self.globals_only, "vs_grid", "fs_grid", &[], None, false, wgpu::CompareFunction::LessEqual, Some(wgpu::BlendState::ALPHA_BLENDING)),
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

    fn upload_texture(device: &wgpu::Device, queue: &wgpu::Queue, label: &str, mips: texture::MipChain) -> wgpu::TextureView {
        let (w, h, _) = &mips.levels[0];
        let tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d { width: *w, height: *h, depth_or_array_layers: 1 },
            mip_level_count: mips.levels.len() as u32,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
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
            Ok((w, h, px)) => Some(Arc::new(Self::upload_texture(&self.device, &self.queue, &key, texture::build_mips(w, h, px, cutoff)))),
            Err(e) => {
                maker_engine::log::error("render", format!("textura {key}: {e}"));
                None
            }
        };
        self.textures.insert(key, view.clone());
        view.unwrap_or_else(|| self.white.clone())
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
        self.models.clear();
        self.model_index.clear();
        self.materials.clear();
        self.textures.clear();
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
        let i = self.upload_model(&data);
        self.model_index.insert(src.clone(), i);
        Some(i)
    }

    fn upload_model(&mut self, data: &ModelData) -> usize {
        let m = &data.mesh;
        let verts: Vec<Vertex> = (0..m.positions.len())
            .map(|i| Vertex {
                pos: m.positions[i].to_array(),
                normal: m.normals.get(i).copied().unwrap_or(Vec3::Y).to_array(),
                uv: m.uvs.get(i).copied().unwrap_or_default().to_array(),
                color: m.colors.get(i).copied().unwrap_or([255; 4]),
            })
            .collect();
        let vbuf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("mesh vertices"), contents: bytemuck::cast_slice(&verts), usage: wgpu::BufferUsages::VERTEX });
        let smooth: Vec<[f32; 3]> = m.smooth_normals().iter().map(|n| n.to_array()).collect();
        let smooth = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("mesh smooth normals"), contents: bytemuck::cast_slice(&smooth), usage: wgpu::BufferUsages::VERTEX });
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
        for mm in &data.materials {
            let cutoff = if let AlphaMode::Mask(c) = mm.alpha { Some(c) } else { None };
            let tex = match &mm.texture {
                Some(t) => self.texture(t, cutoff),
                None => self.white.clone(),
            };
            mat_bg.push(self.material_bind_group(mm.base_color, cutoff.unwrap_or(0.0), &tex));
        }
        let subs = data
            .submeshes
            .iter()
            .map(|s| {
                let mm = &data.materials[s.material];
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
        self.models.push(GpuModel { vbuf, smooth, skin, ibuf, subs, aabb, joints: if skinned { data.anim.skeleton.joints.len() as u32 } else { 0 } });
        self.models.len() - 1
    }

    /// Dibuja la escena en la textura de salida y envía los comandos a la GPU.
    pub fn render(&mut self, scene: &Scene, view: &RenderView, opts: &RenderOptions) {
        let Some((w, h)) = self.targets.as_ref().map(|t| t.size) else { return };
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
            let Some(mi) = self.ensure_model(&mr.mesh) else { continue };
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
            let mat = &env.global_shading.apply(&mr.material);
            bytes.extend_from_slice(bytemuck::bytes_of(&object_uniform(&m, mat, skin_base)));
            bytes.resize((slot as usize + 1) * OBJ_STRIDE as usize, 0);
            let ink = mat.shading.has_outline() && mat.style.outline_width > 0.0;
            let depth = wb.center().distance_squared(view.eye);
            let selected = opts.selected.contains(id);
            for (si, s) in model.subs.iter().enumerate() {
                let key = PipeKey { alpha: s.alpha, pipe: pipe_of(mat.shading), skinned: model.joints > 0, double_sided: s.double_sided, hdr };
                draws.push(Draw {
                    model: mi,
                    sub: si,
                    key,
                    mat: s.mat,
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
                    da.key.cmp(&db.key).then(da.mat.cmp(&db.mat)).then(da.model.cmp(&db.model))
                }
            })
        });
        self.ensure_fixed(Fixed::Sky, hdr);
        if opts.show_grid {
            self.ensure_fixed(Fixed::Grid, hdr);
        }
        for d in &draws {
            if d.visible {
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
            let draw_list = |pass: &mut wgpu::RenderPass, list: &[usize], stats: &mut FrameStats, bound: &mut Option<PipeKey>, last: &mut (u32, usize, usize)| {
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
                    if d.mat != last.1 {
                        pass.set_bind_group(2, &self.materials[d.mat], &[]);
                        last.1 = d.mat;
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
            let mut last = (u32::MAX, usize::MAX, usize::MAX);
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
                last = (u32::MAX, usize::MAX, usize::MAX);
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
}
