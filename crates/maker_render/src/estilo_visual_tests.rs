//! Cada estilo visual integrado renderiza una escena pequeña sin errores y
//! cambia la imagen respecto a «Ninguno».

use crate::renderer::tests::device;
use crate::{RenderOptions, RenderView, SceneRenderer};
use maker_engine::estilo_visual::{EstiloVisual, INTEGRADOS, Partes};
use maker_engine::postproceso::{PostKind, Tonemapper};
use maker_engine::scene::Scene;

/// Diferencia media por canal entre dos imágenes RGBA del mismo tamaño.
fn media_dif(a: &[u8], b: &[u8]) -> f64 {
    a.iter().zip(b).map(|(x, y)| (*x as i32 - *y as i32).unsigned_abs() as u64).sum::<u64>() as f64 / a.len().max(1) as f64
}

fn imagen(r: &mut SceneRenderer, scene: &Scene, view: &RenderView) -> Vec<u8> {
    r.render(scene, view, &RenderOptions { shadows: true, post: true, ..Default::default() });
    r.read_pixels().expect("lectura de píxeles").2
}

#[test]
fn estilos_integrados_renderizan_y_cambian_la_imagen() {
    let Some((device, queue, _gpu)) = device() else {
        eprintln!("sin adaptador GPU: test omitido");
        return;
    };
    // Proyecto con el efecto CRT de la demo para que también se pruebe ese estilo.
    let raiz = std::env::temp_dir().join(format!("maker-estilo-render-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&raiz);
    std::fs::create_dir_all(raiz.join(maker_engine::postproceso::CUSTOM_DIR)).unwrap();
    for (n, src) in maker_engine::postproceso::EXAMPLES {
        std::fs::write(raiz.join(maker_engine::postproceso::CUSTOM_DIR).join(n), src).unwrap();
    }
    let mut r = SceneRenderer::new(&device, &queue);
    r.project_root = Some(raiz.clone());
    let (w, h) = (160, 100);
    r.resize(w, h);
    let mut scene = Scene::starter();
    let (cam_id, cam) = scene.primary_camera().unwrap();
    let view = RenderView::from_camera(scene.global(cam_id), &cam, w as f32 / h as f32);

    EstiloVisual::default().aplicar(&mut scene.env, Partes::Todo);
    let base = imagen(&mut r, &scene, &view);
    for i in &INTEGRADOS {
        let e = i.estilo();
        assert!(e.disponible(&raiz), "{}", i.id);
        e.aplicar(&mut scene.env, Partes::Todo);
        let px = imagen(&mut r, &scene, &view);
        for c in e.efectos_propios() {
            assert_eq!(r.post_fx().custom_info(c, Some(&raiz)).error, None, "{}: {c}", i.id);
        }
        assert_eq!(px.len(), base.len());
        let dif = media_dif(&px, &base);
        assert!(dif > 1.0, "{}: casi igual a Ninguno (diferencia media {dif:.2})", i.id);
        // Sin imágenes negras ni vacías.
        let media = px.chunks(4).map(|p| p[0] as u64 + p[1] as u64 + p[2] as u64).sum::<u64>() as f64 / (3 * w * h) as f64;
        assert!(media > 15.0, "{}: imagen demasiado oscura ({media:.1})", i.id);
        // Al 0 % de intensidad la imagen cambia; si el estilo usa el tonemap
        // por defecto (ACES), además se acerca a «Ninguno». (Un operador
        // distinto, como AgX, no se escala: es parte del aspecto base.)
        e.escalado(0.0).aplicar(&mut scene.env, Partes::Todo);
        let cero = imagen(&mut r, &scene, &view);
        let dif0 = media_dif(&cero, &base);
        assert!(media_dif(&cero, &px) > 0.5, "{}: la intensidad no cambia nada", i.id);
        let agx = e.post.iter().any(|f| matches!(&f.kind, PostKind::Tonemap(t) if t.op != Tonemapper::Aces));
        assert!(agx || dif0 < dif, "{}: la intensidad no reduce el efecto ({dif0:.2} ≥ {dif:.2})", i.id);
    }
    let _ = std::fs::remove_dir_all(&raiz);
}
