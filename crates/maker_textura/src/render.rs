//! Render sin ventana de una sesión (vista previa para la CLI y el agente). Adaptado de
//! `crates/automation/src/render.rs` de TextureCraft (MIT): subir malla, texture sets y teselas
//! y dibujar un fotograma.

use crate::tc::Session;
use crate::tc::compose::{DisplayKind, all_tiles, tile_bytes};
use crate::tc_render::{Frame, Shade, ViewKind};
use rayon::prelude::*;

/// PNG de la vista 3D de la sesión (`ancho`×`alto`) con su cámara, girada `yaw` grados.
pub fn png(s: &mut Session, ancho: u32, alto: u32, yaw: Option<f32>) -> Result<Vec<u8>, String> {
    let mut h = crate::tc_render::headless::Headless::new()?;
    s.update();
    while s.update() {}
    h.renderer.set_mesh(&h.device, &h.queue, &s.mesh, s.mesh_id);
    let sets: Vec<((u32, u32), usize)> = s.doc.project.texture_sets.iter().map(|t| (t.size(), t.material)).collect();
    h.renderer.set_texture_sets(&h.device, &h.queue, &sets, s.mesh.materials.len().max(1));
    let d = &s.doc.project.display;
    let image = d.environment_resource().and_then(|n| s.images.get(n));
    h.renderer.set_environment(&h.device, &h.queue, &d.environment, image.map(|i| i.as_ref()));
    for ts in 0..s.caches.len() {
        let _ = s.take_uploads(ts);
        let Some(cache) = s.caches.get(ts) else { continue };
        let empty = Default::default();
        let baked = s.doc.baked.get(&ts).unwrap_or(&empty);
        let comp = &cache.composite;
        let bytes: Vec<_> = all_tiles(cache.size.0, cache.size.1)
            .par_iter()
            .flat_map_iter(|&(tx, ty)| DisplayKind::ALL.into_iter().map(move |k| (k, tx, ty)))
            .map(|(k, tx, ty)| (k, tx, ty, tile_bytes(comp, baked, k, tx, ty)))
            .collect();
        for (k, tx, ty, b) in bytes {
            h.renderer.upload_tile(&h.device, &h.queue, ts, k, tx, ty, &b);
        }
    }
    let mut camera = s.camera;
    if let Some(y) = yaw {
        camera.yaw = y.to_radians();
    }
    let frame = Frame {
        view_id: 1,
        size: [ancho.clamp(16, 4096), alto.clamp(16, 4096)],
        kind: ViewKind::Scene,
        camera,
        shade: Shade::from_project(&s.doc.project),
        visible: s.doc.project.texture_sets.iter().map(|t| t.visible).collect(),
    };
    let (w, hgt, rgba) = h.render(&frame).ok_or("la GPU no devolvió imagen")?;
    let mut out = Vec::new();
    image::RgbaImage::from_raw(w, hgt, rgba)
        .ok_or("imagen incompleta")?
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    Ok(out)
}
