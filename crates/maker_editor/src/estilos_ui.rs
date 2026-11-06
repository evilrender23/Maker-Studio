//! Pestaña Estilos: hojas de estilos (`.mestilo`) y temas (`.mtema`) del
//! proyecto, con edición por estado y parte y una vista previa en vivo de los
//! cinco estados renderizada con el mismo pase de interfaz que el juego.

use crate::editor::Editor;
use crate::theme;
use crate::ui_inspector::{Cambio, props_de, rejilla_props};
use egui::{RichText, Ui};
use maker_engine::scene::Scene;
use maker_engine::ui::estilo::{Estado, Estilo, HojaEstilos, tema_inicial};
use maker_engine::ui::*;
use maker_render::{ObjetivoUi, UiRenderer};

#[derive(Clone, Debug, PartialEq)]
enum Sel {
    Estilo(String),
    Tema(String),
}

struct Vista {
    sistema: SistemaUi,
    escena: Scene,
    objetivo: ObjetivoUi,
    render: UiRenderer,
    tex: Option<egui::TextureId>,
    clave: String,
}

pub struct EstilosUi {
    sel: Option<Sel>,
    tipo: TipoWidget,
    estado: Estado,
    parte: Option<String>,
    nombre: String,
    vista: Option<Vista>,
}

impl Default for EstilosUi {
    fn default() -> Self {
        Self { sel: None, tipo: TipoWidget::Boton, estado: Estado::Normal, parte: None, nombre: String::new(), vista: None }
    }
}

impl EstilosUi {
    pub fn abrir_estilo(&mut self, n: &str) {
        self.sel = Some(Sel::Estilo(n.to_string()));
    }
    pub fn abrir_tema(&mut self, n: &str) {
        self.sel = Some(Sel::Tema(n.to_string()));
    }
    /// Abre el primer estilo de una hoja (doble clic en Contenido).
    pub fn abrir_hoja(&mut self, ed_ui: &RecursosUi, ruta: &str) {
        if let Some(n) = ed_ui.hojas.get(ruta).and_then(|h| h.estilos.keys().next()) {
            self.abrir_estilo(n);
        }
    }
}

fn nombre_valido(n: &str) -> Option<String> {
    let n = n.trim();
    (!n.is_empty() && !n.contains(['/', '\\', '.'])).then(|| n.to_string())
}

pub fn show(ed: &mut Editor, ui: &mut Ui, rs: &egui_wgpu::RenderState) {
    let jugando = ed.is_playing();
    egui::Panel::left("estilos-lista").resizable(true).default_size(230.0).min_size(170.0).frame(egui::Frame::new().inner_margin(egui::Margin { right: 6, ..Default::default() })).show(ui, |ui| {
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| lista(ed, ui, jugando));
    });
    match ed.estilos.sel.clone() {
        Some(Sel::Estilo(n)) if ed.ui.recursos.estilo(&n).is_some() => {
            egui::Panel::right("estilos-vista").resizable(true).default_size(420.0).min_size(260.0).show(ui, |ui| vista(ed, ui, rs, Some(&n), None));
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| ui.add_enabled_ui(!jugando, |ui| editar_estilo(ed, ui, &n)));
        }
        Some(Sel::Tema(t)) if ed.ui.recursos.temas.contains_key(&t) => {
            egui::Panel::right("estilos-vista").resizable(true).default_size(420.0).min_size(260.0).show(ui, |ui| vista(ed, ui, rs, None, Some(&t)));
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| ui.add_enabled_ui(!jugando, |ui| editar_tema(ed, ui, &t)));
        }
        _ => {
            ui.add_space(8.0);
            if ed.ui.recursos.hojas.is_empty() && ui.add_enabled(!jugando, egui::Button::new("Crear tema inicial")).on_hover_text("ui/estilos/base.mestilo y ui/temas/base.mtema, para copiar y editar").clicked() {
                crear_tema_inicial(ed);
            }
        }
    }
}

fn crear_tema_inicial(ed: &mut Editor) {
    let (hoja, tema) = tema_inicial();
    let r = &mut ed.ui.recursos;
    let ruta = format!("{DIR_UI}/estilos/base.{EXT_ESTILO}");
    if !r.hojas.contains_key(&ruta) {
        r.hojas.insert(ruta, hoja);
    }
    if !r.temas.contains_key("base") {
        r.rutas_temas.insert("base".into(), format!("{DIR_UI}/temas/base.{EXT_TEMA}"));
        r.temas.insert("base".into(), tema);
    }
    r.reindexar();
    fijar_tema_proyecto(ed, "base");
    ed.commit("Crear tema inicial");
    ed.estilos.sel = Some(Sel::Tema("base".into()));
}

fn fijar_tema_proyecto(ed: &mut Editor, t: &str) {
    ed.project.file.tema_ui = t.to_string();
    if let Err(e) = ed.project.save() {
        maker_engine::log::error("interfaz", e);
    }
    ed.ui.recursos.tema = t.to_string();
    ed.ui.recursos.reindexar();
}

fn lista(ed: &mut Editor, ui: &mut Ui, jugando: bool) {
    let temas: Vec<String> = ed.ui.recursos.temas.keys().cloned().collect();
    ui.horizontal(|ui| {
        ui.label("Tema del proyecto");
    });
    let mut t = ed.ui.recursos.tema.clone();
    egui::ComboBox::from_id_salt("tema-proyecto").selected_text(if t.is_empty() { "Ninguno" } else { t.as_str() }).width(ui.available_width() - 8.0).show_ui(ui, |ui| {
        ui.selectable_value(&mut t, String::new(), "Ninguno");
        for n in &temas {
            ui.selectable_value(&mut t, n.clone(), n);
        }
    });
    if t != ed.ui.recursos.tema && !jugando {
        fijar_tema_proyecto(ed, &t);
    }
    ui.add_space(6.0);
    ui.add_enabled_ui(!jugando, |ui| {
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut ed.estilos.nombre).hint_text("Nombre").desired_width(110.0));
            ui.menu_button("Nuevo", |ui| {
                let nombre = nombre_valido(&ed.estilos.nombre);
                if ui.add_enabled(nombre.is_some(), egui::Button::new("Hoja de estilos")).clicked() {
                    let n = nombre.clone().unwrap();
                    let ruta = format!("{DIR_UI}/estilos/{n}.{EXT_ESTILO}");
                    let mut h = HojaEstilos::default();
                    h.estilos.insert(n.clone(), Estilo::default());
                    ed.ui.recursos.hojas.entry(ruta).or_insert(h);
                    ed.ui.recursos.reindexar();
                    ed.commit("Nueva hoja de estilos");
                    ed.estilos.sel = Some(Sel::Estilo(n));
                    ed.estilos.nombre.clear();
                    ui.close();
                }
                if ui.add_enabled(nombre.is_some(), egui::Button::new("Tema")).clicked() {
                    let n = nombre.clone().unwrap();
                    ed.ui.recursos.rutas_temas.insert(n.clone(), format!("{DIR_UI}/temas/{n}.{EXT_TEMA}"));
                    ed.ui.recursos.temas.entry(n.clone()).or_default();
                    ed.ui.recursos.reindexar();
                    ed.commit("Nuevo tema");
                    ed.estilos.sel = Some(Sel::Tema(n));
                    ed.estilos.nombre.clear();
                    ui.close();
                }
                ui.separator();
                if ui.button("Tema inicial").clicked() {
                    crear_tema_inicial(ed);
                    ui.close();
                }
            });
        });
    });
    ui.add_space(4.0);
    let hojas: Vec<(String, Vec<String>)> = ed.ui.recursos.hojas.iter().map(|(r, h)| (r.clone(), h.estilos.keys().cloned().collect())).collect();
    let mut accion: Option<(String, String, u8)> = None;
    for (ruta, estilos) in hojas {
        let nombre = ruta.rsplit('/').next().unwrap_or(&ruta).to_string();
        egui::CollapsingHeader::new(RichText::new(nombre).strong()).id_salt(("hoja", &ruta)).default_open(true).show(ui, |ui| {
            for n in estilos {
                let sel = ed.estilos.sel == Some(Sel::Estilo(n.clone()));
                let r = ui.add(egui::Button::selectable(sel, &n).min_size(egui::vec2(ui.available_width(), 0.0)));
                if r.clicked() {
                    ed.estilos.sel = Some(Sel::Estilo(n.clone()));
                }
                r.context_menu(|ui| {
                    if ui.button("Duplicar").clicked() {
                        accion = Some((ruta.clone(), n.clone(), 0));
                        ui.close();
                    }
                    if nombre_valido(&ed.estilos.nombre).is_some() && ui.button(format!("Renombrar a «{}»", ed.estilos.nombre.trim())).clicked() {
                        accion = Some((ruta.clone(), n.clone(), 2));
                        ui.close();
                    }
                    if ui.button(RichText::new("Eliminar").color(theme::RED)).clicked() {
                        accion = Some((ruta.clone(), n.clone(), 1));
                        ui.close();
                    }
                });
            }
            if !jugando && let Some(n) = nombre_valido(&ed.estilos.nombre)
                && ui.small_button(format!("+ «{n}»")).on_hover_text("Nuevo estilo en esta hoja").clicked()
            {
                accion = Some((ruta.clone(), n, 3));
            }
        });
    }
    if let Some((ruta, n, op)) = accion.filter(|_| !jugando) {
        let r = &mut ed.ui.recursos;
        if let Some(h) = r.hojas.get_mut(&ruta) {
            match op {
                0 => {
                    let mut k = 2;
                    while h.estilos.contains_key(&format!("{n}_{k}")) {
                        k += 1;
                    }
                    let copia = h.estilos[&n].clone();
                    h.estilos.insert(format!("{n}_{k}"), copia);
                }
                1 => {
                    h.estilos.remove(&n);
                }
                2 => {
                    let nuevo = ed.estilos.nombre.trim().to_string();
                    if let Some(e) = h.estilos.remove(&n) {
                        h.estilos.insert(nuevo.clone(), e);
                    }
                    // Las referencias de la escena también.
                    let ids: Vec<_> = ed.scene.world.query::<(&maker_engine::components::EntityId, &WidgetUi)>().iter().filter(|(_, w)| w.estilo == n).map(|(i, _)| *i).collect();
                    for i in ids {
                        if let Some(mut w) = ed.scene.get_mut::<WidgetUi>(i) {
                            w.estilo = nuevo.clone();
                        }
                    }
                    ed.estilos.sel = Some(Sel::Estilo(nuevo));
                    ed.estilos.nombre.clear();
                }
                _ => {
                    h.estilos.entry(n.clone()).or_default();
                    ed.estilos.sel = Some(Sel::Estilo(n));
                    ed.estilos.nombre.clear();
                }
            }
            r.reindexar();
            ed.commit("Editar hoja de estilos");
        }
    }
    ui.add_space(6.0);
    egui::CollapsingHeader::new(RichText::new("Temas").strong()).id_salt("temas").default_open(true).show(ui, |ui| {
        for n in &temas {
            let sel = ed.estilos.sel == Some(Sel::Tema(n.clone()));
            let marca = if *n == ed.ui.recursos.tema { format!("{n} ●") } else { n.clone() };
            if ui.add(egui::Button::selectable(sel, marca).min_size(egui::vec2(ui.available_width(), 0.0))).clicked() {
                ed.estilos.sel = Some(Sel::Tema(n.clone()));
            }
        }
    });
}

fn editar_estilo(ed: &mut Editor, ui: &mut Ui, n: &str) {
    let Some(ruta) = ed.ui.recursos.hoja_de(n).map(str::to_owned) else { return };
    let mut e = ed.ui.recursos.estilo(n).cloned().unwrap_or_default();
    let antes = e.clone();
    let nombres = ed.ui.recursos.nombres_estilos();
    ui.horizontal(|ui| {
        ui.label(RichText::new(n).strong().size(15.0));
        ui.label(RichText::new(&ruta).color(theme::TEXT4).small());
    });
    egui::Grid::new("estilo-cab").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
        ui.label("Hereda de");
        egui::ComboBox::from_id_salt("estilo-hereda").selected_text(if e.hereda.is_empty() { "Nada" } else { e.hereda.as_str() }).show_ui(ui, |ui| {
            ui.selectable_value(&mut e.hereda, String::new(), "Nada");
            for o in nombres.iter().filter(|o| *o != n) {
                ui.selectable_value(&mut e.hereda, o.clone(), o);
            }
        });
        ui.end_row();
        ui.label("Widget");
        let es = &mut ed.estilos;
        egui::ComboBox::from_id_salt("estilo-tipo").selected_text(es.tipo.nombre()).show_ui(ui, |ui| {
            for t in TipoWidget::TODOS {
                if ui.selectable_value(&mut es.tipo, t, t.nombre()).clicked() {
                    es.parte = None;
                }
            }
        });
        ui.end_row();
    });
    let es = &mut ed.estilos;
    ui.horizontal_wrapped(|ui| {
        for st in Estado::TODOS {
            let k = e.capa(es.parte.as_deref()).map(|c| c.de(st).definidas().len()).unwrap_or(0);
            ui.selectable_value(&mut es.estado, st, if k > 0 { format!("{} · {k}", st.nombre()) } else { st.nombre().to_string() });
        }
    });
    if !es.tipo.partes().is_empty() {
        ui.horizontal_wrapped(|ui| {
            ui.selectable_value(&mut es.parte, None, "Principal");
            for p in es.tipo.partes() {
                ui.selectable_value(&mut es.parte, Some(p.to_string()), *p);
            }
        });
    }
    let (tipo, estado, parte) = (es.tipo, es.estado, es.parte.clone());
    // Lo que da el estilo (con lo heredado y los valores neutros del motor), sin tema.
    let w = WidgetUi { clase: tipo.nuevo(), estilo: n.to_string(), local: Estilo::default() };
    let valor = ed.ui.recursos.resolver(&w, "\u{0}", parte.as_deref(), estado);
    let capa = e.capa(parte.as_deref()).map(|c| c.de(estado).clone()).unwrap_or_default();
    match rejilla_props(ui, "estilo", &ed.project.root, &valor, &capa, &props_de(Some(tipo), parte.as_deref())) {
        Some(Cambio::Poner(p, v)) => e.capa_mut(parte.as_deref()).de_mut(estado).copiar(p, &v),
        Some(Cambio::Quitar(p)) => {
            e.capa_mut(parte.as_deref()).de_mut(estado).quitar(p);
            e.limpiar();
        }
        None => {}
    }
    if e != antes {
        if let Some(h) = ed.ui.recursos.hojas.get_mut(&ruta) {
            h.estilos.insert(n.to_string(), e);
        }
        ed.ui.recursos.reindexar();
        ed.pending_edit = Some(format!("Editar estilo {n}"));
    }
}

fn editar_tema(ed: &mut Editor, ui: &mut Ui, t: &str) {
    let Some(mut tema) = ed.ui.recursos.temas.get(t).cloned() else { return };
    let antes = tema.clone();
    let nombres = ed.ui.recursos.nombres_estilos();
    let temas: Vec<String> = ed.ui.recursos.temas.keys().filter(|x| *x != t).cloned().collect();
    ui.horizontal(|ui| {
        ui.label(RichText::new(t).strong().size(15.0));
        if ed.ui.recursos.tema != t && ui.small_button("Usar en el proyecto").clicked() {
            fijar_tema_proyecto(ed, t);
        }
    });
    egui::Grid::new("tema-grid").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
        ui.label("Hereda de");
        egui::ComboBox::from_id_salt("tema-hereda").selected_text(if tema.hereda.is_empty() { "Nada" } else { tema.hereda.as_str() }).show_ui(ui, |ui| {
            ui.selectable_value(&mut tema.hereda, String::new(), "Nada");
            for o in &temas {
                ui.selectable_value(&mut tema.hereda, o.clone(), o);
            }
        });
        ui.end_row();
        for tipo in TipoWidget::TODOS {
            let k = tipo.clave().to_string();
            let mut v = tema.tipos.get(&k).cloned().unwrap_or_default();
            let heredado = if v.is_empty() { ed.ui.recursos.estilo_de_tipo(&tema.hereda, tipo).map(|s| format!("({s})")) } else { None };
            if ui.selectable_label(ed.estilos.tipo == tipo, tipo.nombre()).clicked() {
                ed.estilos.tipo = tipo;
            }
            egui::ComboBox::from_id_salt(("tema-tipo", tipo)).selected_text(if v.is_empty() { heredado.unwrap_or_else(|| "Motor".into()) } else { v.clone() }).width(170.0).show_ui(ui, |ui| {
                ui.selectable_value(&mut v, String::new(), "Sin estilo");
                for n in &nombres {
                    ui.selectable_value(&mut v, n.clone(), n);
                }
            });
            if v.is_empty() {
                tema.tipos.remove(&k);
            } else {
                tema.tipos.insert(k, v.clone());
            }
            if !v.is_empty() && ui.small_button("Editar").clicked() {
                ed.estilos.tipo = tipo;
                ed.estilos.sel = Some(Sel::Estilo(v));
            }
            ui.end_row();
        }
    });
    if tema != antes {
        ed.ui.recursos.temas.insert(t.to_string(), tema);
        ed.ui.recursos.reindexar();
        ed.commit(&format!("Editar tema {t}"));
    }
}

/// Vista previa: el widget elegido en los cinco estados, con el pase real.
fn vista(ed: &mut Editor, ui: &mut Ui, rs: &egui_wgpu::RenderState, estilo: Option<&str>, tema: Option<&str>) {
    let tipo = ed.estilos.tipo;
    let ppp = ui.ctx().pixels_per_point();
    let ancho = ui.available_width().max(100.0);
    let alto = (ancho / 5.0 * 0.8).clamp(80.0, 220.0);
    let (pw, ph) = ((ancho * ppp).round() as u32, (alto * ppp).round() as u32);
    let clave = format!("{tipo:?}|{estilo:?}|{tema:?}|{pw}x{ph}");
    let v = ed.estilos.vista.get_or_insert_with(|| Vista {
        sistema: SistemaUi::new(RecursosUi::default(), Some(&ed.project.root)),
        escena: Scene::new(),
        objetivo: ObjetivoUi::new(&rs.device, pw, ph),
        render: UiRenderer::new(&rs.device, &rs.queue, maker_render::COLOR_FORMAT),
        tex: None,
        clave: String::new(),
    });
    if v.sistema.recursos.generacion != ed.ui.recursos.generacion {
        v.sistema.recursos = ed.ui.recursos.clone();
    }
    if v.clave != clave {
        v.clave = clave;
        let mut s = Scene::new();
        let l = s.spawn("Lienzo");
        s.insert(l, Lienzo { modo: ModoEscala::PixelConstante, escala: ppp, tema: tema.unwrap_or("\u{0}").to_string(), ..Default::default() });
        v.sistema.forzar.clear();
        let celda = ancho / 5.0;
        let [tw, th] = tipo.tam_inicial();
        let k = (celda - 12.0).min(tw) / tw;
        let k = k.min((alto - 12.0) / th).max(0.05);
        for (i, e) in Estado::TODOS.into_iter().enumerate() {
            let id = s.spawn(e.nombre());
            s.insert(id, RectUi { ancla_min: [(i as f32 + 0.5) / 5.0, 0.5], ancla_max: [(i as f32 + 0.5) / 5.0, 0.5], ..RectUi::centrado(tw * k, th * k) });
            let mut w = WidgetUi::nuevo(tipo);
            if let ClaseWidget::Texto(t) = &mut w.clase {
                t.texto = "Aa".into();
            }
            if let ClaseWidget::Casilla(c) = &mut w.clase {
                c.activa = true;
            }
            if let Some(n) = estilo {
                w.estilo = n.to_string();
            }
            s.insert(id, w);
            s.set_parent(id, Some(l), false);
            v.sistema.forzar.insert(id, e);
        }
        if v.objetivo.tam != (pw, ph) {
            v.objetivo = ObjetivoUi::new(&rs.device, pw, ph);
            if let Some(id) = v.tex {
                rs.renderer.write().update_egui_texture_from_wgpu_texture(&rs.device, &v.objetivo.vista, wgpu::FilterMode::Linear, id);
            }
        }
        v.escena = s;
        v.sistema.marcar_sucio();
    }
    v.sistema.actualizar(&v.escena, [pw as f32, ph as f32], ui.input(|i| i.stable_dt).min(0.1));
    // Fondo gris medio: se ven igual estilos claros y oscuros.
    v.render.dibujar(&mut v.sistema, &v.objetivo.vista, v.objetivo.tam, Some(wgpu::Color { r: 0.18, g: 0.18, b: 0.2, a: 1.0 }));
    let id = *v.tex.get_or_insert_with(|| rs.renderer.write().register_native_texture(&rs.device, &v.objetivo.vista, wgpu::FilterMode::Linear));
    let (r, _) = ui.allocate_exact_size(egui::vec2(ancho, alto), egui::Sense::hover());
    ui.painter().image(id, r, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
    ui.horizontal(|ui| {
        let w = ancho / 5.0 - ui.spacing().item_spacing.x;
        for e in Estado::TODOS {
            ui.add_sized([w, 16.0], egui::Label::new(RichText::new(e.nombre()).small().color(theme::TEXT3)));
        }
    });
    ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
}

