//! Pestaña «Biblioteca»: los modelos CC0 integrados en el motor, por pack y
//! categoría, con búsqueda y miniaturas. Arrastrar al viewport o a la
//! jerarquía crea la entidad; doble clic la añade delante de la cámara.

use crate::editor::Editor;
use crate::theme;
use crate::viewport::AssetDrag;
use egui::{Color32, RichText, Ui};
use maker_engine::biblioteca::{self, Catalog, CatalogEntry};
use maker_engine::components::*;
use std::collections::HashMap;
use std::sync::Arc;

const CELL: f32 = 104.0;
const THUMB: f32 = 80.0;

#[derive(Default)]
pub struct BibliotecaUi {
    catalog: Option<Result<Arc<Catalog>, String>>,
    pack: Option<String>,
    category: Option<String>,
    search: String,
    selected: Option<String>,
    thumbs: HashMap<String, Option<egui::TextureHandle>>,
}

impl BibliotecaUi {
    pub fn catalog(&mut self) -> Option<&Catalog> {
        self.catalog.get_or_insert_with(|| Catalog::load_default().map(Arc::new)).as_ref().ok().map(|c| c.as_ref())
    }

    fn thumb(&mut self, ctx: &egui::Context, e: &CatalogEntry) -> Option<egui::TextureHandle> {
        if let Some(t) = self.thumbs.get(&e.thumbnail) {
            return t.clone();
        }
        let tex = biblioteca::library_dir().and_then(|d| image::open(d.join(&e.thumbnail)).ok()).map(|img| {
            let img = img.to_rgba8();
            let ci = egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw());
            ctx.load_texture(format!("bib-{}", e.thumbnail), ci, egui::TextureOptions::LINEAR)
        });
        self.thumbs.insert(e.thumbnail.clone(), tex.clone());
        tex
    }
}

/// Animator por defecto para un modelo con piel: sus propios clips si los
/// tiene, si no la Universal Animation Library en reposo.
pub fn default_animator(e: Option<&CatalogEntry>) -> Animator {
    match e {
        Some(e) if !e.clips.is_empty() => {
            let clip = e.clips.iter().find(|c| c.contains("Idle")).or(e.clips.first()).cloned().unwrap_or_default();
            Animator::new("", &clip)
        }
        _ => Animator::new(biblioteca::DEFAULT_CLIPS, biblioteca::DEFAULT_IDLE),
    }
}

/// Ajustes de un modelo recién añadido: nombre del catálogo, material para
/// texturas y, si tiene piel, un Animator.
pub fn decorate_new_model(ed: &mut Editor, id: EntityId, path: &str) {
    if !biblioteca::is_library_path(path) {
        return;
    }
    let entry = ed.biblioteca.catalog().and_then(|c| c.entry(path)).cloned();
    if let Some(e) = &entry {
        ed.scene.set_name(id, &e.name);
    }
    if let Some(mut mr) = ed.scene.get::<MeshRenderer>(id) {
        mr.material = maker_engine::project::library_material();
        ed.scene.insert(id, mr);
    }
    if entry.as_ref().is_some_and(|e| e.has_skin) {
        ed.scene.insert(id, default_animator(entry.as_ref()));
    }
}

pub fn show(ed: &mut Editor, ui: &mut Ui) {
    if ed.biblioteca.catalog().is_none() {
        let err = match &ed.biblioteca.catalog {
            Some(Err(e)) => e.clone(),
            _ => String::new(),
        };
        ui.label(RichText::new(err).color(theme::RED));
        return;
    }
    let ctx = ui.ctx().clone();
    let b = &mut ed.biblioteca;
    let Some(Ok(cat)) = b.catalog.clone() else { return };
    let mut add: Option<String> = None;
    egui::Panel::left("bib-packs").resizable(true).default_size(190.0).min_size(140.0).frame(egui::Frame::new().inner_margin(egui::Margin { right: 6, ..Default::default() })).show(ui, |ui| {
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let total = cat.models.len();
            if ui.add(egui::Button::selectable(b.pack.is_none(), format!("Todos  {total}")).min_size(egui::vec2(ui.available_width(), 0.0))).clicked() {
                b.pack = None;
                b.category = None;
            }
            for p in &cat.packs {
                let sel = b.pack.as_deref() == Some(p.id.as_str());
                let r = ui.add(egui::Button::selectable(sel, format!("{}  {}", p.name, p.models)).min_size(egui::vec2(ui.available_width(), 0.0)));
                if r.clicked() && !sel {
                    b.pack = Some(p.id.clone());
                    b.category = None;
                }
                r.on_hover_text(format!("{} · {} · {}", p.author, p.license, p.source));
            }
        });
    });
    ui.horizontal(|ui| {
        let cats = cat.categories(b.pack.as_deref());
        egui::ComboBox::from_id_salt("bib-cat").selected_text(b.category.clone().unwrap_or_else(|| "Todas las categorías".into())).width(170.0).show_ui(ui, |ui| {
            ui.selectable_value(&mut b.category, None, "Todas las categorías");
            for c in cats {
                ui.selectable_value(&mut b.category, Some(c.clone()), c);
            }
        });
        ui.add(egui::TextEdit::singleline(&mut b.search).hint_text("Buscar…").desired_width(220.0));
    });
    ui.add_space(4.0);
    let (pack, category, query) = (b.pack.clone(), b.category.clone(), b.search.clone());
    let items: Vec<&CatalogEntry> = cat.filter(pack.as_deref(), category.as_deref(), &query).collect();
    let cols = ((ui.available_width() / CELL).floor() as usize).max(1);
    let rows = items.len().div_ceil(cols);
    egui::ScrollArea::vertical().auto_shrink([false, false]).show_rows(ui, CELL + 4.0, rows, |ui, range| {
        for row in range {
            ui.horizontal(|ui| {
                for e in items.iter().skip(row * cols).take(cols) {
                    let selected = b.selected.as_deref() == Some(e.path.as_str());
                    let tex = b.thumb(&ctx, e);
                    let (rect, resp) = ui.allocate_exact_size(egui::vec2(CELL - 4.0, CELL), egui::Sense::click_and_drag());
                    let p = ui.painter_at(rect);
                    let bg = if selected {
                        theme::ACCENT_SOFT
                    } else if resp.hovered() {
                        theme::ROW_HOVER
                    } else {
                        Color32::TRANSPARENT
                    };
                    p.rect_filled(rect, 4.0, bg);
                    if selected {
                        p.rect_stroke(rect, 4.0, egui::Stroke::new(1.0, theme::ACCENT), egui::StrokeKind::Inside);
                    }
                    let img_rect = egui::Rect::from_center_size(egui::pos2(rect.center().x, rect.top() + 4.0 + THUMB * 0.5), egui::vec2(THUMB, THUMB));
                    if let Some(t) = &tex {
                        p.image(t.id(), img_rect, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
                    } else {
                        p.rect_filled(img_rect, 4.0, theme::FILL_TERTIARY);
                    }
                    let name = elide(ui, &e.name, CELL - 10.0);
                    p.text(egui::pos2(rect.center().x, rect.bottom() - 4.0), egui::Align2::CENTER_BOTTOM, name, egui::FontId::proportional(11.0), theme::TEXT1);
                    if resp.drag_started() {
                        egui::DragAndDrop::set_payload(&resp.ctx, AssetDrag(e.uri()));
                    }
                    if resp.clicked() {
                        b.selected = Some(e.path.clone());
                    }
                    if resp.double_clicked() {
                        add = Some(e.uri());
                    }
                    resp.on_hover_text(format!("{}\n{} triángulos", e.name, e.triangles));
                }
            });
        }
    });
    if let Some(uri) = add {
        ed.add_model(&uri);
        ed.status = format!("Añadido {}", ed.selection.map(|s| ed.scene.name(s)).unwrap_or_default());
    }
}

/// Recorta un texto con «…» para que quepa en `width` puntos.
fn elide(ui: &Ui, text: &str, width: f32) -> String {
    let font = egui::FontId::proportional(11.0);
    let fits = |s: &str| ui.ctx().fonts_mut(|f| f.layout_no_wrap(s.to_string(), font.clone(), Color32::BLACK).size().x) <= width;
    if fits(text) {
        return text.to_string();
    }
    let mut chars: Vec<char> = text.chars().collect();
    while !chars.is_empty() {
        chars.pop();
        let s: String = chars.iter().collect::<String>() + "…";
        if fits(&s) {
            return s;
        }
    }
    String::new()
}
