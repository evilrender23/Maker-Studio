//! Puente entre la sesión de TextureCraft y su renderer wgpu dentro de egui: sincroniza malla,
//! texture sets, entorno y teselas sucias cada fotograma y dibuja la vista con un callback.
//! Adaptado de `crates/ui-egui/src/gpu.rs` de TextureCraft (MIT, commit e01dac2).

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use egui_wgpu::{CallbackResources, CallbackTrait, ScreenDescriptor};
use maker_textura::tc::Session;
use maker_textura::tc::compose::{DisplayKind, ImageRes, tile_bytes};
use maker_textura::tc::core::{BakedMap, LayerId};
use maker_textura::tc::geom::Mesh;
use maker_textura::tc_render::{Frame, Renderer};
use rayon::prelude::*;

/// Lo que muestra la textura auxiliar (mapas horneados, máscaras).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Aux {
    Mapa(BakedMap),
    Mascara(LayerId),
}

type Layout = ((u32, u32), usize);

#[derive(Default)]
pub struct Sync {
    mesh: Option<(Arc<Mesh>, u64)>,
    sets: Option<(Vec<Layout>, usize)>,
    env: Option<(String, Option<Arc<ImageRes>>)>,
    tiles: Vec<(usize, DisplayKind, u32, u32, Vec<u8>)>,
    aux: Vec<(usize, u32, Vec<u8>)>,
}

/// Vive en los recursos de callback de egui.
pub struct Gpu {
    pub renderer: Renderer,
    reset: Arc<AtomicBool>,
}

/// Lo que la GPU ya tiene, del lado de la interfaz.
pub struct Enlace {
    reset: Arc<AtomicBool>,
    perdido: Arc<AtomicBool>,
    mesh_id: u64,
    sets: Vec<Layout>,
    env: (String, usize),
    aux: BTreeMap<usize, (String, u64)>,
}

/// Composición por fotograma: un texture set grande se recompone en varios.
const PRESUPUESTO_MS: f64 = 30.0;
const MIN_MS: f64 = 4.0;
const TROZO: usize = 64;

pub fn iniciar(rs: &egui_wgpu::RenderState) -> Enlace {
    let reset = Arc::new(AtomicBool::new(false));
    let renderer = Renderer::new(&rs.device, &rs.queue, rs.target_format);
    rs.renderer.write().callback_resources.insert(Gpu { renderer, reset: reset.clone() });
    Enlace { reset, perdido: Arc::new(AtomicBool::new(false)), mesh_id: 0, sets: Vec::new(), env: Default::default(), aux: BTreeMap::new() }
}

/// Libera el renderer al salir del modo.
pub fn cerrar(rs: &egui_wgpu::RenderState) {
    rs.renderer.write().callback_resources.remove::<Gpu>();
}

impl Enlace {
    /// Compone lo pendiente (con presupuesto) y reúne lo que la GPU necesita.
    pub fn sync(&mut self, s: &mut Session, aux: Option<Aux>) -> Sync {
        let t0 = Instant::now();
        let ms = || t0.elapsed().as_secs_f64() * 1000.0;
        let clock = move || t0.elapsed().as_secs_f64() * 1000.0;
        s.update_for(PRESUPUESTO_MS, clock);
        let mut out = Sync::default();
        if self.perdido.swap(false, Ordering::AcqRel) {
            self.mesh_id = 0;
            self.sets.clear();
            self.env = Default::default();
            self.reset.store(true, Ordering::Release);
        }
        if self.mesh_id != s.mesh_id {
            out.mesh = Some((s.mesh.clone(), s.mesh_id));
            self.mesh_id = s.mesh_id;
            self.aux.clear();
        }
        let sets: Vec<Layout> = s.doc.project.texture_sets.iter().map(|t| (t.size(), t.material)).collect();
        if sets != self.sets || out.mesh.is_some() {
            out.sets = Some((sets.clone(), s.mesh.materials.len().max(1)));
            self.sets = sets;
        }
        let d = &s.doc.project.display;
        let image = d.environment_resource().and_then(|n| s.images.get(n));
        let ptr = image.map_or(0, |i| Arc::as_ptr(i) as usize);
        if self.env.0 != d.environment || self.env.1 != ptr {
            self.env = (d.environment.clone(), ptr);
            out.env = Some((d.environment.clone(), image.cloned()));
        }
        if self.reset.swap(false, Ordering::AcqRel) {
            for c in &mut s.caches {
                c.upload.extend(maker_textura::tc::compose::all_tiles(c.size.0, c.size.1));
            }
            self.aux.clear();
        }
        for ts in 0..s.caches.len() {
            let tiles = s.take_uploads(ts);
            if tiles.is_empty() {
                continue;
            }
            let mut hechas = 0;
            if let Some(cache) = s.caches.get(ts) {
                let vacio = Default::default();
                let baked = s.doc.baked.get(&ts).unwrap_or(&vacio);
                let comp = &cache.composite;
                for trozo in tiles.chunks(TROZO) {
                    let bytes: Vec<_> = trozo
                        .par_iter()
                        .flat_map_iter(|&(tx, ty)| DisplayKind::ALL.into_iter().map(move |k| (k, tx, ty)))
                        .map(|(k, tx, ty)| (ts, k, tx, ty, tile_bytes(comp, baked, k, tx, ty)))
                        .collect();
                    out.tiles.extend(bytes);
                    hechas += trozo.len();
                    if PRESUPUESTO_MS + 5.0 - ms() < MIN_MS {
                        break;
                    }
                }
            }
            if let (Some(resto), Some(cache)) = (tiles.get(hechas..), s.caches.get_mut(ts)) {
                cache.upload.extend(resto.iter().copied());
            }
        }
        match aux {
            Some(Aux::Mapa(m)) => {
                for ts in 0..s.caches.len() {
                    let rev = s.doc.baked.get(&ts).and_then(|b| b.maps.get(&m)).map(|d| Arc::as_ptr(d) as u64).unwrap_or(0);
                    let key = (format!("{m:?}"), rev);
                    if self.aux.get(&ts) == Some(&key) {
                        continue;
                    }
                    let res = s.caches.get(ts).map_or(1024, |c| maker_textura::tc_render::aux_size(c.size));
                    out.aux.push((ts, res, maker_textura::tc_render::mesh_maps::mesh_map_rgba(s.doc.baked.get(&ts), m, res)));
                    self.aux.insert(ts, key);
                }
            }
            Some(Aux::Mascara(id)) => {
                if let Some((ts, _, _)) = s.doc.project.locate(id) {
                    let key = (format!("mask{}", id.0), s.revision);
                    if self.aux.get(&ts) != Some(&key) && !s.stroke_active() {
                        let res = s.caches.get(ts).map_or(1024, |c| maker_textura::tc_render::aux_size(c.size));
                        if let Some(rgba) = s.mask_view_rgba(id, res) {
                            out.aux.push((ts, res, rgba));
                        }
                        self.aux.insert(ts, key);
                    }
                }
            }
            None => {}
        }
        out
    }

    pub fn callback(&self, frame: Frame, sync: Option<Sync>, rect: egui::Rect) -> Vista {
        Vista { frame, sync: Mutex::new(sync), rect, perdido: self.perdido.clone() }
    }
}

/// Callback de dibujo de una vista.
pub struct Vista {
    frame: Frame,
    sync: Mutex<Option<Sync>>,
    rect: egui::Rect,
    perdido: Arc<AtomicBool>,
}

impl Drop for Vista {
    fn drop(&mut self) {
        // Un fotograma que no llegó a dibujarse se lleva su sincronización: se repite entera.
        if self.sync.get_mut().unwrap_or_else(PoisonError::into_inner).is_some() {
            self.perdido.store(true, Ordering::Release);
        }
    }
}

impl CallbackTrait for Vista {
    fn prepare(&self, device: &wgpu::Device, queue: &wgpu::Queue, screen: &ScreenDescriptor, encoder: &mut wgpu::CommandEncoder, res: &mut CallbackResources) -> Vec<wgpu::CommandBuffer> {
        let Some(gpu) = res.get_mut::<Gpu>() else { return Vec::new() };
        if let Some(sync) = self.sync.lock().unwrap_or_else(PoisonError::into_inner).take() {
            if let Some((env, image)) = &sync.env {
                gpu.renderer.set_environment(device, queue, env, image.as_deref());
            }
            if let Some((mesh, id)) = &sync.mesh {
                gpu.renderer.set_mesh(device, queue, mesh, *id);
            }
            if let Some((sets, mats)) = &sync.sets
                && !gpu.renderer.set_texture_sets(device, queue, sets, *mats).is_empty()
            {
                gpu.reset.store(true, Ordering::Release);
            }
            for (ts, k, tx, ty, bytes) in &sync.tiles {
                gpu.renderer.upload_tile(device, queue, *ts, *k, *tx, *ty, bytes);
            }
            for (ts, size, bytes) in &sync.aux {
                gpu.renderer.upload_aux(queue, *ts, *size, bytes);
            }
        }
        gpu.renderer.prepare(device, queue, encoder, &self.frame);
        let ppp = screen.pixels_per_point;
        let r = self.rect;
        gpu.renderer.set_blit_rect(queue, self.frame.view_id, [r.min.x * ppp, r.min.y * ppp, r.width() * ppp, r.height() * ppp], screen.size_in_pixels);
        Vec::new()
    }

    fn paint(&self, info: egui::PaintCallbackInfo, pass: &mut wgpu::RenderPass<'static>, res: &CallbackResources) {
        let Some(gpu) = res.get::<Gpu>() else { return };
        let [sw, sh] = info.screen_size_px;
        let clip = info.clip_rect_in_pixels();
        if clip.width_px <= 0 || clip.height_px <= 0 || sw == 0 || sh == 0 {
            return;
        }
        pass.set_viewport(0.0, 0.0, sw as f32, sh as f32, 0.0, 1.0);
        pass.set_scissor_rect(clip.left_px.max(0) as u32, clip.top_px.max(0) as u32, clip.width_px as u32, clip.height_px as u32);
        gpu.renderer.blit_prepared(pass, self.frame.view_id);
    }
}
