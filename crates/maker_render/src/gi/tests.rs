//! Pruebas de píxeles de la GI sin ventana. Todas usan el `device()`
//! compartido (cerrojo: las pruebas de GPU van de una en una).

use super::*;
use crate::renderer::tests::device;
use crate::{RenderOptions, SceneRenderer};
use maker_engine::components::*;
use maker_engine::iluminacion::ModoGi;
use maker_engine::scene::Scene;

const W: u32 = 320;
const H: u32 = 180;

fn caja(s: &mut Scene, nombre: &str, pos: Vec3, escala: Vec3, mat: Material) -> EntityId {
    let e = s.spawn(nombre);
    s.set_transform(e, Transform::from_translation(pos).with_scale(escala));
    s.insert(e, MeshRenderer { mesh: MeshSource::Primitive(PrimitiveShape::Cube), material: mat, cast_shadows: true });
    e
}

fn camara(s: &mut Scene, ojo: Vec3, mira: Vec3) {
    let c = s.spawn("Cámara");
    let f = (mira - ojo).normalize();
    // Sin alabeo: guiñada y cabeceo.
    let (yaw, pitch) = (f.x.atan2(-f.z), f.y.asin());
    let q = glam::Quat::from_rotation_y(-yaw) * glam::Quat::from_rotation_x(pitch);
    s.set_transform(c, Transform::from_translation(ojo).with_rotation(q));
    s.insert(c, Camera::default());
}

fn sol(s: &mut Scene, dir: Vec3, intensidad: f32) {
    let e = s.spawn("Sol");
    let q = glam::Quat::from_rotation_arc(Vec3::NEG_Z, dir.normalize());
    s.set_transform(e, Transform::default().with_rotation(q));
    s.insert(e, Light { intensity: intensidad, ..Light::directional() });
}

fn noche(s: &mut Scene) {
    s.env.sky_top = [0.0; 3];
    s.env.sky_horizon = [0.0; 3];
    s.env.ground = [0.0; 3];
    s.env.ambient_intensity = 0.0;
    s.env.fog_density = 0.0;
}

fn con_gi(s: &mut Scene, on: bool) {
    s.env.iluminacion_global = IluminacionGlobal { modo: if on { ModoGi::Software } else { ModoGi::Desactivada }, ..Default::default() };
}

/// Renderiza `frames` fotogramas (la GI acumula) y devuelve los píxeles.
fn render(r: &mut SceneRenderer, s: &Scene, frames: u32, debug: GiDebug) -> Vec<u8> {
    let (id, cam) = s.primary_camera().unwrap();
    let view = RenderView::from_camera(s.global(id), &cam, W as f32 / H as f32);
    for _ in 0..frames {
        r.render(s, &view, &RenderOptions { shadows: true, gi: true, gi_debug: debug, ..Default::default() });
    }
    r.read_pixels().unwrap().2
}

fn pantalla(s: &Scene, p: Vec3) -> (u32, u32) {
    let (id, cam) = s.primary_camera().unwrap();
    let view = RenderView::from_camera(s.global(id), &cam, W as f32 / H as f32);
    let q = maker_engine::math::world_to_screen(view.view_proj(), p, glam::Vec2::new(W as f32, H as f32)).unwrap();
    (q.x as u32, q.y as u32)
}

/// Media lineal de un cuadrado de 5×5 píxeles.
fn media(px: &[u8], (x, y): (u32, u32)) -> Vec3 {
    let mut acc = Vec3::ZERO;
    for dy in -2i32..=2 {
        for dx in -2i32..=2 {
            let (xx, yy) = ((x as i32 + dx).clamp(0, W as i32 - 1) as u32, (y as i32 + dy).clamp(0, H as i32 - 1) as u32);
            let i = ((yy * W + xx) * 4) as usize;
            let l = |c: u8| {
                let c = c as f32 / 255.0;
                if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
            };
            acc += Vec3::new(l(px[i]), l(px[i + 1]), l(px[i + 2]));
        }
    }
    acc / 25.0
}

fn lum(c: Vec3) -> f32 {
    c.dot(Vec3::new(0.2126, 0.7152, 0.0722))
}

/// Pared roja iluminada por el sol junto a una caja blanca: con GI la cara
/// de la caja que mira a la pared (a la sombra del sol) se tiñe de rojo.
#[test]
fn red_wall_bleeds_onto_white_box() {
    let Some((device, queue, _gpu)) = device() else { return };
    let mut s = Scene::new();
    s.env.fog_density = 0.0;
    caja(&mut s, "Suelo", Vec3::new(0.0, -0.1, 0.0), Vec3::new(8.0, 0.2, 8.0), Material::color([0.7, 0.7, 0.7]));
    caja(&mut s, "Pared roja", Vec3::new(-1.0, 1.0, 0.0), Vec3::new(0.3, 2.0, 3.0), Material::color([0.9, 0.05, 0.05]));
    caja(&mut s, "Caja blanca", Vec3::new(1.0, 0.6, 0.0), Vec3::new(1.0, 1.2, 1.0), Material::color([0.9, 0.9, 0.9]));
    sol(&mut s, Vec3::new(-1.0, -0.7, -0.2), 4.0);
    camara(&mut s, Vec3::new(-0.4, 1.3, 4.0), Vec3::new(0.5, 0.6, 0.0));
    s.update_transforms();
    let punto = pantalla(&s, Vec3::new(0.49, 0.6, 0.2));
    let mut r = SceneRenderer::new(&device, &queue);
    r.resize(W, H);
    con_gi(&mut s, false);
    let off = media(&render(&mut r, &s, 1, GiDebug::Ninguna), punto);
    assert!(r.gi.is_none(), "sin GI no se reserva nada");
    con_gi(&mut s, true);
    let on = media(&render(&mut r, &s, 12, GiDebug::Ninguna), punto);
    assert!(r.stats.gi, "{:?}", r.stats);
    let rojez = |c: Vec3| c.x / (c.y + c.z + 1e-3);
    eprintln!("pared roja: off {off:?} ({}) on {on:?} ({})", rojez(off), rojez(on));
    assert!(rojez(on) > rojez(off) * 1.5 && on.x > off.x, "off {off:?} on {on:?}");
}

/// Un cubo emisivo en una escena sin más luz ilumina el suelo de alrededor.
#[test]
fn emissive_cube_lights_the_floor() {
    let Some((device, queue, _gpu)) = device() else { return };
    let mut s = Scene::new();
    noche(&mut s);
    caja(&mut s, "Suelo", Vec3::new(0.0, -0.1, 0.0), Vec3::new(8.0, 0.2, 8.0), Material::color([0.8, 0.8, 0.8]));
    caja(&mut s, "Cubo", Vec3::new(0.0, 0.5, 0.0), Vec3::ONE, Material { emissive: [0.2, 1.0, 0.3], emissive_strength: 6.0, ..Material::color([0.2, 1.0, 0.3]) });
    camara(&mut s, Vec3::new(0.0, 3.0, 5.0), Vec3::new(0.0, 0.0, 0.5));
    s.update_transforms();
    let suelo = pantalla(&s, Vec3::new(1.0, 0.0, 1.0));
    let mut r = SceneRenderer::new(&device, &queue);
    r.resize(W, H);
    con_gi(&mut s, false);
    let off = media(&render(&mut r, &s, 1, GiDebug::Ninguna), suelo);
    con_gi(&mut s, true);
    let on = media(&render(&mut r, &s, 12, GiDebug::Ninguna), suelo);
    eprintln!("cubo emisivo: off {off:?} on {on:?}");
    assert!(lum(on) > lum(off) + 0.03 && on.y > on.x, "off {off:?} on {on:?}");
}

fn habitacion(techo: bool) -> Scene {
    let mut s = Scene::new();
    s.env.fog_density = 0.0;
    s.env.ambient_intensity = 1.0;
    let b = Material::color([0.8, 0.8, 0.8]);
    caja(&mut s, "Suelo", Vec3::new(0.0, -0.15, 0.0), Vec3::new(6.0, 0.3, 6.0), b.clone());
    for (n, p, e) in [
        ("Pared N", Vec3::new(0.0, 1.5, -2.85), Vec3::new(6.0, 3.6, 0.3)),
        ("Pared S", Vec3::new(0.0, 1.5, 2.85), Vec3::new(6.0, 3.6, 0.3)),
        ("Pared E", Vec3::new(2.85, 1.5, 0.0), Vec3::new(0.3, 3.6, 6.0)),
        ("Pared O", Vec3::new(-2.85, 1.5, 0.0), Vec3::new(0.3, 3.6, 6.0)),
    ] {
        caja(&mut s, n, p, e, b.clone());
    }
    if techo {
        caja(&mut s, "Techo", Vec3::new(0.0, 3.15, 0.0), Vec3::new(6.0, 0.3, 6.0), b.clone());
    }
    camara(&mut s, Vec3::new(0.0, 2.0, 2.0), Vec3::new(0.0, 0.0, -0.8));
    con_gi(&mut s, true);
    s.update_transforms();
    s
}

/// Una habitación cerrada, solo con luz del cielo, queda mucho más oscura
/// que la misma sin techo (el ambiente plano de siempre no lo distingue).
#[test]
fn enclosed_room_is_darker_than_open_sky() {
    let Some((device, queue, _gpu)) = device() else { return };
    let abierta = habitacion(false);
    let cerrada = habitacion(true);
    let punto = pantalla(&abierta, Vec3::new(0.0, 0.0, -0.8));
    // Un renderizador por escena: el historial temporal de una no ayuda a la otra.
    let mut r = SceneRenderer::new(&device, &queue);
    r.resize(W, H);
    let a = media(&render(&mut r, &abierta, 12, GiDebug::Ninguna), punto);
    let mut r = SceneRenderer::new(&device, &queue);
    r.resize(W, H);
    let c = media(&render(&mut r, &cerrada, 12, GiDebug::Ninguna), punto);
    eprintln!("habitación: abierta {a:?} cerrada {c:?}");
    assert!(lum(c) < lum(a) * 0.4, "abierta {a:?} cerrada {c:?}");
}

/// Un suelo pulido refleja el objeto que tiene delante (sin GI refleja el cielo).
#[test]
fn glossy_floor_reflects_object() {
    let Some((device, queue, _gpu)) = device() else { return };
    let mut s = Scene::new();
    s.env.fog_density = 0.0;
    caja(&mut s, "Suelo", Vec3::new(0.0, -0.1, 0.0), Vec3::new(10.0, 0.2, 10.0), Material { metallic: 1.0, roughness: 0.05, ..Material::color([0.9, 0.9, 0.9]) });
    caja(&mut s, "Caja roja", Vec3::new(0.0, 0.5, 0.0), Vec3::ONE, Material { emissive: [1.0, 0.1, 0.05], emissive_strength: 1.0, ..Material::color([0.9, 0.08, 0.05]) });
    sol(&mut s, Vec3::new(0.3, -1.0, -0.6), 3.0);
    camara(&mut s, Vec3::new(0.0, 1.5, 4.0), Vec3::new(0.0, 0.3, 0.0));
    s.update_transforms();
    // Punto del suelo donde se ve, en espejo, la cara frontal de la caja.
    let punto = pantalla(&s, Vec3::new(0.0, 0.0, 1.375));
    let mut r = SceneRenderer::new(&device, &queue);
    r.resize(W, H);
    con_gi(&mut s, false);
    let off = media(&render(&mut r, &s, 1, GiDebug::Ninguna), punto);
    con_gi(&mut s, true);
    let on = media(&render(&mut r, &s, 12, GiDebug::Ninguna), punto);
    eprintln!("reflejo: off {off:?} on {on:?}");
    assert!(on.x > on.z * 2.0 && on.x > off.x, "off {off:?} on {on:?}");
}

/// Lo estático queda en caché (no se revoxeliza) y mover un objeto
/// revoxeliza solo lo necesario; redimensionar y cambiar de calidad no rompe.
#[test]
fn static_cache_resize_and_quality_levels() {
    let Some((device, queue, _gpu)) = device() else { return };
    let mut s = maker_engine::iluminacion::cornell_scene();
    let mut r = SceneRenderer::new(&device, &queue);
    r.resize(W, H);
    render(&mut r, &s, 1, GiDebug::Ninguna);
    assert!(r.stats.gi_revoxelized >= 1, "{:?}", r.stats);
    render(&mut r, &s, 1, GiDebug::Ninguna);
    assert_eq!(r.stats.gi_revoxelized, 0, "escena quieta: nada que revoxelizar");
    let cubo = s.find_by_name("Cubo emisivo").unwrap();
    let mut t = s.get::<Transform>(cubo).unwrap();
    t.translation += Vec3::new(0.0, 0.3, 0.0);
    s.set_transform(cubo, t);
    s.update_transforms();
    render(&mut r, &s, 1, GiDebug::Ninguna);
    assert!(r.stats.gi_revoxelized >= 1);
    r.resize(200, 120);
    render(&mut r, &s, 2, GiDebug::Ninguna);
    r.resize(W, H);
    for c in CalidadGi::ALL {
        s.env.iluminacion_global.calidad = c;
        render(&mut r, &s, 2, GiDebug::Ninguna);
        assert!(r.stats.gi);
    }
    // Todas las vistas de depuración y los demás modelos de sombreado.
    for d in GiDebug::ALL {
        let px = render(&mut r, &s, 2, d);
        assert!(px.iter().any(|&c| c > 10), "{d:?}");
    }
    s.env.global_shading.model = Some(ShadingModel::Toon);
    render(&mut r, &s, 2, GiDebug::Ninguna);
    s.env.global_shading.model = Some(ShadingModel::Hatching);
    render(&mut r, &s, 2, GiDebug::Ninguna);
    s.env.global_shading.model = Some(ShadingModel::Unlit);
    render(&mut r, &s, 2, GiDebug::Ninguna);
    // Apagar la GI libera sus recursos.
    s.env.iluminacion_global.modo = ModoGi::Desactivada;
    render(&mut r, &s, 1, GiDebug::Ninguna);
    assert!(r.gi.is_none() && !r.stats.gi);
}

/// Con la GI pedida por la escena pero no permitida por la vista
/// (miniaturas) la imagen es idéntica a la de siempre.
#[test]
fn gi_disallowed_view_is_identical() {
    let Some((device, queue, _gpu)) = device() else { return };
    let mut s = Scene::starter();
    let (id, cam) = s.primary_camera().unwrap();
    let view = RenderView::from_camera(s.global(id), &cam, W as f32 / H as f32);
    let mut r = SceneRenderer::new(&device, &queue);
    r.resize(W, H);
    r.render(&s, &view, &RenderOptions { shadows: true, ..Default::default() });
    let base = r.read_pixels().unwrap().2;
    con_gi(&mut s, true);
    r.render(&s, &view, &RenderOptions { shadows: true, ..Default::default() });
    assert_eq!(r.read_pixels().unwrap().2, base);
    assert!(r.gi.is_none());
}

#[test]
fn quality_fits_device_limits() {
    let mut l = wgpu::Limits::default();
    let q = ajustar(calidad(CalidadGi::Epica), &l).unwrap();
    assert!((q.n as u64).pow(3) * PALABRAS * 4 <= l.max_storage_buffer_binding_size);
    // El alcance de la cascada 0 se conserva al bajar la resolución.
    let e = calidad(CalidadGi::Epica);
    assert!(((q.n as f32 * q.voxel) - (e.n as f32 * e.voxel)).abs() < 1e-3);
    l.max_texture_dimension_3d = 64;
    assert!(ajustar(calidad(CalidadGi::Baja), &l).is_none());
    // Origen ajustado: moverse menos de un salto no cambia la cascada.
    let m = calidad(CalidadGi::Media);
    let (a, _) = origen_cascada(Vec3::new(0.1, 0.0, 0.0), &m, 0);
    let (b, _) = origen_cascada(Vec3::new(0.2, 0.0, 0.0), &m, 0);
    assert_eq!(a, b);
}

/// Vuelca imágenes de depuración (manual: `cargo t -p maker_render volcar -- --ignored`).
#[test]
#[ignore]
fn volcar_imagenes() {
    let Some((device, queue, _gpu)) = device() else { return };
    let dir = std::env::var("GI_VOLCAR").unwrap_or_else(|_| std::env::temp_dir().display().to_string());
    let mut sin_rebotes = habitacion(true);
    sin_rebotes.env.iluminacion_global.rebotes_multiples = false;
    for (nombre, mut s) in [("cornell", maker_engine::iluminacion::cornell_scene()), ("cerrada", habitacion(true)), ("abierta", habitacion(false)), ("cerrada1", sin_rebotes)] {
        for d in GiDebug::ALL {
            let mut r = SceneRenderer::new(&device, &queue);
            r.resize(W, H);
            let px = render(&mut r, &s, 16, d);
            eprintln!("{nombre} {d:?}: centro {:?}", media(&px, (W / 2, H / 2)));
            let img = image::RgbaImage::from_raw(W, H, px).unwrap();
            img.save(format!("{dir}/{nombre}-{d:?}.png")).unwrap();
        }
        con_gi(&mut s, false);
        let mut r = SceneRenderer::new(&device, &queue);
        r.resize(W, H);
        let px = render(&mut r, &s, 1, GiDebug::Ninguna);
        image::RgbaImage::from_raw(W, H, px).unwrap().save(format!("{dir}/{nombre}-sin-gi.png")).unwrap();
    }
}

