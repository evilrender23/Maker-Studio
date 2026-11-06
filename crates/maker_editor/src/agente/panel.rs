//! Panel lateral del modo agente.

use super::cli::Backend;
use super::contexto::Chip;
use super::eventos::TipoHerramienta;
use super::sesion::Mensaje;
use super::{Agente, Clase};
use crate::editor::Editor;
use crate::prefs::{self, Preferencias};
use crate::theme;
use egui::{Color32, CornerRadius, Margin, RichText, Stroke, Ui};

fn miles(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1} M", n as f64 / 1e6)
    } else if n >= 10_000 {
        format!("{} k", n / 1000)
    } else if n >= 1000 {
        format!("{:.1} k", n as f64 / 1e3)
    } else {
        n.to_string()
    }
}

pub fn mostrar(ag: &mut Agente, ed: &mut Editor, prefs: &Preferencias, ui: &mut Ui) {
    let ocupado = ag.ocupado();
    cabecera(ag, prefs, ui, ocupado);
    ui.add_space(2.0);
    selectores(ag, prefs, ui, ocupado);
    ui.add_space(4.0);
    medidor(ag, ed, prefs, ui, ocupado);
    ui.add_space(4.0);

    egui::Panel::bottom("agente-entrada").frame(egui::Frame::new().inner_margin(Margin { top: 6, ..Default::default() })).show_separator_line(false).show(ui, |ui| entrada(ag, ed, prefs, ui));
    egui::CentralPanel::no_frame().show(ui, |ui| conversacion(ag, ui));
}

fn cabecera(ag: &mut Agente, prefs: &Preferencias, ui: &mut Ui, ocupado: bool) {
    ui.horizontal(|ui| {
        if let Some(nombre) = &mut ag.renombrando {
            let r = ui.add(egui::TextEdit::singleline(nombre).desired_width(ui.available_width() - 60.0));
            r.request_focus();
            let fin = ui.input(|i| i.key_pressed(egui::Key::Enter));
            if fin || ui.small_button("✔").clicked() {
                let t = nombre.trim().to_string();
                if !t.is_empty() {
                    ag.sesion_mut().titulo = t;
                    ag.guardar();
                }
                ag.renombrando = None;
            } else if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                ag.renombrando = None;
            }
            return;
        }
        let ancho = (ui.available_width() - 110.0).max(120.0);
        let titulo = ag.sesion().titulo.clone();
        let mut elegir = None;
        ui.add_enabled_ui(!ocupado, |ui| {
            egui::ComboBox::from_id_salt("agente-sesion").selected_text(RichText::new(titulo).strong()).width(ancho).truncate().show_ui(ui, |ui| {
                for (i, s) in ag.sesiones.iter().enumerate() {
                    let r = ui.selectable_label(i == ag.actual, format!("{}  ·  {}", s.titulo, s.backend.nombre()));
                    if r.clicked() {
                        elegir = Some(i);
                    }
                }
            });
        });
        if let Some(i) = elegir {
            ag.actual = i;
        }
        if ui.add_enabled(!ocupado, egui::Button::new("+").small()).on_hover_text("Sesión nueva").clicked() {
            ag.nueva_sesion(prefs);
        }
        if ui.add_enabled(!ocupado, egui::Button::new("✎").small()).on_hover_text("Renombrar").clicked() {
            ag.renombrando = Some(ag.sesion().titulo.clone());
        }
        if ui.add_enabled(!ocupado, egui::Button::new("✕").small()).on_hover_text("Borrar sesión").clicked() {
            ag.borrar_sesion(prefs);
        }
    });
}

fn selectores(ag: &mut Agente, prefs: &Preferencias, ui: &mut Ui, ocupado: bool) {
    let ancho = ((ui.available_width() - 2.0 * 62.0 - 2.0 * 30.0 - 20.0) / 2.0).clamp(90.0, 240.0);
    let catalogo = ag.catalogo.clone();
    let mut cambio = false;
    ui.add_enabled_ui(!ocupado, |ui| {
        egui::Grid::new("agente-selectores").num_columns(4).spacing([6.0, 4.0]).show(ui, |ui| {
            let fijo = ag.sesion().id_nativo.is_some();
            let b = ag.sesion().backend;
            ui.label(RichText::new("CLI").color(theme::TEXT3));
            let mut nuevo = None;
            ui.add_enabled_ui(!fijo, |ui| {
                egui::ComboBox::from_id_salt("agente-cli")
                    .selected_text(b.nombre())
                    .width(ancho)
                    .show_ui(ui, |ui| {
                        for x in Backend::TODOS {
                            let info = catalogo.info(x);
                            let ok = info.ruta.is_some();
                            let r = ui.add_enabled(ok, egui::Button::selectable(b == x, x.nombre()));
                            let r = if ok { r.on_hover_text(info.version.unwrap_or_default()) } else { r.on_disabled_hover_text(if info.comprobado { "No encontrada" } else { "Buscando…" }) };
                            if r.clicked() {
                                nuevo = Some(x);
                            }
                        }
                    })
                    .response
                    .on_disabled_hover_text("Crea una sesión nueva para cambiar de CLI");
            });
            if let Some(x) = nuevo {
                ag.cambiar_backend(x, prefs);
                cambio = true;
            }
            ui.label(RichText::new("Modelo").color(theme::TEXT3));
            ui.vertical(|ui| {
                let s = ag.sesion_mut();
                cambio |= prefs::selector_modelo(ui, "agente", s.backend, &mut s.modelo, &catalogo, ancho);
            });
            ui.end_row();
            ui.label(RichText::new("Esfuerzo").color(theme::TEXT3));
            let s = ag.sesion_mut();
            let modelo = s.modelo.clone();
            cambio |= prefs::selector_esfuerzo(ui, "agente", s.backend, &modelo, &mut s.esfuerzo, &catalogo, ancho);
            ui.label(RichText::new("Permisos").color(theme::TEXT3));
            cambio |= prefs::selector_permisos(ui, "agente", s.backend, &mut s.permisos, ancho);
            ui.end_row();
        });
    });
    if cambio {
        ag.guardar();
    }
}

fn medidor(ag: &mut Agente, ed: &Editor, prefs: &Preferencias, ui: &mut Ui, ocupado: bool) {
    let usado = ag.sesion().uso.contexto;
    let ventana = ag.ventana().filter(|v| *v > 0);
    let f = ventana.map(|v| (usado as f32 / v as f32).clamp(0.0, 1.0)).unwrap_or(0.0);
    let umbral = prefs.auto_compactar as f32 / 100.0;
    let color = if umbral > 0.0 && f >= umbral { theme::ORANGE } else if f > 0.9 { theme::RED } else { theme::ACCENT };
    ui.horizontal(|ui| {
        let puede = !ocupado && ag.sesion().id_nativo.is_some();
        let ancho = (ui.available_width() - 90.0).max(60.0);
        let (rect, r) = ui.allocate_exact_size(egui::vec2(ancho, 18.0), egui::Sense::hover());
        let p = ui.painter();
        p.rect(rect, CornerRadius::same(3), theme::CARD, Stroke::new(1.0, theme::STROKE3), egui::StrokeKind::Inside);
        if f > 0.0 {
            let mut lleno = rect.shrink(2.0);
            lleno.set_width((lleno.width() * f).max(2.0));
            p.rect_filled(lleno, CornerRadius::same(2), color.gamma_multiply(0.35));
        }
        let texto = match ventana {
            Some(v) => format!("{} / {} tokens · {:.0} %", miles(usado), miles(v), f * 100.0),
            None => format!("{} tokens", miles(usado)),
        };
        p.text(rect.center(), egui::Align2::CENTER_CENTER, texto, egui::FontId::proportional(11.0), theme::TEXT2);
        r.on_hover_text(if umbral > 0.0 { format!("Se compacta solo al {:.0} %", umbral * 100.0) } else { "Compactación automática desactivada".into() });
        let boton = ui.add_enabled(puede, egui::Button::new("Compactar").small());
        if boton.on_hover_text(if ag.sesion().backend == Backend::Claude { "/compact de Claude Code" } else { "Resumen y conversación nueva" }).clicked() {
            ag.compactar(ed, prefs);
        }
    });
}

fn entrada(ag: &mut Agente, ed: &mut Editor, prefs: &Preferencias, ui: &mut Ui) {
    let ocupado = ag.ocupado();
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for c in Chip::TODOS {
            let disp = c.disponible(ed);
            let activo = ag.chips.contains(&c);
            let (fondo, borde, texto) = if activo && disp.is_ok() { (theme::ACCENT_SOFT, theme::STROKE1, theme::ACCENT) } else { (theme::CARD, theme::STROKE3, theme::TEXT3) };
            let b = egui::Button::new(RichText::new(c.nombre()).small().color(texto)).fill(fondo).stroke(Stroke::new(1.0, borde)).corner_radius(CornerRadius::same(9));
            let r = ui.add_enabled(disp.is_ok(), b);
            let r = match disp {
                Ok(()) => r.on_hover_text(c.ayuda(ed)),
                Err(e) => r.on_disabled_hover_text(e),
            };
            if r.clicked() {
                if activo {
                    ag.chips.retain(|x| *x != c);
                } else {
                    ag.chips.push(c);
                }
            }
        }
    });
    adjuntos_ui(ag, ed, ui, ocupado);
    ui.add_space(2.0);
    let shift_enter = egui::KeyboardShortcut::new(egui::Modifiers::SHIFT, egui::Key::Enter);
    let enviar_tecla = ui.input(|i| i.key_pressed(egui::Key::Enter) && !i.modifiers.shift);
    let pista = format!("Mensaje para {}", ag.sesion().backend.nombre());
    let r = ui.add_enabled(
        !ocupado,
        egui::TextEdit::multiline(&mut ag.entrada).desired_rows(3).desired_width(f32::INFINITY).return_key(shift_enter).hint_text(pista),
    );
    let mut enviar = r.has_focus() && enviar_tecla;
    // Soltar en el cuadro de texto un modelo de Contenido o de la Biblioteca.
    if let Some(a) = r.dnd_release_payload::<crate::viewport::AssetDrag>() {
        let p = maker_engine::biblioteca::resolve(&a.0, Some(&ed.project.root));
        if p.exists() && !ag.adjuntos.contains(&p) {
            ag.adjuntos.push(p);
        }
    }
    if r.dnd_hover_payload::<crate::viewport::AssetDrag>().is_some() {
        ui.painter().rect_stroke(r.rect, 3.0, Stroke::new(2.0, theme::ACCENT), egui::StrokeKind::Inside);
    }
    ui.horizontal(|ui| {
        if let Some(t) = &ag.turno {
            ui.spinner();
            let s = t.ej.inicio.elapsed().as_secs();
            let que = match t.clase {
                Clase::Normal => "Trabajando",
                _ => "Compactando",
            };
            ui.label(RichText::new(format!("{que} · {}:{:02}", s / 60, s % 60)).color(theme::TEXT2));
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
        } else {
            let u = &ag.sesion().uso;
            let mut partes = Vec::new();
            if u.entrada > 0 || u.salida > 0 {
                partes.push(format!("↑ {} · ↓ {}", miles(u.entrada), miles(u.salida)));
            }
            if u.coste > 0.0 {
                partes.push(format!("{:.4} $", u.coste));
            }
            if !partes.is_empty() {
                ui.label(RichText::new(partes.join(" · ")).small().color(theme::TEXT3)).on_hover_text("Tokens de entrada y salida y coste acumulados de la sesión");
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ocupado {
                if ui.add(egui::Button::new(RichText::new("■  Detener").color(Color32::WHITE)).fill(theme::RED)).clicked() {
                    ag.parar();
                }
            } else if ui.add_enabled(!ag.entrada.trim().is_empty(), egui::Button::new(RichText::new("Enviar").color(Color32::WHITE)).fill(theme::ACCENT)).on_hover_text("Intro · Mayús+Intro para nueva línea").clicked() {
                enviar = true;
            }
        });
    });
    if enviar && !ocupado {
        let texto = std::mem::take(&mut ag.entrada);
        let texto = texto.trim_end_matches('\n').to_string();
        if texto.trim().is_empty() {
            return;
        }
        ag.enviar(ed, prefs, &texto);
    }
}

fn conversacion(ag: &mut Agente, ui: &mut Ui) {
    egui::ScrollArea::vertical().auto_shrink([false, false]).stick_to_bottom(true).show(ui, |ui| {
        ui.set_width(ui.available_width());
        let ocupado = ag.ocupado();
        let raiz = format!("{}{}", ag.raiz.display(), std::path::MAIN_SEPARATOR);
        for (i, m) in ag.sesion().mensajes.iter().enumerate() {
            match m {
                Mensaje::Usuario { texto, contexto } => burbuja(ui, texto, contexto),
                Mensaje::Asistente { texto } => {
                    ui.add_space(2.0);
                    markdown(ui, texto);
                    ui.add_space(2.0);
                }
                Mensaje::Herramienta { id, clase, titulo, salida, error, terminada } => {
                    let titulo = titulo.replace(&raiz, "");
                    herramienta(ui, i, id, *clase, &titulo, salida, *error, *terminada || !ocupado)
                }
                Mensaje::Error { texto } => {
                    egui::Frame::new().fill(Color32::from_rgb(0xfb, 0xea, 0xee)).stroke(Stroke::new(1.0, Color32::from_rgb(0xef, 0xb8, 0xc6))).corner_radius(CornerRadius::same(4)).inner_margin(Margin::symmetric(8, 5)).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(RichText::new(texto).color(theme::RED));
                    });
                }
                Mensaje::Nota { texto } => {
                    ui.label(RichText::new(texto).small().color(theme::TEXT3));
                }
            }
        }
    });
}

fn burbuja(ui: &mut Ui, texto: &str, contexto: &[String]) {
    ui.add_space(4.0);
    ui.with_layout(egui::Layout::top_down(egui::Align::Max), |ui| {
        let max = ui.available_width() * 0.88;
        egui::Frame::new().fill(theme::ACCENT_SOFT).stroke(Stroke::new(1.0, theme::STROKE3)).corner_radius(CornerRadius::same(8)).inner_margin(Margin::symmetric(10, 6)).show(ui, |ui| {
            ui.set_max_width(max);
            ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.add(egui::Label::new(RichText::new(texto).color(theme::TEXT1)).wrap());
                if !contexto.is_empty() {
                    ui.label(RichText::new(format!("+ {}", contexto.join(", "))).small().color(theme::TEXT3));
                }
            });
        });
    });
    ui.add_space(4.0);
}

#[allow(clippy::too_many_arguments)]
fn herramienta(ui: &mut Ui, i: usize, id: &str, clase: TipoHerramienta, titulo: &str, salida: &str, error: bool, terminada: bool) {
    let (icono, nombre) = match clase {
        TipoHerramienta::Comando => ("$", "Comando"),
        TipoHerramienta::Lectura => ("◔", "Leído"),
        TipoHerramienta::Edicion => ("✎", "Editado"),
        TipoHerramienta::Busqueda => ("⌕", "Búsqueda"),
        TipoHerramienta::Otra => ("◆", "Herramienta"),
    };
    let color = if error { theme::RED } else { theme::TEXT2 };
    let cab = egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), ui.make_persistent_id(("herr", i, id)), false);
    egui::Frame::new().fill(theme::CARD).stroke(Stroke::new(1.0, theme::STROKE4)).corner_radius(CornerRadius::same(4)).inner_margin(Margin::symmetric(6, 2)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        cab.show_header(ui, |ui| {
            if !terminada {
                ui.add(egui::Spinner::new().size(11.0));
            } else {
                ui.label(RichText::new(icono).monospace().color(if error { theme::RED } else { theme::ACCENT }));
            }
            ui.label(RichText::new(nombre).small().strong().color(color));
            let corto = titulo.lines().next().unwrap_or("");
            ui.add(egui::Label::new(RichText::new(corto).monospace().size(11.5).color(color)).truncate()).on_hover_text(titulo);
        })
        .body(|ui| {
            let t = if salida.trim().is_empty() { "(sin salida)" } else { salida };
            egui::ScrollArea::vertical().max_height(220.0).id_salt(("salida", i)).show(ui, |ui| {
                ui.add(egui::Label::new(RichText::new(t).monospace().size(11.5).color(if error { theme::RED } else { theme::TEXT2 })).wrap());
            });
        });
    });
}

/// Markdown mínimo: bloques ``` en monoespaciada, títulos en negrita, `código` en línea.
fn markdown(ui: &mut Ui, texto: &str) {
    let mut en_codigo = false;
    let mut bloque = String::new();
    let mut parrafo = String::new();
    let volcar = |ui: &mut Ui, p: &mut String| {
        if !p.trim().is_empty() {
            linea_rica(ui, p.trim_end());
        }
        p.clear();
    };
    for l in texto.lines() {
        if l.trim_start().starts_with("```") {
            if en_codigo {
                codigo(ui, &bloque);
                bloque.clear();
            } else {
                volcar(ui, &mut parrafo);
            }
            en_codigo = !en_codigo;
            continue;
        }
        if en_codigo {
            bloque.push_str(l);
            bloque.push('\n');
        } else if let Some(t) = l.strip_prefix("### ").or_else(|| l.strip_prefix("## ")).or_else(|| l.strip_prefix("# ")) {
            volcar(ui, &mut parrafo);
            ui.label(RichText::new(t).strong().color(theme::TEXT1));
        } else if l.trim().is_empty() {
            volcar(ui, &mut parrafo);
        } else {
            parrafo.push_str(l);
            parrafo.push('\n');
        }
    }
    if en_codigo {
        codigo(ui, &bloque);
    }
    volcar(ui, &mut parrafo);
}

fn codigo(ui: &mut Ui, t: &str) {
    egui::Frame::new().fill(Color32::from_rgb(0xf0, 0xf2, 0xf6)).corner_radius(CornerRadius::same(4)).inner_margin(Margin::symmetric(8, 6)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.add(egui::Label::new(RichText::new(t.trim_end()).monospace().size(11.5).color(theme::TEXT1)).wrap());
    });
}

/// Párrafo con `código`, **negrita** y viñetas.
fn linea_rica(ui: &mut Ui, t: &str) {
    use egui::text::{LayoutJob, TextFormat};
    let mut job = LayoutJob::default();
    let body = egui::FontId::proportional(13.0);
    let mono = egui::FontId::monospace(12.0);
    let normal = TextFormat { font_id: body.clone(), color: theme::TEXT1, ..Default::default() };
    for (k, linea) in t.lines().enumerate() {
        if k > 0 {
            job.append("\n", 0.0, normal.clone());
        }
        let linea = match linea.trim_start().strip_prefix("- ").or_else(|| linea.trim_start().strip_prefix("* ")) {
            Some(r) => {
                job.append("•  ", 0.0, normal.clone());
                r
            }
            None => linea,
        };
        let mut codigo = false;
        for (j, trozo) in linea.split('`').enumerate() {
            if j > 0 {
                codigo = !codigo;
            }
            if codigo {
                job.append(trozo, 0.0, TextFormat { font_id: mono.clone(), color: theme::TEXT1, background: Color32::from_rgb(0xec, 0xef, 0xf5), ..Default::default() });
            } else {
                for (b, s) in trozo.split("**").enumerate() {
                    let negrita = b % 2 == 1;
                    job.append(s, 0.0, TextFormat { color: if negrita { Color32::BLACK } else { theme::TEXT1 }, ..normal.clone() });
                }
            }
        }
    }
    job.wrap.max_width = ui.available_width();
    ui.add(egui::Label::new(job).wrap());
}

/// Adjuntos del próximo mensaje: los añadidos, «Adjuntar», archivos soltados sobre el panel e
/// imágenes pegadas con Ctrl+V.
fn adjuntos_ui(ag: &mut Agente, ed: &Editor, ui: &mut Ui, ocupado: bool) {
    let raiz = ed.project.root.clone();
    // Archivos soltados desde el sistema o imágenes pegadas con el puntero sobre el panel.
    let encima = ui.ctx().pointer_hover_pos().is_some_and(|p| ui.max_rect().expand(400.0).contains(p) && ui.clip_rect().x_range().contains(p.x));
    if encima && !ocupado {
        let (soltados, pegadas) = ui.input(|i| {
            let s: Vec<std::path::PathBuf> = i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).filter(|p| !p.as_os_str().is_empty()).collect();
            let p: Vec<egui::ColorImage> = i.events.iter().filter_map(|e| if let egui::Event::PasteImage(img) = e { Some((**img).clone()) } else { None }).collect();
            (s, p)
        });
        for p in soltados {
            if !ag.adjuntos.contains(&p) {
                ag.adjuntos.push(p);
            }
        }
        for img in pegadas {
            match super::adjuntos::guardar_imagen(&img) {
                Ok(p) => ag.adjuntos.push(p),
                Err(e) => maker_engine::log::error("agente", format!("no se pudo guardar la imagen pegada: {e}")),
            }
        }
    }
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let mut quitar = None;
        for (i, a) in ag.adjuntos.iter().enumerate() {
            let texto = format!("{}  ✕", super::adjuntos::nombre(a));
            let b = egui::Button::new(RichText::new(texto).small().color(theme::TEXT1)).fill(theme::CARD).stroke(Stroke::new(1.0, theme::STROKE2)).corner_radius(CornerRadius::same(9));
            if ui.add_enabled(!ocupado, b).on_hover_text(a.display().to_string()).clicked() {
                quitar = Some(i);
            }
        }
        if let Some(i) = quitar {
            ag.adjuntos.remove(i);
        }
        ui.add_enabled_ui(!ocupado, |ui| {
            let id = ui.make_persistent_id("agente-adjuntar");
            ui.menu_button(RichText::new("+ Adjuntar").small(), |ui| {
                ui.set_min_width(320.0);
                let (mut ruta, mut filtro): (String, String) = ui.data(|d| d.get_temp(id)).unwrap_or_default();
                ui.horizontal(|ui| {
                    let r = ui.add(egui::TextEdit::singleline(&mut ruta).hint_text("Ruta de un archivo o carpeta").desired_width(250.0));
                    let ok = super::adjuntos::normalizar(&ruta, &raiz);
                    let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if (ui.add_enabled(ok.is_some(), egui::Button::new("Añadir")).clicked() || enter)
                        && let Some(p) = ok
                    {
                        if !ag.adjuntos.contains(&p) {
                            ag.adjuntos.push(p);
                        }
                        ruta.clear();
                    }
                });
                ui.separator();
                ui.add(egui::TextEdit::singleline(&mut filtro).hint_text("Buscar en el proyecto…").desired_width(f32::INFINITY));
                let f = filtro.to_lowercase();
                let mut archivos = Vec::new();
                maker_engine::project::walk(&raiz, &mut |p| {
                    let rel = p.strip_prefix(&raiz).map(|r| r.to_string_lossy().replace('\\', "/")).unwrap_or_default();
                    if f.is_empty() || rel.to_lowercase().contains(&f) {
                        archivos.push((rel, p.to_path_buf()));
                    }
                });
                egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
                    for (rel, p) in archivos.into_iter().take(300) {
                        let ya = ag.adjuntos.contains(&p);
                        if ui.selectable_label(ya, rel).clicked() {
                            if ya {
                                ag.adjuntos.retain(|a| *a != p);
                            } else {
                                ag.adjuntos.push(p);
                            }
                        }
                    }
                });
                ui.data_mut(|d| d.insert_temp(id, (ruta, filtro)));
            })
            .response
            .on_hover_text("También puedes soltar archivos aquí o pegar una imagen (Ctrl+V)");
        });
    });
}
