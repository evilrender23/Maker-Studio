//! Malla esculpida en la GPU: una copia del modelo con el búfer de vértices escribible, de la
//! que cada fotograma solo se reescriben los rangos que cambiaron (`queue.write_buffer` por
//! rango). La máscara oscurece el color de vértice; la normal de cada vértice se recalcula en
//! la CPU y la tangente se reortogonaliza contra ella.

use super::*;
use std::ops::Range;

pub(crate) struct SculptGpu {
    pub(crate) entity: EntityId,
    pub(crate) model: usize,
    cpu: Vec<Vertex>,
    smooth: Vec<[f32; 3]>,
    /// Color y tangente originales de cada vértice de render.
    color0: Vec<[u8; 4]>,
    tangent0: Vec<Vec4>,
    pub(crate) wire: Option<(wgpu::Buffer, u32)>,
    /// Sube con cada escritura (huella de la GI).
    pub(crate) version: u64,
    pub(crate) uploaded: u64,
}

/// Cuánto oscurece la máscara (1 = negro).
const MASK_SHADE: f32 = 0.6;

fn snorm(t: Vec4) -> [i16; 4] {
    t.to_array().map(|c| (c.clamp(-1.0, 1.0) * 32767.0).round() as i16)
}

impl SceneRenderer {
    /// Empieza a dibujar `entity` con una copia dinámica de `data` (en vez de su malla).
    pub fn sculpt_begin(&mut self, entity: EntityId, data: &ModelData) {
        self.sculpt_end();
        let m = &data.mesh;
        let n = m.positions.len();
        let mi = self.upload_model(data, true);
        // Reutilizar siempre el mismo hueco para no crecer la lista de modelos.
        let mi = match self.sculpt_slot {
            Some(slot) if slot != mi => {
                self.models.swap(slot, mi);
                self.models.pop();
                slot
            }
            _ => mi,
        };
        self.sculpt_slot = Some(mi);
        let cpu: Vec<Vertex> = (0..n)
            .map(|i| Vertex {
                pos: m.positions[i].to_array(),
                normal: m.normals.get(i).copied().unwrap_or(Vec3::Y).to_array(),
                uv: m.uvs.get(i).copied().unwrap_or_default().to_array(),
                color: m.colors.get(i).copied().unwrap_or([255; 4]),
                tangent: snorm(m.tangents.get(i).copied().unwrap_or(Vec4::new(1.0, 0.0, 0.0, 1.0))),
            })
            .collect();
        self.sculpt = Some(SculptGpu {
            entity,
            model: mi,
            smooth: cpu.iter().map(|v| v.normal).collect(),
            color0: cpu.iter().map(|v| v.color).collect(),
            tangent0: (0..n).map(|i| m.tangents.get(i).copied().unwrap_or(Vec4::new(1.0, 0.0, 0.0, 1.0))).collect(),
            cpu,
            wire: None,
            version: 0,
            uploaded: 0,
        });
    }

    /// Deja de dibujar la malla dinámica (la entidad vuelve a su malla) y libera su memoria.
    pub fn sculpt_end(&mut self) {
        if let Some(s) = self.sculpt.take() {
            // El hueco queda con un modelo mínimo hasta la próxima sesión.
            let tiny = ModelData::from_mesh(MeshData { positions: vec![Vec3::ZERO; 3], normals: vec![Vec3::Y; 3], indices: vec![0, 1, 2], ..Default::default() });
            let i = self.upload_model(&tiny, false);
            self.models.swap(s.model, i);
            self.models.pop();
        }
    }

    /// Entidad que se dibuja con la malla dinámica.
    pub fn sculpt_entity(&self) -> Option<EntityId> {
        self.sculpt.as_ref().map(|s| s.entity)
    }

    /// Reescribe los vértices de render de `ranges`. `vertex(i)` da posición, normal y
    /// máscara (0..1). Devuelve los bytes subidos.
    pub fn sculpt_write(&mut self, ranges: &[Range<u32>], vertex: impl Fn(u32) -> (Vec3, Vec3, f32)) -> u64 {
        let Some(s) = self.sculpt.as_mut() else { return 0 };
        let model = &self.models[s.model];
        let stride = std::mem::size_of::<Vertex>() as u64;
        let mut bytes = 0;
        for r in ranges {
            let (a, b) = (r.start as usize, (r.end as usize).min(s.cpu.len()));
            if a >= b {
                continue;
            }
            for i in a..b {
                let (p, n, mask) = vertex(i as u32);
                let v = &mut s.cpu[i];
                v.pos = p.to_array();
                v.normal = n.to_array();
                let k = 1.0 - MASK_SHADE * mask.clamp(0.0, 1.0);
                let c = s.color0[i];
                v.color = [(c[0] as f32 * k) as u8, (c[1] as f32 * k) as u8, (c[2] as f32 * k) as u8, c[3]];
                let t0 = s.tangent0[i];
                let t = (t0.truncate() - n * t0.truncate().dot(n)).try_normalize().unwrap_or_else(|| n.any_orthonormal_vector());
                v.tangent = snorm(t.extend(t0.w));
                s.smooth[i] = v.normal;
            }
            self.queue.write_buffer(&model.vbuf, a as u64 * stride, bytemuck::cast_slice(&s.cpu[a..b]));
            self.queue.write_buffer(&model.smooth, a as u64 * 12, bytemuck::cast_slice(&s.smooth[a..b]));
            bytes += (b - a) as u64 * (stride + 12);
        }
        if bytes > 0 {
            s.version = s.version.wrapping_add(1);
            s.uploaded += bytes;
        }
        bytes
    }

    /// Caja local de la malla esculpida (recorte por frustum y selección).
    pub fn sculpt_set_bounds(&mut self, aabb: Aabb) {
        if let Some(s) = &self.sculpt {
            self.models[s.model].aabb = aabb;
        }
    }

    /// Aristas (pares de índices de render) para la vista de alambre; `None` las libera.
    pub fn sculpt_set_wire(&mut self, edges: Option<&[u32]>) {
        let Some(s) = self.sculpt.as_mut() else { return };
        s.wire = edges.filter(|e| !e.is_empty()).map(|e| {
            let buf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("sculpt wire"), contents: bytemuck::cast_slice(e), usage: wgpu::BufferUsages::INDEX });
            (buf, e.len() as u32)
        });
    }

    pub fn sculpt_has_wire(&self) -> bool {
        self.sculpt.as_ref().is_some_and(|s| s.wire.is_some())
    }

    /// Pipeline de líneas de la vista de alambre (encima de la superficie, semitransparente).
    pub(crate) fn wire_pipeline(&self, hdr: bool) -> wgpu::RenderPipeline {
        let vb = [Some(vertex_layout())];
        let targets = [Some(wgpu::ColorTargetState { format: target_format(hdr), blend: Some(wgpu::BlendState::ALPHA_BLENDING), write_mask: wgpu::ColorWrites::ALL })];
        self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("sculpt wire"),
            layout: Some(&self.ink_layout),
            vertex: wgpu::VertexState { module: &self.module, entry_point: Some("vs_wire"), compilation_options: Default::default(), buffers: &vb },
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::LineList, ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState { count: SAMPLES, mask: !0, alpha_to_coverage_enabled: false },
            fragment: Some(wgpu::FragmentState { module: &self.module, entry_point: Some("fs_wire"), compilation_options: Default::default(), targets: &targets }),
            multiview_mask: None,
            cache: None,
        })
    }
}
