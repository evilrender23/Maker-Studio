//! Interfaz en el viewport: se dibuja encima del 3D (con el mismo pase que el
//! juego) y, en modo 2D, se seleccionan, mueven y redimensionan sus elementos
//! con asas.

use crate::editor::Editor;
use crate::theme;
use egui::{Pos2, Rect, Stroke};
use maker_engine::components::EntityId;
use maker_engine::ui::RectUi;
use maker_render::UiRenderer;

/// Lados que mueve un arrastre (izquierda, arriba, derecha, abajo).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lados(pub [bool; 4]);

#[derive(Clone, Debug)]
pub struct Arrastre {
    pub id: EntityId,
    pub lados: Lados,
    /// Punto de partida (píxeles) y rectángulo antes de arrastrar.
    inicio: [f32; 2],
    antes: RectUi,
}

const MOVER: Lados = Lados([true, true, true, true]);

/// Pone al día la interfaz (la del juego en Play, la de edición si no) y la
/// dibuja sobre la imagen del viewport.
pub fn pintar(ed: &mut Editor, tam: (u32, u32), dt: f32) {
    let Editor { ui, play, renderer, ui_render, scene, mostrar_ui, .. } = ed;
    let sistema = match play {
        Some(p) => &mut p.runtime.ui,
        None => {
            // En edición la escena cambia por muchas vías (inspector, deshacer…):
            // el layout de unas decenas de elementos cuesta microsegundos.
            ui.marcar_sucio();
            ui.actualizar(scene, [tam.0 as f32, tam.1 as f32], dt);
            ui
        }
    };
    if !*mostrar_ui {
        return;
    }
    let Some(vista) = renderer.output_view() else { return };
    let (d, q) = renderer.gpu();
    let r = ui_render.get_or_insert_with(|| UiRenderer::new(d, q, maker_render::COLOR_FORMAT));
    r.dibujar(sistema, vista, tam, None);
}

fn a_px(p: Pos2, rect: Rect, ppp: f32) -> [f32; 2] {
    [(p.x - rect.min.x) * ppp, (p.y - rect.min.y) * ppp]
}

fn a_pos(p: [f32; 2], rect: Rect, ppp: f32) -> Pos2 {
    Pos2::new(rect.min.x + p[0] / ppp, rect.min.y + p[1] / ppp)
}

/// Asas: 4 esquinas y 4 lados del elemento.
fn asas(c: &[[f32; 2]; 4]) -> [([f32; 2], Lados); 8] {
    let mid = |a: [f32; 2], b: [f32; 2]| [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5];
    [
        (c[0], Lados([true, true, false, false])),
        (c[1], Lados([false, true, true, false])),
        (c[2], Lados([false, false, true, true])),
        (c[3], Lados([true, false, false, true])),
        (mid(c[0], c[1]), Lados([false, true, false, false])),
        (mid(c[1], c[2]), Lados([false, false, true, false])),
        (mid(c[2], c[3]), Lados([false, false, false, true])),
        (mid(c[3], c[0]), Lados([true, false, false, false])),
    ]
}

fn contorno(painter: &egui::Painter, c: &[[f32; 2]; 4], rect: Rect, ppp: f32, trazo: Stroke) {
    let pts: Vec<Pos2> = c.iter().map(|p| a_pos(*p, rect, ppp)).collect();
    painter.add(egui::Shape::closed_line(pts, trazo));
}

/// Contorno del elemento de interfaz seleccionado (también fuera del modo 2D).
pub fn contorno_seleccion(ed: &Editor, painter: &egui::Painter, rect: Rect, ppp: f32) {
    let Some(sel) = ed.selection else { return };
    let sistema = match &ed.play {
        Some(p) => &p.runtime.ui,
        None => &ed.ui,
    };
    if let Some(c) = sistema.esquinas(sel) {
        contorno(painter, &c, rect, ppp, Stroke::new(1.5, theme::ACCENT));
    }
}

/// Modo 2D: seleccionar, mover y redimensionar elementos con el ratón.
pub fn editar(ed: &mut Editor, ui: &egui::Ui, resp: &egui::Response, rect: Rect, painter: &egui::Painter) {
    let ppp = ui.ctx().pixels_per_point();
    let ctx = ui.ctx().clone();
    let raton = ctx.input(|i| i.pointer.hover_pos());
    let encima = raton.filter(|_| resp.hovered()).and_then(|m| ed.ui.elemento_en(a_px(m, rect, ppp)));

    // Borde del lienzo (ocupa la vista).
    if ed.ui.lienzos().next().is_some() {
        painter.rect_stroke(rect.shrink(1.0), 0.0, Stroke::new(1.0, theme::STROKE1), egui::StrokeKind::Inside);
    }
    if let Some(h) = encima.filter(|h| Some(*h) != ed.selection)
        && let Some(c) = ed.ui.esquinas(h)
    {
        contorno(painter, &c, rect, ppp, Stroke::new(1.0, theme::STROKE1));
    }

    let sel = ed.selection.filter(|s| ed.scene.has::<RectUi>(*s));
    let lado_asa = 7.0;
    if resp.drag_started_by(egui::PointerButton::Primary)
        && let Some(m) = ctx.input(|i| i.pointer.press_origin())
    {
        let p = a_px(m, rect, ppp);
        let mut cogida = None;
        let nuevo = |ed: &Editor, id: EntityId, lados| ed.scene.get::<RectUi>(id).map(|antes| Arrastre { id, lados, inicio: p, antes });
        if let Some(s) = sel
            && let Some(c) = ed.ui.esquinas(s)
        {
            for (q, lados) in asas(&c) {
                if (a_pos(q, rect, ppp) - m).length() <= lado_asa {
                    cogida = nuevo(ed, s, lados);
                }
            }
        }
        if cogida.is_none()
            && let Some(h) = ed.ui.elemento_en(p)
        {
            ed.selection = Some(h);
            cogida = nuevo(ed, h, MOVER);
        }
        ed.ui_arrastre = cogida;
    }
    if let Some(a) = ed.ui_arrastre.clone() {
        if let Some(m) = raton
            && let (Some(l0), Some(l1)) = (ed.ui.a_local_del_padre(a.id, a.inicio), ed.ui.a_local_del_padre(a.id, a_px(m, rect, ppp)))
        {
            let mut d = [l1[0] - l0[0], l1[1] - l0[1]];
            // Ctrl: a múltiplos de 8 unidades.
            if ctx.input(|i| i.modifiers.command) {
                d = d.map(|v| (v / 8.0).round() * 8.0);
            }
            let mut r = a.antes.clone();
            let [li, ar, de, ab] = a.lados.0;
            if li {
                r.offset_min[0] += d[0];
            }
            if ar {
                r.offset_min[1] += d[1];
            }
            if de {
                r.offset_max[0] += d[0];
            }
            if ab {
                r.offset_max[1] += d[1];
            }
            if ed.scene.get::<RectUi>(a.id).as_ref() != Some(&r) {
                ed.scene.insert(a.id, r);
                ed.dirty = true;
            }
        }
        if !ctx.input(|i| i.pointer.primary_down()) {
            ed.ui_arrastre = None;
            ed.commit(if a.lados == MOVER { "Mover elemento" } else { "Redimensionar elemento" });
        }
        ctx.request_repaint();
    } else if resp.clicked_by(egui::PointerButton::Primary) {
        ed.selection = encima;
        ed.euler_cache = None;
    }

    // Selección con asas.
    if let Some(s) = ed.selection
        && let Some(c) = ed.ui.esquinas(s)
    {
        contorno(painter, &c, rect, ppp, Stroke::new(1.5, theme::ACCENT));
        if !ed.ui.en_contenedor(s) {
            for (q, _) in asas(&c) {
                let r = Rect::from_center_size(a_pos(q, rect, ppp), egui::vec2(lado_asa, lado_asa));
                painter.rect(r, 1.0, theme::CARD, Stroke::new(1.5, theme::ACCENT), egui::StrokeKind::Middle);
            }
        }
    }
}
