//! Ventana sin el marco del sistema: barra de título propia (logo, menús, título y botones de
//! ventana), bordes para redimensionar y un filo de 1 px alrededor.

use egui::{Color32, CursorIcon, Id, Pos2, Rect, ResizeDirection, Sense, Ui, Vec2, ViewportCommand};

use crate::icons::{self, Icon};
use crate::theme;

pub const ALTO: f32 = 34.0;
const BOTON: f32 = 46.0;
const BORDE: f32 = 5.0;

fn maximizada(ctx: &egui::Context) -> bool {
    ctx.input(|i| i.viewport().maximized.unwrap_or(false) || i.viewport().fullscreen.unwrap_or(false))
}

/// Barra de título: el logo de Maker a la izquierda (si `logo`), luego `contenido` (los menús)
/// y los botones de ventana a la derecha. Arrastrar el fondo mueve la ventana; doble clic maximiza.
pub fn show<R>(ui: &mut Ui, logo: bool, contenido: impl FnOnce(&mut Ui) -> R) -> R {
    let frame = egui::Frame::new().fill(theme::CHROME);
    egui::Panel::top("titlebar")
        .frame(frame)
        .exact_size(ALTO)
        .resizable(false)
        .show(ui, |ui| {
            let ctx = ui.ctx().clone();
            let rect = ui.max_rect();
            // Primero el fondo: lo que se añade después (menús, botones) tiene prioridad.
            let fondo = ui.interact(rect, Id::unique("titlebar-fondo"), Sense::click_and_drag());
            if fondo.double_clicked() {
                ctx.send_viewport_cmd(ViewportCommand::Maximized(!maximizada(&ctx)));
            } else if fondo.drag_started_by(egui::PointerButton::Primary) {
                ctx.send_viewport_cmd(ViewportCommand::StartDrag);
            }
            // Los botones de ventana, reservados antes que los menús para que nunca se queden sin sitio.
            let max = maximizada(&ctx);
            let botones: [(Icon, &str, bool); 3] = [
                (Icon::Minimizar, "Minimizar", false),
                (if max { Icon::Restaurar } else { Icon::Maximizar }, if max { "Restaurar" } else { "Maximizar" }, false),
                (Icon::Cerrar, "Cerrar", true),
            ];
            for (k, (icono, texto, cerrar)) in botones.into_iter().enumerate() {
                let x = rect.right() - BOTON * (3 - k) as f32;
                let r = Rect::from_min_size(Pos2::new(x, rect.top()), Vec2::new(BOTON, ALTO));
                if boton(ui, r, icono, cerrar).on_hover_text(texto).clicked() {
                    ctx.send_viewport_cmd(match k {
                        0 => ViewportCommand::Minimized(true),
                        1 => ViewportCommand::Maximized(!max),
                        _ => ViewportCommand::Close,
                    });
                }
            }
            let mut izquierda = rect;
            izquierda.min.x += 10.0;
            izquierda.max.x -= BOTON * 3.0 + 8.0;
            ui.scope_builder(egui::UiBuilder::new().max_rect(izquierda).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
                if logo {
                    let (t, size) = icons::logo(&ctx, 15.0);
                    ui.add(egui::Image::new((t.id(), size)));
                    ui.add_space(8.0);
                }
                contenido(ui)
            })
            .inner
        })
        .inner
}

fn boton(ui: &mut Ui, rect: Rect, icono: Icon, cerrar: bool) -> egui::Response {
    let resp = ui.interact(rect, Id::unique(("titlebar-boton", icono as u8)), Sense::click());
    let (fondo, tinta) = match (resp.hovered(), cerrar) {
        (true, true) => (theme::RED, Color32::WHITE),
        (true, false) => (theme::CONTROL_HOVER, theme::TEXT1),
        _ => (Color32::TRANSPARENT, theme::TEXT2),
    };
    ui.painter().rect_filled(rect, 0.0, fondo);
    let s = icons::side(ui.ctx());
    let r = Rect::from_center_size(snap(ui.ctx(), rect.center()), Vec2::splat(s));
    egui::Image::new((icons::texture(ui.ctx(), icono).id(), Vec2::splat(s))).tint(tinta).paint_at(ui, r);
    resp
}

/// Alinea la esquina del icono a un píxel físico para que no se emborrone.
fn snap(ctx: &egui::Context, centro: Pos2) -> Pos2 {
    let ppp = ctx.pixels_per_point();
    let s = icons::side(ctx);
    let esquina = centro - Vec2::splat(s / 2.0);
    Pos2::new((esquina.x * ppp).round() / ppp, (esquina.y * ppp).round() / ppp) + Vec2::splat(s / 2.0)
}

/// Bordes y esquinas para redimensionar la ventana sin marco, y el filo alrededor.
pub fn bordes(ctx: &egui::Context) {
    if maximizada(ctx) {
        return;
    }
    let pantalla = ctx.content_rect();
    ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, Id::unique("filo-ventana")))
        .rect_stroke(pantalla, 0.0, egui::Stroke::new(1.0, theme::STROKE2), egui::StrokeKind::Inside);
    let Some(p) = ctx.input(|i| i.pointer.hover_pos()) else { return };
    let (izq, der) = (p.x <= pantalla.left() + BORDE, p.x >= pantalla.right() - BORDE);
    let (arr, aba) = (p.y <= pantalla.top() + BORDE, p.y >= pantalla.bottom() - BORDE);
    let dir = match (arr, aba, izq, der) {
        (true, _, true, _) => ResizeDirection::NorthWest,
        (true, _, _, true) => ResizeDirection::NorthEast,
        (_, true, true, _) => ResizeDirection::SouthWest,
        (_, true, _, true) => ResizeDirection::SouthEast,
        (true, ..) => ResizeDirection::North,
        (_, true, ..) => ResizeDirection::South,
        (_, _, true, _) => ResizeDirection::West,
        (_, _, _, true) => ResizeDirection::East,
        _ => return,
    };
    ctx.set_cursor_icon(match dir {
        ResizeDirection::North => CursorIcon::ResizeNorth,
        ResizeDirection::South => CursorIcon::ResizeSouth,
        ResizeDirection::East => CursorIcon::ResizeEast,
        ResizeDirection::West => CursorIcon::ResizeWest,
        ResizeDirection::NorthEast => CursorIcon::ResizeNorthEast,
        ResizeDirection::NorthWest => CursorIcon::ResizeNorthWest,
        ResizeDirection::SouthEast => CursorIcon::ResizeSouthEast,
        ResizeDirection::SouthWest => CursorIcon::ResizeSouthWest,
    });
    if ctx.input(|i| i.pointer.primary_pressed()) {
        ctx.send_viewport_cmd(ViewportCommand::BeginResize(dir));
    }
}
