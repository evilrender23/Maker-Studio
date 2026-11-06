//! Renderizador de escena sobre wgpu (Vulkan / Metal / DX12 / GL).
//!
//! Dibuja a una textura fuera de pantalla (MSAA 4× resuelta a sRGB) que el
//! editor muestra en el viewport y el reproductor a pantalla completa.

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3, Vec4};
use maker_engine::components::*;
use maker_engine::math::{directional_light_view_proj, look_at};
use maker_engine::mesh::{Aabb, MeshData};
use maker_engine::scene::Scene;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use wgpu::util::DeviceExt;

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
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ObjectU {
    model: [[f32; 4]; 4],
    normal_m: [[f32; 4]; 4],
    base_color: [f32; 4],
    emissive: [f32; 4],
    params: [f32; 4],
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
}

#[derive(Clone, Copy, Debug, Default)]
pub struct FrameStats {
    pub draw_calls: u32,
    pub triangles: u64,
    pub culled: u32,
    pub shadow_casters: u32,
}

struct GpuMesh {
    vbuf: wgpu::Buffer,
    ibuf: wgpu::Buffer,
    count: u32,
    aabb: Aabb,
}

struct Targets {
    size: (u32, u32),
    msaa: wgpu::TextureView,
    depth: wgpu::TextureView,
    resolve: wgpu::Texture,
    resolve_view: wgpu::TextureView,
}

struct Draw {
    mesh: MeshSource,
    slot: u32,
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
    object_buf: wgpu::Buffer,
    object_bg: wgpu::BindGroup,
    object_capacity: u64,
    opaque: wgpu::RenderPipeline,
    outline: wgpu::RenderPipeline,
    sky: wgpu::RenderPipeline,
    grid: wgpu::RenderPipeline,
    shadow: wgpu::RenderPipeline,
    shadow_view: wgpu::TextureView,
    targets: Option<Targets>,
    meshes: HashMap<MeshSource, GpuMesh>,
    failed_models: HashSet<String>,
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

impl SceneRenderer {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let device = device.clone();
        let queue = queue.clone();
        let main = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("maker main"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders.wgsl").into()),
        });
        let shadow_mod = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("maker shadow"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shadow.wgsl").into()),
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
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                    count: None,
                },
            ],
        });
        let shadow_globals_bgl =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("shadow globals"), entries: &[uniform_entry(0, false)] });
        let object_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("object"), entries: &[uniform_entry(0, true)] });

        let globals_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globals"),
            size: std::mem::size_of::<GlobalsU>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let shadow_globals_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("shadow globals"),
            size: 64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

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
        let object_capacity = 256;
        let (object_buf, object_bg) = Self::make_object_buffer(&device, &object_bgl, object_capacity);

        let main_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("main"),
            bind_group_layouts: &[Some(&globals_bgl), Some(&object_bgl)],
            immediate_size: 0,
        });
        let globals_only = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("globals only"),
            bind_group_layouts: &[Some(&globals_bgl)],
            immediate_size: 0,
        });
        let shadow_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("shadow"),
            bind_group_layouts: &[Some(&shadow_globals_bgl), Some(&object_bgl)],
            immediate_size: 0,
        });

        let vertex_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x2],
        };
        let depth = |write: bool, cmp: wgpu::CompareFunction, bias: wgpu::DepthBiasState| {
            Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(write),
                depth_compare: Some(cmp),
                stencil: Default::default(),
                bias,
            })
        };
        let msaa = wgpu::MultisampleState { count: SAMPLES, mask: !0, alpha_to_coverage_enabled: false };
        let color_target = |blend: Option<wgpu::BlendState>| [Some(wgpu::ColorTargetState { format: COLOR_FORMAT, blend, write_mask: wgpu::ColorWrites::ALL })];

        let mk = |label: &str,
                  layout: &wgpu::PipelineLayout,
                  module: &wgpu::ShaderModule,
                  vs: &str,
                  fs: Option<&str>,
                  buffers: &[Option<wgpu::VertexBufferLayout>],
                  cull: Option<wgpu::Face>,
                  depth_stencil: Option<wgpu::DepthStencilState>,
                  multisample: wgpu::MultisampleState,
                  targets: &[Option<wgpu::ColorTargetState>]| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(layout),
                vertex: wgpu::VertexState { module, entry_point: Some(vs), compilation_options: Default::default(), buffers },
                primitive: wgpu::PrimitiveState { cull_mode: cull, ..Default::default() },
                depth_stencil,
                multisample,
                fragment: fs.map(|fs| wgpu::FragmentState { module, entry_point: Some(fs), compilation_options: Default::default(), targets }),
                multiview_mask: None,
                cache: None,
            })
        };
        let vb = [Some(vertex_layout.clone())];
        let opaque = mk(
            "opaque",
            &main_layout,
            &main,
            "vs_main",
            Some("fs_main"),
            &vb,
            Some(wgpu::Face::Back),
            depth(true, wgpu::CompareFunction::Less, Default::default()),
            msaa,
            &color_target(None),
        );
        let outline = mk(
            "outline",
            &main_layout,
            &main,
            "vs_outline",
            Some("fs_outline"),
            &vb,
            Some(wgpu::Face::Front),
            depth(true, wgpu::CompareFunction::LessEqual, Default::default()),
            msaa,
            &color_target(None),
        );
        let sky = mk("sky", &globals_only, &main, "vs_sky", Some("fs_sky"), &[], None, depth(false, wgpu::CompareFunction::Always, Default::default()), msaa, &color_target(None));
        let grid = mk(
            "grid",
            &globals_only,
            &main,
            "vs_grid",
            Some("fs_grid"),
            &[],
            None,
            depth(false, wgpu::CompareFunction::LessEqual, Default::default()),
            msaa,
            &color_target(Some(wgpu::BlendState::ALPHA_BLENDING)),
        );
        let shadow = mk(
            "shadow",
            &shadow_layout,
            &shadow_mod,
            "vs_shadow",
            None,
            &vb,
            Some(wgpu::Face::Back),
            depth(true, wgpu::CompareFunction::LessEqual, wgpu::DepthBiasState { constant: 2, slope_scale: 2.0, clamp: 0.0 }),
            wgpu::MultisampleState::default(),
            &[],
        );

        Self {
            device,
            queue,
            globals_buf,
            globals_bg,
            shadow_globals_buf,
            shadow_globals_bg,
            object_bgl,
            object_buf,
            object_bg,
            object_capacity,
            opaque,
            outline,
            sky,
            grid,
            shadow,
            shadow_view,
            targets: None,
            meshes: HashMap::new(),
            failed_models: HashSet::new(),
            project_root: None,
            target_generation: 0,
            stats: FrameStats::default(),
        }
    }

    fn make_object_buffer(device: &wgpu::Device, bgl: &wgpu::BindGroupLayout, capacity: u64) -> (wgpu::Buffer, wgpu::BindGroup) {
        let buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("objects"),
            size: capacity * OBJ_STRIDE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("objects"),
            layout: bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &buf,
                    offset: 0,
                    size: wgpu::BufferSize::new(std::mem::size_of::<ObjectU>() as u64),
                }),
            }],
        });
        (buf, bg)
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

    pub fn resize(&mut self, w: u32, h: u32) {
        let (w, h) = (w.clamp(1, 8192), h.clamp(1, 8192));
        if self.targets.as_ref().is_some_and(|t| t.size == (w, h)) {
            return;
        }
        let size = wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 };
        let tex = |label: &str, format, samples, usage| {
            self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size,
                mip_level_count: 1,
                sample_count: samples,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let msaa = tex("msaa color", COLOR_FORMAT, SAMPLES, wgpu::TextureUsages::RENDER_ATTACHMENT).create_view(&Default::default());
        let depth = tex("depth", DEPTH_FORMAT, SAMPLES, wgpu::TextureUsages::RENDER_ATTACHMENT).create_view(&Default::default());
        let resolve = tex(
            "scene output",
            COLOR_FORMAT,
            1,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
        );
        let resolve_view = resolve.create_view(&Default::default());
        self.targets = Some(Targets { size: (w, h), msaa, depth, resolve, resolve_view });
        self.target_generation += 1;
    }

    /// AABB local de una malla (la carga si hace falta). Para selección.
    pub fn mesh_aabb(&mut self, src: &MeshSource) -> Option<Aabb> {
        self.ensure_mesh(src);
        self.meshes.get(src).map(|m| m.aabb)
    }

    /// Olvida modelos cacheados (tras reimportar archivos).
    pub fn clear_model_cache(&mut self) {
        self.meshes.retain(|k, _| matches!(k, MeshSource::Primitive(_)));
        self.failed_models.clear();
    }

    fn ensure_mesh(&mut self, src: &MeshSource) -> bool {
        if self.meshes.contains_key(src) {
            return true;
        }
        let data = match src {
            MeshSource::Primitive(p) => MeshData::primitive(*p),
            MeshSource::Model(path) => {
                if self.failed_models.contains(path) {
                    return false;
                }
                let full = match &self.project_root {
                    Some(r) => r.join(path),
                    None => PathBuf::from(path),
                };
                match MeshData::load_gltf(&full) {
                    Ok(d) => {
                        maker_engine::log::info("render", format!("modelo cargado: {path} ({} triángulos)", d.indices.len() / 3));
                        d
                    }
                    Err(e) => {
                        maker_engine::log::error("render", e);
                        self.failed_models.insert(path.clone());
                        return false;
                    }
                }
            }
        };
        let verts: Vec<Vertex> = (0..data.positions.len())
            .map(|i| Vertex {
                pos: data.positions[i].to_array(),
                normal: data.normals.get(i).copied().unwrap_or(Vec3::Y).to_array(),
                uv: data.uvs.get(i).copied().unwrap_or_default().to_array(),
            })
            .collect();
        let vbuf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("mesh vertices"),
            contents: bytemuck::cast_slice(&verts),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let ibuf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("mesh indices"),
            contents: bytemuck::cast_slice(&data.indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        self.meshes.insert(src.clone(), GpuMesh { vbuf, ibuf, count: data.indices.len() as u32, aabb: data.aabb() });
        true
    }

    /// Dibuja la escena en la textura de salida y envía los comandos a la GPU.
    pub fn render(&mut self, scene: &Scene, view: &RenderView, opts: &RenderOptions) {
        let Some((w, h)) = self.targets.as_ref().map(|t| t.size) else { return };
        let _ = (w, h);
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

        // Mapa de sombras centrado delante de la cámara, ajustado a texel.
        let shadow_radius = 22.0;
        let light_vp = if let Some((dir, _, true)) = sun {
            let fwd = view.view.inverse().transform_vector3(Vec3::NEG_Z);
            let focus = view.eye + fwd * shadow_radius * 0.6;
            let vp = directional_light_view_proj(dir, focus, shadow_radius);
            let texel = 2.0 * shadow_radius / SHADOW_SIZE as f32;
            let p = vp.project_point3(focus);
            let _ = p;
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
        let mut pts = [PointU::default(); MAX_POINTS];
        for (i, (_, p)) in points.iter().enumerate() {
            pts[i] = *p;
        }
        let globals = GlobalsU {
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
            params: [points.len() as f32, env.exposure, 0.0, SHADOW_SIZE as f32],
            points: pts,
        };
        self.queue.write_buffer(&self.globals_buf, 0, bytemuck::bytes_of(&globals));
        self.queue.write_buffer(&self.shadow_globals_buf, 0, bytemuck::bytes_of(&light_vp.to_cols_array_2d()));

        // Lista de dibujo + uniformes por objeto.
        let mut items: Vec<(EntityId, MeshRenderer, Mat4)> = Vec::new();
        for (id, mr, gt) in scene.world.query::<(&EntityId, &MeshRenderer, &GlobalTransform)>().iter() {
            if !hidden.contains(id) {
                items.push((*id, mr.clone(), gt.0));
            }
        }
        let mut draws: Vec<Draw> = Vec::with_capacity(items.len());
        let mut bytes: Vec<u8> = Vec::with_capacity(items.len() * OBJ_STRIDE as usize);
        for (id, mr, m) in items {
            if !self.ensure_mesh(&mr.mesh) {
                continue;
            }
            let aabb = self.meshes[&mr.mesh].aabb;
            let visible = !outside_frustum(&view_proj, &aabb.transformed(&m));
            if !visible {
                stats.culled += 1;
            }
            if !visible && !(mr.cast_shadows && sun.is_some_and(|s| s.2)) {
                continue;
            }
            let mat = &mr.material;
            let bc = lin3([mat.base_color[0], mat.base_color[1], mat.base_color[2]]);
            let obj = ObjectU {
                model: m.to_cols_array_2d(),
                normal_m: m.inverse().transpose().to_cols_array_2d(),
                base_color: v4(bc, mat.base_color[3]),
                emissive: v4(lin3(mat.emissive), mat.emissive_strength),
                params: [mat.metallic, mat.roughness, if mat.unlit { 1.0 } else { 0.0 }, 0.0],
            };
            let slot = draws.len() as u32;
            bytes.extend_from_slice(bytemuck::bytes_of(&obj));
            bytes.resize((slot as usize + 1) * OBJ_STRIDE as usize, 0);
            draws.push(Draw { mesh: mr.mesh, slot, shadow: mr.cast_shadows, visible, selected: opts.selected.contains(&id) });
        }
        if draws.len() as u64 > self.object_capacity {
            let cap = (draws.len() as u64).next_power_of_two();
            let (b, bg) = Self::make_object_buffer(&self.device, &self.object_bgl, cap);
            self.object_buf = b;
            self.object_bg = bg;
            self.object_capacity = cap;
        }
        if !bytes.is_empty() {
            self.queue.write_buffer(&self.object_buf, 0, &bytes);
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
            if sun.is_some_and(|s| s.2) {
                pass.set_pipeline(&self.shadow);
                pass.set_bind_group(0, &self.shadow_globals_bg, &[]);
                for d in draws.iter().filter(|d| d.shadow) {
                    let m = &self.meshes[&d.mesh];
                    pass.set_bind_group(1, &self.object_bg, &[(d.slot as u64 * OBJ_STRIDE) as u32]);
                    pass.set_vertex_buffer(0, m.vbuf.slice(..));
                    pass.set_index_buffer(m.ibuf.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..m.count, 0, 0..1);
                    stats.shadow_casters += 1;
                }
            }
        }
        // Color.
        {
            let t = self.targets.as_ref().unwrap();
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("main pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &t.msaa,
                    depth_slice: None,
                    resolve_target: Some(&t.resolve_view),
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Discard },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &t.depth,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Discard }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &self.globals_bg, &[]);
            pass.set_pipeline(&self.sky);
            pass.draw(0..3, 0..1);
            pass.set_pipeline(&self.opaque);
            for d in draws.iter().filter(|d| d.visible) {
                let m = &self.meshes[&d.mesh];
                pass.set_bind_group(1, &self.object_bg, &[(d.slot as u64 * OBJ_STRIDE) as u32]);
                pass.set_vertex_buffer(0, m.vbuf.slice(..));
                pass.set_index_buffer(m.ibuf.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..m.count, 0, 0..1);
                stats.draw_calls += 1;
                stats.triangles += m.count as u64 / 3;
            }
            if draws.iter().any(|d| d.selected && d.visible) {
                pass.set_pipeline(&self.outline);
                for d in draws.iter().filter(|d| d.selected && d.visible) {
                    let m = &self.meshes[&d.mesh];
                    pass.set_bind_group(1, &self.object_bg, &[(d.slot as u64 * OBJ_STRIDE) as u32]);
                    pass.set_vertex_buffer(0, m.vbuf.slice(..));
                    pass.set_index_buffer(m.ibuf.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..m.count, 0, 0..1);
                    stats.draw_calls += 1;
                }
            }
            if opts.show_grid {
                pass.set_pipeline(&self.grid);
                pass.set_bind_group(0, &self.globals_bg, &[]);
                pass.draw(0..6, 0..1);
            }
        }
        self.queue.submit(Some(enc.finish()));
        self.stats = stats;
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
            wgpu::TexelCopyBufferInfo {
                buffer: &buf,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(bpr), rows_per_image: Some(h) },
            },
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
mod tests {
    use super::*;

    fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).ok()?;
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()
    }

    /// Renderiza la escena inicial sin ventana y comprueba que el cubo azul,
    /// el suelo y el cielo aparecen donde deben.
    #[test]
    fn renders_starter_scene_headless() {
        let Some((device, queue)) = device() else {
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
    }
}
