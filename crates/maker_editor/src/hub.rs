//! Project Hub (como Unity Hub o el gestor de proyectos de Godot): proyectos
//! recientes, crear uno nuevo (vacío o demo jugable) y abrir uno existente.

use crate::theme::{self, section_label};
use egui::RichText;
use maker_engine::Project;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub recent: Vec<String>,
}

fn config_path() -> Option<PathBuf> {
    let base = std::env::var_os("MAKER_STUDIO_CONFIG")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("XDG_CONFIG_HOME").map(|p| PathBuf::from(p).join("maker-studio")))
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config/maker-studio")))
        .or_else(|| std::env::var_os("APPDATA").map(|h| PathBuf::from(h).join("maker-studio")))?;
    Some(base.join("config.json"))
}

impl Config {
    pub fn load() -> Config {
        config_path().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }
    pub fn save(&self) {
        if let Some(p) = config_path() {
            if let Some(d) = p.parent() {
                let _ = std::fs::create_dir_all(d);
            }
            let _ = std::fs::write(p, serde_json::to_string_pretty(self).unwrap_or_default());
        }
    }
    pub fn touch(&mut self, root: &std::path::Path) {
        let s = root.to_string_lossy().into_owned();
        self.recent.retain(|r| *r != s);
        self.recent.insert(0, s);
        self.recent.truncate(20);
        self.save();
    }
}

pub struct Hub {
    pub config: Config,
    pub new_name: String,
    pub new_location: String,
    pub demo: bool,
    pub open_path: String,
    pub error: Option<String>,
}

impl Hub {
    pub fn new() -> Self {
        let home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).unwrap_or_else(|_| ".".into());
        let docs = ["Documentos", "Documents"].iter().map(|d| PathBuf::from(&home).join(d)).find(|p| p.exists()).unwrap_or_else(|| PathBuf::from(&home));
        Self {
            config: Config::load(),
            new_name: "Mi juego".into(),
            new_location: docs.join("MakerProjects").to_string_lossy().into_owned(),
            demo: true,
            open_path: String::new(),
            error: None,
        }
    }

    /// Dibuja el hub. Devuelve el proyecto a abrir.
    pub fn ui(&mut self, ui: &mut egui::Ui) -> Option<Project> {
        let mut open = None;
        egui::CentralPanel::default_margins().show(ui, |ui| {
            ui.add_space(28.0);
            let (logo, size) = crate::icons::logo(ui.ctx(), 67.0);
            ui.vertical_centered(|ui| {
                ui.add(egui::Image::new((logo.id(), size)));
            });
            ui.add_space(22.0);
            let width = ui.available_width().min(980.0);
            ui.horizontal(|ui| {
                ui.add_space(((ui.available_width() - width) * 0.5).max(0.0));
                ui.allocate_ui(egui::vec2(width, ui.available_height()), |ui| {
                    ui.columns(2, |cols| {
                        // Recientes.
                        theme::card_frame().show(&mut cols[0], |ui| {
                            ui.set_min_height(380.0);
                            ui.set_width(ui.available_width());
                            section_label(ui, "Proyectos recientes");
                            ui.add_space(4.0);
                            if self.config.recent.is_empty() {
                                ui.label(RichText::new("Todavía no has abierto ningún proyecto.").color(theme::TEXT3));
                            }
                            let mut remove = None;
                            egui::ScrollArea::vertical().max_height(340.0).show(ui, |ui| {
                                for (i, r) in self.config.recent.iter().enumerate() {
                                    let path = PathBuf::from(r);
                                    let exists = path.join(maker_engine::project::PROJECT_FILE).exists();
                                    let name = Project::open(&path).map(|p| p.file.name).unwrap_or_else(|_| path.file_name().unwrap_or_default().to_string_lossy().into_owned());
                                    egui::Frame::new().fill(theme::SIDEBAR).corner_radius(4.0).inner_margin(8.0).show(ui, |ui| {
                                        ui.set_width(ui.available_width());
                                        ui.horizontal(|ui| {
                                            ui.vertical(|ui| {
                                                ui.label(RichText::new(&name).strong().color(if exists { theme::TEXT1 } else { theme::TEXT4 }));
                                                ui.label(RichText::new(r).small().color(theme::TEXT3));
                                            });
                                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                                if ui.small_button("✕").on_hover_text("Quitar de la lista").clicked() {
                                                    remove = Some(i);
                                                }
                                                if ui.add_enabled(exists, egui::Button::new("Abrir")).clicked() {
                                                    match Project::open(&path) {
                                                        Ok(p) => open = Some(p),
                                                        Err(e) => self.error = Some(e),
                                                    }
                                                }
                                            });
                                        });
                                    });
                                    ui.add_space(3.0);
                                }
                            });
                            if let Some(i) = remove {
                                self.config.recent.remove(i);
                                self.config.save();
                            }
                        });
                        // Nuevo / abrir.
                        theme::card_frame().show(&mut cols[1], |ui| {
                            ui.set_min_height(380.0);
                            ui.set_width(ui.available_width());
                            section_label(ui, "Nuevo proyecto");
                            egui::Grid::new("new-grid").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
                                ui.label("Nombre");
                                ui.add(egui::TextEdit::singleline(&mut self.new_name).desired_width(f32::INFINITY));
                                ui.end_row();
                                ui.label("Ubicación");
                                ui.add(egui::TextEdit::singleline(&mut self.new_location).desired_width(f32::INFINITY));
                                ui.end_row();
                                ui.label("Plantilla");
                                ui.horizontal(|ui| {
                                    ui.radio_value(&mut self.demo, true, "Demo jugable");
                                    ui.radio_value(&mut self.demo, false, "Escena básica");
                                });
                                ui.end_row();
                            });
                            let folder = PathBuf::from(&self.new_location).join(self.new_name.trim());
                            ui.label(RichText::new(format!("Se creará en {}", folder.display())).small().color(theme::TEXT3));
                            ui.add_space(4.0);
                            let primary = egui::Button::new(RichText::new("Crear proyecto").color(egui::Color32::WHITE)).fill(theme::ACCENT);
                            if ui.add_enabled(!self.new_name.trim().is_empty(), primary).clicked() {
                                match Project::create(&folder, self.new_name.trim(), self.demo) {
                                    Ok(p) => {
                                        maker_engine::log::info("hub", format!("proyecto creado en {}", p.root.display()));
                                        open = Some(p);
                                    }
                                    Err(e) => self.error = Some(e),
                                }
                            }
                            ui.add_space(16.0);
                            section_label(ui, "Abrir proyecto");
                            ui.horizontal(|ui| {
                                ui.add(egui::TextEdit::singleline(&mut self.open_path).hint_text("Carpeta con project.maker").desired_width(ui.available_width() - 70.0));
                                if ui.button("Abrir").clicked() {
                                    let raw = self.open_path.trim().trim_matches(['"', '\'']).trim_start_matches("file://");
                                    match Project::open(std::path::Path::new(raw)) {
                                        Ok(p) => open = Some(p),
                                        Err(e) => self.error = Some(e),
                                    }
                                }
                            });
                            if let Some(e) = &self.error {
                                ui.add_space(6.0);
                                ui.colored_label(theme::RED, e);
                            }
                        });
                    });
                });
            });
        });
        if let Some(p) = &open {
            self.config.touch(&p.root);
        }
        open
    }
}
