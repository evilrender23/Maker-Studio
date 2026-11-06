//! Sesiones del agente guardadas por proyecto en `<proyecto>/.maker/agente/sesiones/*.json`.

use super::cli::{Backend, Permisos};
use super::eventos::TipoHerramienta;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn ahora() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as f64 / 1000.0).unwrap_or(0.0)
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "tipo", rename_all = "snake_case")]
pub enum Mensaje {
    Usuario {
        texto: String,
        #[serde(default)]
        contexto: Vec<String>,
    },
    Asistente {
        texto: String,
    },
    Herramienta {
        id: String,
        clase: TipoHerramienta,
        titulo: String,
        #[serde(default)]
        salida: String,
        #[serde(default)]
        error: bool,
        #[serde(default)]
        terminada: bool,
    },
    Error {
        texto: String,
    },
    /// Avisos del editor (compactación, recargas…).
    Nota {
        texto: String,
    },
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UsoSesion {
    pub contexto: u64,
    pub ventana: u64,
    pub entrada: u64,
    pub salida: u64,
    pub coste: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Sesion {
    pub id: String,
    pub titulo: String,
    pub backend: Backend,
    pub modelo: String,
    pub esfuerzo: String,
    pub permisos: Permisos,
    /// Id de la conversación en la CLI (`session_id`, `thread_id` o `conversation_id`).
    pub id_nativo: Option<String>,
    /// El contexto de Maker Studio ya se envió en esta conversación nativa.
    pub contexto_enviado: bool,
    /// Resumen de una compactación, para sembrar la próxima conversación nativa.
    pub semilla: Option<String>,
    pub mensajes: Vec<Mensaje>,
    pub uso: UsoSesion,
    pub creada: f64,
    pub actualizada: f64,
}

impl Default for Sesion {
    fn default() -> Self {
        let t = ahora();
        Sesion {
            id: String::new(),
            titulo: "Sesión nueva".into(),
            backend: Backend::Claude,
            modelo: String::new(),
            esfuerzo: String::new(),
            permisos: Permisos::Edicion,
            id_nativo: None,
            contexto_enviado: false,
            semilla: None,
            mensajes: Vec::new(),
            uso: UsoSesion::default(),
            creada: t,
            actualizada: t,
        }
    }
}

impl Sesion {
    pub fn nueva(backend: Backend, modelo: &str, esfuerzo: &str, permisos: Permisos) -> Sesion {
        let t = ahora();
        // Id ordenable por fecha y único aunque se creen dos en el mismo segundo.
        let id = format!("{}-{:04x}", (t * 1000.0) as u64, std::process::id() as u16 ^ ((t.fract() * 65535.0) as u16));
        Sesion { id, backend, modelo: modelo.into(), esfuerzo: esfuerzo.into(), permisos, creada: t, actualizada: t, ..Default::default() }
    }

    /// Título a partir del primer mensaje.
    pub fn titular(&mut self, texto: &str) {
        if self.titulo == "Sesión nueva" {
            let l = texto.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
            let mut t: String = l.chars().take(48).collect();
            if l.chars().count() > 48 {
                t.push('…');
            }
            if !t.is_empty() {
                self.titulo = t;
            }
        }
    }
}

/// Carpeta `<proyecto>/.maker/agente`, con un `.gitignore` que la deja fuera de git.
pub fn carpeta(proyecto: &Path) -> PathBuf {
    proyecto.join(".maker").join("agente")
}

fn carpeta_sesiones(proyecto: &Path) -> PathBuf {
    carpeta(proyecto).join("sesiones")
}

fn nombre_valido(id: &str) -> bool {
    !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

pub fn guardar(proyecto: &Path, s: &Sesion) -> Result<(), String> {
    if !nombre_valido(&s.id) {
        return Err(format!("id de sesión no válido: {}", s.id));
    }
    let dir = carpeta_sesiones(proyecto);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let gi = carpeta(proyecto).join(".gitignore");
    if !gi.exists() {
        let _ = std::fs::write(&gi, "*\n");
    }
    let tmp = dir.join(format!("{}.json.tmp", s.id));
    let json = serde_json::to_string_pretty(s).map_err(|e| e.to_string())?;
    std::fs::write(&tmp, json).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, dir.join(format!("{}.json", s.id))).map_err(|e| e.to_string())
}

/// Todas las sesiones del proyecto, la más reciente primero.
pub fn cargar_todas(proyecto: &Path) -> Vec<Sesion> {
    let mut v: Vec<Sesion> = std::fs::read_dir(carpeta_sesiones(proyecto))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| serde_json::from_str(&std::fs::read_to_string(e.path()).ok()?).ok())
        .collect();
    v.sort_by(|a: &Sesion, b: &Sesion| b.actualizada.total_cmp(&a.actualizada));
    v
}

pub fn borrar(proyecto: &Path, id: &str) {
    if nombre_valido(id) {
        let _ = std::fs::remove_file(carpeta_sesiones(proyecto).join(format!("{id}.json")));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guardar_cargar_y_borrar() {
        let dir = std::env::temp_dir().join(format!("maker-agente-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut a = Sesion::nueva(Backend::Codex, "gpt-6-sol", "low", Permisos::Lectura);
        a.titular("¿Qué hace el script Rotator?\nmás");
        a.id_nativo = Some("01a1".into());
        a.mensajes.push(Mensaje::Usuario { texto: "hola".into(), contexto: vec!["Escena".into()] });
        a.mensajes.push(Mensaje::Herramienta { id: "item_1".into(), clase: TipoHerramienta::Comando, titulo: "ls".into(), salida: "a".into(), error: false, terminada: true });
        guardar(&dir, &a).unwrap();
        let mut b = Sesion::nueva(Backend::Claude, "", "", Permisos::Edicion);
        b.id.push('b');
        b.actualizada = a.actualizada + 10.0;
        guardar(&dir, &b).unwrap();
        assert_eq!(std::fs::read_to_string(carpeta(&dir).join(".gitignore")).unwrap(), "*\n");
        let todas = cargar_todas(&dir);
        assert_eq!(todas.len(), 2);
        assert_eq!(todas[0].id, b.id, "la más reciente primero");
        assert!((todas[1].creada - a.creada).abs() < 1e-3);
        assert_eq!(Sesion { creada: a.creada, actualizada: a.actualizada, ..todas[1].clone() }, a);
        assert_eq!(todas[1].titulo, "¿Qué hace el script Rotator?");
        borrar(&dir, &a.id);
        assert_eq!(cargar_todas(&dir).len(), 1);
        assert!(guardar(&dir, &Sesion { id: "../x".into(), ..Default::default() }).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
