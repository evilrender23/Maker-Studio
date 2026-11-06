//! Paneles del editor: Jerarquía, Inspector, Contenido, Consola y Script.

use crate::editor::{BottomTab, Create, Editor};
use crate::theme::{self, section_label};
use crate::viewport::AssetDrag;
use egui::{Color32, RichText, Ui};
use glam::Vec3;
use maker_engine::components::*;
use maker_engine::log;
use maker_engine::scene::Hierarchy;

// ───────────────────────────── Jerarquía ─────────────────────────────

enum HAction {
    Select(EntityId),
    Reparent(EntityId, Option<EntityId>),
    MoveBefore(EntityId, EntityId),
    Delete(EntityId),
    Duplicate(EntityId),
    StartRename(EntityId),
    ToggleActive(EntityId),
    CreateChild(EntityId, Create),
    Focus(EntityId),
    /// Modelo soltado desde Contenido o la Biblioteca (como hijo de una entidad).
    AddModel(String, Option<EntityId>),
}

pub fn create_menu(ui: &mut Ui) -> Option<Create> {
    let mut out = None;
    if ui.button("Entidad vacía").clicked() {
        out = Some(Create::Empty);
    }
    ui.menu_button("Forma 3D", |ui| {
        for s in PrimitiveShape::ALL {
            if ui.button(s.label()).clicked() {
                out = Some(Create::Shape(s));
            }
        }
    });
    ui.menu_button("Física", |ui| {
        for s in [PrimitiveShape::Cube, PrimitiveShape::Sphere, PrimitiveShape::Capsule, PrimitiveShape::Cylinder] {
            if ui.button(format!("{} con cuerpo rígido", s.label())).clicked() {
                out = Some(Create::PhysicsShape(s));
            }
        }
    });
    ui.menu_button("Luz", |ui| {
        if ui.button("Luz direccional").clicked() {
            out = Some(Create::DirectionalLight);
        }
        if ui.button("Luz puntual").clicked() {
            out = Some(Create::PointLight);
        }
    });
    if ui.button("Cámara").clicked() {
        out = Some(Create::Camera);
    }
    out
}

/// Icono pixel art y color de cada entidad: cámara, luz (sol o bombilla), la forma de su malla
/// (naranja si tiene cuerpo físico) o el de objeto vacío.
fn icon_for(ed: &Editor, id: EntityId) -> (crate::icons::Icon, Color32) {
    use crate::icons::Icon;
    if ed.scene.has::<Camera>(id) {
        return (Icon::Camara, theme::CYAN);
    }
    if let Some(l) = ed.scene.get::<Light>(id) {
        return (if l.kind == LightKind::Directional { Icon::Sol } else { Icon::Bombilla }, theme::YELLOW);
    }
    if ed.scene.has::<Animator>(id) {
        return (Icon::Personaje, theme::PURPLE);
    }
    let Some(m) = ed.scene.get::<MeshRenderer>(id) else { return (Icon::Vacio, theme::TEXT3) };
    let icon = match &m.mesh {
        MeshSource::Model(_) => Icon::Modelo,
        MeshSource::Primitive(p) => match p {
            PrimitiveShape::Cube => Icon::Cubo,
            PrimitiveShape::Sphere => Icon::Esfera,
            PrimitiveShape::Plane => Icon::Plano,
            PrimitiveShape::Cylinder => Icon::Cilindro,
            PrimitiveShape::Cone => Icon::Cono,
            PrimitiveShape::Capsule => Icon::Capsula,
            PrimitiveShape::Torus => Icon::Toro,
        },
    };
    (icon, if ed.scene.has::<RigidBody>(id) { theme::ORANGE } else { theme::ACCENT })
}

pub fn hierarchy(ed: &mut Editor, ui: &mut Ui) {
    ui.horizontal(|ui| {
        section_label(ui, "Jerarquía");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.menu_button("+", |ui| {
                if let Some(c) = create_menu(ui) {
                    ed.create(c);
                }
            })
            .response
            .on_hover_text("Crear entidad");
        });
    });
    ui.add(egui::TextEdit::singleline(&mut ed.hierarchy_filter).hint_text("Buscar…").desired_width(f32::INFINITY));
    ui.add_space(2.0);
    let h = ed.scene.hierarchy();
    let mut actions = Vec::new();
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        let filter = ed.hierarchy_filter.trim().to_lowercase();
        ui.spacing_mut().item_spacing.y = 0.0;
        if filter.is_empty() {
            let n = h.roots.len();
            for (i, &r) in h.roots.iter().enumerate() {
                node(ed, ui, &h, r, &mut Vec::new(), i + 1 == n, &mut actions);
            }
        } else {
            for (id, _) in h.depth_first() {
                if ed.scene.name(id).to_lowercase().contains(&filter) {
                    row(ed, ui, id, &Guias::default(), &mut actions);
                }
            }
        }
        // Zona libre: soltar aquí saca la entidad a la raíz.
        let rest = ui.available_rect_before_wrap();
        let rest = egui::Rect::from_min_size(rest.min, egui::vec2(rest.width(), rest.height().max(40.0)));
        let resp = ui.interact(rest, ui.scope_id().with("hier-empty"), egui::Sense::click());
        if let Some(p) = resp.dnd_release_payload::<EntityId>() {
            actions.push(HAction::Reparent(*p, None));
        }
        if let Some(p) = resp.dnd_release_payload::<AssetDrag>() {
            actions.push(HAction::AddModel(p.0.clone(), None));
        }
        if resp.clicked() {
            ed.selection = None;
        }
        resp.context_menu(|ui| {
            if let Some(c) = create_menu(ui) {
                ed.create(c);
            }
        });
    });
    for a in actions {
        apply(ed, a);
    }
}

fn apply(ed: &mut Editor, a: HAction) {
    match a {
        HAction::Select(id) => {
            ed.selection = Some(id);
            ed.euler_cache = None;
        }
        HAction::Reparent(c, p) => {
            if ed.scene.set_parent(c, p, true) {
                ed.commit("Cambiar padre");
            }
        }
        HAction::MoveBefore(c, b) => {
            if ed.scene.move_before(c, b) {
                ed.commit("Reordenar");
            }
        }
        HAction::Delete(id) => {
            ed.selection = Some(id);
            ed.delete_selected();
        }
        HAction::Duplicate(id) => {
            ed.selection = Some(id);
            ed.duplicate_selected();
        }
        HAction::StartRename(id) => ed.rename = Some((id, ed.scene.name(id))),
        HAction::ToggleActive(id) => {
            if ed.scene.is_active(id) {
                ed.scene.insert(id, Inactive);
            } else {
                ed.scene.remove::<Inactive>(id);
            }
            ed.commit("Activar/desactivar");
        }
        HAction::CreateChild(parent, what) => {
            let id = ed.create(what);
            ed.scene.set_parent(id, Some(parent), false);
            let mut t = ed.scene.transform(id);
            t.translation = Vec3::ZERO;
            ed.scene.set_transform(id, t);
            ed.scene.update_transforms();
            ed.commit("Crear hijo");
        }
        HAction::Focus(id) => {
            ed.selection = Some(id);
            ed.focus_selected();
        }
        HAction::AddModel(path, parent) => {
            let id = ed.add_model(&path);
            if let Some(p) = parent {
                ed.scene.set_parent(id, Some(p), false);
                ed.scene.set_transform(id, Transform { translation: Vec3::ZERO, ..ed.scene.transform(id) });
                ed.scene.update_transforms();
                ed.commit("Añadir hijo");
            }
        }
    }
}

/// Alto de fila e indentación de la jerarquía (compacta, como Godot).
const FILA: f32 = 22.0;
const SANGRIA: f32 = 16.0;

/// Dónde dibujar las líneas del árbol de una fila.
#[derive(Default)]
struct Guias {
    /// Por cada antepasado: si aún le quedan hermanos debajo (sigue su línea vertical).
    siguen: Vec<bool>,
    /// Es el último de sus hermanos (la línea acaba en └).
    ultimo: bool,
    /// Tiene hijos: (desplegada).
    hijos: Option<bool>,
}

fn node(ed: &mut Editor, ui: &mut Ui, h: &Hierarchy, id: EntityId, siguen: &mut Vec<bool>, ultimo: bool, actions: &mut Vec<HAction>) {
    let children = h.children_of(id);
    let cid = ui.make_persistent_id(("hier-abierto", id.0));
    let abierto = ui.data(|d| d.get_temp::<bool>(cid)).unwrap_or(true);
    let guias = Guias { siguen: siguen.clone(), ultimo, hijos: (!children.is_empty()).then_some(abierto) };
    if row(ed, ui, id, &guias, actions) {
        ui.data_mut(|d| d.insert_temp(cid, !abierto));
    }
    if abierto && !children.is_empty() {
        siguen.push(!ultimo);
        let n = children.len();
        for (i, &c) in children.iter().enumerate() {
            node(ed, ui, h, c, siguen, i + 1 == n, actions);
        }
        siguen.pop();
    }
}

/// Una fila de la jerarquía. Devuelve `true` si se pulsó la flecha de desplegar.
fn row(ed: &mut Editor, ui: &mut Ui, id: EntityId, g: &Guias, actions: &mut Vec<HAction>) -> bool {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), FILA), egui::Sense::click_and_drag());
    let nivel = g.siguen.len() as f32;
    let x0 = rect.left() + 4.0;
    let cy = rect.center().y;
    let selected = ed.selection == Some(id);
    let p = ui.painter().clone();
    if selected {
        p.rect_filled(rect, 3.0, theme::ROW_ACTIVE);
    } else if resp.hovered() {
        p.rect_filled(rect, 3.0, theme::ROW_HOVER);
    }
    // Líneas del árbol.
    let linea = egui::Stroke::new(1.0, theme::STROKE2);
    for (k, &sigue) in g.siguen.iter().enumerate() {
        let x = x0 + k as f32 * SANGRIA + SANGRIA / 2.0;
        let ultima = k + 1 == g.siguen.len();
        if ultima {
            let fin = if g.ultimo { cy } else { rect.bottom() };
            p.vline(x, rect.top()..=fin, linea);
            let hasta = if g.hijos.is_some() { x0 + nivel * SANGRIA - 1.0 } else { x0 + nivel * SANGRIA + 3.0 };
            p.hline(x..=hasta, cy, linea);
        } else if sigue {
            p.vline(x, rect.top()..=rect.bottom(), linea);
        }
    }
    // Flecha de desplegar.
    let mut plegar = false;
    let xf = x0 + nivel * SANGRIA;
    if let Some(abierto) = g.hijos {
        let r = egui::Rect::from_center_size(egui::pos2(xf + SANGRIA / 2.0, cy), egui::vec2(SANGRIA, FILA));
        let rf = ui.interact(r, ui.make_persistent_id(("hier-flecha", id.0)), egui::Sense::click());
        plegar = rf.clicked();
        let c = egui::pos2(xf + SANGRIA / 2.0, cy);
        let col = if rf.hovered() { theme::TEXT1 } else { theme::TEXT3 };
        let pts = if abierto {
            vec![c + egui::vec2(-4.0, -2.0), c + egui::vec2(4.0, -2.0), c + egui::vec2(0.0, 3.0)]
        } else {
            vec![c + egui::vec2(-2.0, -4.0), c + egui::vec2(3.0, 0.0), c + egui::vec2(-2.0, 4.0)]
        };
        p.add(egui::Shape::convex_polygon(pts, col, egui::Stroke::NONE));
    }
    let xi = xf + SANGRIA + 2.0;
    // Ojo: activar/desactivar.
    let active = ed.scene.is_active(id);
    let s = crate::icons::side(ui.ctx());
    let ro = egui::Rect::from_center_size(egui::pos2(rect.right() - 12.0, cy), egui::vec2(20.0, FILA));
    let rojo = ui.interact(ro, ui.make_persistent_id(("hier-ojo", id.0)), egui::Sense::click());
    if rojo.clone().on_hover_text(if active { "Desactivar" } else { "Activar" }).clicked() {
        actions.push(HAction::ToggleActive(id));
    }
    let (icono_ojo, tinta_ojo) = if active { (crate::icons::Icon::Ojo, if rojo.hovered() { theme::TEXT1 } else { theme::TEXT3 }) } else { (crate::icons::Icon::OjoCerrado, theme::TEXT4) };
    crate::icons::image(ui.ctx(), icono_ojo, tinta_ojo).paint_at(ui, egui::Rect::from_center_size(ro.center(), egui::vec2(s, s)));
    if let Some((rid, text)) = &mut ed.rename
        && *rid == id
    {
        let r = egui::Rect::from_min_max(egui::pos2(xi, rect.top() + 1.0), egui::pos2(ro.left() - 4.0, rect.bottom() - 1.0));
        let te = ui.put(r, egui::TextEdit::singleline(text).margin(egui::vec2(4.0, 1.0)));
        te.request_focus();
        if te.lost_focus() {
            let (rid, text) = ed.rename.take().unwrap();
            if !text.trim().is_empty() {
                ed.scene.set_name(rid, text.trim());
                ed.commit("Renombrar");
            }
        }
        return plegar;
    }
    let (icon, icol) = icon_for(ed, id);
    let name = ed.scene.name(id);
    let text_col = if active { theme::TEXT1 } else { theme::TEXT4 };
    crate::icons::image(ui.ctx(), icon, if active { icol } else { theme::TEXT4 }).paint_at(ui, egui::Rect::from_min_size(egui::pos2(xi, cy - s / 2.0), egui::vec2(s, s)));
    let fuente = egui::TextStyle::Body.resolve(ui.style());
    let ancho = (ro.left() - 4.0 - (xi + s + 5.0)).max(0.0);
    let gal = ui.fonts_mut(|f| f.layout(name.clone(), fuente, text_col, f32::INFINITY));
    let pos = egui::pos2(xi + s + 5.0, cy - gal.size().y / 2.0);
    p.with_clip_rect(egui::Rect::from_min_size(pos, egui::vec2(ancho, FILA))).galley(pos, gal, text_col);
    if resp.clicked() {
        actions.push(HAction::Select(id));
    }
    if resp.double_clicked() {
        actions.push(HAction::Focus(id));
    }
    if resp.drag_started() {
        egui::DragAndDrop::set_payload(ui.ctx(), id);
    }
    if let Some(p) = resp.dnd_hover_payload::<EntityId>()
        && *p != id
    {
        let r = resp.rect;
        let top = ui.ctx().pointer_hover_pos().is_some_and(|m| m.y < r.top() + r.height() * 0.3);
        let stroke = egui::Stroke::new(2.0, theme::ACCENT);
        if top {
            ui.painter().hline(r.x_range(), r.top(), stroke);
        } else {
            ui.painter().rect_stroke(r, 3.0, stroke, egui::StrokeKind::Inside);
        }
    }
    if let Some(p) = resp.dnd_release_payload::<AssetDrag>() {
        actions.push(HAction::AddModel(p.0.clone(), Some(id)));
    }
    if resp.dnd_hover_payload::<AssetDrag>().is_some() {
        ui.painter().rect_stroke(resp.rect, 3.0, egui::Stroke::new(2.0, theme::ACCENT), egui::StrokeKind::Inside);
    }
    if let Some(p) = resp.dnd_release_payload::<EntityId>()
        && *p != id
    {
        let r = resp.rect;
        let top = ui.ctx().pointer_interact_pos().is_some_and(|m| m.y < r.top() + r.height() * 0.3);
        actions.push(if top { HAction::MoveBefore(*p, id) } else { HAction::Reparent(*p, Some(id)) });
    }
    resp.context_menu(|ui| {
        ui.menu_button("Crear hijo", |ui| {
            if let Some(c) = create_menu(ui) {
                actions.push(HAction::CreateChild(id, c));
            }
        });
        if ui.button("Renombrar  (F2)").clicked() {
            actions.push(HAction::StartRename(id));
        }
        if ui.button("Duplicar  (Ctrl+D)").clicked() {
            actions.push(HAction::Duplicate(id));
        }
        if ui.button(if active { "Desactivar" } else { "Activar" }).clicked() {
            actions.push(HAction::ToggleActive(id));
        }
        if ed.scene.parent(id).is_some() && ui.button("Sacar a la raíz").clicked() {
            actions.push(HAction::Reparent(id, None));
        }
        ui.separator();
        if ui.button(RichText::new("Eliminar  (Supr)").color(theme::RED)).clicked() {
            actions.push(HAction::Delete(id));
        }
    });
    plegar
}

// ───────────────────────────── Inspector ─────────────────────────────

fn axis_drag(ui: &mut Ui, v: &mut Vec3, speed: f64, suffix: &str) -> egui::Response {
    let mut resp: Option<egui::Response> = None;
    let w = ((ui.available_width() - 3.0 * 14.0 - 12.0) / 3.0).max(40.0);
    for (i, (label, col)) in [("X", theme::AXIS_X), ("Y", theme::AXIS_Y), ("Z", theme::AXIS_Z)].into_iter().enumerate() {
        ui.label(RichText::new(label).color(col).strong());
        let r = ui.add_sized([w, 20.0], egui::DragValue::new(&mut v[i]).speed(speed).max_decimals(3).suffix(suffix));
        resp = Some(match resp {
            Some(p) => p.union(r),
            None => r,
        });
    }
    resp.unwrap()
}

/// El selector de color de egui convierte a HSVA y vuelve cada fotograma, lo
/// que altera los decimales aunque no se toque: solo se escribe al cambiar.
fn color3(ui: &mut Ui, c: &mut [f32; 3]) -> egui::Response {
    let mut tmp = *c;
    let r = ui.color_edit_button_rgb(&mut tmp);
    if r.changed() {
        *c = tmp;
    }
    r
}

fn color4(ui: &mut Ui, c: &mut [f32; 4]) -> egui::Response {
    let mut tmp = *c;
    let r = ui.color_edit_button_rgba_unmultiplied(&mut tmp);
    if r.changed() {
        *c = tmp;
    }
    r
}

pub(crate) fn component_header(ui: &mut Ui, title: &str, id: &str, removable: bool) -> (bool, bool) {
    let mut remove = false;
    let state = egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), ui.make_persistent_id(id), true);
    let open = state.is_open();
    let (_t, _h, _) = state
        .show_header(ui, |ui| {
            ui.label(RichText::new(title).strong().color(theme::TEXT1));
            if removable {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("✕").on_hover_text("Quitar componente").clicked() {
                        remove = true;
                    }
                });
            }
        })
        .body(|_| {});
    (open, remove)
}

/// Marca que hubo una edición; se confirma en el historial al soltar el ratón.
fn touched(ed: &mut Editor, r: &egui::Response, label: &str) {
    if r.changed() {
        ed.pending_edit = Some(label.to_string());
        ed.dirty = true;
    }
}

pub fn inspector(ed: &mut Editor, ui: &mut Ui) {
    section_label(ui, "Inspector");
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        let Some(id) = ed.selection.filter(|s| ed.scene.contains(*s)) else {
            environment(ed, ui);
            return;
        };
        // Cabecera: activo + nombre.
        ui.horizontal(|ui| {
            let mut active = ed.scene.is_active(id);
            if ui.checkbox(&mut active, "").on_hover_text("Activa").changed() {
                if active {
                    ed.scene.remove::<Inactive>(id);
                } else {
                    ed.scene.insert(id, Inactive);
                }
                ed.commit("Activar/desactivar");
            }
            let mut name = ed.scene.name(id);
            let r = ui.add(egui::TextEdit::singleline(&mut name).desired_width(f32::INFINITY).font(egui::TextStyle::Heading));
            if r.changed() {
                ed.scene.set_name(id, &name);
                ed.pending_edit = Some("Renombrar".into());
            }
        });
        ui.label(RichText::new(format!("id {}", id.0)).small().color(theme::TEXT4));
        ui.add_space(4.0);

        // Transform.
        let (open, _) = component_header(ui, "Transform", "insp-transform", false);
        if open {
            let mut t = ed.scene.transform(id);
            let euler = match ed.euler_cache {
                Some((cid, q, e)) if cid == id && q == t.rotation => e,
                _ => t.euler_degrees(),
            };
            let mut pos = t.translation;
            let mut eul = euler;
            let mut scl = t.scale;
            egui::Grid::new("transform-grid").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
                ui.label("Posición");
                ui.horizontal(|ui| {
                    let r = axis_drag(ui, &mut pos, 0.02, "");
                    touched(ed, &r, "Mover");
                });
                ui.end_row();
                ui.label("Rotación");
                ui.horizontal(|ui| {
                    let r = axis_drag(ui, &mut eul, 0.5, "°");
                    touched(ed, &r, "Rotar");
                });
                ui.end_row();
                ui.label("Escala");
                ui.horizontal(|ui| {
                    let r = axis_drag(ui, &mut scl, 0.01, "");
                    touched(ed, &r, "Escalar");
                });
                ui.end_row();
            });
            if pos != t.translation || scl != t.scale || eul != euler {
                t.translation = pos;
                t.scale = scl;
                if eul != euler {
                    t.set_euler_degrees(eul);
                }
                ed.euler_cache = Some((id, t.rotation, eul));
                ed.scene.set_transform(id, t);
                ed.scene.update_transforms();
            }
            ui.horizontal(|ui| {
                if ui.small_button("Restablecer").clicked() {
                    ed.scene.set_transform(id, Transform::default());
                    ed.scene.update_transforms();
                    ed.commit("Restablecer transform");
                }
                if ui.small_button("Encuadrar (F)").clicked() {
                    ed.focus_selected();
                }
            });
        }
        ui.separator();

        mesh_renderer(ed, ui, id);
        light(ed, ui, id);
        camera(ed, ui, id);
        rigid_body(ed, ui, id);
        collider(ed, ui, id);
        scripts(ed, ui, id);
        crate::animator_ui::animator(ed, ui, id);

        ui.add_space(6.0);
        ui.vertical_centered_justified(|ui| {
            ui.menu_button("Añadir componente", |ui| add_component_menu(ed, ui, id));
        });
    });
}

fn add_component_menu(ed: &mut Editor, ui: &mut Ui, id: EntityId) {
    let s = &mut ed.scene;
    let mut added = None;
    if !s.has::<MeshRenderer>(id) && ui.button("Malla (Mesh Renderer)").clicked() {
        s.insert(id, MeshRenderer::primitive(PrimitiveShape::Cube));
        added = Some("Malla");
    }
    if !s.has::<Light>(id) {
        if ui.button("Luz puntual").clicked() {
            s.insert(id, Light::point());
            added = Some("Luz");
        }
        if ui.button("Luz direccional").clicked() {
            s.insert(id, Light::directional());
            added = Some("Luz");
        }
    }
    if !s.has::<Camera>(id) && ui.button("Cámara").clicked() {
        s.insert(id, Camera { primary: false, ..Default::default() });
        added = Some("Cámara");
    }
    if !s.has::<RigidBody>(id) && ui.button("Cuerpo rígido").clicked() {
        s.insert(id, RigidBody::dynamic());
        if !s.has::<Collider>(id) {
            let shape = s.get::<MeshRenderer>(id).and_then(|m| if let MeshSource::Primitive(p) = m.mesh { Some(p) } else { None }).unwrap_or(PrimitiveShape::Cube);
            s.insert(id, Collider::new(ColliderShape::fit(shape)));
        }
        added = Some("Cuerpo rígido");
    }
    if !s.has::<Animator>(id) && ui.button("Animador").clicked() {
        s.insert(id, crate::animator_ui::default_for(s, id));
        added = Some("Animador");
    }
    if !s.has::<Collider>(id) && ui.button("Colisionador").clicked() {
        let shape = s.get::<MeshRenderer>(id).and_then(|m| if let MeshSource::Primitive(p) = m.mesh { Some(p) } else { None }).unwrap_or(PrimitiveShape::Cube);
        s.insert(id, Collider::new(ColliderShape::fit(shape)));
        added = Some("Colisionador");
    }
    ui.separator();
    let names = ed.script_names();
    if names.is_empty() {
        ui.label(RichText::new("Sin scripts compilados (F7)").color(theme::TEXT3));
    }
    for n in names {
        if ui.button(format!("Script: {n}")).clicked() {
            let mut sc = ed.scene.get::<Scripts>(id).unwrap_or_default();
            sc.0.push(ScriptSlot { name: n.clone(), enabled: true, ..Default::default() });
            ed.scene.insert(id, sc);
            added = Some("Script");
        }
    }
    if let Some(a) = added {
        ed.commit(&format!("Añadir {a}"));
    }
}

fn mesh_renderer(ed: &mut Editor, ui: &mut Ui, id: EntityId) {
    let Some(mut mr) = ed.scene.get::<MeshRenderer>(id) else { return };
    let (open, remove) = component_header(ui, "Malla", "insp-mesh", true);
    if remove {
        ed.scene.remove::<MeshRenderer>(id);
        ed.commit("Quitar malla");
        return;
    }
    if !open {
        ui.separator();
        return;
    }
    let before = mr.clone();
    egui::Grid::new("mesh-grid").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
        ui.label("Malla");
        let text = match &mr.mesh {
            MeshSource::Primitive(p) => p.label().to_string(),
            MeshSource::Model(m) => m.clone(),
        };
        egui::ComboBox::from_id_salt("mesh-src").selected_text(text).width(170.0).show_ui(ui, |ui| {
            for p in PrimitiveShape::ALL {
                ui.selectable_value(&mut mr.mesh, MeshSource::Primitive(p), p.label());
            }
            let mut models = Vec::new();
            maker_engine::project::walk(&ed.project.root.join("assets"), &mut |p| {
                if p.extension().is_some_and(|e| e == "gltf" || e == "glb") {
                    models.push(ed.project.relative(p));
                }
            });
            if !models.is_empty() {
                ui.separator();
            }
            for m in models {
                ui.selectable_value(&mut mr.mesh, MeshSource::Model(m.clone()), m);
            }
        });
        ui.end_row();
        material_fields(ui, &mut mr.material);
        ui.label("Proyecta sombras");
        ui.checkbox(&mut mr.cast_shadows, "");
        ui.end_row();
    });
    if mr != before {
        ed.scene.insert(id, mr);
        ed.pending_edit = Some("Editar malla".into());
    }
    ui.separator();
}

/// Campos del material dentro de la rejilla del inspector: el modelo de
/// sombreado y solo los parámetros que ese modelo usa.
fn material_fields(ui: &mut Ui, m: &mut Material) {
    ui.label("Sombreado");
    egui::ComboBox::from_id_salt("mat-shading").selected_text(m.shading.label()).width(170.0).show_ui(ui, |ui| {
        for s in ShadingModel::ALL {
            ui.selectable_value(&mut m.shading, s, s.label());
        }
    });
    ui.end_row();
    ui.label(if m.shading == ShadingModel::Hatching { "Papel" } else { "Color base" });
    color4(ui, &mut m.base_color);
    ui.end_row();
    let st = &mut m.style;
    match m.shading {
        ShadingModel::Pbr => {
            ui.label("Metálico");
            ui.add(egui::Slider::new(&mut m.metallic, 0.0..=1.0));
            ui.end_row();
            ui.label("Rugosidad");
            ui.add(egui::Slider::new(&mut m.roughness, 0.0..=1.0));
            ui.end_row();
        }
        ShadingModel::Toon => {
            ui.label("Bandas");
            ui.add(egui::Slider::new(&mut st.bands, 2..=4));
            ui.end_row();
            ui.label("Suavidad");
            ui.add(egui::Slider::new(&mut st.softness, 0.0..=0.5));
            ui.end_row();
            ui.label("Sombra");
            color3(ui, &mut st.shadow_color);
            ui.end_row();
            ui.label("Brillo");
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(&mut st.specular).speed(0.01).range(0.0..=4.0).prefix("× "));
                ui.add(egui::Slider::new(&mut st.specular_size, 0.0..=1.0).show_value(false)).on_hover_text("Tamaño");
            });
            ui.end_row();
            ui.label("Borde");
            ui.horizontal(|ui| {
                color3(ui, &mut st.rim_color);
                ui.add(egui::DragValue::new(&mut st.rim_strength).speed(0.01).range(0.0..=4.0).prefix("× "));
                ui.add(egui::Slider::new(&mut st.rim_width, 0.0..=1.0).show_value(false)).on_hover_text("Ancho");
            });
            ui.end_row();
        }
        ShadingModel::Hatching => {
            ui.label("Tinta");
            color3(ui, &mut st.outline_color);
            ui.end_row();
            ui.label("Separación");
            ui.add(egui::Slider::new(&mut st.hatch_scale, 3.0..=24.0).suffix(" px"));
            ui.end_row();
            ui.label("Suavidad");
            ui.add(egui::Slider::new(&mut st.softness, 0.0..=0.5));
            ui.end_row();
        }
        ShadingModel::Unlit => {}
    }
    if m.shading.has_outline() {
        ui.label("Contorno");
        ui.horizontal(|ui| {
            if m.shading == ShadingModel::Toon {
                color3(ui, &mut st.outline_color);
            }
            ui.add(egui::DragValue::new(&mut st.outline_width).speed(0.05).range(0.0..=20.0).suffix(" px"));
        });
        ui.end_row();
    }
    ui.label("Emisión");
    ui.horizontal(|ui| {
        color3(ui, &mut m.emissive);
        ui.add(egui::DragValue::new(&mut m.emissive_strength).speed(0.05).range(0.0..=100.0).prefix("× "));
    });
    ui.end_row();
}

fn light(ed: &mut Editor, ui: &mut Ui, id: EntityId) {
    let Some(mut l) = ed.scene.get::<Light>(id) else { return };
    let (open, remove) = component_header(ui, "Luz", "insp-light", true);
    if remove {
        ed.scene.remove::<Light>(id);
        ed.commit("Quitar luz");
        return;
    }
    if open {
        let before = l;
        egui::Grid::new("light-grid").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
            ui.label("Tipo");
            egui::ComboBox::from_id_salt("light-kind").selected_text(if l.kind == LightKind::Directional { "Direccional" } else { "Puntual" }).show_ui(ui, |ui| {
                ui.selectable_value(&mut l.kind, LightKind::Directional, "Direccional");
                ui.selectable_value(&mut l.kind, LightKind::Point, "Puntual");
            });
            ui.end_row();
            ui.label("Color");
            color3(ui, &mut l.color);
            ui.end_row();
            ui.label("Intensidad");
            ui.add(egui::DragValue::new(&mut l.intensity).speed(0.05).range(0.0..=1000.0));
            ui.end_row();
            if l.kind == LightKind::Point {
                ui.label("Alcance");
                ui.add(egui::DragValue::new(&mut l.range).speed(0.05).range(0.1..=1000.0).suffix(" m"));
                ui.end_row();
            } else {
                ui.label("Sombras");
                ui.checkbox(&mut l.cast_shadows, "");
                ui.end_row();
            }
        });
        if l != before {
            ed.scene.insert(id, l);
            ed.pending_edit = Some("Editar luz".into());
        }
    }
    ui.separator();
}

fn camera(ed: &mut Editor, ui: &mut Ui, id: EntityId) {
    let Some(mut c) = ed.scene.get::<Camera>(id) else { return };
    let (open, remove) = component_header(ui, "Cámara", "insp-camera", true);
    if remove {
        ed.scene.remove::<Camera>(id);
        ed.commit("Quitar cámara");
        return;
    }
    if open {
        let before = c;
        egui::Grid::new("cam-grid").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
            ui.label("Campo de visión");
            ui.add(egui::Slider::new(&mut c.fov_y, 10.0..=120.0).suffix("°"));
            ui.end_row();
            ui.label("Cerca / lejos");
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(&mut c.near).speed(0.01).range(0.001..=10.0));
                ui.add(egui::DragValue::new(&mut c.far).speed(1.0).range(1.0..=100000.0));
            });
            ui.end_row();
            ui.label("Principal");
            ui.checkbox(&mut c.primary, "");
            ui.end_row();
        });
        if ui.small_button("Alinear con la vista del editor").clicked() {
            let eye = ed.cam.eye();
            let m = glam::Mat4::from_rotation_translation(glam::Quat::from_mat4(&ed.cam.view().inverse()), eye);
            ed.scene.set_world_matrix(id, m);
            ed.scene.update_transforms();
            ed.commit("Alinear cámara");
        }
        if c != before {
            if c.primary && !before.primary {
                // Solo una cámara principal.
                let others: Vec<EntityId> = ed.scene.world.query::<(&EntityId, &Camera)>().iter().filter(|(o, _)| **o != id).map(|(o, _)| *o).collect();
                for o in others {
                    if let Some(mut oc) = ed.scene.get_mut::<Camera>(o) {
                        oc.primary = false;
                    }
                }
            }
            ed.scene.insert(id, c);
            ed.pending_edit = Some("Editar cámara".into());
        }
    }
    ui.separator();
}

fn rigid_body(ed: &mut Editor, ui: &mut Ui, id: EntityId) {
    let Some(mut b) = ed.scene.get::<RigidBody>(id) else { return };
    let (open, remove) = component_header(ui, "Cuerpo rígido", "insp-rb", true);
    if remove {
        ed.scene.remove::<RigidBody>(id);
        ed.commit("Quitar cuerpo rígido");
        return;
    }
    if open {
        let before = b;
        egui::Grid::new("rb-grid").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
            ui.label("Tipo");
            let label = |k: BodyKind| match k {
                BodyKind::Dynamic => "Dinámico",
                BodyKind::Fixed => "Estático",
                BodyKind::Kinematic => "Cinemático",
            };
            egui::ComboBox::from_id_salt("rb-kind").selected_text(label(b.kind)).show_ui(ui, |ui| {
                for k in [BodyKind::Dynamic, BodyKind::Fixed, BodyKind::Kinematic] {
                    ui.selectable_value(&mut b.kind, k, label(k));
                }
            });
            ui.end_row();
            ui.label("Masa");
            ui.add(egui::DragValue::new(&mut b.mass).speed(0.05).range(0.001..=100000.0).suffix(" kg"));
            ui.end_row();
            ui.label("Amort. lineal / angular");
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(&mut b.linear_damping).speed(0.01).range(0.0..=100.0));
                ui.add(egui::DragValue::new(&mut b.angular_damping).speed(0.01).range(0.0..=100.0));
            });
            ui.end_row();
            ui.label("Escala de gravedad");
            ui.add(egui::DragValue::new(&mut b.gravity_scale).speed(0.02));
            ui.end_row();
            ui.label("Bloquear rotación");
            ui.checkbox(&mut b.lock_rotation, "");
            ui.end_row();
            ui.label("CCD (rápidos)");
            ui.checkbox(&mut b.ccd, "");
            ui.end_row();
        });
        if b != before {
            ed.scene.insert(id, b);
            ed.pending_edit = Some("Editar cuerpo rígido".into());
        }
    }
    ui.separator();
}

fn collider(ed: &mut Editor, ui: &mut Ui, id: EntityId) {
    let Some(mut c) = ed.scene.get::<Collider>(id) else { return };
    let (open, remove) = component_header(ui, "Colisionador", "insp-col", true);
    if remove {
        ed.scene.remove::<Collider>(id);
        ed.commit("Quitar colisionador");
        return;
    }
    if open {
        let before = c;
        egui::Grid::new("col-grid").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
            ui.label("Forma");
            egui::ComboBox::from_id_salt("col-shape").selected_text(c.shape.label()).show_ui(ui, |ui| {
                let opts = [
                    ColliderShape::Box { half_extents: Vec3::splat(0.5) },
                    ColliderShape::Sphere { radius: 0.5 },
                    ColliderShape::Capsule { half_height: 0.5, radius: 0.5 },
                    ColliderShape::Cylinder { half_height: 0.5, radius: 0.5 },
                ];
                for o in opts {
                    if ui.selectable_label(std::mem::discriminant(&c.shape) == std::mem::discriminant(&o), o.label()).clicked() {
                        c.shape = o;
                    }
                }
            });
            ui.end_row();
            match &mut c.shape {
                ColliderShape::Box { half_extents } => {
                    ui.label("Medias ext.");
                    ui.horizontal(|ui| {
                        axis_drag(ui, half_extents, 0.01, "");
                    });
                    ui.end_row();
                }
                ColliderShape::Sphere { radius } => {
                    ui.label("Radio");
                    ui.add(egui::DragValue::new(radius).speed(0.01).range(0.001..=1000.0));
                    ui.end_row();
                }
                ColliderShape::Capsule { half_height, radius } | ColliderShape::Cylinder { half_height, radius } => {
                    ui.label("Media altura / radio");
                    ui.horizontal(|ui| {
                        ui.add(egui::DragValue::new(half_height).speed(0.01).range(0.001..=1000.0));
                        ui.add(egui::DragValue::new(radius).speed(0.01).range(0.001..=1000.0));
                    });
                    ui.end_row();
                }
            }
            ui.label("Fricción");
            ui.add(egui::Slider::new(&mut c.friction, 0.0..=2.0));
            ui.end_row();
            ui.label("Rebote");
            ui.add(egui::Slider::new(&mut c.restitution, 0.0..=1.0));
            ui.end_row();
            ui.label("Disparador");
            ui.checkbox(&mut c.is_trigger, "");
            ui.end_row();
        });
        if let Some(MeshSource::Primitive(p)) = ed.scene.get::<MeshRenderer>(id).map(|m| m.mesh)
            && ui.small_button("Ajustar a la malla").clicked()
        {
            c.shape = ColliderShape::fit(p);
        }
        if c != before {
            ed.scene.insert(id, c);
            ed.pending_edit = Some("Editar colisionador".into());
        }
    }
    ui.separator();
}

fn scripts(ed: &mut Editor, ui: &mut Ui, id: EntityId) {
    let Some(mut sc) = ed.scene.get::<Scripts>(id) else { return };
    let before = sc.clone();
    let mut remove_idx = None;
    for (i, slot) in sc.0.iter_mut().enumerate() {
        let (open, remove) = component_header(ui, &format!("Script · {}", slot.name), &format!("insp-script-{i}"), true);
        if remove {
            remove_idx = Some(i);
            continue;
        }
        if !open {
            continue;
        }
        ui.checkbox(&mut slot.enabled, "Habilitado");
        let lib = ed.lib.clone();
        let known = lib.as_ref().is_some_and(|l| l.index_of(&slot.name).is_some());
        if !known {
            ui.colored_label(theme::YELLOW, "Script no encontrado en la biblioteca compilada (F7 para compilar).");
        }
        let defaults = lib.map(|l| l.default_props(&slot.name)).unwrap_or_default();
        egui::Grid::new(("script-grid", i)).num_columns(3).spacing([8.0, 4.0]).show(ui, |ui| {
            for (name, def) in &defaults {
                let overridden = slot.props.contains_key(name);
                let mut v = slot.props.get(name).copied().unwrap_or(*def);
                ui.label(if overridden { RichText::new(name).strong() } else { RichText::new(name) });
                let changed = match &mut v {
                    PropVal::Float(f) => ui.add(egui::DragValue::new(f).speed(0.05)).changed(),
                    PropVal::Int(n) => ui.add(egui::DragValue::new(n)).changed(),
                    PropVal::Bool(b) => ui.checkbox(b, "").changed(),
                    PropVal::Vec3(a) => {
                        let mut vv = Vec3::from(*a);
                        let r = ui.horizontal(|ui| axis_drag(ui, &mut vv, 0.05, "")).inner;
                        *a = vv.to_array();
                        r.changed()
                    }
                    PropVal::Color(c) => color4(ui, c).changed(),
                };
                if changed {
                    slot.props.insert(name.clone(), v);
                }
                if overridden {
                    if ui.small_button("↺").on_hover_text("Volver al valor del script").clicked() {
                        slot.props.remove(name);
                    }
                } else {
                    ui.label("");
                }
                ui.end_row();
            }
            // Valores guardados de propiedades que el script ya no declara.
            for (name, v) in slot.props.iter() {
                if !defaults.iter().any(|(n, _)| n == name) {
                    ui.label(RichText::new(name).color(theme::TEXT4));
                    ui.label(RichText::new(format!("{v:?} (no declarada)")).color(theme::TEXT4));
                    ui.label("");
                    ui.end_row();
                }
            }
        });
        if ui.small_button("Editar código").clicked() {
            ed.open_script(ed.project.scripts_dir().join("src/lib.rs"));
        }
    }
    if let Some(i) = remove_idx {
        sc.0.remove(i);
    }
    if sc != before {
        if sc.0.is_empty() {
            ed.scene.remove::<Scripts>(id);
        } else {
            ed.scene.insert(id, sc);
        }
        ed.pending_edit = Some("Editar script".into());
    }
    ui.separator();
}

fn environment(ed: &mut Editor, ui: &mut Ui) {
    ui.label(RichText::new("Entorno de la escena").strong());
    ui.add_space(4.0);
    let mut env = ed.scene.env;
    egui::Grid::new("env-grid").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
        ui.label("Cielo (cénit)");
        color3(ui, &mut env.sky_top);
        ui.end_row();
        ui.label("Cielo (horizonte)");
        color3(ui, &mut env.sky_horizon);
        ui.end_row();
        ui.label("Suelo");
        color3(ui, &mut env.ground);
        ui.end_row();
        ui.label("Luz ambiente");
        ui.add(egui::Slider::new(&mut env.ambient_intensity, 0.0..=3.0));
        ui.end_row();
        ui.label("Niebla");
        ui.horizontal(|ui| {
            color3(ui, &mut env.fog_color);
            ui.add(egui::DragValue::new(&mut env.fog_density).speed(0.0005).range(0.0..=1.0).max_decimals(4));
        });
        ui.end_row();
        ui.label("Exposición");
        ui.add(egui::Slider::new(&mut env.exposure, 0.05..=8.0).logarithmic(true));
        ui.end_row();
        ui.label("Gravedad");
        ui.horizontal(|ui| {
            let mut g = Vec3::from(env.gravity);
            axis_drag(ui, &mut g, 0.05, "");
            env.gravity = g.to_array();
        });
        ui.end_row();
    });
    if env != ed.scene.env {
        ed.scene.env = env;
        ed.pending_edit = Some("Editar entorno".into());
    }
    ui.add_space(10.0);
    section_label(ui, "Proyecto");
    ui.label(format!("{} · {}", ed.project.name(), ed.project.root.display()));
    ui.label(format!("Escena: {}", ed.project.relative(&ed.scene_path)));
    match &ed.lib {
        Some(l) => ui.label(format!("Scripts: {}", l.names().join(", "))),
        None => ui.colored_label(theme::YELLOW, ed.lib_error.clone().unwrap_or_else(|| "Scripts sin cargar".into())),
    };
}

// ───────────────────────────── Panel inferior ─────────────────────────────

pub fn bottom(ed: &mut Editor, ui: &mut Ui) {
    let new_logs = log::len().saturating_sub(ed.console_seen);
    let errors = log::since(0).iter().filter(|e| e.level == log::Level::Error).count();
    ui.horizontal(|ui| {
        ui.selectable_value(&mut ed.bottom_tab, BottomTab::Content, "Contenido");
        let lib = ed.bottom_tab == BottomTab::Library;
        if ui.add(egui::Button::selectable(lib, (crate::icons::image(ui.ctx(), crate::icons::Icon::Biblioteca, if lib { theme::ACCENT } else { theme::TEXT2 }), "Biblioteca"))).clicked() {
            ed.bottom_tab = BottomTab::Library;
        }
        let console_label = if errors > 0 { format!("Consola · {errors} errores") } else if new_logs > 0 && ed.bottom_tab != BottomTab::Console { format!("Consola · {new_logs}") } else { "Consola".to_string() };
        ui.selectable_value(&mut ed.bottom_tab, BottomTab::Console, console_label);
        let script_label = match &ed.script {
            Some(s) if s.text != s.saved => "Script ●".to_string(),
            _ => "Script".to_string(),
        };
        ui.selectable_value(&mut ed.bottom_tab, BottomTab::Script, script_label);
    });
    ui.separator();
    match ed.bottom_tab {
        BottomTab::Content => crate::contenido_ui::show(ed, ui),
        BottomTab::Library => crate::biblioteca_ui::show(ed, ui),
        BottomTab::Console => console(ed, ui),
        BottomTab::Script => script_editor(ed, ui),
    }
}


pub fn import_model(ed: &mut Editor) {
    let raw = ed.import_path.trim().trim_matches(['"', '\'']).trim_start_matches("file://").to_string();
    let src = if let Some(rest) = raw.strip_prefix("~/") { std::env::var("HOME").map(|h| format!("{h}/{rest}")).unwrap_or(raw.clone()) } else { raw.clone() };
    let src = std::path::PathBuf::from(src);
    let Some(name) = src.file_name() else {
        log::error("editor", "ruta vacía");
        return;
    };
    let dst_dir = ed.project.root.join("assets/models");
    let _ = std::fs::create_dir_all(&dst_dir);
    let dst = dst_dir.join(name);
    match std::fs::copy(&src, &dst) {
        Ok(_) => {
            // Un .gltf puede referenciar un .bin y texturas junto a él.
            if src.extension().is_some_and(|e| e == "gltf")
                && let Ok(txt) = std::fs::read_to_string(&src)
            {
                for part in txt.split('"').filter(|s| s.ends_with(".bin") || s.ends_with(".png") || s.ends_with(".jpg")) {
                    if let Some(dir) = src.parent() {
                        let _ = std::fs::copy(dir.join(part), dst_dir.join(part));
                    }
                }
            }
            let rel = ed.project.relative(&dst);
            log::info("editor", format!("modelo importado: {rel}"));
            ed.import_path.clear();
            ed.add_model(&rel);
        }
        Err(e) => log::error("editor", format!("{}: {e}", src.display())),
    }
}

fn console(ed: &mut Editor, ui: &mut Ui) {
    let entries = log::since(0);
    ed.console_seen = entries.len();
    ui.horizontal(|ui| {
        let counts = [log::Level::Info, log::Level::Warn, log::Level::Error].map(|l| entries.iter().filter(|e| e.level == l).count());
        ui.toggle_value(&mut ed.console_filter[0], format!("Info {}", counts[0]));
        ui.toggle_value(&mut ed.console_filter[1], RichText::new(format!("Avisos {}", counts[1])).color(theme::YELLOW));
        ui.toggle_value(&mut ed.console_filter[2], RichText::new(format!("Errores {}", counts[2])).color(theme::RED));
        if ui.button("Limpiar").clicked() {
            log::clear();
        }
    });
    egui::ScrollArea::vertical().auto_shrink([false, false]).stick_to_bottom(true).show(ui, |ui| {
        for e in entries.iter().filter(|e| ed.console_filter[e.level as usize]) {
            let (tag, col) = match e.level {
                log::Level::Info => ("INFO", theme::TEXT3),
                log::Level::Warn => ("AVISO", theme::YELLOW),
                log::Level::Error => ("ERROR", theme::RED),
            };
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(tag).monospace().strong().color(col));
                ui.label(RichText::new(format!("[{}]", e.source)).monospace().color(theme::TEXT3));
                ui.add(egui::Label::new(RichText::new(&e.text).monospace().color(if e.level == log::Level::Info { theme::TEXT1 } else { col })).selectable(true));
            });
        }
    });
}

fn script_editor(ed: &mut Editor, ui: &mut Ui) {
    let files: Vec<std::path::PathBuf> = {
        let mut v = Vec::new();
        maker_engine::project::walk(&ed.project.scripts_dir().join("src"), &mut |p| {
            if p.extension().is_some_and(|e| e == "rs") {
                v.push(p.to_path_buf());
            }
        });
        v
    };
    if ed.script.is_none()
        && let Some(f) = files.first()
    {
        ed.open_script(f.clone());
        ed.bottom_tab = BottomTab::Script;
    }
    ui.horizontal(|ui| {
        for f in &files {
            let name = f.file_name().unwrap_or_default().to_string_lossy().into_owned();
            let open = ed.script.as_ref().is_some_and(|s| &s.path == f);
            if ui.selectable_label(open, &name).clicked() && !open {
                ed.open_script(f.clone());
            }
        }
        ui.separator();
        let modified = ed.script.as_ref().is_some_and(|s| s.text != s.saved);
        if ui.add_enabled(modified, egui::Button::new("Guardar  (Ctrl+S)")).clicked() {
            ed.save_script();
        }
        let building = ed.build.is_some();
        if ui.add_enabled(!building, egui::Button::new(if building { "Compilando…" } else { "Compilar y recargar  (F7)" })).clicked() {
            ed.build_scripts();
        }
        if building {
            ui.spinner();
        } else if let Some(ok) = ed.last_build_ok {
            ui.label(if ok { RichText::new("✔ compilado").color(theme::GREEN) } else { RichText::new("✖ errores (ver Consola)").color(theme::RED) });
        }
    });
    let Some(s) = &mut ed.script else {
        ui.label("No hay scripts en scripts/src.");
        return;
    };
    egui::Frame::new().fill(theme::CARD).stroke(egui::Stroke::new(1.0, theme::STROKE3)).corner_radius(4.0).inner_margin(6.0).show(ui, |ui| {
        egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
            let r = crate::code::code_editor(ui, &mut s.text, "script-editor");
            if r.has_focus() && ui.input_mut(|i| i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::S))) {
                ed.pending_save_script = true;
            }
        });
    });
    if ed.pending_save_script {
        ed.pending_save_script = false;
        ed.save_script();
    }
}
