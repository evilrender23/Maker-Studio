//! Preferencias del editor (JSON en la carpeta de configuración) y su ventana.

use crate::agente::cli::{Backend, Catalogo, Permisos};
use crate::theme::{self, section_label};
use egui::RichText;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// `MAKER_STUDIO_CONFIG`, `XDG_CONFIG_HOME/maker-studio`, `~/.config/maker-studio` o `%APPDATA%\maker-studio`.
pub fn carpeta_config() -> Option<PathBuf> {
    std::env::var_os("MAKER_STUDIO_CONFIG")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("XDG_CONFIG_HOME").map(|p| PathBuf::from(p).join("maker-studio")))
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config/maker-studio")))
        .or_else(|| std::env::var_os("APPDATA").map(|h| PathBuf::from(h).join("maker-studio")))
}

fn archivo() -> Option<PathBuf> {
    carpeta_config().map(|c| c.join("preferencias.json"))
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AjusteCli {
    /// Ejecutable personalizado (vacío: buscar en el PATH).
    pub ruta: String,
    /// Modelo predeterminado (vacío: el de la CLI).
    pub modelo: String,
    /// Esfuerzo predeterminado (vacío: el de la CLI).
    pub esfuerzo: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferencias {
    pub modo_agente: bool,
    pub cli: Backend,
    pub claude: AjusteCli,
    pub codex: AjusteCli,
    pub agy: AjusteCli,
    pub permisos: Permisos,
    /// Porcentaje de contexto a partir del que se compacta solo (0 = nunca).
    pub auto_compactar: u8,
    /// Compilar los scripts cuando el agente cambia `scripts/src`.
    pub recompilar: bool,
    /// Panel del agente visible.
    pub panel_agente: bool,
}

impl Default for Preferencias {
    fn default() -> Self {
        Preferencias {
            modo_agente: false,
            cli: Backend::Claude,
            claude: AjusteCli::default(),
            codex: AjusteCli::default(),
            agy: AjusteCli::default(),
            permisos: Permisos::Edicion,
            auto_compactar: 85,
            recompilar: true,
            panel_agente: true,
        }
    }
}

impl Preferencias {
    pub fn cargar() -> Preferencias {
        archivo().and_then(|f| std::fs::read_to_string(f).ok()).and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
    }

    pub fn guardar(&self) {
        let Some(f) = archivo() else { return };
        if let Some(d) = f.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        if let Ok(t) = serde_json::to_string_pretty(self)
            && let Err(e) = std::fs::write(&f, t)
        {
            maker_engine::log::error("preferencias", format!("{}: {e}", f.display()));
        }
    }

    pub fn cli(&self, b: Backend) -> &AjusteCli {
        match b {
            Backend::Claude => &self.claude,
            Backend::Codex => &self.codex,
            Backend::Antigravity => &self.agy,
        }
    }

    pub fn cli_mut(&mut self, b: Backend) -> &mut AjusteCli {
        match b {
            Backend::Claude => &mut self.claude,
            Backend::Codex => &mut self.codex,
            Backend::Antigravity => &mut self.agy,
        }
    }

    pub fn rutas(&self) -> [String; 3] {
        Backend::TODOS.map(|b| self.cli(b).ruta.clone())
    }
}

/// Desplegable de modelo: «Predeterminado», los descubiertos y uno personalizado.
pub fn selector_modelo(ui: &mut egui::Ui, id: impl std::hash::Hash + std::fmt::Debug, b: Backend, modelo: &mut String, catalogo: &Catalogo, ancho: f32) -> bool {
    let info = catalogo.info(b);
    let id = ui.make_persistent_id(("modelo", id));
    let conocido = modelo.is_empty() || info.modelos.iter().any(|m| m.id == *modelo);
    let mut otro = !conocido || (modelo.is_empty() && ui.data(|d| d.get_temp::<bool>(id)).unwrap_or(false));
    let mut cambio = false;
    let texto = if otro {
        "Otro…".to_string()
    } else if modelo.is_empty() {
        "Predeterminado".into()
    } else {
        info.modelos.iter().find(|m| m.id == *modelo).map(|m| m.nombre.clone()).unwrap_or(modelo.clone())
    };
    egui::ComboBox::from_id_salt(id).selected_text(texto).width(ancho).show_ui(ui, |ui| {
        if ui.selectable_label(!otro && modelo.is_empty(), "Predeterminado").clicked() {
            modelo.clear();
            otro = false;
            cambio = true;
        }
        for m in &info.modelos {
            let r = ui.selectable_label(!otro && *modelo == m.id, &m.nombre);
            if r.on_hover_text(&m.id).clicked() {
                *modelo = m.id.clone();
                otro = false;
                cambio = true;
            }
        }
        if ui.selectable_label(otro, "Otro…").clicked() {
            otro = true;
            modelo.clear();
            cambio = true;
        }
    });
    ui.data_mut(|d| d.insert_temp(id, otro));
    if otro {
        cambio |= ui.add(egui::TextEdit::singleline(modelo).hint_text("id del modelo").desired_width(ancho)).changed();
    }
    cambio
}

/// Desplegable de esfuerzo con los valores válidos para la CLI y el modelo.
pub fn selector_esfuerzo(ui: &mut egui::Ui, id: impl std::hash::Hash + std::fmt::Debug, b: Backend, modelo: &str, esfuerzo: &mut String, catalogo: &Catalogo, ancho: f32) -> bool {
    let validos = catalogo.esfuerzos(b, modelo);
    let mut cambio = false;
    if !esfuerzo.is_empty() && !validos.contains(esfuerzo) {
        esfuerzo.clear();
        cambio = true;
    }
    let texto = if esfuerzo.is_empty() { "Predeterminado".to_string() } else { esfuerzo.clone() };
    egui::ComboBox::from_id_salt(("esfuerzo", id)).selected_text(texto).width(ancho).show_ui(ui, |ui| {
        if ui.selectable_label(esfuerzo.is_empty(), "Predeterminado").clicked() {
            esfuerzo.clear();
            cambio = true;
        }
        for e in &validos {
            if ui.selectable_label(esfuerzo == e, e).clicked() {
                *esfuerzo = e.clone();
                cambio = true;
            }
        }
    });
    cambio
}

pub fn selector_permisos(ui: &mut egui::Ui, id: impl std::hash::Hash + std::fmt::Debug, b: Backend, p: &mut Permisos, ancho: f32) -> bool {
    let mut cambio = false;
    egui::ComboBox::from_id_salt(("permisos", id)).selected_text(p.nombre()).width(ancho).show_ui(ui, |ui| {
        for x in Permisos::TODOS {
            if ui.selectable_label(*p == x, x.nombre()).on_hover_text(x.ayuda(b)).clicked() {
                *p = x;
                cambio = true;
            }
        }
    });
    cambio
}

/// Ventana de Preferencias. Devuelve `true` si cambió algo (ya guardado).
pub fn ventana(ctx: &egui::Context, abierta: &mut bool, p: &mut Preferencias, catalogo: &Catalogo) -> bool {
    let antes = p.clone();
    let mut redetectar = false;
    egui::Window::new("Preferencias")
        .open(abierta)
        .collapsible(false)
        .resizable(false)
        .default_width(560.0)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.set_width(560.0);
            section_label(ui, "General");
            let instalada = crate::update::version_instalada().is_some();
            let mut auto = crate::update::automatico();
            if ui
                .add_enabled(instalada, egui::Checkbox::new(&mut auto, "Actualizar automáticamente"))
                .on_disabled_hover_text("Solo las copias instaladas con instalar.sh / instalar.ps1 se actualizan solas")
                .changed()
            {
                crate::update::cambiar_automatico(auto);
            }
            ui.add_space(8.0);
            section_label(ui, "Modo agente");
            if ui.checkbox(&mut p.modo_agente, "Modo agente").on_hover_text("Panel para trabajar con Claude Code, Codex o Antigravity sobre el proyecto").changed() && p.modo_agente {
                redetectar = true;
            }
            if !p.modo_agente {
                return;
            }
            ui.add_space(4.0);
            egui::Grid::new("prefs-agente").num_columns(2).min_col_width(140.0).spacing([12.0, 6.0]).show(ui, |ui| {
                ui.label("CLI predeterminada");
                egui::ComboBox::from_id_salt("prefs-cli").selected_text(p.cli.nombre()).width(180.0).show_ui(ui, |ui| {
                    for b in Backend::TODOS {
                        ui.selectable_value(&mut p.cli, b, b.nombre());
                    }
                });
                ui.end_row();
                ui.label("Permisos");
                let cli = p.cli;
                selector_permisos(ui, "prefs", cli, &mut p.permisos, 180.0);
                ui.end_row();
                ui.label("Compactar al llegar a");
                ui.horizontal(|ui| {
                    let mut activo = p.auto_compactar > 0;
                    if ui.checkbox(&mut activo, "").changed() {
                        p.auto_compactar = if activo { 85 } else { 0 };
                    }
                    ui.add_enabled(activo, egui::Slider::new(&mut p.auto_compactar, 50..=95).suffix(" %"));
                    if p.auto_compactar == 0 && activo {
                        p.auto_compactar = 50;
                    }
                });
                ui.end_row();
                ui.label("");
                ui.checkbox(&mut p.recompilar, "Recompilar scripts tras cambios del agente");
                ui.end_row();
            });
            for b in Backend::TODOS {
                ui.add_space(8.0);
                let info = catalogo.info(b);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(b.nombre()).strong());
                    match (&info.ruta, info.comprobado) {
                        (_, false) => {
                            ui.spinner();
                        }
                        (Some(r), true) => {
                            ui.label(RichText::new("●").color(theme::GREEN)).on_hover_text(r.display().to_string());
                            ui.label(RichText::new(info.version.clone().unwrap_or_default()).color(theme::TEXT3));
                        }
                        (None, true) => {
                            ui.label(RichText::new("● no encontrada").color(theme::RED));
                        }
                    }
                });
                let a = p.cli_mut(b);
                egui::Grid::new(("prefs-cli", b.clave())).num_columns(2).min_col_width(140.0).spacing([12.0, 6.0]).show(ui, |ui| {
                    ui.label("Ejecutable");
                    let hint = info.ruta.as_ref().filter(|_| a.ruta.is_empty()).map(|r| r.display().to_string()).unwrap_or_else(|| b.ejecutable().into());
                    if ui.add(egui::TextEdit::singleline(&mut a.ruta).hint_text(hint).desired_width(380.0)).lost_focus() {
                        redetectar = true;
                    }
                    ui.end_row();
                    ui.label("Modelo");
                    ui.horizontal(|ui| {
                        selector_modelo(ui, ("prefs", b.clave()), b, &mut a.modelo, catalogo, 220.0);
                    });
                    ui.end_row();
                    ui.label("Esfuerzo");
                    let modelo = a.modelo.clone();
                    selector_esfuerzo(ui, ("prefs", b.clave()), b, &modelo, &mut a.esfuerzo, catalogo, 220.0);
                    ui.end_row();
                });
            }
            ui.add_space(6.0);
            if ui.button("Volver a detectar").clicked() {
                redetectar = true;
            }
        });
    if redetectar && p.modo_agente {
        catalogo.detectar(p.rutas(), Some(ctx.clone()));
    }
    let cambio = *p != antes;
    if cambio {
        p.guardar();
    }
    cambio
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preferencias_ida_y_vuelta() {
        let p: Preferencias = serde_json::from_str("{}").unwrap();
        assert!(!p.modo_agente, "el modo agente viene apagado");
        assert_eq!(p.auto_compactar, 85);
        let mut q = p.clone();
        q.modo_agente = true;
        q.cli = Backend::Codex;
        q.cli_mut(Backend::Codex).esfuerzo = "low".into();
        let r: Preferencias = serde_json::from_str(&serde_json::to_string(&q).unwrap()).unwrap();
        assert_eq!(r, q);
    }
}
