//! Componente Animador en el Inspector: fuente de clips, clip (con
//! búsqueda), reproducir/pausa, bucle, velocidad y «en el sitio».

use crate::editor::Editor;
use crate::panels::component_header;
use crate::theme;
use egui::{RichText, Ui};
use maker_engine::animation;
use maker_engine::biblioteca;
use maker_engine::components::*;
use maker_engine::scene::Scene;

/// Animator para «Añadir componente»: los clips del propio modelo si los
/// tiene; si no, la Universal Animation Library.
pub fn default_for(scene: &Scene, id: EntityId) -> Animator {
    if let Some(MeshSource::Model(p)) = scene.get::<MeshRenderer>(id).map(|m| m.mesh) {
        let own = animation::clip_names(&p, None);
        if let Some(c) = own.iter().find(|c| c.contains("Idle")).or(own.first()) {
            return Animator::new("", c);
        }
    }
    Animator::new(biblioteca::DEFAULT_CLIPS, biblioteca::DEFAULT_IDLE)
}

fn source_label(s: &str) -> String {
    if s.is_empty() {
        return "Del modelo".into();
    }
    let file = s.rsplit('/').next().unwrap_or(s);
    file.trim_end_matches(".glb").trim_end_matches(".gltf").replace('_', " ")
}

pub fn animator(ed: &mut Editor, ui: &mut Ui, id: EntityId) {
    let Some(mut a) = ed.scene.get::<Animator>(id) else { return };
    let (open, remove) = component_header(ui, "Animador", "insp-animator", true);
    if remove {
        ed.scene.remove::<Animator>(id);
        ed.scene.remove::<SkinPose>(id);
        ed.commit("Quitar animador");
        return;
    }
    if !open {
        ui.separator();
        return;
    }
    let before = a.clone();
    let root = ed.project.root.clone();
    let own_model = match ed.scene.get::<MeshRenderer>(id).map(|m| m.mesh) {
        Some(MeshSource::Model(p)) => Some(p),
        _ => None,
    };
    let clip_file = if a.source.is_empty() { own_model.clone() } else { Some(a.source.clone()) };
    let clips = clip_file.map(|f| animation::clip_names(&f, Some(&root))).unwrap_or_default();
    let mut sources: Vec<String> = vec![String::new()];
    if let Some(cat) = ed.biblioteca.catalog() {
        sources.extend(cat.animation_sources().filter(|e| e.pack.starts_with("animaciones")).map(|e| e.uri()));
    }
    egui::Grid::new("animator-grid").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
        ui.label("Clips");
        egui::ComboBox::from_id_salt("anim-src").selected_text(source_label(&a.source)).width(170.0).show_ui(ui, |ui| {
            for s in &sources {
                ui.selectable_value(&mut a.source, s.clone(), source_label(s));
            }
        });
        ui.end_row();
        ui.label("Clip");
        let search_id = ui.make_persistent_id("anim-clip-search");
        egui::ComboBox::from_id_salt("anim-clip").selected_text(if a.clip.is_empty() { "—".to_string() } else { a.clip.clone() }).width(170.0).height(320.0).show_ui(ui, |ui| {
            let mut q: String = ui.data_mut(|d| d.get_temp(search_id).unwrap_or_default());
            let r = ui.add(egui::TextEdit::singleline(&mut q).hint_text("Buscar…").desired_width(160.0));
            r.request_focus();
            ui.data_mut(|d| d.insert_temp(search_id, q.clone()));
            let q = biblioteca::fold(&q);
            for c in clips.iter().filter(|c| q.is_empty() || biblioteca::fold(c).contains(&q)) {
                ui.selectable_value(&mut a.clip, c.clone(), c);
            }
        });
        ui.end_row();
        ui.label("Reproducir");
        ui.horizontal(|ui| {
            let label = if a.playing { "❚❚ Pausa" } else { "▶ Reproducir" };
            if ui.button(label).clicked() {
                a.playing = !a.playing;
            }
            ui.checkbox(&mut a.looping, "Bucle");
        });
        ui.end_row();
        ui.label("Velocidad");
        ui.add(egui::DragValue::new(&mut a.speed).speed(0.01).range(-4.0..=4.0).prefix("× "));
        ui.end_row();
        ui.label("En el sitio");
        ui.checkbox(&mut a.in_place, "");
        ui.end_row();
    });
    if !clips.is_empty() && !a.clip.is_empty() && !clips.contains(&a.clip) {
        ui.label(RichText::new(format!("No existe «{}» en estos clips", a.clip)).small().color(theme::YELLOW));
    }
    if a != before {
        if a.source != before.source && !clips.is_empty() {
            // Al cambiar de fuente, el clip actual puede no existir en la nueva.
            let next = clip_file_names(&a, own_model.as_deref(), &root);
            if !next.contains(&a.clip) {
                a.clip = next.iter().find(|c| c.contains("Idle")).or(next.first()).cloned().unwrap_or_default();
            }
        }
        ed.scene.insert(id, a.clone());
        ed.pending_edit = Some("Editar animador".into());
    }
    // Vista previa en el editor: el Animator seleccionado avanza mientras suena.
    if a.playing && !ed.is_playing() {
        ui.ctx().request_repaint();
    }
    ui.separator();
}

fn clip_file_names(a: &Animator, own: Option<&str>, root: &std::path::Path) -> Vec<String> {
    let file = if a.source.is_empty() { own.map(str::to_string) } else { Some(a.source.clone()) };
    file.map(|f| animation::clip_names(&f, Some(root))).unwrap_or_default()
}
