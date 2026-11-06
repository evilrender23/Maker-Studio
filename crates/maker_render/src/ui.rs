//! Pase de interfaz: dibuja la lista de `maker_engine::ui` encima de la
//! imagen final (después del 3D y del posproceso; la interfaz no se
//! posprocesa).
//!
//! Un único pipeline de quads instanciados: las instancias se suben en un
//! búfer que crece por potencias de dos y se reutiliza; el atlas de glifos (R8)
//! solo sube las filas que cambiaron; cada lote (misma imagen y recorte) es
//! un `draw` de 4 vértices × N instancias. Las imágenes se cargan una vez, con
//! mipmaps, como texturas sRGB.

use maker_engine::ui::SistemaUi;
use maker_engine::ui::dibujo::{QuadUi, SIN_IMAGEN};
use std::collections::HashMap;
use std::path::PathBuf;

use crate::texture;

const ATTRS: [wgpu::VertexAttribute; 9] = wgpu::vertex_attr_array![
    0 => Float32x2, 1 => Float32x2, 2 => Float32x2, 3 => Float32x4, 4 => Float32x4,
    5 => Float32x4, 6 => Float32x4, 7 => Float32x4, 8 => Float32x4
];

/// Desplazamiento de `uv` en `QuadUi` (tras centro, ejes y relleno).
const OFF_UV: u64 = 32;

pub struct UiRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    globales: wgpu::Buffer,
    bgl0: wgpu::BindGroupLayout,
    bgl1: wgpu::BindGroupLayout,
    muestreo: wgpu::Sampler,
    bg0: Option<wgpu::BindGroup>,
    atlas: Option<(wgpu::Texture, u64, u32)>,
    instancias: wgpu::Buffer,
    capacidad: u64,
    blanco: wgpu::BindGroup,
    imagenes: HashMap<PathBuf, Option<wgpu::BindGroup>>,
    /// Dibujos del último pase.
    pub draw_calls: u32,
}

fn vista_rgba(device: &wgpu::Device, queue: &wgpu::Queue, label: &str, mips: texture::MipChain) -> wgpu::TextureView {
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

impl UiRenderer {
    /// `formato`: el del destino (el del renderizador de escena es `COLOR_FORMAT`).
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, formato: wgpu::TextureFormat) -> UiRenderer {
        let device = device.clone();
        let queue = queue.clone();
        let modulo = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("ui"), source: wgpu::ShaderSource::Wgsl(include_str!("ui.wgsl").into()) });
        let tex_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
            count: None,
        };
        let bgl0 = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ui globales"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                tex_entry(1),
                wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
            ],
        });
        let bgl1 = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("ui imagen"), entries: &[tex_entry(0)] });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("ui"), bind_group_layouts: &[Some(&bgl0), Some(&bgl1)], immediate_size: 0 });
        let premul = wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha, operation: wgpu::BlendOperation::Add };
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("ui"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &modulo,
                entry_point: Some("vs_ui"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<QuadUi>() as u64, step_mode: wgpu::VertexStepMode::Instance, attributes: &ATTRS_UI })],
            },
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleStrip, ..Default::default() },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &modulo,
                entry_point: Some("fs_ui"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format: formato, blend: Some(wgpu::BlendState { color: premul, alpha: premul }), write_mask: wgpu::ColorWrites::ALL })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let globales = device.create_buffer(&wgpu::BufferDescriptor { label: Some("ui globales"), size: 16, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let muestreo = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("ui"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let blanco_v = vista_rgba(&device, &queue, "ui blanco", texture::build_mips(1, 1, vec![255; 4], None));
        let blanco = device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("ui blanco"), layout: &bgl1, entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&blanco_v) }] });
        let capacidad = 256;
        let instancias = Self::buffer(&device, capacidad);
        UiRenderer { device, queue, pipeline, globales, bgl0, bgl1, muestreo, bg0: None, atlas: None, instancias, capacidad, blanco, imagenes: HashMap::new(), draw_calls: 0 }
    }

    fn buffer(device: &wgpu::Device, n: u64) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ui instancias"),
            size: n * std::mem::size_of::<QuadUi>() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    /// Sube el atlas de glifos: entero si cambió de tamaño, si no solo las filas sucias.
    fn subir_atlas(&mut self, ui: &mut SistemaUi) {
        let a = &mut ui.atlas;
        let recrear = self.atlas.as_ref().is_none_or(|(_, g, t)| *g != a.generacion || *t != a.tam);
        if recrear {
            let tex = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("ui atlas"),
                size: wgpu::Extent3d { width: a.tam, height: a.tam, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let vista = tex.create_view(&Default::default());
            self.bg0 = Some(self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("ui globales"),
                layout: &self.bgl0,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: self.globales.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&vista) },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&self.muestreo) },
                ],
            }));
            self.atlas = Some((tex, a.generacion, a.tam));
            a.sucio = Some((0, a.tam));
        }
        if let (Some((y0, y1)), Some((tex, _, _))) = (a.sucio.take(), &self.atlas) {
            let y1 = y1.min(a.tam);
            if y1 > y0 {
                self.queue.write_texture(
                    wgpu::TexelCopyTextureInfo { texture: tex, mip_level: 0, origin: wgpu::Origin3d { x: 0, y: y0, z: 0 }, aspect: wgpu::TextureAspect::All },
                    &a.pixeles[(y0 * a.tam) as usize..(y1 * a.tam) as usize],
                    wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(a.tam), rows_per_image: Some(y1 - y0) },
                    wgpu::Extent3d { width: a.tam, height: y1 - y0, depth_or_array_layers: 1 },
                );
            }
        }
    }

    fn imagen(&mut self, ruta: &std::path::Path) -> Option<&wgpu::BindGroup> {
        if !self.imagenes.contains_key(ruta) {
            let bg = match std::fs::read(ruta).map_err(|e| e.to_string()).and_then(|b| texture::decode(&b)) {
                Ok((w, h, px)) => {
                    let v = vista_rgba(&self.device, &self.queue, "ui imagen", texture::build_mips(w, h, px, None));
                    Some(self.device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("ui imagen"), layout: &self.bgl1, entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&v) }] }))
                }
                Err(e) => {
                    maker_engine::log::error("interfaz", format!("{}: {e}", ruta.display()));
                    None
                }
            };
            self.imagenes.insert(ruta.to_path_buf(), bg);
        }
        self.imagenes.get(ruta).and_then(|b| b.as_ref())
    }

    /// Olvida las imágenes cargadas (tras reimportarlas).
    pub fn olvidar_imagenes(&mut self) {
        self.imagenes.clear();
    }

    /// Dibuja la lista de `ui` sobre `destino`, de `tam` píxeles (si la lista
    /// se hizo para otro tamaño, p. ej. el fotograma en que cambia la ventana,
    /// se escala). `limpiar`: borra antes con ese color (vistas previas).
    pub fn dibujar(&mut self, ui: &mut SistemaUi, destino: &wgpu::TextureView, tam_destino: (u32, u32), limpiar: Option<wgpu::Color>) {
        self.draw_calls = 0;
        if ui.lista.quads.is_empty() && limpiar.is_none() {
            return;
        }
        self.subir_atlas(ui);
        let lista = &ui.lista;
        let n = lista.quads.len() as u64;
        if n > self.capacidad {
            self.capacidad = n.next_power_of_two();
            self.instancias = Self::buffer(&self.device, self.capacidad);
        }
        if n > 0 {
            self.queue.write_buffer(&self.instancias, 0, bytemuck::cast_slice(&lista.quads));
        }
        let tam = [lista.tam[0].max(1) as f32, lista.tam[1].max(1) as f32, 0.0, 0.0];
        self.queue.write_buffer(&self.globales, 0, bytemuck::cast_slice(&tam));
        // Imágenes de los lotes (las nuevas se cargan aquí, fuera del pase).
        for l in &lista.lotes {
            if l.imagen != SIN_IMAGEN
                && let Some(r) = ui.imagenes.ruta(l.imagen)
            {
                let r = r.to_path_buf();
                self.imagen(&r);
            }
        }
        let mut enc = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("ui") });
        {
            let mut pase = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("ui"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: destino,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: limpiar.map(wgpu::LoadOp::Clear).unwrap_or(wgpu::LoadOp::Load), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if n > 0
                && let Some(bg0) = &self.bg0
            {
                pase.set_pipeline(&self.pipeline);
                pase.set_bind_group(0, bg0, &[]);
                pase.set_vertex_buffer(0, self.instancias.slice(..));
                let mut ligada: Option<u32> = None;
                for l in &lista.lotes {
                    let clave = if l.imagen == SIN_IMAGEN { u32::MAX } else { l.imagen };
                    if ligada != Some(clave) {
                        let bg = ui.imagenes.ruta(l.imagen).and_then(|r| self.imagenes.get(r)).and_then(|b| b.as_ref()).unwrap_or(&self.blanco);
                        pase.set_bind_group(1, bg, &[]);
                        ligada = Some(clave);
                    }
                    // Recorte de la lista a píxeles del destino, dentro de sus límites.
                    let (dw, dh) = (tam_destino.0.max(1), tam_destino.1.max(1));
                    let kx = dw as f32 / lista.tam[0].max(1) as f32;
                    let ky = dh as f32 / lista.tam[1].max(1) as f32;
                    let [x, y, w, h] = l.recorte;
                    let x0 = ((x as f32 * kx).floor() as u32).min(dw);
                    let y0 = ((y as f32 * ky).floor() as u32).min(dh);
                    let x1 = (((x + w) as f32 * kx).ceil() as u32).min(dw);
                    let y1 = (((y + h) as f32 * ky).ceil() as u32).min(dh);
                    let (x, y, w, h) = (x0, y0, x1.saturating_sub(x0), y1.saturating_sub(y0));
                    if w == 0 || h == 0 {
                        continue;
                    }
                    pase.set_scissor_rect(x, y, w, h);
                    pase.draw(0..4, l.inicio..l.fin);
                    self.draw_calls += 1;
                }
            }
        }
        self.queue.submit(Some(enc.finish()));
    }
}

const ATTRS_UI: [wgpu::VertexAttribute; 9] = {
    // Igual que `ATTRS` pero saltando el relleno `_r` de `QuadUi` tras los ejes.
    let mut a = ATTRS;
    let mut k = 3;
    while k < 9 {
        a[k].offset = OFF_UV + (k as u64 - 3) * 16;
        k += 1;
    }
    a
};

/// Textura de color fuera de pantalla para vistas previas y pruebas.
pub struct ObjetivoUi {
    pub textura: wgpu::Texture,
    pub vista: wgpu::TextureView,
    pub tam: (u32, u32),
}

impl ObjetivoUi {
    pub fn new(device: &wgpu::Device, w: u32, h: u32) -> ObjetivoUi {
        let textura = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("ui vista previa"),
            size: wgpu::Extent3d { width: w.max(1), height: h.max(1), depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: crate::COLOR_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let vista = textura.create_view(&Default::default());
        ObjetivoUi { textura, vista, tam: (w.max(1), h.max(1)) }
    }

    /// RGBA8 de la textura (bloquea; para pruebas).
    pub fn leer(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> Vec<u8> {
        let (w, h) = self.tam;
        let bpr = (w * 4).div_ceil(256) * 256;
        let buf = device.create_buffer(&wgpu::BufferDescriptor { label: Some("ui lectura"), size: (bpr * h) as u64, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
        let mut enc = device.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(
            self.textura.as_image_copy(),
            wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(bpr), rows_per_image: Some(h) } },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        queue.submit(Some(enc.finish()));
        let slice = buf.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        let mut out = Vec::with_capacity((w * h * 4) as usize);
        if rx.recv().is_ok_and(|r| r.is_ok())
            && let Ok(data) = slice.get_mapped_range()
        {
            for row in 0..h {
                let s = (row * bpr) as usize;
                out.extend_from_slice(&data[s..s + (w * 4) as usize]);
            }
        }
        buf.unmap();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use maker_engine::scene::Scene;
    use maker_engine::ui::estilo::{Relleno, Sombra};
    use maker_engine::ui::*;

    fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).ok()?;
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()
    }

    #[test]
    fn el_formato_de_quadui_coincide_con_el_sombreador() {
        assert_eq!(std::mem::size_of::<QuadUi>(), 32 + 6 * 16);
        assert_eq!(ATTRS_UI[3].offset, 32);
        assert_eq!(ATTRS_UI[8].offset, 32 + 5 * 16);
        assert_eq!(ATTRS_UI[2].offset, 16);
    }

    /// Interfaz pequeña sin ventana: panel redondeado con borde y sombra, una
    /// barra a medias, un texto y una máscara; comprueba píxeles.
    #[test]
    fn renderiza_una_interfaz_sin_ventana() {
        let Some((device, queue)) = device() else {
            eprintln!("sin adaptador GPU: test omitido");
            return;
        };
        let mut s = Scene::new();
        let l = s.spawn("Lienzo");
        s.insert(l, Lienzo { resolucion: [200.0, 100.0], ..Default::default() });
        let panel = s.spawn("Panel");
        s.insert(panel, RectUi::anclado([0.0, 0.0], [0.0, 0.0], [20.0, 20.0], [80.0, 60.0]));
        let mut w = WidgetUi::nuevo(TipoWidget::Panel);
        let n = &mut w.local.estados.normal;
        n.fondo = Some(Relleno::Color([1.0, 0.0, 0.0, 1.0]));
        n.borde_color = Some([0.0, 0.0, 1.0, 1.0]);
        n.borde_ancho = Some(4.0);
        n.radio = Some([20.0; 4]);
        n.sombra = Some(Sombra { color: [0.0, 0.0, 0.0, 0.6], desplazamiento: [0.0, 0.0], difuminado: 6.0, expansion: 0.0 });
        s.insert(panel, w);
        s.set_parent(panel, Some(l), false);
        let barra = s.spawn("Barra");
        s.insert(barra, RectUi::anclado([0.0, 0.0], [0.0, 0.0], [120.0, 10.0], [60.0, 20.0]));
        let mut b = WidgetUi::nuevo(TipoWidget::Barra);
        b.clase.fijar_valor(0.5);
        b.local.estados.normal.fondo = Some(Relleno::Color([0.0, 0.0, 0.0, 1.0]));
        b.local.capa_mut(Some("relleno")).normal.fondo = Some(Relleno::Color([0.0, 1.0, 0.0, 1.0]));
        s.insert(barra, b);
        s.set_parent(barra, Some(l), false);
        let texto = s.spawn("Texto");
        s.insert(texto, RectUi::anclado([0.0, 0.0], [0.0, 0.0], [110.0, 40.0], [90.0, 50.0]));
        let mut t = WidgetUi { clase: ClaseWidget::Texto(Texto { texto: "HH".into(), ..Default::default() }), ..Default::default() };
        t.local.estados.normal.tam_texto = Some(40.0);
        t.local.estados.normal.color_texto = Some([1.0, 1.0, 0.0, 1.0]);
        s.insert(texto, t);
        s.set_parent(texto, Some(l), false);

        let mut ui = SistemaUi::default();
        ui.actualizar(&s, [200.0, 100.0], 0.0);
        let obj = ObjetivoUi::new(&device, 200, 100);
        let mut r = UiRenderer::new(&device, &queue, crate::COLOR_FORMAT);
        r.dibujar(&mut ui, &obj.vista, obj.tam, Some(wgpu::Color::WHITE));
        assert_eq!(r.draw_calls, 1, "todo en un lote");
        let px = obj.leer(&device, &queue);
        let at = |x: u32, y: u32| {
            let i = ((y * 200 + x) * 4) as usize;
            [px[i], px[i + 1], px[i + 2], px[i + 3]]
        };
        assert_eq!(at(60, 50), [255, 0, 0, 255], "relleno del panel");
        let borde = at(22, 50);
        assert!(borde[2] > 200 && borde[0] < 40, "borde azul: {borde:?}");
        let esquina = at(24, 24);
        assert!(esquina[2] <= esquina[0] + 4, "la esquina redondeada no es borde: {esquina:?}");
        assert!(esquina[0] < 250 || esquina[1] < 250, "pero sí sombra: {esquina:?}");
        assert_eq!(at(5, 95), [255, 255, 255, 255], "fondo intacto lejos de todo");
        assert_eq!(at(130, 20), [0, 255, 0, 255], "mitad llena de la barra");
        assert_eq!(at(170, 20), [0, 0, 0, 255], "mitad vacía");
        let amarillos = (110..200).flat_map(|x| (40..90).map(move |y| (x, y))).filter(|&(x, y)| {
            let p = at(x, y);
            p[0] > 200 && p[1] > 200 && p[2] < 80
        });
        assert!(amarillos.count() > 80, "el texto se ve");

        // Máscara: el panel recorta a un hijo que se sale.
        let hijo = s.spawn("Hijo");
        s.insert(hijo, RectUi::anclado([0.0, 0.0], [0.0, 0.0], [40.0, 30.0], [200.0, 20.0]));
        let mut w = WidgetUi::nuevo(TipoWidget::Panel);
        w.local.estados.normal.fondo = Some(Relleno::Color([0.0, 1.0, 0.0, 1.0]));
        s.insert(hijo, w);
        s.set_parent(hijo, Some(panel), false);
        s.get_mut::<RectUi>(panel).unwrap().recortar = true;
        ui.marcar_sucio();
        ui.actualizar(&s, [200.0, 100.0], 0.0);
        r.dibujar(&mut ui, &obj.vista, obj.tam, Some(wgpu::Color::WHITE));
        let px2 = obj.leer(&device, &queue);
        let at2 = |x: u32, y: u32| {
            let i = ((y * 200 + x) * 4) as usize;
            [px2[i], px2[i + 1], px2[i + 2]]
        };
        assert_eq!(at2(80, 55), [0, 255, 0], "hijo dentro del panel");
        assert_eq!(at2(108, 55), [255, 255, 255], "recortado fuera del panel");
        assert!(r.draw_calls >= 2);
    }
}
