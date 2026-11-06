//! Pila de posprocesado: la escena se dibuja en un objetivo HDR
//! (`Rgba16Float`), los efectos se encadenan alternando dos texturas y el
//! último escribe en la salida sRGB. Los objetivos solo se crean al cambiar
//! de tamaño; los efectos apagados no cuestan nada y, sin efectos, el
//! tonemap se hace en el propio shader de la escena (cero pases extra).
//!
//! Los efectos propios del proyecto (`shaders/post/*.wgsl`) se compilan con
//! un preludio de enlaces fijos (`post_preludio.wgsl`), se recompilan al
//! cambiar el archivo y, si fallan, se omiten con el error en la consola.

use crate::renderer::{COLOR_FORMAT, RenderView, lin3};
use bytemuck::{Pod, Zeroable};
use maker_engine::log;
use maker_engine::postproceso::{CustomParam, ParamKind, Plan, PostKind, Step, parse_params, plan};
use maker_engine::scene::Environment;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

pub const HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
pub const PRELUDE: &str = include_str!("post_preludio.wgsl");
const BUILTIN: &str = include_str!("post_efectos.wgsl");
/// Entry point de los efectos propios.
pub const CUSTOM_ENTRY: &str = "efecto";
/// Cada cuánto se mira si cambió un efecto propio.
const RELOAD_CHECK: Duration = Duration::from_millis(500);
/// Niveles máximos de la cadena de bloom (cada uno a la mitad del anterior).
const MAX_BLOOM_LEVELS: usize = 8;

/// Uniforme de cada pase (`struct Post` del preludio).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct PostU {
    resolucion: [f32; 4],
    texel: [f32; 4],
    tiempo: f32,
    intensidad: f32,
    escala: f32,
    hdr: f32,
    inv_vista_proy: [[f32; 4]; 4],
    camara: [f32; 4],
    p: [[f32; 4]; 16],
}

const _: () = assert!(std::mem::size_of::<PostU>() == 384);

/// Origen del shader de un pipeline.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Src {
    Builtin(&'static str),
    Custom(String),
}

/// Textura de entrada o de salida de un pase.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Tex {
    Scene,
    Ping(u8),
    Bloom(u8),
    Lut(String),
    White,
}

#[derive(Clone, Debug, PartialEq)]
enum Out {
    Tex(Tex),
    Final,
}

struct Pass {
    src: Src,
    input: Tex,
    aux: Tex,
    out: Out,
    additive: bool,
    u: PostU,
}

struct Target {
    view: wgpu::TextureView,
    size: (u32, u32),
}

struct Targets {
    size: (u32, u32),
    msaa: wgpu::TextureView,
    scene: Target,
    ping: [Target; 2],
    bloom: Vec<Target>,
}

struct Custom {
    mtime: Option<SystemTime>,
    params: Vec<CustomParam>,
    module: Option<wgpu::ShaderModule>,
    error: Option<String>,
    checked: Instant,
}

/// Lo que el Inspector necesita de un efecto propio.
pub struct CustomInfo<'a> {
    pub params: &'a [CustomParam],
    pub error: Option<&'a str>,
}

pub struct PostFx {
    device: wgpu::Device,
    queue: wgpu::Queue,
    bgl: wgpu::BindGroupLayout,
    layout: wgpu::PipelineLayout,
    builtin: wgpu::ShaderModule,
    sampler: wgpu::Sampler,
    pipelines: HashMap<(Src, wgpu::TextureFormat, bool), wgpu::RenderPipeline>,
    targets: Option<Targets>,
    depth: Option<wgpu::TextureView>,
    uniform: wgpu::Buffer,
    stride: u64,
    slots: u64,
    bind_groups: HashMap<(Tex, Tex), wgpu::BindGroup>,
    customs: HashMap<String, Custom>,
    luts: HashMap<String, Option<(wgpu::TextureView, SystemTime)>>,
    white: wgpu::TextureView,
    start: Instant,
    bytes: Vec<u8>,
    /// Pases de posprocesado del último fotograma.
    pub passes: u32,
}

fn tex_2d(device: &wgpu::Device, label: &str, (w, h): (u32, u32), format: wgpu::TextureFormat, samples: u32, usage: wgpu::TextureUsages) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: samples,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        })
        .create_view(&Default::default())
}

fn resolve_path(root: Option<&Path>, rel: &str) -> PathBuf {
    match root {
        Some(r) => r.join(rel),
        None => PathBuf::from(rel),
    }
}

impl PostFx {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let tex_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
            count: None,
        };
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("post"),
            entries: &[
                tex_entry(0),
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: true },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: true, min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<PostU>() as u64) },
                    count: None,
                },
                tex_entry(4),
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("post"), bind_group_layouts: &[Some(&bgl)], immediate_size: 0 });
        let builtin = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("post builtin"),
            source: wgpu::ShaderSource::Wgsl(format!("{PRELUDE}\n{BUILTIN}").into()),
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("post"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        // Alineación de los offsets dinámicos: la del dispositivo, no un 256 supuesto.
        let align = device.limits().min_uniform_buffer_offset_alignment.max(1) as u64;
        let stride = (std::mem::size_of::<PostU>() as u64).div_ceil(align) * align;
        let slots = 16;
        let uniform = Self::make_uniform(device, stride * slots);
        let white = {
            let t = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("post white"),
                size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            queue.write_texture(
                t.as_image_copy(),
                &[255; 4],
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4), rows_per_image: Some(1) },
                wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
            );
            t.create_view(&Default::default())
        };
        Self {
            device: device.clone(),
            queue: queue.clone(),
            bgl,
            layout,
            builtin,
            sampler,
            pipelines: HashMap::new(),
            targets: None,
            depth: None,
            uniform,
            stride,
            slots,
            bind_groups: HashMap::new(),
            customs: HashMap::new(),
            luts: HashMap::new(),
            white,
            start: Instant::now(),
            bytes: Vec::new(),
            passes: 0,
        }
    }

    fn make_uniform(device: &wgpu::Device, size: u64) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor { label: Some("post uniforms"), size, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false })
    }

    /// Objetivos HDR del tamaño de la salida (solo se recrean al cambiar).
    pub(crate) fn ensure_targets(&mut self, size: (u32, u32), samples: u32, depth: &wgpu::TextureView) {
        if self.targets.as_ref().is_some_and(|t| t.size == size) && self.depth.as_ref() == Some(depth) {
            return;
        }
        let d = &self.device;
        let rt = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
        let target = |label: &str, s: (u32, u32)| Target { view: tex_2d(d, label, s, HDR_FORMAT, 1, rt), size: s };
        let mut bloom = Vec::new();
        for i in 1..=MAX_BLOOM_LEVELS as u32 {
            let s = ((size.0 >> i).max(1), (size.1 >> i).max(1));
            if i > 1 && s.0.min(s.1) < 4 {
                break;
            }
            bloom.push(target("bloom", s));
        }
        self.targets = Some(Targets {
            size,
            msaa: tex_2d(d, "hdr msaa", size, HDR_FORMAT, samples, wgpu::TextureUsages::RENDER_ATTACHMENT),
            scene: target("hdr scene", size),
            ping: [target("post ping", size), target("post pong", size)],
            bloom,
        });
        self.depth = Some(depth.clone());
        self.bind_groups.clear();
    }

    /// (MSAA, resuelto) donde se dibuja la escena en HDR.
    pub(crate) fn scene_attachment(&self) -> Option<(&wgpu::TextureView, &wgpu::TextureView)> {
        self.targets.as_ref().map(|t| (&t.msaa, &t.scene.view))
    }

    /// Plan del fotograma. Recarga los efectos propios que cambiaron y omite
    /// los que no compilan.
    pub fn plan(&mut self, env: &Environment, root: Option<&Path>) -> Plan {
        for e in &env.post {
            if let PostKind::Custom(c) = &e.kind {
                self.refresh_custom(&c.path, root, false);
            }
        }
        plan(&env.post, |_, e| match &e.kind {
            PostKind::Custom(c) => self.customs.get(&c.path).is_some_and(|c| c.module.is_some()),
            _ => true,
        })
    }

    /// Parámetros y error de un efecto propio (lo carga si hace falta).
    pub fn custom_info(&mut self, path: &str, root: Option<&Path>) -> CustomInfo<'_> {
        self.refresh_custom(path, root, false);
        let c = &self.customs[path];
        CustomInfo { params: &c.params, error: c.error.as_deref() }
    }

    /// Fuerza la recompilación de un efecto propio en el siguiente uso.
    pub fn reload_custom(&mut self, path: &str, root: Option<&Path>) {
        self.refresh_custom(path, root, true);
    }

    fn refresh_custom(&mut self, rel: &str, root: Option<&Path>, force: bool) {
        if let Some(c) = self.customs.get(rel)
            && !force
            && c.checked.elapsed() < RELOAD_CHECK
        {
            return;
        }
        let full = resolve_path(root, rel);
        let mtime = std::fs::metadata(&full).and_then(|m| m.modified()).ok();
        if let Some(c) = self.customs.get_mut(rel) {
            c.checked = Instant::now();
            if c.mtime == mtime && !force {
                return;
            }
        }
        let had_error = self.customs.get(rel).is_some_and(|c| c.error.is_some());
        let result = match std::fs::read_to_string(&full) {
            Ok(src) => parse_params(&src).map_err(|e| format!("{rel}: {e}")).and_then(|params| self.compile(rel, &src, &params).map(|m| (params, m))),
            Err(e) => Err(format!("{rel}: {e}")),
        };
        self.pipelines.retain(|(src, _, _), _| *src != Src::Custom(rel.to_string()));
        let c = match result {
            Ok((params, module)) => {
                if had_error || self.customs.contains_key(rel) {
                    log::info("posprocesado", format!("{rel} compilado"));
                }
                Custom { mtime, params, module: Some(module), error: None, checked: Instant::now() }
            }
            Err(e) => {
                log::error("posprocesado", format!("{e}\nel efecto se omite hasta que compile"));
                let params = self.customs.get(rel).map(|c| c.params.clone()).unwrap_or_default();
                Custom { mtime, params, module: None, error: Some(e), checked: Instant::now() }
            }
        };
        self.customs.insert(rel.to_string(), c);
    }

    /// Código completo de un efecto propio: el del usuario primero (las líneas
    /// de los errores coinciden con su archivo), luego el preludio y los
    /// accesos a sus parámetros.
    pub fn custom_source(src: &str, params: &[CustomParam]) -> String {
        let mut full = String::with_capacity(src.len() + PRELUDE.len() + 64 * params.len());
        full.push_str(src);
        full.push_str("\n\n");
        full.push_str(PRELUDE);
        for (i, p) in params.iter().enumerate() {
            match p.kind {
                ParamKind::Float { .. } => full.push_str(&format!("fn {}() -> f32 {{ return post.p[{i}].x; }}\n", p.name)),
                ParamKind::Color => full.push_str(&format!("fn {}() -> vec3<f32> {{ return post.p[{i}].rgb; }}\n", p.name)),
            }
        }
        full
    }

    /// Valida con naga (mensajes con línea y columna) y crea el módulo y un
    /// pipeline dentro de un ámbito de errores: un shader roto nunca tumba el
    /// editor.
    fn compile(&mut self, rel: &str, src: &str, params: &[CustomParam]) -> Result<wgpu::ShaderModule, String> {
        use wgpu::naga;
        let full = Self::custom_source(src, params);
        let module = naga::front::wgsl::parse_str(&full).map_err(|e| e.emit_to_string_with_path(&full, rel))?;
        if !module.entry_points.iter().any(|e| e.name == CUSTOM_ENTRY && e.stage == naga::ShaderStage::Fragment) {
            return Err(format!("{rel}: falta `@fragment fn {CUSTOM_ENTRY}(in: PostIn) -> @location(0) vec4<f32>`"));
        }
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
            .validate(&module)
            .map_err(|e| e.emit_to_string_with_path(&full, rel))?;
        let scope = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let m = self.device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some(rel), source: wgpu::ShaderSource::Wgsl(full.into()) });
        let p = self.make_pipeline(&m, CUSTOM_ENTRY, COLOR_FORMAT, false);
        if let Some(e) = pollster::block_on(scope.pop()) {
            return Err(format!("{rel}: {e}"));
        }
        self.pipelines.insert((Src::Custom(rel.to_string()), COLOR_FORMAT, false), p);
        Ok(m)
    }

    fn make_pipeline(&self, module: &wgpu::ShaderModule, fs: &str, format: wgpu::TextureFormat, additive: bool) -> wgpu::RenderPipeline {
        let blend = additive.then_some(wgpu::BlendState {
            color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
            alpha: wgpu::BlendComponent::REPLACE,
        });
        self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(fs),
            layout: Some(&self.layout),
            vertex: wgpu::VertexState { module, entry_point: Some("vs_post"), compilation_options: Default::default(), buffers: &[] },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module,
                entry_point: Some(fs),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format, blend, write_mask: wgpu::ColorWrites::ALL })],
            }),
            multiview_mask: None,
            cache: None,
        })
    }

    fn ensure_pipeline(&mut self, src: &Src, format: wgpu::TextureFormat, additive: bool) -> bool {
        let key = (src.clone(), format, additive);
        if self.pipelines.contains_key(&key) {
            return true;
        }
        let p = match src {
            Src::Builtin(fs) => self.make_pipeline(&self.builtin, fs, format, additive),
            Src::Custom(path) => {
                let Some(m) = self.customs.get(path).and_then(|c| c.module.clone()) else { return false };
                let scope = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
                let p = self.make_pipeline(&m, CUSTOM_ENTRY, format, additive);
                if let Some(e) = pollster::block_on(scope.pop()) {
                    log::error("posprocesado", format!("{path}: {e}"));
                    if let Some(c) = self.customs.get_mut(path) {
                        c.module = None;
                        c.error = Some(e.to_string());
                    }
                    return false;
                }
                p
            }
        };
        self.pipelines.insert(key, p);
        true
    }

    /// LUT en tira (N²×N) sin conversión sRGB: los valores se usan tal cual.
    fn lut(&mut self, rel: &str, root: Option<&Path>) -> bool {
        let full = resolve_path(root, rel);
        let mtime = std::fs::metadata(&full).and_then(|m| m.modified()).ok();
        if let Some(entry) = self.luts.get(rel) {
            match (entry, mtime) {
                (Some((_, t)), Some(m)) if *t == m => return true,
                (None, _) if mtime.is_none() => return false,
                _ => {}
            }
        }
        let loaded = std::fs::read(&full).map_err(|e| e.to_string()).and_then(|b| crate::texture::decode(&b)).and_then(|(w, h, px)| {
            if w != h * h {
                return Err(format!("{w}×{h}: una LUT en tira mide N²×N (p. ej. 256×16)"));
            }
            let t = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(rel),
                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            self.queue.write_texture(
                t.as_image_copy(),
                &px,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 4), rows_per_image: Some(h) },
                wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            );
            Ok(t.create_view(&Default::default()))
        });
        let ok = match loaded {
            Ok(v) => {
                self.luts.insert(rel.to_string(), Some((v, mtime.unwrap_or(SystemTime::UNIX_EPOCH))));
                true
            }
            Err(e) => {
                log::error("posprocesado", format!("LUT {rel}: {e}"));
                self.luts.insert(rel.to_string(), None);
                false
            }
        };
        self.bind_groups.retain(|(_, aux), _| *aux != Tex::Lut(rel.to_string()));
        ok
    }

    fn view(&self, t: &Tex) -> &wgpu::TextureView {
        let tg = self.targets.as_ref().expect("objetivos de posprocesado");
        match t {
            Tex::Scene => &tg.scene.view,
            Tex::Ping(i) => &tg.ping[*i as usize].view,
            Tex::Bloom(i) => &tg.bloom[*i as usize].view,
            Tex::Lut(p) => self.luts.get(p).and_then(|l| l.as_ref()).map(|l| &l.0).unwrap_or(&self.white),
            Tex::White => &self.white,
        }
    }

    fn size_of(&self, t: &Tex) -> (u32, u32) {
        let tg = self.targets.as_ref().expect("objetivos de posprocesado");
        match t {
            Tex::Bloom(i) => tg.bloom[*i as usize].size,
            Tex::Ping(i) => tg.ping[*i as usize].size,
            _ => tg.scene.size,
        }
    }

    fn bind_group(&mut self, input: &Tex, aux: &Tex) {
        let key = (input.clone(), aux.clone());
        if self.bind_groups.contains_key(&key) {
            return;
        }
        let depth = self.depth.as_ref().expect("profundidad");
        let bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("post"),
            layout: &self.bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(self.view(input)) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(depth) },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: &self.uniform, offset: 0, size: wgpu::BufferSize::new(std::mem::size_of::<PostU>() as u64) }),
                },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(self.view(aux)) },
            ],
        });
        self.bind_groups.insert(key, bg);
    }

    /// Codifica los pases del plan: de la escena HDR a `output` (sRGB).
    pub(crate) fn run(&mut self, enc: &mut wgpu::CommandEncoder, env: &Environment, plan: &Plan, view: &RenderView, root: Option<&Path>, output: &wgpu::TextureView) {
        self.passes = 0;
        let Some(size) = self.targets.as_ref().map(|t| t.size) else { return };
        if plan.steps.is_empty() {
            return;
        }
        let base = PostU {
            resolucion: [0.0; 4],
            texel: [0.0; 4],
            tiempo: self.start.elapsed().as_secs_f32(),
            intensidad: 1.0,
            escala: size.1 as f32 / 1080.0,
            hdr: 0.0,
            inv_vista_proy: view.view_proj().inverse().to_cols_array_2d(),
            camara: [view.eye.x, view.eye.y, view.eye.z, 1.0],
            p: [[0.0; 4]; 16],
        };
        let bloom_levels = self.targets.as_ref().map(|t| t.bloom.len()).unwrap_or(0);
        let mut passes: Vec<Pass> = Vec::new();
        let mut input = Tex::Scene;
        let mut next_ping = 0u8;
        let mut hdr = plan.scene_tonemap.is_none();
        let last = plan.steps.len() - 1;
        for (si, step) in plan.steps.iter().enumerate() {
            let out = if si == last { Out::Final } else { Out::Tex(Tex::Ping(next_ping)) };
            let mut u = PostU { hdr: hdr as u32 as f32, ..base };
            let pass = |src: &'static str, input: Tex, aux: Tex, out: Out, u: PostU| Pass { src: Src::Builtin(src), input, aux, out, additive: false, u };
            match step {
                Step::Tonemap { op, exposure_ev } => {
                    u.p[0] = [op.code() as f32, 2f32.powf(*exposure_ev), 0.0, 0.0];
                    passes.push(pass("fs_tonemap", input.clone(), Tex::White, out.clone(), u));
                    hdr = false;
                }
                Step::Effect(i) => {
                    let e = &env.post[*i];
                    u.intensidad = e.intensity;
                    match &e.kind {
                        PostKind::Tonemap(_) => {}
                        PostKind::ColorGrading(g) => {
                            let has_lut = !g.lut.is_empty() && self.lut(&g.lut, root);
                            u.p[0] = [g.contrast, g.saturation, g.temperature, g.tint];
                            u.p[1] = [g.lift[0], g.lift[1], g.lift[2], has_lut as u32 as f32];
                            u.p[2] = [g.gamma[0], g.gamma[1], g.gamma[2], 0.0];
                            u.p[3] = [g.gain[0], g.gain[1], g.gain[2], 0.0];
                            let aux = if has_lut { Tex::Lut(g.lut.clone()) } else { Tex::White };
                            passes.push(pass("fs_grading", input.clone(), aux, out.clone(), u));
                        }
                        PostKind::Bloom(b) if bloom_levels > 0 => {
                            let levels = (1 + (b.radius.clamp(0.0, 1.0) * (bloom_levels - 1) as f32).round() as usize).min(bloom_levels);
                            u.p[0] = [b.threshold.max(0.0), 0.0, 0.0, 0.0];
                            passes.push(pass("fs_bloom_umbral", input.clone(), Tex::White, Out::Tex(Tex::Bloom(0)), u));
                            for l in 0..levels - 1 {
                                passes.push(pass("fs_bloom_reduce", Tex::Bloom(l as u8), Tex::White, Out::Tex(Tex::Bloom(l as u8 + 1)), u));
                            }
                            for l in (0..levels - 1).rev() {
                                passes.push(Pass { additive: true, ..pass("fs_bloom_amplia", Tex::Bloom(l as u8 + 1), Tex::White, Out::Tex(Tex::Bloom(l as u8)), u) });
                            }
                            u.p[0] = [1.0 / levels as f32, 0.0, 0.0, 0.0];
                            passes.push(pass("fs_bloom_mezcla", input.clone(), Tex::Bloom(0), out.clone(), u));
                        }
                        PostKind::Bloom(_) => continue,
                        PostKind::Vignette(v) => {
                            u.p[0] = [v.radius, v.softness, 0.0, 0.0];
                            u.p[1] = lin4(v.color);
                            passes.push(pass("fs_vineta", input.clone(), Tex::White, out.clone(), u));
                        }
                        PostKind::ChromaticAberration(c) => {
                            u.p[0] = [c.amount, 0.0, 0.0, 0.0];
                            passes.push(pass("fs_aberracion", input.clone(), Tex::White, out.clone(), u));
                        }
                        PostKind::FilmGrain(g) => {
                            u.p[0] = [g.size, 0.0, 0.0, 0.0];
                            passes.push(pass("fs_grano", input.clone(), Tex::White, out.clone(), u));
                        }
                        PostKind::Retro(r) => {
                            u.p[0] = [r.pixel_size, r.levels as f32, 0.0, 0.0];
                            passes.push(pass("fs_retro", input.clone(), Tex::White, out.clone(), u));
                        }
                        PostKind::Sharpen(s) => {
                            u.p[0] = [s.radius, 0.0, 0.0, 0.0];
                            passes.push(pass("fs_enfoque", input.clone(), Tex::White, out.clone(), u));
                        }
                        PostKind::Fxaa => passes.push(pass("fs_fxaa", input.clone(), Tex::White, out.clone(), u)),
                        PostKind::EdgeOutline(o) => {
                            u.p[0] = [o.thickness, o.depth_threshold, o.normal_threshold, 0.0];
                            u.p[1] = lin4(o.color);
                            passes.push(pass("fs_contorno", input.clone(), Tex::White, out.clone(), u));
                        }
                        PostKind::HeightFog(f) => {
                            u.p[0] = [f.density, f.height, f.falloff, 0.0];
                            u.p[1] = lin4(f.color);
                            passes.push(pass("fs_niebla_altura", input.clone(), Tex::White, out.clone(), u));
                        }
                        PostKind::DepthOfField(d) => {
                            u.p[0] = [d.focus, d.range, d.max_blur, 0.0];
                            passes.push(pass("fs_profundidad_campo", input.clone(), Tex::White, out.clone(), u));
                        }
                        PostKind::Custom(c) => {
                            let Some(cu) = self.customs.get(&c.path) else { continue };
                            for (k, p) in cu.params.iter().enumerate() {
                                let v = c.values.get(&p.name).copied().unwrap_or(p.default);
                                u.p[k] = match p.kind {
                                    ParamKind::Float { .. } => v,
                                    ParamKind::Color => lin4([v[0], v[1], v[2]]),
                                };
                            }
                            passes.push(Pass { src: Src::Custom(c.path.clone()), input: input.clone(), aux: Tex::Scene, out: out.clone(), additive: false, u });
                        }
                    }
                }
            }
            if let Out::Tex(t) = out {
                input = t;
                next_ping ^= 1;
            }
        }
        // Si el último paso se omitió, el resultado aún está en un intermedio: copia con el tonemap neutro.
        if passes.last().is_none_or(|p| p.out != Out::Final) {
            let mut u = base;
            u.p[0] = [3.0, 1.0, 0.0, 0.0];
            passes.push(Pass { src: Src::Builtin("fs_tonemap"), input: input.clone(), aux: Tex::White, out: Out::Final, additive: false, u });
        }

        // Uniformes de todos los pases en una sola escritura.
        let n = passes.len() as u64;
        if n > self.slots {
            self.slots = n.next_power_of_two();
            self.uniform = Self::make_uniform(&self.device, self.stride * self.slots);
            self.bind_groups.clear();
        }
        self.bytes.clear();
        for p in passes.iter_mut() {
            let in_size = self.size_of(&p.input);
            let out_size = match &p.out {
                Out::Final => size,
                Out::Tex(t) => self.size_of(t),
            };
            p.u.resolucion = [out_size.0 as f32, out_size.1 as f32, 1.0 / out_size.0 as f32, 1.0 / out_size.1 as f32];
            p.u.texel = [1.0 / in_size.0 as f32, 1.0 / in_size.1 as f32, in_size.0 as f32, in_size.1 as f32];
            self.bytes.extend_from_slice(bytemuck::bytes_of(&p.u));
            self.bytes.resize(self.bytes.len().next_multiple_of(self.stride as usize), 0);
        }
        self.queue.write_buffer(&self.uniform, 0, &self.bytes);
        for p in &passes {
            let format = if p.out == Out::Final { COLOR_FORMAT } else { HDR_FORMAT };
            if !self.ensure_pipeline(&p.src, format, p.additive) {
                // Efecto propio que falló al crear su pipeline: se salta este fotograma.
                continue;
            }
            self.bind_group(&p.input, &p.aux);
        }
        for (i, p) in passes.iter().enumerate() {
            let format = if p.out == Out::Final { COLOR_FORMAT } else { HDR_FORMAT };
            let (Some(pipe), Some(bg)) = (self.pipelines.get(&(p.src.clone(), format, p.additive)), self.bind_groups.get(&(p.input.clone(), p.aux.clone()))) else {
                continue;
            };
            let target = match &p.out {
                Out::Final => output,
                Out::Tex(t) => self.view(t),
            };
            let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("post"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: if p.additive { wgpu::LoadOp::Load } else { wgpu::LoadOp::Clear(wgpu::Color::BLACK) }, store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            rp.set_pipeline(pipe);
            rp.set_bind_group(0, bg, &[(i as u64 * self.stride) as u32]);
            rp.draw(0..3, 0..1);
            self.passes += 1;
        }
    }
}

fn lin4(c: [f32; 3]) -> [f32; 4] {
    let l = lin3(c);
    [l[0], l[1], l[2], 1.0]
}
