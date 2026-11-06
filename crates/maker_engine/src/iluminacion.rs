//! Iluminación global dinámica: ajustes que guarda la escena
//! (`environment.iluminacion_global`) y la escena de demostración tipo
//! caja de Cornell. El trazado vive en `maker_render::gi`; ver
//! `docs/ILUMINACION.md`.

use crate::components::*;
use crate::scene::Scene;
use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};

/// Qué técnica usa la iluminación global.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ModoGi {
    /// Sin GI: ambiente hemisférico y reflejo aproximado del cielo (lo de siempre).
    #[default]
    Desactivada,
    /// Trazado por software: pantalla primero y después un volumen de vóxeles
    /// en cascadas centrado en la cámara.
    Software,
}

impl ModoGi {
    pub const ALL: [ModoGi; 2] = [ModoGi::Desactivada, ModoGi::Software];
    pub fn label(self) -> &'static str {
        match self {
            ModoGi::Desactivada => "Desactivada",
            ModoGi::Software => "Lumen (software)",
        }
    }
}

/// Nivel de calidad: resolución del volumen, cascadas, resolución del
/// trazado en pantalla y rayos por píxel.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum CalidadGi {
    Baja,
    #[default]
    Media,
    Alta,
    Epica,
}

impl CalidadGi {
    pub const ALL: [CalidadGi; 4] = [CalidadGi::Baja, CalidadGi::Media, CalidadGi::Alta, CalidadGi::Epica];
    pub fn label(self) -> &'static str {
        match self {
            CalidadGi::Baja => "Baja",
            CalidadGi::Media => "Media",
            CalidadGi::Alta => "Alta",
            CalidadGi::Epica => "Épica",
        }
    }
}

/// Ajustes de la iluminación global de una escena. Las escenas antiguas
/// cargan con la GI desactivada.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct IluminacionGlobal {
    pub modo: ModoGi,
    pub calidad: CalidadGi,
    /// Multiplica la luz indirecta difusa.
    pub intensidad_indirecta: f32,
    /// Multiplica los reflejos.
    pub intensidad_reflejos: f32,
    /// Realimenta la luz indirecta en el volumen (rebotes múltiples que
    /// convergen en unos fotogramas). Apagado: un solo rebote.
    pub rebotes_multiples: bool,
}

impl Default for IluminacionGlobal {
    fn default() -> Self {
        Self { modo: ModoGi::Desactivada, calidad: CalidadGi::Media, intensidad_indirecta: 1.0, intensidad_reflejos: 1.0, rebotes_multiples: true }
    }
}

impl IluminacionGlobal {
    pub fn activa(&self) -> bool {
        self.modo != ModoGi::Desactivada
    }
    /// Para `skip_serializing_if`: las escenas sin GI no cambian en disco.
    pub fn es_defecto(&self) -> bool {
        *self == Self::default()
    }
}

/// Caja de Cornell: paredes roja y verde, techo con una lámpara emisiva,
/// una caja blanca alta, un cubo emisivo y un suelo brillante. La GI está
/// activada en calidad media y no hay sol (solo la luz de la lámpara entra
/// por la abertura delantera).
pub fn cornell_scene() -> Scene {
    let mut s = Scene::new();
    s.env.iluminacion_global = IluminacionGlobal { modo: ModoGi::Software, ..Default::default() };
    s.env.ambient_intensity = 0.35;
    s.env.fog_density = 0.0;
    let cam = s.spawn("Cámara principal");
    s.set_transform(cam, Transform::from_translation(Vec3::new(0.0, 2.0, 6.4)).with_rotation(Quat::from_rotation_x(-0.06)));
    s.insert(cam, Camera::default());
    let pared = |s: &mut Scene, nombre: &str, pos: Vec3, escala: Vec3, color: [f32; 3], rough: f32| {
        let e = s.spawn(nombre);
        s.set_transform(e, Transform::from_translation(pos).with_scale(escala));
        let mut mr = MeshRenderer::primitive(PrimitiveShape::Cube);
        mr.material = Material { roughness: rough, ..Material::color(color) };
        s.insert(e, mr);
        s.insert(e, RigidBody::fixed());
        s.insert(e, Collider::new(ColliderShape::fit(PrimitiveShape::Cube)));
        e
    };
    let blanco = [0.85, 0.85, 0.82];
    pared(&mut s, "Suelo", Vec3::new(0.0, -0.15, 0.0), Vec3::new(4.6, 0.3, 4.6), blanco, 0.12);
    pared(&mut s, "Techo", Vec3::new(0.0, 4.15, 0.0), Vec3::new(4.6, 0.3, 4.6), blanco, 0.8);
    pared(&mut s, "Fondo", Vec3::new(0.0, 2.0, -2.15), Vec3::new(4.6, 4.6, 0.3), blanco, 0.8);
    pared(&mut s, "Pared roja", Vec3::new(-2.15, 2.0, 0.0), Vec3::new(0.3, 4.6, 4.6), [0.75, 0.06, 0.05], 0.8);
    pared(&mut s, "Pared verde", Vec3::new(2.15, 2.0, 0.0), Vec3::new(0.3, 4.6, 4.6), [0.10, 0.55, 0.12], 0.8);
    let caja = pared(&mut s, "Caja alta", Vec3::new(-0.75, 1.2, -0.7), Vec3::new(1.1, 2.4, 1.1), blanco, 0.7);
    s.set_transform(caja, Transform::from_translation(Vec3::new(-0.75, 1.2, -0.7)).with_scale(Vec3::new(1.1, 2.4, 1.1)).with_rotation(Quat::from_rotation_y(0.3)));
    let lampara = s.spawn("Lámpara");
    s.set_transform(lampara, Transform::from_translation(Vec3::new(0.0, 3.97, 0.0)).with_scale(Vec3::new(1.2, 0.06, 1.2)));
    let mut mr = MeshRenderer::primitive(PrimitiveShape::Cube);
    mr.material = Material { emissive: [1.0, 0.92, 0.8], emissive_strength: 6.0, ..Material::color([1.0, 0.95, 0.9]) };
    mr.cast_shadows = false;
    s.insert(lampara, mr);
    let luz = s.spawn("Luz de la lámpara");
    s.set_transform(luz, Transform::from_translation(Vec3::new(0.0, 3.6, 0.0)));
    s.insert(luz, Light { intensity: 9.0, range: 9.0, color: [1.0, 0.92, 0.8], ..Light::point() });
    let cubo = s.spawn("Cubo emisivo");
    s.set_transform(cubo, Transform::from_translation(Vec3::new(1.0, 0.35, 0.6)).with_scale(Vec3::splat(0.7)).with_rotation(Quat::from_rotation_y(-0.4)));
    let mut mr = MeshRenderer::primitive(PrimitiveShape::Cube);
    mr.material = Material { emissive: [0.2, 0.55, 1.0], emissive_strength: 5.0, ..Material::color([0.2, 0.5, 1.0]) };
    s.insert(cubo, mr);
    let esfera = s.spawn("Esfera metálica");
    s.set_transform(esfera, Transform::from_translation(Vec3::new(0.9, 1.3, -1.0)).with_scale(Vec3::splat(0.9)));
    let mut mr = MeshRenderer::primitive(PrimitiveShape::Sphere);
    mr.material = Material { metallic: 1.0, roughness: 0.15, ..Material::color([0.95, 0.93, 0.9]) };
    s.insert(esfera, mr);
    s.update_transforms();
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_scenes_load_with_gi_off_and_roundtrip() {
        let mut s = Scene::starter();
        let json = s.to_json();
        assert!(!json.contains("iluminacion_global"), "sin GI la escena no cambia en disco");
        let back = Scene::from_json(&json).unwrap();
        assert_eq!(back.env.iluminacion_global, IluminacionGlobal::default());
        assert!(!back.env.iluminacion_global.activa());
        s.env.iluminacion_global = IluminacionGlobal { modo: ModoGi::Software, calidad: CalidadGi::Epica, intensidad_indirecta: 1.5, intensidad_reflejos: 0.5, rebotes_multiples: false };
        let back = Scene::from_json(&s.to_json()).unwrap();
        assert_eq!(back.env.iluminacion_global, s.env.iluminacion_global);
        // Campos que faltan toman su valor por defecto.
        let parcial: IluminacionGlobal = serde_json::from_str(r#"{"modo":"Software"}"#).unwrap();
        assert_eq!(parcial.calidad, CalidadGi::Media);
        assert!(parcial.activa());
    }

    #[test]
    fn cornell_scene_has_gi_and_emissive() {
        let s = cornell_scene();
        assert!(s.env.iluminacion_global.activa());
        let cubo = s.find_by_name("Cubo emisivo").unwrap();
        assert!(s.get::<MeshRenderer>(cubo).unwrap().material.emissive_strength > 0.0);
        assert!(s.primary_camera().is_some());
    }
}
