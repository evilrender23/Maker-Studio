//! Iluminación global dinámica y reflejos ("Lumen (software)").
//!
//! Híbrido por pasos, todo en este módulo; el renderizador solo lo llama
//! entre el mapa de sombras y el pase de color y enlaza el resultado como
//! grupo 3 del sombreado:
//!
//! 1. **Voxelizar** (`voxelizar.wgsl`): clipmap de 3–4 cascadas de N³ vóxeles
//!    centradas en la cámara. Cada cascada solo se rehace cuando cambia algo
//!    dentro (huella de transform, malla y material de lo que la toca) o
//!    cuando su origen salta; lo estático queda en caché.
//! 2. **Inyectar** (`inyectar.wgsl`): luz directa (sol con el mapa de
//!    sombras, puntuales con oclusión por vóxeles, emisión) + realimentación
//!    de la radiancia del fotograma anterior (rebotes múltiples).
//! 3. **Prepase**: normal, rugosidad y profundidad a la resolución del
//!    trazado (½ o ¼ de la salida).
//! 4. **Recoger** (`recoger.wgsl`): por píxel, rayos difusos y un rayo de
//!    reflejo; primero contra la profundidad de la pantalla, después conos
//!    por el volumen y por último el cielo.
//! 5. **Temporal + filtro** (`temporal.wgsl`, `filtrar.wgsl`): reproyección
//!    con rechazo por geometría y dos pasadas à-trous.
//!
//! Todos los recursos se crean solo con la GI activa, se reutilizan entre
//! fotogramas y los de pantalla se rehacen al cambiar de tamaño.

use crate::renderer::{GpuModel, RenderView, lin3};
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use maker_engine::iluminacion::{CalidadGi, IluminacionGlobal};
use maker_engine::scene::Environment;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

const COMUN: &str = include_str!("comun.wgsl");
const CONO: &str = include_str!("cono.wgsl");
const GB_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const GI_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const DEPTH_FORMAT: wgpu::TextureFormat = crate::renderer::DEPTH_FORMAT;
/// Palabras por vóxel del acumulador (albedo rgb + cuenta, normal xyz,
/// emisión rgb, orientación sin signo xyz).
const PALABRAS: u64 = 13;
pub const MAX_CASCADAS: usize = 4;

/// Vista de depuración de la GI (barra del visor).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum GiDebug {
    #[default]
    Ninguna,
    /// El volumen de vóxeles visto desde la cámara (radiancia).
    Voxeles,
    /// Solo la luz indirecta difusa (sobre blanco).
    Indirecta,
    /// Solo los reflejos.
    Reflejos,
}

impl GiDebug {
    pub const ALL: [GiDebug; 4] = [GiDebug::Ninguna, GiDebug::Voxeles, GiDebug::Indirecta, GiDebug::Reflejos];
    pub fn label(self) -> &'static str {
        match self {
            GiDebug::Ninguna => "Imagen final",
            GiDebug::Voxeles => "Vóxeles",
            GiDebug::Indirecta => "Solo luz indirecta",
            GiDebug::Reflejos => "Solo reflejos",
        }
    }
    pub(crate) fn code(self) -> f32 {
        match self {
            GiDebug::Ninguna => 0.0,
            GiDebug::Voxeles => 1.0,
            GiDebug::Indirecta => 2.0,
            GiDebug::Reflejos => 3.0,
        }
    }
}

/// Parámetros de un nivel de calidad.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Calidad {
    /// Vóxeles por lado de cada cascada.
    pub n: u32,
    pub cascadas: u32,
    /// Tamaño de vóxel de la cascada 0 (m); cada cascada lo dobla.
    pub voxel: f32,
    /// Divisor de la resolución del trazado en pantalla.
    pub div: u32,
    /// Rayos difusos por píxel y fotograma.
    pub rayos: u32,
    /// Pasos del trazado en pantalla de los reflejos.
    pub pasos_reflejo: u32,
}

pub fn calidad(c: CalidadGi) -> Calidad {
    match c {
        CalidadGi::Baja => Calidad { n: 48, cascadas: 3, voxel: 0.25, div: 4, rayos: 2, pasos_reflejo: 12 },
        CalidadGi::Media => Calidad { n: 64, cascadas: 4, voxel: 0.16, div: 2, rayos: 2, pasos_reflejo: 16 },
        CalidadGi::Alta => Calidad { n: 96, cascadas: 4, voxel: 0.12, div: 2, rayos: 4, pasos_reflejo: 24 },
        CalidadGi::Epica => Calidad { n: 128, cascadas: 4, voxel: 0.09, div: 2, rayos: 6, pasos_reflejo: 32 },
    }
}

/// Ajusta la calidad a los límites reales del dispositivo (tamaño de búfer y
/// de textura 3D). `None` si ni la mínima cabe.
pub fn ajustar(mut q: Calidad, l: &wgpu::Limits) -> Option<Calidad> {
    let cabe = |q: &Calidad| {
        let bytes = (q.n as u64).pow(3) * PALABRAS * 4;
        bytes <= l.max_storage_buffer_binding_size && bytes <= l.max_buffer_size && q.n * q.cascadas <= l.max_texture_dimension_3d
    };
    while !cabe(&q) {
        if q.n <= 32 {
            return None;
        }
        // Menos vóxeles, mismo alcance.
        let n = q.n - 16;
        q.voxel *= q.n as f32 / n as f32;
        q.n = n;
    }
    Some(q)
}

/// ¿Puede este dispositivo ejecutar la GI por software? (compute, texturas
/// de almacenamiento y cuatro grupos de enlace, según sus límites reales.)
pub fn soportada(device: &wgpu::Device) -> bool {
    let l = device.limits();
    l.max_bind_groups >= 4
        && l.max_compute_invocations_per_workgroup >= 64
        && l.max_storage_textures_per_shader_stage >= 3
        && l.max_storage_buffers_per_shader_stage >= 1
        && l.max_texture_dimension_3d >= 96
        && ajustar(calidad(CalidadGi::Baja), &l).is_some()
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GiU {
    view_proj: [[f32; 4]; 4],
    inv_view_proj: [[f32; 4]; 4],
    prev_view_proj: [[f32; 4]; 4],
    prev_inv_view_proj: [[f32; 4]; 4],
    cam_pos: [f32; 4],
    res: [f32; 4],
    cascadas: [[f32; 4]; MAX_CASCADAS],
    prev_cascadas: [[f32; 4]; MAX_CASCADAS],
    vol: [u32; 4],
    params: [f32; 4],
    sky_top: [f32; 4],
    sky_horizon: [f32; 4],
    ground: [f32; 4],
}

const _: () = assert!(std::mem::size_of::<GiU>() == 496);

/// Un dibujo opaco o recortado de la escena, visto por la GI.
#[derive(Clone, Copy, Debug)]
pub(crate) struct GiDraw {
    pub model: usize,
    pub sub: usize,
    pub slot: u32,
    pub mat: usize,
    pub skinned: bool,
    pub mask: bool,
    pub double_sided: bool,
    /// Dentro del frustum (prepase).
    pub visible: bool,
    pub min: Vec3,
    pub max: Vec3,
    /// Huella de lo que afecta a la voxelización (transform, malla, material).
    pub huella: u64,
}

/// Lo que la GI necesita del renderizador en un fotograma.
pub(crate) struct Frame<'a> {
    pub draws: &'a [GiDraw],
    pub models: &'a [GpuModel],
    pub materials: &'a [wgpu::BindGroup],
    pub object_bg: &'a wgpu::BindGroup,
    pub obj_stride: u64,
    pub view: &'a RenderView,
    pub env: &'a Environment,
    pub size: (u32, u32),
    pub debug: GiDebug,
}

/// Tiempos de GPU de la GI en el último fotograma medido (ms).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GiTiempos {
    pub voxelizar: f32,
    pub inyectar: f32,
    pub prepase: f32,
    pub recoger: f32,
    pub filtrar: f32,
}

impl GiTiempos {
    pub fn total(&self) -> f32 {
        self.voxelizar + self.inyectar + self.prepase + self.recoger + self.filtrar
    }
}

const SEGMENTOS: usize = 5;

/// Consultas de marca de tiempo (solo si el dispositivo las tiene).
struct Cronometro {
    qs: wgpu::QuerySet,
    resolver: wgpu::Buffer,
    lectura: wgpu::Buffer,
    periodo: f32,
    listo: Arc<AtomicBool>,
    esperando: bool,
    activo: bool,
    escritos: [bool; SEGMENTOS],
    pendientes: [bool; SEGMENTOS],
    ultimo: Option<GiTiempos>,
}

impl Cronometro {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Option<Self> {
        if !device.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
            return None;
        }
        let n = (SEGMENTOS * 2) as u32;
        let qs = device.create_query_set(&wgpu::QuerySetDescriptor { label: Some("gi tiempos"), ty: wgpu::QueryType::Timestamp, count: n });
        let size = n as u64 * 8;
        let resolver = device.create_buffer(&wgpu::BufferDescriptor { label: Some("gi tiempos"), size, usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false });
        let lectura = device.create_buffer(&wgpu::BufferDescriptor { label: Some("gi tiempos lectura"), size, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
        Some(Self {
            qs,
            resolver,
            lectura,
            periodo: queue.get_timestamp_period(),
            listo: Arc::new(AtomicBool::new(false)),
            esperando: false,
            activo: false,
            escritos: [false; SEGMENTOS],
            pendientes: [false; SEGMENTOS],
            ultimo: None,
        })
    }

    /// Recoge la medida anterior si ya llegó y decide si este fotograma mide.
    fn empezar(&mut self) {
        if self.esperando && self.listo.load(Ordering::Acquire) {
            let datos = self.lectura.slice(..).get_mapped_range().ok().map(|d| bytemuck::cast_slice::<u8, u64>(&d).to_vec());
            self.lectura.unmap();
            self.esperando = false;
            self.listo.store(false, Ordering::Release);
            if let Some(v) = datos {
                let ms = |i: usize| if self.pendientes[i] { v[i * 2 + 1].saturating_sub(v[i * 2]) as f32 * self.periodo / 1e6 } else { 0.0 };
                self.ultimo = Some(GiTiempos { voxelizar: ms(0), inyectar: ms(1), prepase: ms(2), recoger: ms(3), filtrar: ms(4) });
            }
        }
        self.activo = !self.esperando;
        self.escritos = [false; SEGMENTOS];
    }

    fn escrituras(&mut self, seg: usize, primero: bool, ultimo: bool) -> Option<(&wgpu::QuerySet, Option<u32>, Option<u32>)> {
        if !self.activo || !(primero || ultimo) {
            return None;
        }
        self.escritos[seg] = true;
        Some((&self.qs, primero.then_some(seg as u32 * 2), ultimo.then_some(seg as u32 * 2 + 1)))
    }

    fn compute(&mut self, seg: usize, primero: bool, ultimo: bool) -> Option<wgpu::ComputePassTimestampWrites<'_>> {
        self.escrituras(seg, primero, ultimo).map(|(q, b, e)| wgpu::ComputePassTimestampWrites { query_set: q, beginning_of_pass_write_index: b, end_of_pass_write_index: e })
    }

    fn render(&mut self, seg: usize, primero: bool, ultimo: bool) -> Option<wgpu::RenderPassTimestampWrites<'_>> {
        self.escrituras(seg, primero, ultimo).map(|(q, b, e)| wgpu::RenderPassTimestampWrites { query_set: q, beginning_of_pass_write_index: b, end_of_pass_write_index: e })
    }

    fn terminar(&mut self, enc: &mut wgpu::CommandEncoder) {
        if !self.activo || !self.escritos.iter().any(|&e| e) {
            self.activo = false;
            return;
        }
        // Las consultas sin escribir se resuelven igual; se ignoran al leer.
        enc.resolve_query_set(&self.qs, 0..(SEGMENTOS * 2) as u32, &self.resolver, 0);
        enc.copy_buffer_to_buffer(&self.resolver, 0, &self.lectura, 0, None);
        self.pendientes = self.escritos;
    }

    fn tras_enviar(&mut self) {
        if !self.activo {
            return;
        }
        self.activo = false;
        self.esperando = true;
        let listo = self.listo.clone();
        self.lectura.slice(..).map_async(wgpu::MapMode::Read, move |r| {
            if r.is_ok() {
                listo.store(true, Ordering::Release);
            }
        });
    }
}

struct Volumen {
    q: Calidad,
    acum: wgpu::Buffer,
    /// Destino de color vacío del rasterizado de la voxelización (N×N).
    falso: wgpu::TextureView,
    bg_vox: wgpu::BindGroup,
    bg_resolver: wgpu::BindGroup,
    bg_resolver_out: wgpu::BindGroup,
    /// [cur]: lee la radiancia 1-cur, escribe cur.
    bg_inyectar: [wgpu::BindGroup; 2],
    rad: [wgpu::TextureView; 2],
}

struct Pantalla {
    size: (u32, u32),
    gb_normal: [wgpu::TextureView; 2],
    gb_depth: [wgpu::TextureView; 2],
    raw: [wgpu::TextureView; 2],
    hist_d: [wgpu::TextureView; 2],
    hist_s: [wgpu::TextureView; 2],
    tmp: [wgpu::TextureView; 2],
    out: [wgpu::TextureView; 2],
}

/// Grupos que dependen a la vez del volumen y de la pantalla, por índice cur.
struct Enlaces {
    recoger: [wgpu::BindGroup; 2],
    temporal: [wgpu::BindGroup; 2],
    filtrar1: [wgpu::BindGroup; 2],
    filtrar2: [wgpu::BindGroup; 2],
    /// Grupo 3 del sombreado: [final, sin filtrar (depuración de vóxeles)][cur].
    sombreado: [[wgpu::BindGroup; 2]; 2],
}

pub struct GiSystem {
    device: wgpu::Device,
    queue: wgpu::Queue,
    bgl_vox: wgpu::BindGroupLayout,
    bgl_resolver: wgpu::BindGroupLayout,
    bgl_resolver_out: wgpu::BindGroupLayout,
    bgl_inyectar: wgpu::BindGroupLayout,
    bgl_recoger: wgpu::BindGroupLayout,
    bgl_temporal: wgpu::BindGroupLayout,
    bgl_filtrar: wgpu::BindGroupLayout,
    shading_bgl: wgpu::BindGroupLayout,
    vox_module: wgpu::ShaderModule,
    vox_layout: wgpu::PipelineLayout,
    pre_layout: wgpu::PipelineLayout,
    vox_pipes: std::collections::HashMap<(bool, bool), wgpu::RenderPipeline>,
    pre_pipes: std::collections::HashMap<(bool, bool, bool), wgpu::RenderPipeline>,
    p_resolver: wgpu::ComputePipeline,
    p_inyectar: wgpu::ComputePipeline,
    p_recoger: wgpu::ComputePipeline,
    p_ver_voxeles: wgpu::ComputePipeline,
    p_temporal: wgpu::ComputePipeline,
    p_filtrar: [wgpu::ComputePipeline; 2],
    uniform: wgpu::Buffer,
    voxu: wgpu::Buffer,
    voxu_stride: u64,
    bg_base: wgpu::BindGroup,
    globals_buf: wgpu::Buffer,
    shadow_view: wgpu::TextureView,
    shadow_sampler: wgpu::Sampler,
    sampler: wgpu::Sampler,
    vol: Option<Volumen>,
    pantalla: Option<Pantalla>,
    enlaces: Option<Enlaces>,
    cur: usize,
    frame: u32,
    cascadas: [[f32; 4]; MAX_CASCADAS],
    huellas: [u64; MAX_CASCADAS],
    prev_vp: Mat4,
    prev_inv: Mat4,
    historial: bool,
    /// El volumen ya tiene radiancia de un fotograma anterior.
    vol_listo: bool,
    cronometro: Option<Cronometro>,
    /// Cascadas revoxelizadas en el último fotograma.
    pub revoxelizadas: u32,
}

fn uniform_entry(binding: u32, vis: wgpu::ShaderStages, dynamic: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry { binding, visibility: vis, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: dynamic, min_binding_size: None }, count: None }
}
fn tex_entry(binding: u32, vis: wgpu::ShaderStages, dim: wgpu::TextureViewDimension, filterable: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry { binding, visibility: vis, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable }, view_dimension: dim, multisampled: false }, count: None }
}
fn depth_entry(binding: u32, vis: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry { binding, visibility: vis, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None }
}
fn storage_tex(binding: u32, format: wgpu::TextureFormat, dim: wgpu::TextureViewDimension) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry { binding, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::StorageTexture { access: wgpu::StorageTextureAccess::WriteOnly, format, view_dimension: dim }, count: None }
}
fn view(r: &wgpu::TextureView) -> wgpu::BindingResource<'_> {
    wgpu::BindingResource::TextureView(r)
}

/// Grupo 3 del sombreado (difusa, reflejos, profundidad del trazado).
pub fn shading_bgl(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    let f = wgpu::ShaderStages::FRAGMENT;
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("gi sombreado"),
        entries: &[tex_entry(0, f, wgpu::TextureViewDimension::D2, false), tex_entry(1, f, wgpu::TextureViewDimension::D2, false), depth_entry(2, f)],
    })
}

/// Grupo 3 vacío (1×1) para cuando la GI está apagada.
pub fn shading_dummy(device: &wgpu::Device, bgl: &wgpu::BindGroupLayout) -> wgpu::BindGroup {
    let t = |label, format, usage| tex(device, label, (1, 1), format, usage);
    let a = t("gi vacío", GI_FORMAT, wgpu::TextureUsages::TEXTURE_BINDING);
    let d = t("gi vacío prof", DEPTH_FORMAT, wgpu::TextureUsages::TEXTURE_BINDING);
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("gi sombreado vacío"),
        layout: bgl,
        entries: &[wgpu::BindGroupEntry { binding: 0, resource: view(&a) }, wgpu::BindGroupEntry { binding: 1, resource: view(&a) }, wgpu::BindGroupEntry { binding: 2, resource: view(&d) }],
    })
}

fn tex(device: &wgpu::Device, label: &str, (w, h): (u32, u32), format: wgpu::TextureFormat, usage: wgpu::TextureUsages) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        })
        .create_view(&Default::default())
}

fn tex3(device: &wgpu::Device, label: &str, n: u32, c: u32, format: wgpu::TextureFormat) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d { width: n, height: n, depth_or_array_layers: n * c },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D3,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::STORAGE_BINDING,
            view_formats: &[],
        })
        .create_view(&Default::default())
}

/// Huella FNV-1a (para saber si una cascada cambió).
pub(crate) fn huella(bytes: &[u8], semilla: u64) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64 ^ semilla;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    h
}

/// Origen (esquina mínima) de la cascada `c` centrada en `eye`, ajustado a
/// saltos de 4 vóxeles para no revoxelizar a cada paso de la cámara.
pub fn origen_cascada(eye: Vec3, q: &Calidad, c: u32) -> (Vec3, f32) {
    let vs = q.voxel * (1u32 << c) as f32;
    let paso = vs * 4.0;
    let mitad = vs * q.n as f32 * 0.5;
    ((((eye - Vec3::splat(mitad)) / paso).round()) * paso, vs)
}

impl GiSystem {
    pub(crate) fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        object_bgl: &wgpu::BindGroupLayout,
        material_bgl: &wgpu::BindGroupLayout,
        shading_bgl: &wgpu::BindGroupLayout,
        globals_buf: &wgpu::Buffer,
        shadow_view: &wgpu::TextureView,
    ) -> Self {
        let all = wgpu::ShaderStages::VERTEX_FRAGMENT | wgpu::ShaderStages::COMPUTE;
        let cs = wgpu::ShaderStages::COMPUTE;
        let samp = wgpu::BindGroupLayoutEntry { binding: 1, visibility: all, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None };
        let bgl = |label: &str, entries: &[wgpu::BindGroupLayoutEntry]| device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some(label), entries });
        let bgl_base = bgl("gi base", &[uniform_entry(0, all, false), samp]);
        let acum_entry = |read_only: bool, vis| wgpu::BindGroupLayoutEntry {
            binding: 3,
            visibility: vis,
            ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only }, has_dynamic_offset: false, min_binding_size: None },
            count: None,
        };
        let bgl_vox = bgl("gi voxelizar", &[uniform_entry(0, all, false), samp, uniform_entry(2, all, true), acum_entry(false, wgpu::ShaderStages::FRAGMENT)]);
        let bgl_resolver = bgl("gi resolver", &[uniform_entry(0, all, false), samp, uniform_entry(2, all, true), acum_entry(true, cs)]);
        let d3 = wgpu::TextureViewDimension::D3;
        let d2 = wgpu::TextureViewDimension::D2;
        let bgl_resolver_out = bgl(
            "gi resolver salida",
            &[storage_tex(0, wgpu::TextureFormat::Rgba8Unorm, d3), storage_tex(1, wgpu::TextureFormat::Rgba8Snorm, d3), storage_tex(2, GI_FORMAT, d3)],
        );
        let bgl_inyectar = bgl(
            "gi inyectar",
            &[
                tex_entry(0, cs, d3, false),
                tex_entry(1, cs, d3, false),
                tex_entry(2, cs, d3, false),
                tex_entry(3, cs, d3, true),
                storage_tex(4, GI_FORMAT, d3),
                uniform_entry(5, cs, false),
                depth_entry(6, cs),
                wgpu::BindGroupLayoutEntry { binding: 7, visibility: cs, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison), count: None },
            ],
        );
        let bgl_recoger = bgl("gi recoger", &[tex_entry(0, cs, d3, true), tex_entry(1, cs, d2, false), depth_entry(2, cs), storage_tex(3, GI_FORMAT, d2), storage_tex(4, GI_FORMAT, d2)]);
        let bgl_temporal = bgl(
            "gi temporal",
            &[
                tex_entry(0, cs, d2, false),
                tex_entry(1, cs, d2, false),
                tex_entry(2, cs, d2, false),
                tex_entry(3, cs, d2, false),
                tex_entry(4, cs, d2, false),
                depth_entry(5, cs),
                tex_entry(6, cs, d2, false),
                depth_entry(7, cs),
                storage_tex(8, GI_FORMAT, d2),
                storage_tex(9, GI_FORMAT, d2),
            ],
        );
        let bgl_filtrar = bgl(
            "gi filtrar",
            &[tex_entry(0, cs, d2, false), tex_entry(1, cs, d2, false), tex_entry(2, cs, d2, false), depth_entry(3, cs), storage_tex(4, GI_FORMAT, d2), storage_tex(5, GI_FORMAT, d2)],
        );

        let module = |label: &str, src: &str, cono: bool| {
            let code = if cono { format!("{COMUN}\n{CONO}\n{src}") } else { format!("{COMUN}\n{src}") };
            device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some(label), source: wgpu::ShaderSource::Wgsl(code.into()) })
        };
        let vox_module = module("gi voxelizar", include_str!("voxelizar.wgsl"), false);
        let m_resolver = module("gi resolver", include_str!("resolver.wgsl"), false);
        let m_inyectar = module("gi inyectar", include_str!("inyectar.wgsl"), true);
        let m_recoger = module("gi recoger", include_str!("recoger.wgsl"), true);
        let m_temporal = module("gi temporal", include_str!("temporal.wgsl"), false);
        let m_filtrar = module("gi filtrar", include_str!("filtrar.wgsl"), false);

        let layout = |label: &str, groups: &[Option<&wgpu::BindGroupLayout>]| device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some(label), bind_group_layouts: groups, immediate_size: 0 });
        let vox_layout = layout("gi voxelizar", &[Some(&bgl_vox), Some(object_bgl), Some(material_bgl)]);
        let pre_layout = layout("gi prepase", &[Some(&bgl_base), Some(object_bgl), Some(material_bgl)]);
        let compute = |label: &str, l: &wgpu::PipelineLayout, m: &wgpu::ShaderModule, entry: &str, constants: &[(&str, f64)]| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(label),
                layout: Some(l),
                module: m,
                entry_point: Some(entry),
                compilation_options: wgpu::PipelineCompilationOptions { constants, ..Default::default() },
                cache: None,
            })
        };
        let p_resolver = compute("gi resolver", &layout("gi resolver", &[Some(&bgl_resolver), Some(&bgl_resolver_out)]), &m_resolver, "resolver", &[]);
        let p_inyectar = compute("gi inyectar", &layout("gi inyectar", &[Some(&bgl_base), Some(&bgl_inyectar)]), &m_inyectar, "inyectar", &[]);
        let l_recoger = layout("gi recoger", &[Some(&bgl_base), Some(&bgl_recoger)]);
        let p_recoger = compute("gi recoger", &l_recoger, &m_recoger, "recoger", &[]);
        let p_ver_voxeles = compute("gi ver vóxeles", &l_recoger, &m_recoger, "ver_voxeles", &[]);
        let p_temporal = compute("gi temporal", &layout("gi temporal", &[Some(&bgl_base), Some(&bgl_temporal)]), &m_temporal, "temporal", &[]);
        let l_filtrar = layout("gi filtrar", &[Some(&bgl_base), Some(&bgl_filtrar)]);
        let p_filtrar = [compute("gi filtrar 1", &l_filtrar, &m_filtrar, "filtrar", &[("PASO", 1.0)]), compute("gi filtrar 2", &l_filtrar, &m_filtrar, "filtrar", &[("PASO", 2.0)])];

        let uniform = device.create_buffer(&wgpu::BufferDescriptor { label: Some("gi"), size: std::mem::size_of::<GiU>() as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        // Un hueco por cascada con su índice (offset dinámico), con la alineación real del dispositivo.
        let align = device.limits().min_uniform_buffer_offset_alignment.max(16) as u64;
        let voxu_stride = align.max(16);
        let voxu = device.create_buffer(&wgpu::BufferDescriptor { label: Some("gi cascada"), size: voxu_stride * MAX_CASCADAS as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        for c in 0..MAX_CASCADAS as u32 {
            queue.write_buffer(&voxu, c as u64 * voxu_stride, bytemuck::cast_slice(&[c, 0, 0, 0]));
        }
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("gi lineal"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let bg_base = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gi base"),
            layout: &bgl_base,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() }, wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) }],
        });
        let shadow_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("gi sombras"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        });
        Self {
            device: device.clone(),
            queue: queue.clone(),
            bgl_vox,
            bgl_resolver,
            bgl_resolver_out,
            bgl_inyectar,
            bgl_recoger,
            bgl_temporal,
            bgl_filtrar,
            shading_bgl: shading_bgl.clone(),
            vox_module,
            vox_layout,
            pre_layout,
            vox_pipes: Default::default(),
            pre_pipes: Default::default(),
            p_resolver,
            p_inyectar,
            p_recoger,
            p_ver_voxeles,
            p_temporal,
            p_filtrar,
            uniform,
            voxu,
            voxu_stride,
            bg_base,
            globals_buf: globals_buf.clone(),
            shadow_view: shadow_view.clone(),
            shadow_sampler,
            vol: None,
            pantalla: None,
            enlaces: None,
            cur: 0,
            frame: 0,
            cascadas: [[0.0; 4]; MAX_CASCADAS],
            huellas: [0; MAX_CASCADAS],
            prev_vp: Mat4::IDENTITY,
            prev_inv: Mat4::IDENTITY,
            historial: false,
            vol_listo: false,
            cronometro: Cronometro::new(device, queue),
            revoxelizadas: 0,
            sampler,
        }
    }

    fn vox_pipe(&mut self, skinned: bool, mask: bool) {
        if self.vox_pipes.contains_key(&(skinned, mask)) {
            return;
        }
        let vb = [Some(crate::renderer::vertex_layout())];
        let vb_skin = [Some(crate::renderer::vertex_layout()), Some(crate::renderer::skin_layout())];
        let constants = [("ALPHA_TEST", if mask { 1.0 } else { 0.0 })];
        let p = self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("gi voxelizar"),
            layout: Some(&self.vox_layout),
            vertex: wgpu::VertexState { module: &self.vox_module, entry_point: Some(if skinned { "vs_voxel_skinned" } else { "vs_voxel" }), compilation_options: Default::default(), buffers: if skinned { &vb_skin } else { &vb } },
            primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &self.vox_module,
                entry_point: Some("fs_voxel"),
                compilation_options: wgpu::PipelineCompilationOptions { constants: &constants, ..Default::default() },
                targets: &[Some(wgpu::ColorTargetState { format: wgpu::TextureFormat::R8Unorm, blend: None, write_mask: wgpu::ColorWrites::empty() })],
            }),
            multiview_mask: None,
            cache: None,
        });
        self.vox_pipes.insert((skinned, mask), p);
    }

    fn pre_pipe(&mut self, key: (bool, bool, bool)) {
        if self.pre_pipes.contains_key(&key) {
            return;
        }
        let (skinned, mask, double_sided) = key;
        let vb = [Some(crate::renderer::vertex_layout())];
        let vb_skin = [Some(crate::renderer::vertex_layout()), Some(crate::renderer::skin_layout())];
        let constants = [("ALPHA_TEST", if mask { 1.0 } else { 0.0 })];
        let p = self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("gi prepase"),
            layout: Some(&self.pre_layout),
            vertex: wgpu::VertexState { module: &self.vox_module, entry_point: Some(if skinned { "vs_pre_skinned" } else { "vs_pre" }), compilation_options: Default::default(), buffers: if skinned { &vb_skin } else { &vb } },
            primitive: wgpu::PrimitiveState { cull_mode: if double_sided { None } else { Some(wgpu::Face::Back) }, ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &self.vox_module,
                entry_point: Some("fs_pre"),
                compilation_options: wgpu::PipelineCompilationOptions { constants: &constants, ..Default::default() },
                targets: &[Some(wgpu::ColorTargetState { format: GB_FORMAT, blend: None, write_mask: wgpu::ColorWrites::ALL })],
            }),
            multiview_mask: None,
            cache: None,
        });
        self.pre_pipes.insert(key, p);
    }

    /// Volumen para esta calidad (se rehace solo si cambia).
    fn ensure_volumen(&mut self, q: Calidad) {
        if self.vol.as_ref().is_some_and(|v| v.q == q) {
            return;
        }
        let d = &self.device;
        let (n, c) = (q.n, q.cascadas);
        let acum = d.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gi acumulador"),
            size: (n as u64).pow(3) * PALABRAS * 4,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let falso = tex(d, "gi voxelizar destino", (n, n), wgpu::TextureFormat::R8Unorm, wgpu::TextureUsages::RENDER_ATTACHMENT);
        let albedo = tex3(d, "gi albedo", n, c, wgpu::TextureFormat::Rgba8Unorm);
        let normal = tex3(d, "gi normal", n, c, wgpu::TextureFormat::Rgba8Snorm);
        let emision = tex3(d, "gi emision", n, c, GI_FORMAT);
        let rad = [tex3(d, "gi radiancia 0", n, c, GI_FORMAT), tex3(d, "gi radiancia 1", n, c, GI_FORMAT)];
        let voxu = wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: &self.voxu, offset: 0, size: wgpu::BufferSize::new(16) });
        let bg = |label: &str, layout: &wgpu::BindGroupLayout, entries: &[wgpu::BindGroupEntry]| d.create_bind_group(&wgpu::BindGroupDescriptor { label: Some(label), layout, entries });
        let base_acum = |layout: &wgpu::BindGroupLayout, label: &str| {
            bg(
                label,
                layout,
                &[
                    wgpu::BindGroupEntry { binding: 0, resource: self.uniform.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                    wgpu::BindGroupEntry { binding: 2, resource: voxu.clone() },
                    wgpu::BindGroupEntry { binding: 3, resource: acum.as_entire_binding() },
                ],
            )
        };
        let bg_vox = base_acum(&self.bgl_vox, "gi voxelizar");
        let bg_resolver = base_acum(&self.bgl_resolver, "gi resolver");
        let bg_resolver_out = bg(
            "gi resolver salida",
            &self.bgl_resolver_out,
            &[wgpu::BindGroupEntry { binding: 0, resource: view(&albedo) }, wgpu::BindGroupEntry { binding: 1, resource: view(&normal) }, wgpu::BindGroupEntry { binding: 2, resource: view(&emision) }],
        );
        let inyectar = |cur: usize| {
            bg(
                "gi inyectar",
                &self.bgl_inyectar,
                &[
                    wgpu::BindGroupEntry { binding: 0, resource: view(&albedo) },
                    wgpu::BindGroupEntry { binding: 1, resource: view(&normal) },
                    wgpu::BindGroupEntry { binding: 2, resource: view(&emision) },
                    wgpu::BindGroupEntry { binding: 3, resource: view(&rad[1 - cur]) },
                    wgpu::BindGroupEntry { binding: 4, resource: view(&rad[cur]) },
                    wgpu::BindGroupEntry { binding: 5, resource: self.globals_buf.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 6, resource: view(&self.shadow_view) },
                    wgpu::BindGroupEntry { binding: 7, resource: wgpu::BindingResource::Sampler(&self.shadow_sampler) },
                ],
            )
        };
        let bg_inyectar = [inyectar(0), inyectar(1)];
        self.vol = Some(Volumen { q, acum, falso, bg_vox, bg_resolver, bg_resolver_out, bg_inyectar, rad });
        self.enlaces = None;
        self.huellas = [0; MAX_CASCADAS];
        self.historial = false;
        self.vol_listo = false;
    }

    fn ensure_pantalla(&mut self, size: (u32, u32)) {
        if self.pantalla.as_ref().is_some_and(|p| p.size == size) {
            return;
        }
        let d = &self.device;
        let rt = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
        let st = wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING;
        let par = |label: &str, format, usage| [tex(d, label, size, format, usage), tex(d, label, size, format, usage)];
        self.pantalla = Some(Pantalla {
            size,
            gb_normal: par("gi normales", GB_FORMAT, rt),
            gb_depth: par("gi profundidad", DEPTH_FORMAT, rt),
            raw: par("gi recogida", GI_FORMAT, st),
            hist_d: par("gi historial difusa", GI_FORMAT, st),
            hist_s: par("gi historial reflejos", GI_FORMAT, st),
            tmp: par("gi filtro", GI_FORMAT, st),
            out: par("gi salida", GI_FORMAT, st),
        });
        self.enlaces = None;
        self.historial = false;
    }

    fn ensure_enlaces(&mut self) {
        if self.enlaces.is_some() {
            return;
        }
        let (Some(v), Some(p)) = (self.vol.as_ref(), self.pantalla.as_ref()) else { return };
        let d = &self.device;
        let bg = |label: &str, layout: &wgpu::BindGroupLayout, views: &[&wgpu::TextureView]| {
            let entries: Vec<wgpu::BindGroupEntry> = views.iter().enumerate().map(|(i, t)| wgpu::BindGroupEntry { binding: i as u32, resource: view(t) }).collect();
            d.create_bind_group(&wgpu::BindGroupDescriptor { label: Some(label), layout, entries: &entries })
        };
        let recoger = |c: usize| bg("gi recoger", &self.bgl_recoger, &[&v.rad[c], &p.gb_normal[c], &p.gb_depth[c], &p.raw[0], &p.raw[1]]);
        let temporal = |c: usize| {
            bg(
                "gi temporal",
                &self.bgl_temporal,
                &[&p.raw[0], &p.raw[1], &p.hist_d[1 - c], &p.hist_s[1 - c], &p.gb_normal[c], &p.gb_depth[c], &p.gb_normal[1 - c], &p.gb_depth[1 - c], &p.hist_d[c], &p.hist_s[c]],
            )
        };
        let filtrar1 = |c: usize| bg("gi filtrar 1", &self.bgl_filtrar, &[&p.hist_d[c], &p.hist_s[c], &p.gb_normal[c], &p.gb_depth[c], &p.tmp[0], &p.tmp[1]]);
        let filtrar2 = |c: usize| bg("gi filtrar 2", &self.bgl_filtrar, &[&p.tmp[0], &p.tmp[1], &p.gb_normal[c], &p.gb_depth[c], &p.out[0], &p.out[1]]);
        let sombreado = |src: &[wgpu::TextureView; 2], c: usize| bg("gi sombreado", &self.shading_bgl, &[&src[0], &src[1], &p.gb_depth[c]]);
        self.enlaces = Some(Enlaces {
            recoger: [recoger(0), recoger(1)],
            temporal: [temporal(0), temporal(1)],
            filtrar1: [filtrar1(0), filtrar1(1)],
            filtrar2: [filtrar2(0), filtrar2(1)],
            sombreado: [[sombreado(&p.out, 0), sombreado(&p.out, 1)], [sombreado(&p.raw, 0), sombreado(&p.raw, 1)]],
        });
    }

    /// Grupo 3 del pase de color de este fotograma.
    pub(crate) fn shading_bg(&self, debug: GiDebug) -> Option<&wgpu::BindGroup> {
        let e = self.enlaces.as_ref()?;
        Some(&e.sombreado[(debug == GiDebug::Voxeles) as usize][self.cur])
    }

    /// Resolución del trazado en pantalla para una salida de `size`.
    pub fn resolucion(size: (u32, u32), q: &Calidad) -> (u32, u32) {
        ((size.0 / q.div).max(1), (size.1 / q.div).max(1))
    }

    /// Graba todos los pases de la GI (tras el mapa de sombras, antes del color).
    pub(crate) fn encode(&mut self, enc: &mut wgpu::CommandEncoder, f: &Frame, ajustes: &IluminacionGlobal) {
        let lim = self.device.limits();
        let Some(q) = ajustar(calidad(ajustes.calidad), &lim) else { return };
        self.ensure_volumen(q);
        let res = Self::resolucion(f.size, &q);
        self.ensure_pantalla(res);
        self.ensure_enlaces();
        if let Some(c) = self.cronometro.as_mut() {
            c.empezar();
        }
        self.cur = 1 - self.cur;
        self.frame = self.frame.wrapping_add(1);
        let cur = self.cur;

        // Cascadas: origen ajustado y huella de lo que contienen.
        let prev = self.cascadas;
        let mut sucias = [false; MAX_CASCADAS];
        for c in 0..q.cascadas as usize {
            let (o, vs) = origen_cascada(f.view.eye, &q, c as u32);
            self.cascadas[c] = [o.x, o.y, o.z, vs];
            let max = o + Vec3::splat(vs * q.n as f32);
            let mut h = huella(bytemuck::cast_slice(&self.cascadas[c]), c as u64);
            for d in f.draws.iter().filter(|d| d.min.cmple(max).all() && d.max.cmpge(o).all()) {
                // Suma conmutativa: no depende del orden de la lista.
                h = h.wrapping_add(d.huella.wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ (d.huella >> 29));
            }
            if h != self.huellas[c] {
                self.huellas[c] = h;
                sucias[c] = true;
            }
        }
        let vp = f.view.view_proj();
        let inv = vp.inverse();
        if !self.historial {
            self.prev_vp = vp;
            self.prev_inv = inv;
        }
        let env = f.env;
        let u = GiU {
            view_proj: vp.to_cols_array_2d(),
            inv_view_proj: inv.to_cols_array_2d(),
            prev_view_proj: self.prev_vp.to_cols_array_2d(),
            prev_inv_view_proj: self.prev_inv.to_cols_array_2d(),
            cam_pos: [f.view.eye.x, f.view.eye.y, f.view.eye.z, (self.frame % 4096) as f32],
            res: [res.0 as f32, res.1 as f32, 1.0 / res.0 as f32, 1.0 / res.1 as f32],
            cascadas: self.cascadas,
            prev_cascadas: if prev[0][3] > 0.0 { prev } else { self.cascadas },
            vol: [q.n, q.cascadas, q.rayos, q.pasos_reflejo],
            // El primer fotograma de un volumen nuevo no realimenta: el anterior está vacío y
            // sus conos verían el cielo a través de todo (un destello de luz que no existe).
            params: [env.ambient_intensity, if ajustes.rebotes_multiples && self.vol_listo { 1.0 } else { 0.0 }, if self.historial { 1.0 } else { 0.0 }, f.debug.code()],
            sky_top: v4(lin3(env.sky_top)),
            sky_horizon: v4(lin3(env.sky_horizon)),
            ground: v4(lin3(env.ground)),
        };
        self.queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&u));

        // Pipelines que hagan falta.
        for d in f.draws {
            self.vox_pipe(d.skinned, d.mask);
            if d.visible {
                self.pre_pipe((d.skinned, d.mask, d.double_sided));
            }
        }

        let vol = self.vol.as_ref().unwrap();
        let pan = self.pantalla.as_ref().unwrap();
        let Some(enl) = self.enlaces.as_ref() else { return };
        let mut crono = self.cronometro.as_mut();
        let grupos = |n: u32, t: u32| n.div_ceil(t);

        // 1. Voxelizar las cascadas que cambiaron.
        let ultima = (0..q.cascadas as usize).rev().find(|&c| sucias[c]);
        let primera = (0..q.cascadas as usize).find(|&c| sucias[c]);
        self.revoxelizadas = 0;
        for c in 0..q.cascadas as usize {
            if !sucias[c] {
                continue;
            }
            self.revoxelizadas += 1;
            enc.clear_buffer(&vol.acum, 0, None);
            let o = Vec3::from_slice(&self.cascadas[c][..3]);
            let max = o + Vec3::splat(self.cascadas[c][3] * q.n as f32);
            {
                let ts = crono.as_mut().and_then(|t| t.render(0, primera == Some(c), false));
                let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("gi voxelizar"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &vol.falso,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Discard },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: ts,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_bind_group(0, &vol.bg_vox, &[(c as u64 * self.voxu_stride) as u32]);
                let mut bound = None;
                for d in f.draws.iter().filter(|d| d.min.cmple(max).all() && d.max.cmpge(o).all()) {
                    if bound != Some((d.skinned, d.mask)) {
                        pass.set_pipeline(&self.vox_pipes[&(d.skinned, d.mask)]);
                        bound = Some((d.skinned, d.mask));
                    }
                    let m = &f.models[d.model];
                    let s = &m.subs[d.sub];
                    pass.set_bind_group(1, f.object_bg, &[(d.slot as u64 * f.obj_stride) as u32]);
                    pass.set_bind_group(2, &f.materials[d.mat], &[]);
                    pass.set_vertex_buffer(0, m.vbuf.slice(..));
                    if let Some(sk) = &m.skin {
                        pass.set_vertex_buffer(1, sk.slice(..));
                    }
                    pass.set_index_buffer(m.ibuf.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(s.first..s.first + s.count, 0, 0..3);
                }
            }
            let ts = crono.as_mut().and_then(|t| t.compute(0, false, ultima == Some(c)));
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("gi resolver"), timestamp_writes: ts });
            pass.set_pipeline(&self.p_resolver);
            pass.set_bind_group(0, &vol.bg_resolver, &[(c as u64 * self.voxu_stride) as u32]);
            pass.set_bind_group(1, &vol.bg_resolver_out, &[]);
            let g = grupos(q.n, 4);
            pass.dispatch_workgroups(g, g, g);
        }
        // 2. Inyectar la luz.
        {
            let ts = crono.as_mut().and_then(|t| t.compute(1, true, true));
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("gi inyectar"), timestamp_writes: ts });
            pass.set_pipeline(&self.p_inyectar);
            pass.set_bind_group(0, &self.bg_base, &[]);
            pass.set_bind_group(1, &vol.bg_inyectar[cur], &[]);
            pass.dispatch_workgroups(grupos(q.n, 4), grupos(q.n, 4), grupos(q.n * q.cascadas, 4));
        }
        // 3. Prepase de normales y profundidad.
        {
            let ts = crono.as_mut().and_then(|t| t.render(2, true, true));
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("gi prepase"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &pan.gb_normal[cur],
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.0, g: 1.0, b: 0.0, a: -1.0 }), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &pan.gb_depth[cur],
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
                    stencil_ops: None,
                }),
                timestamp_writes: ts,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &self.bg_base, &[]);
            let mut bound = None;
            for d in f.draws.iter().filter(|d| d.visible) {
                let key = (d.skinned, d.mask, d.double_sided);
                if bound != Some(key) {
                    pass.set_pipeline(&self.pre_pipes[&key]);
                    bound = Some(key);
                }
                let m = &f.models[d.model];
                let s = &m.subs[d.sub];
                pass.set_bind_group(1, f.object_bg, &[(d.slot as u64 * f.obj_stride) as u32]);
                pass.set_bind_group(2, &f.materials[d.mat], &[]);
                pass.set_vertex_buffer(0, m.vbuf.slice(..));
                if let Some(sk) = &m.skin {
                    pass.set_vertex_buffer(1, sk.slice(..));
                }
                pass.set_index_buffer(m.ibuf.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(s.first..s.first + s.count, 0, 0..1);
            }
        }
        let (gx, gy) = (grupos(res.0, 8), grupos(res.1, 8));
        // 4. Recolección (o la vista de vóxeles).
        {
            let ts = crono.as_mut().and_then(|t| t.compute(3, true, true));
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("gi recoger"), timestamp_writes: ts });
            pass.set_pipeline(if f.debug == GiDebug::Voxeles { &self.p_ver_voxeles } else { &self.p_recoger });
            pass.set_bind_group(0, &self.bg_base, &[]);
            pass.set_bind_group(1, &enl.recoger[cur], &[]);
            pass.dispatch_workgroups(gx, gy, 1);
        }
        // 5. Temporal + filtro espacial.
        if f.debug != GiDebug::Voxeles {
            for (i, (p, bg)) in [(&self.p_temporal, &enl.temporal[cur]), (&self.p_filtrar[0], &enl.filtrar1[cur]), (&self.p_filtrar[1], &enl.filtrar2[cur])].into_iter().enumerate() {
                let ts = crono.as_mut().and_then(|t| t.compute(4, i == 0, i == 2));
                let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("gi filtrar"), timestamp_writes: ts });
                pass.set_pipeline(p);
                pass.set_bind_group(0, &self.bg_base, &[]);
                pass.set_bind_group(1, bg, &[]);
                pass.dispatch_workgroups(gx, gy, 1);
            }
        }
        if let Some(c) = crono {
            c.terminar(enc);
        }
        self.prev_vp = vp;
        self.prev_inv = inv;
        self.historial = f.debug != GiDebug::Voxeles;
        self.vol_listo = true;
    }

    /// Tras `queue.submit`: pide la lectura de las marcas de tiempo.
    pub(crate) fn tras_enviar(&mut self) {
        if let Some(c) = self.cronometro.as_mut() {
            c.tras_enviar();
        }
    }

    /// Últimos tiempos de GPU medidos (si el dispositivo tiene consultas de
    /// marca de tiempo). Con `esperar` bloquea hasta que llegue la medida.
    pub fn tiempos(&mut self, esperar: bool) -> Option<GiTiempos> {
        let c = self.cronometro.as_mut()?;
        if esperar && c.esperando {
            let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
            c.empezar();
            c.activo = false;
        } else {
            let _ = self.device.poll(wgpu::PollType::Poll);
        }
        c.ultimo
    }

    /// Bytes de GPU reservados por la GI (volumen + pantalla).
    pub fn memoria(&self) -> u64 {
        let v = self.vol.as_ref().map(|v| {
            let vox = (v.q.n as u64).pow(3) * v.q.cascadas as u64;
            vox * (4 + 4 + 8 + 16) + (v.q.n as u64).pow(3) * PALABRAS * 4
        });
        let p = self.pantalla.as_ref().map(|p| (p.size.0 as u64 * p.size.1 as u64) * (2 * 8 + 2 * 4 + 10 * 8));
        v.unwrap_or(0) + p.unwrap_or(0)
    }
}

fn v4(c: [f32; 3]) -> [f32; 4] {
    [c[0], c[1], c[2], 0.0]
}

#[cfg(test)]
mod tests;
