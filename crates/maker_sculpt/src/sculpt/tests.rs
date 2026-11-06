//! Pruebas de los pinceles, la simetría, la máscara, el deshacer y el rendimiento.

use super::*;
use crate::brush::k;
use crate::primitives;
use crate::symmetry::Axis;
use std::time::Instant;

fn sphere(res: u32) -> Sculpt {
    Sculpt::from_mesh(primitives::quad_sphere(res, 1.0))
}

fn plane(res: u32) -> Sculpt {
    Sculpt::from_mesh(primitives::plane(res))
}

fn opts() -> DabOptions {
    DabOptions::default()
}

fn brush(b: Brush, invert: bool) -> Action {
    Action::Brush { brush: b, invert }
}

/// Vértice más cercano a un punto.
fn nearest(sc: &Sculpt, p: Vec3) -> usize {
    (0..sc.mesh.pos.len()).min_by(|&a, &b| sc.mesh.pos[a].distance_squared(p).total_cmp(&sc.mesh.pos[b].distance_squared(p))).unwrap()
}

fn one(sc: &mut Sculpt, a: Action, d: Dab, o: &DabOptions) {
    sc.begin_stroke("t", matches!(a, Action::Mask { .. }));
    sc.dab(a, &d, o);
    sc.end_stroke();
}

/// Plano con ondulaciones (para Aplanar y Recortar).
fn bumpy_plane() -> Sculpt {
    let mut m = primitives::plane(48);
    for p in &mut m.positions {
        p.y = 0.03 * (p.x * 40.0).sin() * (p.z * 40.0).cos();
    }
    m.compute_normals();
    Sculpt::from_mesh(m)
}

fn ys_within(sc: &Sculpt, r: f32) -> Vec<f32> {
    sc.mesh.pos.iter().filter(|p| Vec3::new(p.x, 0.0, p.z).length() < r).map(|p| p.y).collect()
}

fn variance(v: &[f32]) -> f32 {
    let m = v.iter().sum::<f32>() / v.len() as f32;
    v.iter().map(|x| (x - m) * (x - m)).sum::<f32>() / v.len() as f32
}

#[test]
fn standard_raises_and_alt_digs() {
    for (invert, sign) in [(false, 1.0), (true, -1.0)] {
        let mut sc = sphere(32);
        let c = nearest(&sc, Vec3::Z);
        one(&mut sc, brush(Brush::Standard, invert), Dab::at(Vec3::Z, Vec3::Z, 0.3, 1.0), &opts());
        let d = sc.mesh.pos[c].length() - 1.0;
        assert!(d * sign > 0.3 * k::STANDARD * 0.9, "{d}");
    }
}

#[test]
fn inflate_clay_and_block_directions() {
    // Inflar: hacia fuera por la normal; Alt hacia dentro.
    for (invert, sign) in [(false, 1.0), (true, -1.0)] {
        let mut sc = sphere(32);
        let c = nearest(&sc, Vec3::Z);
        one(&mut sc, brush(Brush::Inflate, invert), Dab::at(Vec3::Z, Vec3::Z, 0.3, 1.0), &opts());
        assert!((sc.mesh.pos[c].length() - 1.0) * sign > 0.0);
    }
    // Arcilla sobre un plano: sube (Alt baja), sin pasar de su plano.
    for b in [Brush::Clay, Brush::ClayBlock] {
        for (invert, sign) in [(false, 1.0), (true, -1.0)] {
            let mut sc = plane(64);
            let c = nearest(&sc, Vec3::ZERO);
            for _ in 0..40 {
                sc.dab(brush(b, invert), &Dab::at(Vec3::ZERO, Vec3::Y, 0.2, 1.0), &opts());
                sc.flush();
            }
            sc.end_stroke();
            let y = sc.mesh.pos[c].y;
            assert!(y * sign > 0.0, "{b:?} {invert}: {y}");
        }
    }
    // Bloque: punta cuadrada (la esquina del cuadrado se mueve; con Arcilla no).
    let corner = Vec3::new(0.15, 0.0, 0.15);
    let mut clay = plane(64);
    let mut block = plane(64);
    let (vc, vb) = (nearest(&clay, corner), nearest(&block, corner));
    for _ in 0..5 {
        clay.dab(brush(Brush::Clay, false), &Dab { screen_x: Vec3::X, ..Dab::at(Vec3::ZERO, Vec3::Y, 0.2, 1.0) }, &opts());
        block.dab(brush(Brush::ClayBlock, false), &Dab { screen_x: Vec3::X, ..Dab::at(Vec3::ZERO, Vec3::Y, 0.2, 1.0) }, &opts());
    }
    assert_eq!(clay.mesh.pos[vc].y, 0.0, "fuera del círculo");
    assert!(block.mesh.pos[vb].y > 0.0, "dentro del cuadrado");
}

#[test]
fn clay_fills_hollows_first() {
    // Un hoyo en el centro: la arcilla lo sube más que a la zona plana de alrededor.
    let mut m = primitives::plane(64);
    for p in &mut m.positions {
        let r = (p.x * p.x + p.z * p.z).sqrt();
        if r < 0.05 {
            p.y = -0.02;
        }
    }
    m.compute_normals();
    let mut sc = Sculpt::from_mesh(m);
    let hole = nearest(&sc, Vec3::new(0.0, -0.02, 0.0));
    let flat = nearest(&sc, Vec3::new(0.08, 0.0, 0.0));
    let (h0, f0) = (sc.mesh.pos[hole].y, sc.mesh.pos[flat].y);
    one(&mut sc, brush(Brush::Clay, false), Dab::at(Vec3::ZERO, Vec3::Y, 0.2, 1.0), &opts());
    assert!(sc.mesh.pos[hole].y - h0 > sc.mesh.pos[flat].y - f0);
}

#[test]
fn flatten_and_trim() {
    let r = 0.25;
    let mut sc = bumpy_plane();
    let v0 = variance(&ys_within(&sc, r * 0.5));
    for _ in 0..10 {
        sc.dab(brush(Brush::Flatten, false), &Dab::at(Vec3::ZERO, Vec3::Y, r, 1.0), &opts());
    }
    assert!(variance(&ys_within(&sc, r * 0.5)) < v0 * 0.5);
    let mut sc = bumpy_plane();
    sc.dab(brush(Brush::Flatten, true), &Dab::at(Vec3::ZERO, Vec3::Y, r, 1.0), &opts());
    assert!(variance(&ys_within(&sc, r * 0.5)) > v0, "Alt aleja del plano");
    // Recortar rasura lo alto y no sube lo bajo; con Alt rellena lo bajo.
    let mut sc = bumpy_plane();
    let before = ys_within(&sc, r * 0.5);
    for _ in 0..10 {
        sc.dab(brush(Brush::Trim, false), &Dab::at(Vec3::ZERO, Vec3::Y, r, 1.0), &opts());
    }
    let after = ys_within(&sc, r * 0.5);
    let max = |v: &[f32]| v.iter().copied().fold(f32::MIN, f32::max);
    let min = |v: &[f32]| v.iter().copied().fold(f32::MAX, f32::min);
    assert!(max(&after) < max(&before) * 0.7);
    assert!(before.iter().zip(&after).all(|(b, a)| a <= b));
    let mut sc = bumpy_plane();
    for _ in 0..10 {
        sc.dab(brush(Brush::Trim, true), &Dab::at(Vec3::ZERO, Vec3::Y, r, 1.0), &opts());
    }
    let after = ys_within(&sc, r * 0.5);
    assert!(min(&after) > min(&before) * 0.7);
    assert!(before.iter().zip(&after).all(|(b, a)| a >= b));
}

#[test]
fn pinch_crease_and_sharp_crease() {
    let side = Vec3::new(0.1, 0.0, 0.0);
    for (invert, closer) in [(false, true), (true, false)] {
        let mut sc = plane(64);
        let v = nearest(&sc, side);
        let x0 = sc.mesh.pos[v].x;
        one(&mut sc, brush(Brush::Pinch, invert), Dab::at(Vec3::ZERO, Vec3::Y, 0.2, 1.0), &opts());
        assert_eq!(sc.mesh.pos[v].x < x0, closer);
        assert!(sc.mesh.pos[v].y.abs() < 1e-6, "pellizcar no levanta");
    }
    // Pliegue: hunde el centro y pellizca; Alt levanta cresta (y sigue pellizcando).
    let mut depth = [0.0f32; 2];
    for (i, b) in [Brush::Crease, Brush::SharpCrease].into_iter().enumerate() {
        for (invert, sign) in [(false, -1.0), (true, 1.0)] {
            let mut sc = plane(64);
            let c = nearest(&sc, Vec3::ZERO);
            let v = nearest(&sc, side);
            let x0 = sc.mesh.pos[v].x;
            one(&mut sc, brush(b, invert), Dab::at(Vec3::ZERO, Vec3::Y, 0.2, 1.0), &opts());
            assert!(sc.mesh.pos[c].y * sign > 0.0, "{b:?}");
            assert!(sc.mesh.pos[v].x < x0, "{b:?} pellizca");
            if !invert {
                // Relación entre el hundimiento en el centro y a media distancia.
                let mid = nearest(&plane(64), Vec3::new(0.1, 0.0, 0.0));
                depth[i] = sc.mesh.pos[mid].y / sc.mesh.pos[c].y;
            }
        }
    }
    assert!(depth[1] < depth[0], "el pliegue fino es más estrecho: {depth:?}");
}

#[test]
fn nudge_slides_sideways() {
    let mut sc = plane(64);
    let c = nearest(&sc, Vec3::ZERO);
    let d = Dab { delta: Vec3::new(0.05, 0.02, 0.0), ..Dab::at(Vec3::ZERO, Vec3::Y, 0.2, 1.0) };
    one(&mut sc, brush(Brush::Nudge, false), d, &opts());
    assert!(sc.mesh.pos[c].x > 0.01);
    assert!(sc.mesh.pos[c].y.abs() < 1e-6, "sin levantar");
}

#[test]
fn smooth_reduces_noise() {
    let mut m = primitives::quad_sphere(32, 1.0);
    for (i, p) in m.positions.iter_mut().enumerate() {
        *p *= 1.0 + 0.03 * (((i * 2654435761) % 1000) as f32 / 1000.0 - 0.5);
    }
    let mut sc = Sculpt::from_mesh(m);
    let rough = |sc: &Sculpt| sc.mesh.pos.iter().filter(|p| p.normalize().distance(Vec3::Z) < 0.3).map(|p| (p.length() - 1.0).powi(2)).sum::<f32>();
    let r0 = rough(&sc);
    for _ in 0..5 {
        sc.dab(Action::Smooth { taubin: false }, &Dab::at(Vec3::Z, Vec3::Z, 0.6, 1.0), &opts());
    }
    sc.end_stroke();
    assert!(rough(&sc) < r0 * 0.9);
    // El pincel Suavizar es lo mismo.
    let before = sc.mesh.pos.clone();
    one(&mut sc, brush(Brush::Smooth, false), Dab::at(Vec3::Z, Vec3::Z, 0.6, 1.0), &opts());
    assert_ne!(before, sc.mesh.pos);
}

fn noisy_sphere() -> Sculpt {
    let mut m = primitives::quad_sphere(40, 1.0);
    for (i, p) in m.positions.iter_mut().enumerate() {
        *p *= 1.0 + 0.04 * (((i * 2654435761) % 1000) as f32 / 1000.0 - 0.5);
    }
    Sculpt::from_mesh(m)
}

#[test]
fn taubin_keeps_volume_laplacian_shrinks() {
    let v0 = noisy_sphere().mesh.volume();
    let run = |taubin: bool| {
        let mut sc = noisy_sphere();
        for _ in 0..20 {
            sc.dab(Action::Smooth { taubin }, &Dab::at(Vec3::Z, Vec3::Z, 3.0, 1.0), &opts());
        }
        sc.end_stroke();
        let rough = sc.mesh.pos.iter().map(|p| p.length()).fold((f32::MAX, f32::MIN), |a, l| (a.0.min(l), a.1.max(l)));
        (sc.mesh.volume(), rough.1 - rough.0)
    };
    let (vl, _) = run(false);
    let (vt, spread) = run(true);
    let (dl, dt) = ((vl - v0).abs() / v0, (vt - v0).abs() / v0);
    assert!(dt < 0.01, "Taubin cambia el volumen un {:.2} %", dt * 100.0);
    assert!(dt < dl * 0.25, "Laplace {:.2} %, Taubin {:.2} %", dl * 100.0, dt * 100.0);
    assert!(spread < 0.04, "y también alisa: {spread}");
}

#[test]
fn layer_does_not_stack_within_a_stroke() {
    let mut sc = plane(80);
    let (r, s) = (0.15, 1.0);
    let h = r * k::LAYER * s;
    sc.begin_stroke("capa", false);
    for pass in 0..3 {
        for i in 0..=20 {
            let x = -0.2 + 0.4 * i as f32 / 20.0;
            let x = if pass % 2 == 0 { x } else { -x };
            sc.dab(brush(Brush::Layer, false), &Dab::at(Vec3::new(x, 0.0, 0.0), Vec3::Y, r, s), &opts());
        }
    }
    sc.end_stroke();
    let max = sc.mesh.pos.iter().map(|p| p.y).fold(f32::MIN, f32::max);
    assert!(max <= h * 1.0001 && max > h * 0.95, "{max} vs {h}");
    // Un trazo nuevo sí sube otra capa encima.
    sc.begin_stroke("capa 2", false);
    sc.dab(brush(Brush::Layer, false), &Dab::at(Vec3::ZERO, Vec3::Y, r, s), &opts());
    sc.end_stroke();
    let max2 = sc.mesh.pos.iter().map(|p| p.y).fold(f32::MIN, f32::max);
    assert!(max2 > h * 1.5, "{max2}");
    // Alt baja.
    let mut sc = plane(80);
    one(&mut sc, brush(Brush::Layer, true), Dab::at(Vec3::ZERO, Vec3::Y, r, s), &opts());
    let min = sc.mesh.pos.iter().map(|p| p.y).fold(f32::MAX, f32::min);
    assert!((min + h).abs() < h * 0.05);
}

#[test]
fn mask_holds_every_brush() {
    for b in Brush::ALL.into_iter().filter(|b| *b != Brush::Move) {
        let mut sc = plane(48);
        sc.mesh.ensure_mask();
        for v in 0..sc.mesh.pos.len() {
            if sc.mesh.pos[v].x < 0.0 {
                sc.mesh.mask[v] = 1.0;
            }
        }
        let before = sc.mesh.pos.clone();
        let d = Dab { delta: Vec3::new(0.05, 0.0, 0.0), ..Dab::at(Vec3::ZERO, Vec3::Y, 0.3, 1.0) };
        for _ in 0..3 {
            sc.dab(brush(b, false), &d, &opts());
        }
        sc.end_stroke();
        let mut moved_free = 0;
        for v in 0..before.len() {
            if sc.mesh.mask[v] == 1.0 {
                assert_eq!(before[v], sc.mesh.pos[v], "{b:?} movió un vértice enmascarado");
            } else if before[v] != sc.mesh.pos[v] {
                moved_free += 1;
            }
        }
        if b != Brush::Smooth && b != Brush::Flatten && b != Brush::Trim {
            assert!(moved_free > 0, "{b:?} no hizo nada");
        }
    }
    // Mover también respeta la máscara.
    let mut sc = plane(48);
    sc.mesh.ensure_mask();
    for v in 0..sc.mesh.pos.len() {
        if sc.mesh.pos[v].x < 0.0 {
            sc.mesh.mask[v] = 1.0;
        }
    }
    let before = sc.mesh.pos.clone();
    assert!(sc.grab_begin(&Dab::at(Vec3::ZERO, Vec3::Y, 0.3, 1.0), &opts()));
    sc.grab_update(Vec3::Y * 0.2);
    sc.end_stroke();
    for v in 0..before.len() {
        if sc.mesh.mask[v] == 1.0 {
            assert_eq!(before[v], sc.mesh.pos[v]);
        }
    }
}

#[test]
fn mask_brush_paints_and_erases() {
    let mut sc = plane(32);
    let c = nearest(&sc, Vec3::ZERO);
    for _ in 0..10 {
        sc.dab(Action::Mask { erase: false }, &Dab::at(Vec3::ZERO, Vec3::Y, 0.2, 1.0), &opts());
    }
    sc.end_stroke();
    assert_eq!(sc.mesh.mask[c], 1.0);
    assert!(sc.mesh.pos.iter().all(|p| p.y == 0.0), "la máscara no esculpe");
    one(&mut sc, Action::Mask { erase: true }, Dab::at(Vec3::ZERO, Vec3::Y, 0.2, 1.0), &opts());
    assert!(sc.mesh.mask[c] < 1.0);
}

/// Índice del vértice espejo (en X) de cada vértice, buscándolo por posición.
fn mirror_map(sc: &Sculpt) -> Vec<usize> {
    let mut keyed: Vec<(i64, i64, i64, usize)> = sc.mesh.pos.iter().enumerate().map(|(i, p)| ((p.x * 1e4).round() as i64, (p.y * 1e4).round() as i64, (p.z * 1e4).round() as i64, i)).collect();
    keyed.sort_unstable();
    sc.mesh
        .pos
        .iter()
        .map(|p| {
            let key = ((-p.x * 1e4).round() as i64, (p.y * 1e4).round() as i64, (p.z * 1e4).round() as i64);
            let i = keyed.partition_point(|e| (e.0, e.1, e.2) < key);
            keyed[i].3
        })
        .collect()
}

#[test]
fn x_symmetry_mirrors_exactly() {
    let o = DabOptions { symmetry: Symmetry::default(), ..opts() };
    let c = Vec3::new(0.6, 0.3, 0.74).normalize();
    for b in [Brush::Standard, Brush::Clay, Brush::Inflate, Brush::Crease, Brush::Pinch, Brush::Flatten, Brush::Layer, Brush::Nudge, Brush::Smooth] {
        let mut sc = sphere(32);
        let mm = mirror_map(&sc);
        let d = Dab { delta: Vec3::new(0.02, 0.01, 0.0), screen_x: Vec3::X, view_dir: -c, ..Dab::at(c, c, 0.25, 1.0) };
        for _ in 0..3 {
            sc.dab(brush(b, false), &d, &o);
        }
        sc.end_stroke();
        let mut moved = 0;
        for v in 0..sc.mesh.pos.len() {
            let p = sc.mesh.pos[v];
            let q = sc.mesh.pos[mm[v]];
            assert!((Vec3::new(-p.x, p.y, p.z) - q).length() < 2e-5, "{b:?}: {p} vs {q}");
            moved += (p.length() - 1.0).abs().gt(&1e-6) as usize;
        }
        assert!(moved > 0, "{b:?}");
    }
}

#[test]
fn radial_symmetry_makes_n_equal_copies() {
    let sym = Symmetry { mirror: [false; 3], radial: true, radial_count: 8, radial_axis: Axis::Y, local: false };
    let o = DabOptions { symmetry: sym, ..opts() };
    let mut sc = sphere(48);
    let rise = |sc: &Sculpt, a: f32| {
        let p = Vec3::new(a.cos(), 0.0, -a.sin());
        sc.mesh.pos[nearest(sc, p)].length() - 1.0
    };
    one(&mut sc, brush(Brush::Inflate, false), Dab::at(Vec3::X, Vec3::X, 0.15, 1.0), &o);
    let first = rise(&sc, 0.0);
    assert!(first > 0.0);
    for kk in 0..8 {
        let a = kk as f32 / 8.0 * std::f32::consts::TAU;
        assert!((rise(&sc, a) - first).abs() < first * 0.15, "copia {kk}");
        // Entre dos copias no hay nada.
        assert!(rise(&sc, a + std::f32::consts::TAU / 16.0).abs() < 1e-6);
    }
}

#[test]
fn local_symmetry_uses_given_center() {
    // Una esfera desplazada a x = 3: con el centro en 3 las copias caen sobre ella.
    let mut m = primitives::quad_sphere(32, 1.0);
    for p in &mut m.positions {
        p.x += 3.0;
    }
    let mut sc = Sculpt::from_mesh(m);
    let center = sc.bounds_center();
    assert!((center - Vec3::new(3.0, 0.0, 0.0)).length() < 1e-4);
    let o = DabOptions { symmetry: Symmetry { local: true, ..Default::default() }, center, ..opts() };
    let hit = Vec3::new(3.6, 0.0, 0.8);
    one(&mut sc, brush(Brush::Inflate, false), Dab::at(hit, (hit - center).normalize(), 0.2, 1.0), &o);
    let mirror = Vec3::new(2.4, 0.0, 0.8);
    let v = nearest(&sc, mirror);
    assert!((sc.mesh.pos[v] - center).length() > 1.0 + 1e-4, "la copia local cae en x = 2,4");
}

#[test]
fn move_grab_follows_with_symmetry() {
    let o = DabOptions { symmetry: Symmetry::default(), ..opts() };
    let mut sc = sphere(32);
    let c = Vec3::new(0.6, 0.0, 0.8);
    let (v, w) = (nearest(&sc, c), nearest(&sc, Vec3::new(-0.6, 0.0, 0.8)));
    let (pv, pw) = (sc.mesh.pos[v], sc.mesh.pos[w]);
    assert!(sc.grab_begin(&Dab::at(sc.mesh.pos[v], c, 0.3, 1.0), &o));
    sc.grab_update(Vec3::new(0.1, 0.2, 0.0));
    sc.grab_update(Vec3::new(0.2, 0.0, 0.0));
    sc.end_stroke();
    assert!((sc.mesh.pos[v] - pv - Vec3::new(0.2, 0.0, 0.0)).length() < 1e-4, "el centro sigue al cursor");
    assert!((sc.mesh.pos[w] - pw - Vec3::new(-0.2, 0.0, 0.0)).length() < 1e-4, "la copia, en espejo");
    // Lejos no se mueve nada.
    let far = nearest(&sc, Vec3::new(0.0, 0.0, -1.0));
    assert!((sc.mesh.pos[far].length() - 1.0).abs() < 1e-6);
}

#[test]
fn front_faces_only_spares_the_back() {
    let mut m = primitives::cube(12);
    for p in &mut m.positions {
        p.z *= 0.04;
    }
    m.compute_normals();
    for front in [true, false] {
        let mut sc = Sculpt::from_mesh(m.clone());
        let before = sc.mesh.pos.clone();
        let o = DabOptions { front_only: front, ..opts() };
        let d = Dab { view_dir: Vec3::NEG_Z, ..Dab::at(Vec3::new(0.0, 0.0, 0.02), Vec3::Z, 0.3, 1.0) };
        one(&mut sc, brush(Brush::Inflate, false), d, &o);
        let back_moved = (0..before.len()).any(|v| before[v].z < -0.019 && before[v] != sc.mesh.pos[v]);
        assert_eq!(back_moved, !front);
    }
}

#[test]
fn connected_only_skips_the_neighbour_part() {
    let mut a = primitives::quad_sphere(24, 0.5);
    let b = primitives::quad_sphere(24, 0.5);
    a.append(&b, glam::Mat4::from_translation(Vec3::new(1.05, 0.0, 0.0)));
    for connected in [true, false] {
        let mut sc = Sculpt::from_mesh(a.clone());
        let before = sc.mesh.pos.clone();
        let o = DabOptions { connected, ..opts() };
        one(&mut sc, brush(Brush::Inflate, false), Dab::at(Vec3::new(0.5, 0.0, 0.0), Vec3::X, 0.3, 1.0), &o);
        let other = (0..before.len()).any(|v| before[v].x > 0.54 && before[v] != sc.mesh.pos[v]);
        assert_eq!(other, !connected);
        assert!((0..before.len()).any(|v| before[v].x < 0.51 && before[v] != sc.mesh.pos[v]));
    }
}

#[test]
fn view_falloff_reaches_through_the_model() {
    for cyl in [true, false] {
        let mut sc = sphere(32);
        let back = nearest(&sc, Vec3::NEG_Z);
        let o = DabOptions { view_falloff: cyl, ..opts() };
        let d = Dab { view_dir: Vec3::NEG_Z, ..Dab::at(Vec3::Z, Vec3::Z, 0.2, 1.0) };
        one(&mut sc, brush(Brush::Inflate, false), d, &o);
        assert_eq!(sc.mesh.pos[back].length() > 1.0 + 1e-6, cyl);
    }
}

#[test]
fn undo_and_redo_restore_exactly() {
    let mut sc = sphere(32);
    let p0 = sc.mesh.pos.clone();
    sc.begin_stroke("varios", false);
    for (i, b) in [Brush::Standard, Brush::Clay, Brush::Smooth, Brush::Pinch, Brush::Layer].into_iter().enumerate() {
        let c = Vec3::new(0.1 * i as f32, 0.2, 1.0).normalize();
        sc.dab(brush(b, i % 2 == 0), &Dab::at(c, c, 0.3, 0.8), &DabOptions { symmetry: Symmetry::default(), ..opts() });
    }
    sc.end_stroke();
    let p1 = sc.mesh.pos.clone();
    let n1 = sc.mesh.nrm.clone();
    assert_ne!(p0, p1);
    one(&mut sc, Action::Mask { erase: false }, Dab::at(Vec3::Z, Vec3::Z, 0.3, 1.0), &opts());
    assert!(sc.has_mask());
    assert_eq!(sc.undo().as_deref(), Some("t"));
    assert!(!sc.has_mask(), "deshace la máscara");
    assert_eq!(sc.undo().as_deref(), Some("varios"));
    assert_eq!(sc.mesh.pos, p0, "posiciones idénticas bit a bit");
    assert!(sc.undo().is_none());
    sc.redo();
    assert_eq!(sc.mesh.pos, p1);
    assert_eq!(sc.mesh.nrm, n1, "normales recalculadas igual");
    sc.redo();
    assert!(sc.has_mask());
    // Mover también es un paso.
    assert!(sc.grab_begin(&Dab::at(Vec3::Z, Vec3::Z, 0.3, 1.0), &opts()));
    sc.grab_update(Vec3::X * 0.3);
    sc.end_stroke();
    sc.undo();
    assert_eq!(sc.mesh.pos, p1);
}

#[test]
fn gpu_ranges_cover_every_changed_render_vertex() {
    // Esfera UV del motor: tiene costuras (vértices de render duplicados).
    let data = ModelData::from_mesh(MeshData::primitive(maker_engine::components::PrimitiveShape::Sphere));
    let mut sc = Sculpt::from_model(&data).unwrap();
    assert!(!sc.mesh.render.extra_src.is_empty());
    let mut ranges = Vec::new();
    sc.take_gpu_ranges(&mut ranges);
    assert!(ranges.is_empty());
    let before = sc.to_model();
    // Sobre la costura (u = 0 está en +X).
    one(&mut sc, brush(Brush::Standard, false), Dab::at(Vec3::new(0.5, 0.0, 0.0), Vec3::X, 0.15, 1.0), &opts());
    sc.take_gpu_ranges(&mut ranges);
    let after = sc.to_model();
    assert!(ranges.windows(2).all(|w| w[0].end < w[1].start), "ordenados y separados: {ranges:?}");
    let total: u32 = ranges.iter().map(|r| r.end - r.start).sum();
    assert!((total as usize) < before.mesh.positions.len(), "subida parcial");
    let mut changed = 0;
    for i in 0..before.mesh.positions.len() {
        if before.mesh.positions[i] != after.mesh.positions[i] || before.mesh.normals[i] != after.mesh.normals[i] {
            changed += 1;
            assert!(ranges.iter().any(|r| r.contains(&(i as u32))), "vértice de render {i} cambió fuera de los rangos");
        }
    }
    assert!(changed > 0);
    assert!(ranges.iter().any(|r| r.end > sc.vertex_count() as u32), "incluye duplicados de costura");
    sc.take_gpu_ranges(&mut ranges);
    assert!(ranges.is_empty(), "se vacían al tomarlos");
}

#[test]
fn mask_operations() {
    let mut sc = plane(32);
    sc.mesh.ensure_mask();
    for v in 0..sc.mesh.pos.len() {
        sc.mesh.mask[v] = if sc.mesh.pos[v].x < 0.0 { 1.0 } else { 0.0 };
    }
    let m0 = sc.mesh.mask.clone();
    sc.mask_invert();
    assert!(sc.mesh.mask.iter().zip(&m0).all(|(a, b)| (a + b - 1.0).abs() < 1e-6));
    sc.undo();
    assert_eq!(sc.mesh.mask, m0);
    let contrast = |m: &[f32]| m.iter().map(|x| (x - 0.5).abs()).sum::<f32>();
    sc.mask_blur();
    let blurred = sc.mesh.mask.clone();
    assert!(contrast(&blurred) < contrast(&m0));
    assert!(blurred.iter().any(|&x| x > 0.05 && x < 0.95), "borde suave");
    sc.mask_sharpen();
    assert!(contrast(&sc.mesh.mask) > contrast(&blurred));
    sc.mask_clear();
    assert!(!sc.has_mask());
    sc.undo();
    sc.undo();
    sc.undo();
    assert_eq!(sc.mesh.mask, m0);
}

#[test]
fn whole_mesh_ops_respect_mask() {
    let mut sc = noisy_sphere();
    let v0 = sc.mesh.volume();
    sc.mesh.ensure_mask();
    for v in 0..sc.mesh.pos.len() {
        if sc.mesh.pos[v].y > 0.0 {
            sc.mesh.mask[v] = 1.0;
        }
    }
    let before = sc.mesh.pos.clone();
    sc.inflate_all(50.0);
    for v in 0..before.len() {
        if sc.mesh.mask[v] == 1.0 {
            assert_eq!(before[v], sc.mesh.pos[v]);
        }
    }
    assert!(sc.mesh.volume() > v0);
    sc.mask_clear();
    let rough = |sc: &Sculpt| {
        let mut s = 0.0;
        for v in 0..sc.mesh.pos.len() as u32 {
            let ring = sc.mesh.ring(v);
            let avg = ring.iter().map(|&u| sc.mesh.pos[u as usize]).sum::<Vec3>() / ring.len() as f32;
            s += avg.distance(sc.mesh.pos[v as usize]);
        }
        s
    };
    let r0 = rough(&sc);
    sc.smooth_all(30.0, true);
    assert!(rough(&sc) < r0 * 0.7);
    let vol = sc.mesh.volume();
    sc.relax_all(20.0);
    assert!((sc.mesh.volume() - vol).abs() / vol < 0.01, "relajar no cambia la forma");
    assert_eq!(sc.undo.len(), 4, "hinchar, borrar máscara, suavizar y relajar");
}

#[test]
fn raycast_and_area_normal() {
    let sc = sphere(32);
    let h = sc.raycast(Vec3::new(0.0, 0.0, 5.0), Vec3::NEG_Z).unwrap();
    assert!((h.point.z - 1.0).abs() < 0.01 && h.normal.z > 0.99);
    assert!(sc.raycast(Vec3::new(5.0, 5.0, 5.0), Vec3::Z).is_none());
    let n = sc.area_normal(Vec3::Z, 0.3).unwrap();
    assert!(n.z > 0.99);
}

#[test]
fn steady_state_dabs_reuse_buffers() {
    // Tras el primer dab, los búferes de trabajo no crecen.
    let mut sc = sphere(48);
    let o = DabOptions { symmetry: Symmetry::default(), ..opts() };
    let run = |sc: &mut Sculpt| {
        for i in 0..50 {
            let c = Vec3::new(0.01 * i as f32, 0.0, 1.0).normalize();
            sc.dab(brush(Brush::Standard, false), &Dab::at(c, c, 0.3, 0.5), &o);
            sc.dab(Action::Smooth { taubin: true }, &Dab::at(c, c, 0.3, 0.5), &o);
        }
    };
    run(&mut sc);
    let caps = (sc.sc.leaves.capacity(), sc.sc.weight.capacity(), sc.sc.tmp.capacity(), sc.sc.xforms.capacity());
    run(&mut sc);
    assert_eq!(caps, (sc.sc.leaves.capacity(), sc.sc.weight.capacity(), sc.sc.tmp.capacity(), sc.sc.xforms.capacity()));
}

/// Rendimiento: un *dab* sobre una esfera de ~1 M de vértices. Imprime los tiempos
/// (`cargo t -p maker_sculpt -- --nocapture bench`).
#[test]
fn bench_dab_on_million_vertex_sphere() {
    let t = Instant::now();
    let m = primitives::quad_sphere(408, 1.0);
    let t_gen = t.elapsed();
    let t = Instant::now();
    let mut sc = Sculpt::from_mesh(m);
    let t_build = t.elapsed();
    let n = sc.vertex_count();
    assert!(n > 990_000, "{n}");
    let o = DabOptions { symmetry: Symmetry::default(), ..opts() };
    let mut report = Vec::new();
    for (name, r, a) in [
        ("Estándar r=0,2", 0.2, brush(Brush::Standard, false)),
        ("Estándar r=0,5", 0.5, brush(Brush::Standard, false)),
        ("Arcilla r=0,5", 0.5, brush(Brush::Clay, false)),
        ("Suavizar r=0,5", 0.5, Action::Smooth { taubin: false }),
        ("Suavizar Taubin r=0,5", 0.5, Action::Smooth { taubin: true }),
        ("Estándar r=1,2 (~½ M vértices)", 1.2, brush(Brush::Standard, false)),
    ] {
        sc.begin_stroke("b", false);
        let c = Vec3::new(0.3, 0.2, 1.0).normalize();
        // Calentar y medir la mediana de varios dabs (con su copia en espejo).
        sc.dab(a, &Dab::at(c, c, r, 0.5), &o);
        let mut ts = Vec::new();
        for i in 0..7 {
            let c = Vec3::new(0.3 + 0.002 * i as f32, 0.2, 1.0).normalize();
            let t = Instant::now();
            sc.dab(a, &Dab::at(c, c, r, 0.5), &o);
            ts.push(t.elapsed().as_secs_f64() * 1000.0);
        }
        ts.sort_by(|a, b| a.total_cmp(b));
        let touched: usize = sc.sc.leaves.iter().map(|&l| (sc.bvh.leaves[l as usize].v1 - sc.bvh.leaves[l as usize].v0) as usize).sum();
        let t = Instant::now();
        sc.flush();
        let t_flush = t.elapsed().as_secs_f64() * 1000.0;
        let mut ranges = Vec::new();
        sc.take_gpu_ranges(&mut ranges);
        sc.end_stroke();
        report.push(format!("{name}: {:.2} ms por dab (x2 por el espejo X; ~{} vértices candidatos por copia), vaciado {:.2} ms, {} rangos", ts[3], touched, t_flush, ranges.len()));
    }
    let t = Instant::now();
    let h = sc.raycast(Vec3::new(0.0, 0.0, 5.0), Vec3::NEG_Z);
    let t_ray = t.elapsed();
    assert!(h.is_some());
    println!("esfera de {n} vértices / {} triángulos: generar {:.0} ms, preparar (BVH, adyacencia, normales) {:.0} ms, rayo {:.1} µs", sc.triangle_count(), t_gen.as_secs_f64() * 1000.0, t_build.as_secs_f64() * 1000.0, t_ray.as_secs_f64() * 1e6);
    for r in &report {
        println!("  {r}");
    }
}

#[test]
fn wire_edges_count_each_edge_once() {
    // Esfera cerrada: E = 3T/2. Plano abierto de n×n celdas: E = 3n² + 2n.
    let sc = sphere(8);
    assert_eq!(sc.wire_edges().len() / 2, sc.triangle_count() * 3 / 2);
    let sc = plane(6);
    assert_eq!(sc.wire_edges().len() / 2, 3 * 36 + 2 * 6);
}
