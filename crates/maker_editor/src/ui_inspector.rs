//! Inspector de los componentes de interfaz: Lienzo, Rect UI (con la rejilla
//! de preajustes de anclas), Contenedor, el widget, su estilo con retoques
//! locales y la instancia de plantilla.

use crate::editor::Editor;
use crate::panels::component_header;
use crate::theme;
use egui::{RichText, Ui};
use maker_engine::components::EntityId;
use maker_engine::ui::estilo::{EfectoTexto, Estado, PropsEstilo, Relleno, Sombra};
use maker_engine::ui::*;
use std::path::Path;

/// Edita una copia del color (el selector de egui reescribe decimales cada fotograma).
fn color(ui: &mut Ui, c: &mut [f32; 4]) -> bool {
    let mut tmp = *c;
    let r = ui.color_edit_button_rgba_unmultiplied(&mut tmp);
    if r.changed() {
        *c = tmp;
    }
    r.changed()
}

fn cuatro(ui: &mut Ui, v: &mut [f32; 4], vel: f64, tips: [&str; 4]) -> bool {
    let mut ch = false;
    for (i, t) in tips.iter().enumerate() {
        ch |= ui.add(egui::DragValue::new(&mut v[i]).speed(vel).max_decimals(1)).on_hover_text(*t).changed();
    }
    ch
}

fn dos(ui: &mut Ui, v: &mut [f32; 2], vel: f64, tips: [&str; 2]) -> bool {
    let mut ch = false;
    for (i, t) in tips.iter().enumerate() {
        ch |= ui.add(egui::DragValue::new(&mut v[i]).speed(vel).max_decimals(2)).on_hover_text(*t).changed();
    }
    ch
}

const LTRB: [&str; 4] = ["Izquierda", "Arriba", "Derecha", "Abajo"];
const ESQ: [&str; 4] = ["Sup. izq.", "Sup. der.", "Inf. der.", "Inf. izq."];

/// Archivos del proyecto con alguna de las extensiones (rutas relativas).
pub fn archivos(raiz: &Path, ext: &[&str]) -> Vec<String> {
    let mut v = Vec::new();
    maker_engine::project::walk(raiz, &mut |p| {
        if p.extension().and_then(|e| e.to_str()).is_some_and(|e| ext.contains(&e.to_ascii_lowercase().as_str())) {
            v.push(p.strip_prefix(raiz).unwrap_or(p).to_string_lossy().replace('\\', "/"));
        }
    });
    v
}

pub const EXT_IMAGEN: [&str; 3] = ["png", "jpg", "jpeg"];
pub const EXT_FUENTE: [&str; 2] = ["ttf", "otf"];

/// Selector de archivo del proyecto (o vacío).
fn elegir(ui: &mut Ui, salt: impl std::hash::Hash + std::fmt::Debug, raiz: &Path, ext: &[&str], vacio: &str, v: &mut String) -> bool {
    let mut ch = false;
    let texto = if v.is_empty() { vacio.to_string() } else { v.rsplit('/').next().unwrap_or(v).to_string() };
    egui::ComboBox::from_id_salt(salt).selected_text(texto).width(160.0).show_ui(ui, |ui| {
        if ui.selectable_label(v.is_empty(), vacio).clicked() {
            v.clear();
            ch = true;
        }
        for a in archivos(raiz, ext) {
            if ui.selectable_label(*v == a, &a).clicked() {
                *v = a;
                ch = true;
            }
        }
    });
    ch
}

pub fn nombre_prop(n: &str) -> &'static str {
    match n {
        "fondo" => "Fondo",
        "imagen" => "Imagen",
        "nueve" => "9-slice",
        "tinte" => "Tinte",
        "borde_color" => "Color del borde",
        "borde_ancho" => "Ancho del borde",
        "radio" => "Radio",
        "sombra" => "Sombra",
        "relleno" => "Relleno interior",
        "fuente" => "Fuente",
        "tam_texto" => "Tamaño del texto",
        "color_texto" => "Color del texto",
        "contorno_texto" => "Contorno del texto",
        "sombra_texto" => "Sombra del texto",
        "escala" => "Escala",
        "desplazamiento" => "Desplazamiento",
        "opacidad" => "Opacidad",
        "transicion" => "Transición",
        _ => "?",
    }
}

/// Propiedades que tienen sentido para un tipo y una parte.
pub fn props_de(tipo: Option<TipoWidget>, parte: Option<&str>) -> Vec<&'static str> {
    let caja = ["fondo", "imagen", "nueve", "tinte", "borde_color", "borde_ancho", "radio", "sombra"];
    let texto = ["fuente", "tam_texto", "color_texto", "contorno_texto", "sombra_texto"];
    let estado = ["escala", "desplazamiento", "opacidad", "transicion"];
    match parte {
        Some("marcador") => vec!["color_texto"],
        Some(_) => caja.into_iter().chain(["escala"]).collect(),
        None => {
            let mut v: Vec<&str> = caja.to_vec();
            v.push("relleno");
            if tipo.is_none_or(|t| matches!(t, TipoWidget::Texto | TipoWidget::Boton | TipoWidget::Casilla | TipoWidget::CampoTexto)) {
                v.extend(texto);
            }
            v.extend(estado);
            v
        }
    }
}

/// Cambio pedido en la rejilla de propiedades.
pub enum Cambio {
    /// Define la propiedad con el valor de esta copia.
    Poner(&'static str, Box<PropsEstilo>),
    /// Quita la definición (vuelve a lo heredado).
    Quitar(&'static str),
}

/// Rejilla de propiedades de estilo: muestra `valor` (lo resuelto) y marca en
/// negrita, con ↺, las que define `capa`. Solo devuelve lo que se tocó.
pub fn rejilla_props(ui: &mut Ui, salt: &str, raiz: &Path, valor: &PropsEstilo, capa: &PropsEstilo, nombres: &[&'static str]) -> Option<Cambio> {
    let mut out = None;
    egui::Grid::new(("props", salt)).num_columns(3).spacing([8.0, 4.0]).show(ui, |ui| {
        for &n in nombres {
            let definida = capa.definidas().contains(&n);
            let lbl = RichText::new(nombre_prop(n));
            ui.label(if definida { lbl.strong().color(theme::ACCENT) } else { lbl.color(theme::TEXT2) });
            let mut v = valor.clone();
            let ch = ui
                .horizontal(|ui| match n {
                    "fondo" => {
                        let mut f = v.fondo.unwrap_or(Relleno::Ninguno);
                        let mut ch = false;
                        let tipo = match f {
                            Relleno::Ninguno => 0,
                            Relleno::Color(_) => 1,
                            Relleno::Degradado { .. } => 2,
                        };
                        let mut t = tipo;
                        egui::ComboBox::from_id_salt((salt, "fondo-tipo")).selected_text(["Ninguno", "Color", "Degradado"][t]).width(84.0).show_ui(ui, |ui| {
                            for (k, l) in ["Ninguno", "Color", "Degradado"].iter().enumerate() {
                                ui.selectable_value(&mut t, k, *l);
                            }
                        });
                        if t != tipo {
                            let c = f.color();
                            f = match t {
                                0 => Relleno::Ninguno,
                                1 => Relleno::Color(c),
                                _ => Relleno::Degradado { desde: c, hasta: [c[0] * 0.6, c[1] * 0.6, c[2] * 0.6, c[3]], angulo: 90.0 },
                            };
                            ch = true;
                        }
                        match &mut f {
                            Relleno::Ninguno => {}
                            Relleno::Color(c) => ch |= color(ui, c),
                            Relleno::Degradado { desde, hasta, angulo } => {
                                ch |= color(ui, desde);
                                ch |= color(ui, hasta);
                                ch |= ui.add(egui::DragValue::new(angulo).speed(1.0).suffix("°")).on_hover_text("Ángulo").changed();
                            }
                        }
                        v.fondo = Some(f);
                        ch
                    }
                    "imagen" => {
                        let mut s = v.imagen.clone().unwrap_or_default();
                        let ch = elegir(ui, (salt, "img"), raiz, &EXT_IMAGEN, "Ninguna", &mut s);
                        v.imagen = Some(s);
                        ch
                    }
                    "fuente" => {
                        let mut s = v.fuente.clone().unwrap_or_default();
                        let ch = elegir(ui, (salt, "fuente"), raiz, &EXT_FUENTE, "Del motor", &mut s);
                        v.fuente = Some(s);
                        ch
                    }
                    "nueve" => cuatro(ui, v.nueve.get_or_insert([0.0; 4]), 0.5, LTRB),
                    "relleno" => cuatro(ui, v.relleno.get_or_insert([0.0; 4]), 0.5, LTRB),
                    "radio" => cuatro(ui, v.radio.get_or_insert([0.0; 4]), 0.5, ESQ),
                    "tinte" => color(ui, v.tinte.get_or_insert([1.0; 4])),
                    "borde_color" => color(ui, v.borde_color.get_or_insert([0.0; 4])),
                    "color_texto" => color(ui, v.color_texto.get_or_insert([1.0; 4])),
                    "borde_ancho" => ui.add(egui::DragValue::new(v.borde_ancho.get_or_insert(0.0)).speed(0.1).range(0.0..=200.0)).changed(),
                    "tam_texto" => ui.add(egui::DragValue::new(v.tam_texto.get_or_insert(24.0)).speed(0.2).range(1.0..=512.0)).changed(),
                    "escala" => ui.add(egui::DragValue::new(v.escala.get_or_insert(1.0)).speed(0.005).range(0.0..=10.0)).changed(),
                    "opacidad" => ui.add(egui::Slider::new(v.opacidad.get_or_insert(1.0), 0.0..=1.0)).changed(),
                    "transicion" => ui.add(egui::DragValue::new(v.transicion.get_or_insert(0.0)).speed(0.005).range(0.0..=10.0).suffix(" s")).changed(),
                    "desplazamiento" => dos(ui, v.desplazamiento.get_or_insert([0.0; 2]), 0.2, ["X", "Y"]),
                    "sombra" => {
                        let s = v.sombra.get_or_insert(Sombra::default());
                        let mut ch = color(ui, &mut s.color);
                        ch |= dos(ui, &mut s.desplazamiento, 0.2, ["Desplazamiento X", "Desplazamiento Y"]);
                        ch |= ui.add(egui::DragValue::new(&mut s.difuminado).speed(0.2).range(0.0..=200.0)).on_hover_text("Difuminado").changed();
                        ch |= ui.add(egui::DragValue::new(&mut s.expansion).speed(0.2)).on_hover_text("Expansión").changed();
                        ch
                    }
                    "contorno_texto" => {
                        let s = v.contorno_texto.get_or_insert(EfectoTexto::default());
                        let mut ch = color(ui, &mut s.color);
                        ch |= ui.add(egui::DragValue::new(&mut s.ancho).speed(0.05).range(0.0..=20.0)).on_hover_text("Ancho").changed();
                        ch
                    }
                    "sombra_texto" => {
                        let s = v.sombra_texto.get_or_insert(EfectoTexto::default());
                        let mut ch = color(ui, &mut s.color);
                        ch |= dos(ui, &mut s.desplazamiento, 0.2, ["Desplazamiento X", "Desplazamiento Y"]);
                        ch
                    }
                    _ => false,
                })
                .inner;
            if ch {
                out = Some(Cambio::Poner(n, Box::new(v)));
            }
            if definida {
                if ui.small_button("↺").on_hover_text("Quitar este retoque").clicked() {
                    out = Some(Cambio::Quitar(n));
                }
            } else {
                ui.label("");
            }
            ui.end_row();
        }
    });
    out
}

fn touched(ed: &mut Editor, label: &str) {
    ed.pending_edit = Some(label.to_string());
    ed.dirty = true;
}

/// Secciones de interfaz de una entidad.
pub fn secciones(ed: &mut Editor, ui: &mut Ui, id: EntityId) {
    lienzo(ed, ui, id);
    rect(ed, ui, id);
    contenedor(ed, ui, id);
    widget(ed, ui, id);
    plantilla(ed, ui, id);
}

/// Entradas de «Añadir componente».
pub fn anadir(ed: &mut Editor, ui: &mut Ui, id: EntityId) -> Option<&'static str> {
    let s = &mut ed.scene;
    let mut added = None;
    if !s.has::<Lienzo>(id) && !s.has::<RectUi>(id) && ui.button("Lienzo").clicked() {
        s.insert(id, Lienzo::default());
        added = Some("Lienzo");
    }
    if !s.has::<RectUi>(id) && !s.has::<Lienzo>(id) && ui.button("Rect UI").clicked() {
        s.insert(id, RectUi::default());
        added = Some("Rect UI");
    }
    if (s.has::<RectUi>(id) || s.has::<Lienzo>(id)) && !s.has::<Contenedor>(id) && ui.button("Contenedor").clicked() {
        s.insert(id, Contenedor::default());
        added = Some("Contenedor");
    }
    if s.has::<RectUi>(id) && !s.has::<WidgetUi>(id) {
        ui.menu_button("Widget", |ui| {
            for t in TipoWidget::TODOS {
                if ui.button(t.nombre()).clicked() {
                    s.insert(id, WidgetUi::nuevo(t));
                    added = Some("Widget");
                }
            }
        });
    }
    added
}

fn lienzo(ed: &mut Editor, ui: &mut Ui, id: EntityId) {
    let Some(mut l) = ed.scene.get::<Lienzo>(id) else { return };
    let (open, remove) = component_header(ui, "Lienzo", "insp-lienzo", true);
    if remove {
        ed.scene.remove::<Lienzo>(id);
        ed.commit("Quitar lienzo");
        return;
    }
    if open {
        let antes = l.clone();
        let temas: Vec<String> = ed.ui.recursos.temas.keys().cloned().collect();
        egui::Grid::new("lienzo-grid").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
            ui.label("Escala");
            egui::ComboBox::from_id_salt("lienzo-modo")
                .selected_text(if l.modo == ModoEscala::PixelConstante { "Píxel constante" } else { "Con la pantalla" })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut l.modo, ModoEscala::EscalarConPantalla, "Con la pantalla");
                    ui.selectable_value(&mut l.modo, ModoEscala::PixelConstante, "Píxel constante");
                });
            ui.end_row();
            match l.modo {
                ModoEscala::EscalarConPantalla => {
                    ui.label("Resolución");
                    ui.horizontal(|ui| dos(ui, &mut l.resolucion, 1.0, ["Ancho", "Alto"]));
                    ui.end_row();
                    ui.label("Igualar");
                    ui.add(egui::Slider::new(&mut l.ajuste, 0.0..=1.0)).on_hover_text("0 = ancho, 1 = alto");
                    ui.end_row();
                }
                ModoEscala::PixelConstante => {
                    ui.label("Píxeles por unidad");
                    ui.add(egui::DragValue::new(&mut l.escala).speed(0.01).range(0.05..=16.0));
                    ui.end_row();
                }
            }
            ui.label("Orden");
            ui.add(egui::DragValue::new(&mut l.orden));
            ui.end_row();
            ui.label("Tema");
            egui::ComboBox::from_id_salt("lienzo-tema").selected_text(if l.tema.is_empty() { "Del proyecto" } else { l.tema.as_str() }).show_ui(ui, |ui| {
                ui.selectable_value(&mut l.tema, String::new(), "Del proyecto");
                for t in &temas {
                    ui.selectable_value(&mut l.tema, t.clone(), t);
                }
            });
            ui.end_row();
        });
        if l != antes {
            ed.scene.insert(id, l);
            touched(ed, "Editar lienzo");
        }
    }
    ui.separator();
}

/// Rejilla de preajustes de anclas (4×4 como la de Unity).
fn preajustes(ui: &mut Ui, r: &mut RectUi, padre: [f32; 2]) -> bool {
    const H: [(f32, f32, &str); 4] = [(0.0, 0.0, "Izquierda"), (0.5, 0.5, "Centro"), (1.0, 1.0, "Derecha"), (0.0, 1.0, "Estirar")];
    const V: [(f32, f32, &str); 4] = [(0.0, 0.0, "Arriba"), (0.5, 0.5, "Medio"), (1.0, 1.0, "Abajo"), (0.0, 1.0, "Estirar")];
    let mut cambio = false;
    let (alt, shift) = ui.input(|i| (i.modifiers.alt, i.modifiers.shift));
    egui::Grid::new("anclas-preajustes").spacing([2.0, 2.0]).show(ui, |ui| {
        for (vmin, vmax, vn) in V {
            for (hmin, hmax, hn) in H {
                let actual = r.ancla_min == [hmin, vmin] && r.ancla_max == [hmax, vmax];
                let (rect, resp) = ui.allocate_exact_size(egui::vec2(22.0, 22.0), egui::Sense::click());
                let p = ui.painter();
                p.rect(rect, 2.0, if actual { theme::ACCENT_SOFT } else if resp.hovered() { theme::ROW_HOVER } else { theme::CARD }, egui::Stroke::new(1.0, if actual { theme::ACCENT } else { theme::STROKE2 }), egui::StrokeKind::Inside);
                // El rectángulo hijo y sus anclas dentro del padre.
                let inner = rect.shrink(4.0);
                let x = |t: f32| inner.left() + inner.width() * t;
                let y = |t: f32| inner.top() + inner.height() * t;
                let col = if actual { theme::ACCENT } else { theme::TEXT3 };
                let hijo = egui::Rect::from_min_max(egui::pos2(x(hmin) - if hmin == hmax { 2.0 } else { 0.0 }, y(vmin) - if vmin == vmax { 2.0 } else { 0.0 }), egui::pos2(x(hmax) + if hmin == hmax { 2.0 } else { 0.0 }, y(vmax) + if vmin == vmax { 2.0 } else { 0.0 }));
                p.rect_filled(hijo, 0.0, col);
                if resp.on_hover_text(format!("{vn} · {hn}\nAlt: colocar en el ancla · Mayús: también el pivote")).clicked() {
                    r.cambiar_anclas([hmin, vmin], [hmax, vmax], padre);
                    if shift {
                        r.pivote = [if hmin == hmax { hmin } else { 0.5 }, if vmin == vmax { vmin } else { 0.5 }];
                        r.cambiar_anclas([hmin, vmin], [hmax, vmax], padre);
                    }
                    if alt {
                        let t = r.tam();
                        for k in 0..2 {
                            if r.fijo(k) {
                                // El pivote cae justo en el ancla.
                                r.offset_min[k] = -t[k] * r.pivote[k];
                                r.offset_max[k] = r.offset_min[k] + t[k];
                            } else {
                                r.offset_min[k] = 0.0;
                                r.offset_max[k] = 0.0;
                            }
                        }
                    }
                    cambio = true;
                }
            }
            ui.end_row();
        }
    });
    cambio
}

fn rect(ed: &mut Editor, ui: &mut Ui, id: EntityId) {
    let Some(mut r) = ed.scene.get::<RectUi>(id) else { return };
    let (open, remove) = component_header(ui, "Rect UI", "insp-rectui", true);
    if remove {
        ed.scene.remove::<RectUi>(id);
        ed.commit("Quitar Rect UI");
        return;
    }
    if !open {
        ui.separator();
        return;
    }
    let antes = r.clone();
    let padre = ed.ui.tam_padre(id).unwrap_or([1920.0, 1080.0]);
    let en_cont = ed.ui.en_contenedor(id);
    let padre_cont = ed.scene.parent(id).is_some_and(|p| ed.scene.has::<Contenedor>(p));
    ui.horizontal(|ui| {
        ui.menu_button(RichText::new("Anclas ▾"), |ui| {
            if preajustes(ui, &mut r, padre) {
                ui.close();
            }
        });
        ui.add_space(8.0);
        if en_cont {
            ui.label(RichText::new("En contenedor").color(theme::TEXT3)).on_hover_text("La posición y el tamaño los decide el contenedor padre");
        }
    });
    egui::Grid::new("rectui-grid").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
        ui.add_enabled_ui(!en_cont, |ui| {
            ui.label(if r.fijo(0) { "X · Ancho" } else { "Izq. · Der." });
        });
        ui.add_enabled_ui(!en_cont, |ui| {
            ui.horizontal(|ui| {
                if r.fijo(0) {
                    let mut p = r.posicion();
                    let mut t = r.tam();
                    if ui.add(egui::DragValue::new(&mut p[0]).speed(0.5).max_decimals(1)).changed() {
                        r.fijar_posicion(p);
                    }
                    if ui.add(egui::DragValue::new(&mut t[0]).speed(0.5).max_decimals(1).range(0.0..=100000.0)).changed() {
                        r.fijar_tam(t);
                    }
                } else {
                    ui.add(egui::DragValue::new(&mut r.offset_min[0]).speed(0.5).max_decimals(1));
                    let mut d = -r.offset_max[0];
                    if ui.add(egui::DragValue::new(&mut d).speed(0.5).max_decimals(1)).changed() {
                        r.offset_max[0] = -d;
                    }
                }
            });
        });
        ui.end_row();
        ui.add_enabled_ui(!en_cont, |ui| {
            ui.label(if r.fijo(1) { "Y · Alto" } else { "Arriba · Abajo" });
        });
        ui.add_enabled_ui(!en_cont, |ui| {
            ui.horizontal(|ui| {
                if r.fijo(1) {
                    let mut p = r.posicion();
                    let mut t = r.tam();
                    if ui.add(egui::DragValue::new(&mut p[1]).speed(0.5).max_decimals(1)).changed() {
                        r.fijar_posicion(p);
                    }
                    if ui.add(egui::DragValue::new(&mut t[1]).speed(0.5).max_decimals(1).range(0.0..=100000.0)).changed() {
                        r.fijar_tam(t);
                    }
                } else {
                    ui.add(egui::DragValue::new(&mut r.offset_min[1]).speed(0.5).max_decimals(1));
                    let mut d = -r.offset_max[1];
                    if ui.add(egui::DragValue::new(&mut d).speed(0.5).max_decimals(1)).changed() {
                        r.offset_max[1] = -d;
                    }
                }
            });
        });
        ui.end_row();
        ui.label("Ancla mín.");
        ui.horizontal(|ui| dos(ui, &mut r.ancla_min, 0.01, ["X", "Y"]));
        ui.end_row();
        ui.label("Ancla máx.");
        ui.horizontal(|ui| dos(ui, &mut r.ancla_max, 0.01, ["X", "Y"]));
        ui.end_row();
        ui.label("Pivote");
        ui.horizontal(|ui| dos(ui, &mut r.pivote, 0.01, ["X", "Y"]));
        ui.end_row();
        ui.label("Rotación");
        ui.add(egui::DragValue::new(&mut r.rotacion).speed(0.5).suffix("°"));
        ui.end_row();
        ui.label("Escala");
        ui.horizontal(|ui| dos(ui, &mut r.escala, 0.01, ["X", "Y"]));
        ui.end_row();
        ui.label("Visible");
        ui.checkbox(&mut r.visible, "");
        ui.end_row();
        ui.label("Interactuable");
        ui.checkbox(&mut r.interactuable, "");
        ui.end_row();
        ui.label("Opacidad");
        ui.add(egui::Slider::new(&mut r.opacidad, 0.0..=1.0));
        ui.end_row();
        ui.label("Recortar hijos");
        ui.checkbox(&mut r.recortar, "");
        ui.end_row();
        if padre_cont {
            ui.label("Ignorar contenedor");
            ui.checkbox(&mut r.ignorar_layout, "");
            ui.end_row();
            ui.label("Flex");
            ui.add(egui::DragValue::new(&mut r.flex).speed(0.05).range(0.0..=100.0)).on_hover_text("Parte del espacio libre si el contenedor expande a sus hijos");
            ui.end_row();
        }
    });
    if r != antes {
        ed.scene.insert(id, r);
        touched(ed, "Editar Rect UI");
    }
    ui.separator();
}

fn alinear(ui: &mut Ui, salt: &str, a: &mut Alinear, estirar: bool) {
    egui::ComboBox::from_id_salt(salt).selected_text(a.nombre()).width(90.0).show_ui(ui, |ui| {
        for x in Alinear::TODOS.into_iter().filter(|x| estirar || *x != Alinear::Estirar) {
            ui.selectable_value(a, x, x.nombre());
        }
    });
}

fn contenedor(ed: &mut Editor, ui: &mut Ui, id: EntityId) {
    let Some(mut c) = ed.scene.get::<Contenedor>(id) else { return };
    let (open, remove) = component_header(ui, "Contenedor", "insp-contenedor", true);
    if remove {
        ed.scene.remove::<Contenedor>(id);
        ed.commit("Quitar contenedor");
        return;
    }
    if open {
        let antes = c.clone();
        egui::Grid::new("cont-grid").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
            ui.label("Tipo");
            egui::ComboBox::from_id_salt("cont-tipo")
                .selected_text(match c.tipo {
                    TipoContenedor::Horizontal => "Horizontal",
                    TipoContenedor::Vertical => "Vertical",
                    TipoContenedor::Rejilla => "Rejilla",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut c.tipo, TipoContenedor::Horizontal, "Horizontal");
                    ui.selectable_value(&mut c.tipo, TipoContenedor::Vertical, "Vertical");
                    ui.selectable_value(&mut c.tipo, TipoContenedor::Rejilla, "Rejilla");
                });
            ui.end_row();
            ui.label("Relleno");
            ui.horizontal(|ui| cuatro(ui, &mut c.relleno, 0.5, LTRB));
            ui.end_row();
            ui.label("Espacio");
            ui.horizontal(|ui| dos(ui, &mut c.espacio, 0.5, ["X", "Y"]));
            ui.end_row();
            ui.label(if c.tipo == TipoContenedor::Rejilla { "Alinear X · Y" } else { "Alinear" });
            ui.horizontal(|ui| {
                alinear(ui, "cont-al", &mut c.alinear, false);
                alinear(ui, "cont-cruz", &mut c.cruzado, c.tipo != TipoContenedor::Rejilla);
            });
            ui.end_row();
            if c.tipo == TipoContenedor::Rejilla {
                ui.label("Columnas");
                ui.add(egui::DragValue::new(&mut c.columnas).range(1..=64));
                ui.end_row();
                ui.label("Celda");
                ui.horizontal(|ui| dos(ui, &mut c.celda, 0.5, ["Ancho", "Alto"]));
                ui.end_row();
            } else {
                ui.label("Expandir hijos");
                ui.checkbox(&mut c.expandir_hijos, "");
                ui.end_row();
            }
            ui.label("Ajustar al contenido");
            ui.checkbox(&mut c.ajustar, "");
            ui.end_row();
        });
        if c != antes {
            ed.scene.insert(id, c);
            touched(ed, "Editar contenedor");
        }
    }
    ui.separator();
}

fn widget(ed: &mut Editor, ui: &mut Ui, id: EntityId) {
    let Some(mut w) = ed.scene.get::<WidgetUi>(id) else { return };
    let tipo = w.clase.tipo();
    let (open, remove) = component_header(ui, tipo.nombre(), "insp-widget", true);
    if remove {
        ed.scene.remove::<WidgetUi>(id);
        ed.commit("Quitar widget");
        return;
    }
    if !open {
        ui.separator();
        return;
    }
    let antes = w.clone();
    let raiz = ed.project.root.clone();
    egui::Grid::new("widget-grid").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| match &mut w.clase {
        ClaseWidget::Panel | ClaseWidget::Personalizado => {}
        ClaseWidget::Imagen(im) => {
            ui.label("Imagen");
            ui.horizontal(|ui| elegir(ui, "w-img", &raiz, &EXT_IMAGEN, "Ninguna", &mut im.imagen));
            ui.end_row();
            ui.label("Tinte");
            color(ui, &mut im.tinte);
            ui.end_row();
            ui.label("Conservar aspecto");
            ui.checkbox(&mut im.conservar_aspecto, "");
            ui.end_row();
            ui.label("9-slice");
            ui.horizontal(|ui| cuatro(ui, &mut im.nueve, 0.5, LTRB));
            ui.end_row();
        }
        ClaseWidget::Texto(t) => {
            ui.label("Texto");
            ui.add(egui::TextEdit::multiline(&mut t.texto).desired_rows(2).desired_width(f32::INFINITY));
            ui.end_row();
            ui.label("Alineación");
            ui.horizontal(|ui| {
                for (a, s) in [(AlinH::Izquierda, "⇤"), (AlinH::Centro, "↔"), (AlinH::Derecha, "⇥")] {
                    ui.selectable_value(&mut t.alinear_h, a, s);
                }
                ui.separator();
                for (a, s) in [(AlinV::Arriba, "⤒"), (AlinV::Medio, "↕"), (AlinV::Abajo, "⤓")] {
                    ui.selectable_value(&mut t.alinear_v, a, s);
                }
            });
            ui.end_row();
            ui.label("Partir líneas");
            ui.checkbox(&mut t.ajuste_linea, "");
            ui.end_row();
            ui.label("Tamaño del texto");
            ui.checkbox(&mut t.ajustar_tam, "").on_hover_text("Dentro de un contenedor, ocupa lo que mide el texto");
            ui.end_row();
        }
        ClaseWidget::Boton(b) => {
            ui.label("Texto");
            ui.text_edit_singleline(&mut b.texto);
            ui.end_row();
        }
        ClaseWidget::Barra(b) => {
            ui.label("Valor");
            ui.add(egui::Slider::new(&mut b.valor, b.min.min(b.max)..=b.max.max(b.min)));
            ui.end_row();
            ui.label("Mín. · máx.");
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(&mut b.min).speed(0.05));
                ui.add(egui::DragValue::new(&mut b.max).speed(0.05));
            });
            ui.end_row();
            ui.label("Dirección");
            direccion(ui, "barra-dir", &mut b.direccion);
            ui.end_row();
        }
        ClaseWidget::Deslizador(d) => {
            ui.label("Valor");
            ui.add(egui::Slider::new(&mut d.valor, d.min.min(d.max)..=d.max.max(d.min)));
            ui.end_row();
            ui.label("Mín. · máx. · paso");
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(&mut d.min).speed(0.05));
                ui.add(egui::DragValue::new(&mut d.max).speed(0.05));
                ui.add(egui::DragValue::new(&mut d.paso).speed(0.01).range(0.0..=1e6));
            });
            ui.end_row();
            ui.label("Dirección");
            direccion(ui, "desl-dir", &mut d.direccion);
            ui.end_row();
        }
        ClaseWidget::Casilla(c) => {
            ui.label("Activa");
            ui.checkbox(&mut c.activa, "");
            ui.end_row();
            ui.label("Texto");
            ui.text_edit_singleline(&mut c.texto);
            ui.end_row();
        }
        ClaseWidget::CampoTexto(c) => {
            ui.label("Texto");
            ui.text_edit_singleline(&mut c.texto);
            ui.end_row();
            ui.label("Marcador");
            ui.text_edit_singleline(&mut c.marcador);
            ui.end_row();
            ui.label("Máx. caracteres");
            ui.add(egui::DragValue::new(&mut c.max).range(0..=100000));
            ui.end_row();
        }
    });
    estilo(ed, ui, id, &mut w);
    if w != antes {
        ed.scene.insert(id, w);
        touched(ed, "Editar widget");
    }
    ui.separator();
}

fn direccion(ui: &mut Ui, salt: &str, d: &mut Direccion) {
    egui::ComboBox::from_id_salt(salt).selected_text(d.nombre()).show_ui(ui, |ui| {
        for x in Direccion::TODAS {
            ui.selectable_value(d, x, x.nombre());
        }
    });
}

/// Tema que usa un elemento (el de su lienzo o el del proyecto).
fn tema_de(ed: &Editor, id: EntityId) -> String {
    let mut cur = Some(id);
    while let Some(c) = cur {
        if let Some(l) = ed.scene.get::<Lienzo>(c) {
            return l.tema;
        }
        cur = ed.scene.parent(c);
    }
    String::new()
}

fn estilo(ed: &mut Editor, ui: &mut Ui, id: EntityId, w: &mut WidgetUi) {
    let tipo = w.clase.tipo();
    let tema = tema_de(ed, id);
    ui.add_space(4.0);
    let nombres = ed.ui.recursos.nombres_estilos();
    let del_tema = ed.ui.recursos.estilo_de_tipo(&tema, tipo).map(str::to_owned);
    ui.horizontal(|ui| {
        ui.label("Estilo");
        let txt = if w.estilo.is_empty() { format!("Del tema{}", del_tema.as_ref().map(|s| format!(" ({s})")).unwrap_or_default()) } else { w.estilo.clone() };
        egui::ComboBox::from_id_salt("w-estilo").selected_text(txt).width(170.0).show_ui(ui, |ui| {
            ui.selectable_value(&mut w.estilo, String::new(), "Del tema");
            for n in &nombres {
                ui.selectable_value(&mut w.estilo, n.clone(), n);
            }
        });
        if !w.estilo.is_empty() && ui.small_button("Editar").clicked() {
            ed.estilos.abrir_estilo(&w.estilo);
            ed.bottom_tab = crate::editor::BottomTab::Estilos;
        }
    });
    let n_retoques: usize = std::iter::once(&w.local.estados).chain(w.local.partes.values()).map(|c| Estado::TODOS.iter().map(|e| c.de(*e).definidas().len()).sum::<usize>()).sum();
    let titulo = if n_retoques > 0 { format!("Retoques locales · {n_retoques}") } else { "Retoques locales".to_string() };
    egui::CollapsingHeader::new(titulo).id_salt("w-retoques").default_open(n_retoques > 0).show(ui, |ui| {
        let sel_id = ui.make_persistent_id("w-retoque-sel");
        let (mut estado, mut parte): (Estado, Option<String>) = ui.data(|d| d.get_temp(sel_id)).unwrap_or((Estado::Normal, None));
        ui.horizontal_wrapped(|ui| {
            for e in Estado::TODOS {
                let n = w.local.capa(parte.as_deref()).map(|c| c.de(e).definidas().len()).unwrap_or(0);
                let t = if n > 0 { format!("{} · {n}", e.nombre()) } else { e.nombre().to_string() };
                ui.selectable_value(&mut estado, e, t);
            }
        });
        if !tipo.partes().is_empty() {
            ui.horizontal_wrapped(|ui| {
                ui.selectable_value(&mut parte, None, "Principal");
                for p in tipo.partes() {
                    ui.selectable_value(&mut parte, Some(p.to_string()), *p);
                }
            });
        }
        ui.data_mut(|d| d.insert_temp(sel_id, (estado, parte.clone())));
        let valor = ed.ui.recursos.resolver(w, &tema, parte.as_deref(), estado);
        let capa = w.local.capa(parte.as_deref()).map(|c| c.de(estado).clone()).unwrap_or_default();
        let props = props_de(Some(tipo), parte.as_deref());
        match rejilla_props(ui, "local", &ed.project.root, &valor, &capa, &props) {
            Some(Cambio::Poner(n, v)) => w.local.capa_mut(parte.as_deref()).de_mut(estado).copiar(n, &v),
            Some(Cambio::Quitar(n)) => {
                w.local.capa_mut(parte.as_deref()).de_mut(estado).quitar(n);
                w.local.limpiar();
            }
            None => {}
        }
        if n_retoques > 0 && ui.small_button("Quitar todos los retoques").clicked() {
            w.local = Default::default();
        }
    });
}

fn plantilla(ed: &mut Editor, ui: &mut Ui, id: EntityId) {
    let Some(inst) = ed.scene.get::<InstanciaUi>(id) else { return };
    let (open, _) = component_header(ui, "Plantilla", "insp-plantilla", false);
    if open {
        ui.horizontal(|ui| {
            ui.label(RichText::new(&inst.plantilla).color(theme::TEXT2));
        });
        ui.horizontal(|ui| {
            if ui.button("Aplicar a la plantilla").on_hover_text("Guarda esta instancia como la plantilla y pone al día las demás").clicked() {
                ed.aplicar_a_plantilla(id);
            }
            if ui.add_enabled(!inst.overrides.is_empty(), egui::Button::new("Revertir todo")).clicked() {
                ed.revertir_plantilla(id, None);
            }
            if ui.button("Desvincular").clicked() {
                ed.scene.remove::<InstanciaUi>(id);
                ed.commit("Desvincular plantilla");
            }
        });
        if !inst.overrides.is_empty() {
            let mut revertir = None;
            egui::Grid::new("plantilla-retoques").num_columns(2).spacing([8.0, 2.0]).show(ui, |ui| {
                for o in &inst.overrides {
                    let (local, campo) = o.split_once('/').unwrap_or(("", o));
                    let quien = buscar_local(ed, id, local).map(|e| ed.scene.name(e)).unwrap_or_default();
                    ui.label(RichText::new(format!("{quien} · {campo}")).color(theme::ACCENT).small());
                    if ui.small_button("↺").on_hover_text("Volver al valor de la plantilla").clicked() {
                        revertir = Some(o.clone());
                    }
                    ui.end_row();
                }
            });
            if let Some(o) = revertir {
                ed.revertir_plantilla(id, Some(&o));
            }
        }
    }
    ui.separator();
}

fn buscar_local(ed: &Editor, raiz: EntityId, local: &str) -> Option<EntityId> {
    let local: u64 = local.parse().ok()?;
    let h = ed.scene.hierarchy();
    let mut pila = vec![raiz];
    while let Some(c) = pila.pop() {
        if ed.scene.get::<OrigenPlantilla>(c).is_some_and(|o| o.0 == local) {
            return Some(c);
        }
        pila.extend_from_slice(h.children_of(c));
    }
    None
}
