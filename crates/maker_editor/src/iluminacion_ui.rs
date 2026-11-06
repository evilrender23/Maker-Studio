//! Inspector del entorno: sección «Iluminación global» (modo, calidad,
//! intensidades y rebotes). Edita una copia del entorno; el panel confirma
//! el cambio con el flujo de deshacer de siempre.

use crate::theme;
use egui::Ui;
use maker_engine::iluminacion::{CalidadGi, IluminacionGlobal, ModoGi};

pub fn iluminacion_global(ui: &mut Ui, g: &mut IluminacionGlobal, soportada: bool) {
    if !soportada {
        ui.colored_label(theme::YELLOW, "Esta GPU no admite la iluminación global");
    }
    egui::Grid::new("gi-grid").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
        ui.label("Modo");
        egui::ComboBox::from_id_salt("gi-modo").selected_text(g.modo.label()).width(170.0).show_ui(ui, |ui| {
            for m in ModoGi::ALL {
                ui.selectable_value(&mut g.modo, m, m.label());
            }
        })
        .response
        .on_hover_text("Luz indirecta y reflejos dinámicos: trazado en pantalla y después en un volumen de vóxeles alrededor de la cámara");
        ui.end_row();
        if g.activa() {
            ui.label("Calidad");
            egui::ComboBox::from_id_salt("gi-calidad").selected_text(g.calidad.label()).width(170.0).show_ui(ui, |ui| {
                for c in CalidadGi::ALL {
                    ui.selectable_value(&mut g.calidad, c, c.label());
                }
            })
            .response
            .on_hover_text("Resolución del volumen, del trazado en pantalla y rayos por píxel");
            ui.end_row();
            ui.label("Luz indirecta");
            ui.add(egui::Slider::new(&mut g.intensidad_indirecta, 0.0..=4.0)).on_hover_text("Multiplica la luz que rebota");
            ui.end_row();
            ui.label("Reflejos");
            ui.add(egui::Slider::new(&mut g.intensidad_reflejos, 0.0..=4.0)).on_hover_text("Multiplica los reflejos trazados");
            ui.end_row();
            ui.label("Rebotes múltiples");
            ui.checkbox(&mut g.rebotes_multiples, "").on_hover_text("La luz rebota más de una vez (converge en unos fotogramas). Apagado: un solo rebote");
            ui.end_row();
        }
    });
}
