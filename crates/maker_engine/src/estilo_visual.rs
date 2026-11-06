//! Estilos visuales: el sombreado global y la pila de posprocesado de una
//! escena guardados como un solo dato. Los integrados vienen embebidos
//! (`estilos_visuales/*.mvisual`) y los del usuario viven en
//! `estilos/*.mvisual` del proyecto; ambos tienen el mismo formato JSON, que es
//! el de los campos `global_shading` y `post` de [`Environment`].

use crate::postproceso::{GlobalShading, PostEffect, PostKind};
use crate::scene::Environment;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Carpeta de los estilos del usuario, relativa al proyecto.
pub const DIR: &str = "estilos";
/// Extensión de los archivos de estilo visual.
pub const EXT: &str = "mvisual";

/// Sombreado global + pila de posprocesado.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EstiloVisual {
    pub global_shading: GlobalShading,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub post: Vec<PostEffect>,
}

/// Qué partes del estilo se aplican.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Partes {
    #[default]
    Todo,
    /// Solo el sombreado global; la pila actual se conserva.
    Sombreado,
    /// Solo la pila; el sombreado actual se conserva.
    Posprocesado,
}

impl Partes {
    fn sombreado(self) -> bool {
        self != Partes::Posprocesado
    }
    fn post(self) -> bool {
        self != Partes::Sombreado
    }
}

/// Un estilo integrado: identificador estable, nombre visible y datos.
pub struct Integrado {
    pub id: &'static str,
    pub nombre: &'static str,
    json: &'static str,
}

impl Integrado {
    /// Los datos del estilo (los integrados se validan en los tests).
    pub fn estilo(&self) -> EstiloVisual {
        serde_json::from_str(self.json).unwrap_or_default()
    }
}

macro_rules! integrado {
    ($id:literal, $nombre:literal) => {
        Integrado { id: $id, nombre: $nombre, json: include_str!(concat!("estilos_visuales/", $id, ".mvisual")) }
    };
}

/// Estilos integrados, en el orden de la galería.
pub const INTEGRADOS: [Integrado; 9] = [
    integrado!("realista", "Realista"),
    integrado!("toon", "Toon"),
    integrado!("anime", "Anime"),
    integrado!("comic", "Cómic"),
    integrado!("retro_ps1", "Retro PS1"),
    integrado!("cinematico", "Cinemático"),
    integrado!("noir", "Noir"),
    integrado!("pastel", "Pastel"),
    integrado!("crt", "CRT"),
];

impl EstiloVisual {
    /// El estilo actual de un entorno.
    pub fn de_entorno(env: &Environment) -> Self {
        Self { global_shading: env.global_shading, post: env.post.clone() }
    }

    /// Escala la fuerza de todo el estilo (0 = nada, 1 = como está): la
    /// intensidad de cada efecto, la exposición del tonemap y el brillo, la
    /// luz de borde y el contorno del sombreado.
    pub fn escalado(&self, k: f32) -> Self {
        let k = k.max(0.0);
        let mut e = self.clone();
        let s = &mut e.global_shading.style;
        s.specular *= k;
        s.rim_strength *= k;
        s.outline_width *= k;
        for fx in &mut e.post {
            match &mut fx.kind {
                PostKind::Tonemap(t) => t.exposure_ev *= k,
                _ => fx.intensity *= k,
            }
        }
        e
    }

    /// Pone en `env` las partes elegidas del estilo.
    pub fn aplicar(&self, env: &mut Environment, partes: Partes) {
        if partes.sombreado() {
            env.global_shading = self.global_shading;
        }
        if partes.post() {
            env.post = self.post.clone();
        }
    }

    /// Efectos WGSL del proyecto que usa (rutas relativas).
    pub fn efectos_propios(&self) -> Vec<&str> {
        self.post
            .iter()
            .filter_map(|e| match &e.kind {
                PostKind::Custom(c) => Some(c.path.as_str()),
                _ => None,
            })
            .collect()
    }

    /// ¿Están en el proyecto todos los efectos propios que usa?
    pub fn disponible(&self, raiz: &Path) -> bool {
        self.efectos_propios().iter().all(|p| raiz.join(p).is_file())
    }

    /// ¿El entorno tiene las partes elegidas de este estilo? (Con tolerancia
    /// en los números: los deslizadores y el JSON redondean.)
    pub fn coincide(&self, env: &Environment, partes: Partes) -> bool {
        (!partes.sombreado() || casi_igual(&valor(&self.global_shading), &valor(&env.global_shading)))
            && (!partes.post() || casi_igual(&valor(&self.post), &valor(&env.post)))
    }

    /// Intensidad con la que este estilo está aplicado en `env`, si lo está.
    /// Prueba la que se deduce de la pila, las `candidatas` y 1.
    pub fn intensidad_en(&self, env: &Environment, partes: Partes, candidatas: &[f32]) -> Option<f32> {
        let deducida = if partes.post() && self.post.len() == env.post.len() {
            self.post.iter().zip(&env.post).find(|(p, _)| !matches!(p.kind, PostKind::Tonemap(_)) && p.intensity > 1e-4).map(|(p, e)| e.intensity / p.intensity)
        } else {
            let (p, e) = (&self.global_shading.style, &env.global_shading.style);
            [(p.outline_width, e.outline_width), (p.rim_strength, e.rim_strength), (p.specular, e.specular)].into_iter().find(|(p, _)| *p > 1e-4).map(|(p, e)| e / p)
        };
        deducida.into_iter().chain(candidatas.iter().copied()).chain([1.0]).find(|&k| self.escalado(k).coincide(env, partes))
    }

    pub fn cargar(ruta: &Path) -> Result<Self, String> {
        let txt = std::fs::read_to_string(ruta).map_err(|e| format!("{}: {e}", ruta.display()))?;
        serde_json::from_str(&txt).map_err(|e| format!("{}: {e}", ruta.display()))
    }

    pub fn guardar(&self, ruta: &Path) -> Result<(), String> {
        if let Some(d) = ruta.parent() {
            std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
        }
        let txt = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(ruta, txt + "\n").map_err(|e| format!("{}: {e}", ruta.display()))
    }
}

fn valor<T: Serialize>(v: &T) -> serde_json::Value {
    serde_json::to_value(v).unwrap_or_default()
}

fn casi_igual(a: &serde_json::Value, b: &serde_json::Value) -> bool {
    use serde_json::Value as V;
    match (a, b) {
        (V::Number(x), V::Number(y)) => {
            let (x, y) = (x.as_f64().unwrap_or(0.0), y.as_f64().unwrap_or(0.0));
            (x - y).abs() <= 1e-3 * (1.0 + x.abs().max(y.abs()))
        }
        (V::Array(x), V::Array(y)) => x.len() == y.len() && x.iter().zip(y).all(|(a, b)| casi_igual(a, b)),
        (V::Object(x), V::Object(y)) => x.len() == y.len() && x.iter().all(|(k, a)| y.get(k).is_some_and(|b| casi_igual(a, b))),
        _ => a == b,
    }
}

/// Estilos del usuario (`estilos/*.mvisual`), ordenados por nombre.
pub fn archivos(raiz: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(raiz.join(DIR))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case(EXT)))
        .collect();
    v.sort_by_key(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()));
    v
}

/// Nombre de archivo válido para un estilo (sin separadores ni extensión).
pub fn nombre_valido(n: &str) -> Option<String> {
    let n = n.trim().trim_end_matches(&format!(".{EXT}")).trim();
    let ok = !n.is_empty() && !n.starts_with('.') && !n.contains(['/', '\\', ':', '*', '?', '"', '<', '>', '|']);
    ok.then(|| n.to_string())
}

/// Ruta de un estilo del usuario por nombre.
pub fn ruta(raiz: &Path, nombre: &str) -> PathBuf {
    raiz.join(DIR).join(format!("{nombre}.{EXT}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::ShadingModel;

    #[test]
    fn integrados_se_leen() {
        let mut ids = std::collections::HashSet::new();
        for i in &INTEGRADOS {
            assert!(ids.insert(i.id));
            let e: EstiloVisual = serde_json::from_str(i.json).unwrap_or_else(|err| panic!("{}: {err}", i.id));
            assert_ne!(e, EstiloVisual::default(), "{}", i.id);
            assert!(e.post.iter().all(|f| f.enabled), "{}", i.id);
        }
        let toon = INTEGRADOS.iter().find(|i| i.id == "toon").unwrap().estilo();
        assert_eq!(toon.global_shading.model, Some(ShadingModel::Toon));
        let comic = INTEGRADOS.iter().find(|i| i.id == "comic").unwrap().estilo();
        assert_eq!(comic.global_shading.model, Some(ShadingModel::Hatching));
        let ps1 = INTEGRADOS.iter().find(|i| i.id == "retro_ps1").unwrap().estilo();
        assert!(ps1.post.iter().any(|f| matches!(&f.kind, PostKind::Retro(r) if r.dither > 0.0 && r.levels > 0 && r.pixel_size > 1.0)));
        let noir = INTEGRADOS.iter().find(|i| i.id == "noir").unwrap().estilo();
        assert!(noir.post.iter().any(|f| matches!(&f.kind, PostKind::ColorGrading(g) if g.saturation == 0.0)));
    }

    #[test]
    fn crt_solo_si_el_proyecto_tiene_el_efecto() {
        let crt = INTEGRADOS.iter().find(|i| i.id == "crt").unwrap().estilo();
        assert_eq!(crt.efectos_propios(), vec!["shaders/post/crt.wgsl"]);
        let dir = std::env::temp_dir().join(format!("maker-estilo-crt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("shaders/post")).unwrap();
        assert!(!crt.disponible(&dir));
        std::fs::write(dir.join("shaders/post/crt.wgsl"), crate::postproceso::EXAMPLES[0].1).unwrap();
        assert!(crt.disponible(&dir));
        // Los que no usan efectos propios siempre están.
        assert!(INTEGRADOS.iter().filter(|i| i.id != "crt").all(|i| i.estilo().disponible(Path::new("/no/existe"))));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn aplicar_por_partes() {
        let toon = INTEGRADOS[1].estilo();
        let noir = INTEGRADOS.iter().find(|i| i.id == "noir").unwrap().estilo();
        let mut env = Environment { exposure: 1.7, ..Default::default() };
        toon.aplicar(&mut env, Partes::Todo);
        assert_eq!(env.global_shading, toon.global_shading);
        assert_eq!(env.post, toon.post);
        assert_eq!(env.exposure, 1.7, "el resto del entorno no se toca");
        assert!(toon.coincide(&env, Partes::Todo));
        // Solo posprocesado: conserva el sombreado toon.
        noir.aplicar(&mut env, Partes::Posprocesado);
        assert_eq!(env.global_shading, toon.global_shading);
        assert_eq!(env.post, noir.post);
        assert!(noir.coincide(&env, Partes::Posprocesado) && !noir.coincide(&env, Partes::Todo));
        // Solo sombreado: conserva la pila noir.
        EstiloVisual::default().aplicar(&mut env, Partes::Sombreado);
        assert_eq!(env.global_shading.model, None);
        assert_eq!(env.post, noir.post);
        // Ninguno: PBR de cada material y pila vacía.
        EstiloVisual::default().aplicar(&mut env, Partes::Todo);
        assert_eq!((env.global_shading, env.post.len()), (GlobalShading::default(), 0));
        assert!(EstiloVisual::default().coincide(&env, Partes::Todo));
    }

    #[test]
    fn intensidad_escala_todo() {
        let cine = INTEGRADOS.iter().find(|i| i.id == "cinematico").unwrap().estilo();
        let mitad = cine.escalado(0.5);
        for (a, b) in cine.post.iter().zip(&mitad.post) {
            match (&a.kind, &b.kind) {
                (PostKind::Tonemap(x), PostKind::Tonemap(y)) => assert!((y.exposure_ev - x.exposure_ev * 0.5).abs() < 1e-6),
                _ => assert!((b.intensity - a.intensity * 0.5).abs() < 1e-6),
            }
            assert_eq!(std::mem::discriminant(&a.kind), std::mem::discriminant(&b.kind));
        }
        let toon = INTEGRADOS[1].estilo();
        let t = toon.escalado(0.25).global_shading;
        assert!((t.style.outline_width - toon.global_shading.style.outline_width * 0.25).abs() < 1e-6);
        assert!((t.style.rim_strength - toon.global_shading.style.rim_strength * 0.25).abs() < 1e-6);
        assert_eq!(t.model, toon.global_shading.model);
        assert_eq!(toon.escalado(1.0), toon);
        // Se reconoce el estilo aplicado y con qué intensidad.
        let mut env = Environment::default();
        cine.escalado(0.4).aplicar(&mut env, Partes::Todo);
        let k = cine.intensidad_en(&env, Partes::Todo, &[]).unwrap();
        assert!((k - 0.4).abs() < 1e-3, "{k}");
        assert!(toon.intensidad_en(&env, Partes::Todo, &[]).is_none());
        let mut env = Environment::default();
        toon.escalado(0.6).aplicar(&mut env, Partes::Sombreado);
        assert!((toon.intensidad_en(&env, Partes::Sombreado, &[]).unwrap() - 0.6).abs() < 1e-3);
        // A intensidad 0 la pila no se deduce: vale la candidata.
        let mut env = Environment::default();
        cine.escalado(0.0).aplicar(&mut env, Partes::Todo);
        assert_eq!(cine.intensidad_en(&env, Partes::Todo, &[0.0]), Some(0.0));
    }

    #[test]
    fn guardar_y_cargar() {
        let dir = std::env::temp_dir().join(format!("maker-estilo-io-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut env = Environment::default();
        INTEGRADOS[2].estilo().escalado(0.7).aplicar(&mut env, Partes::Todo);
        env.post[0].enabled = false;
        let e = EstiloVisual::de_entorno(&env);
        let nombre = nombre_valido("  Mi estilo.mvisual ").unwrap();
        assert_eq!(nombre, "Mi estilo");
        assert_eq!(nombre_valido("a/b"), None);
        assert_eq!(nombre_valido("  "), None);
        e.guardar(&ruta(&dir, &nombre)).unwrap();
        std::fs::write(dir.join(DIR).join("otro.txt"), "x").unwrap();
        assert_eq!(archivos(&dir), vec![dir.join("estilos/Mi estilo.mvisual")]);
        let leido = EstiloVisual::cargar(&archivos(&dir)[0]).unwrap();
        assert_eq!(leido, e);
        // Mismo formato que el entorno de una escena.
        let json: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&archivos(&dir)[0]).unwrap()).unwrap();
        let env_json = serde_json::to_value(&env).unwrap();
        assert!(casi_igual(&json["global_shading"], &env_json["global_shading"]));
        assert!(casi_igual(&json["post"], &env_json["post"]));
        // Un archivo mínimo o vacío también se lee.
        assert_eq!(serde_json::from_str::<EstiloVisual>("{}").unwrap(), EstiloVisual::default());
        assert!(EstiloVisual::cargar(&dir.join("no.mvisual")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
