//! Orden de escenas del proyecto (como *Scenes In Build* de Unity): lista
//! ordenada en `project.maker`; la primera activa es la de arranque. Los
//! scripts cambian de escena por nombre, índice o siguiente/anterior dentro
//! de las escenas activas; el cambio se aplica al terminar el fotograma.

use serde::{Deserialize, Serialize};
use std::path::Path;

/// Una fila del orden de escenas.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EntradaEscena {
    /// Ruta relativa a la raíz del proyecto (`scenes/nivel1.mscene`).
    pub ruta: String,
    /// Las inactivas no se exportan ni cuentan para los índices de los scripts.
    #[serde(default = "si")]
    pub activa: bool,
}

fn si() -> bool {
    true
}

impl EntradaEscena {
    pub fn nueva(ruta: &str) -> Self {
        Self { ruta: normalizar(ruta), activa: true }
    }
}

/// Ruta relativa con `/` y sin `./` inicial.
pub fn normalizar(ruta: &str) -> String {
    let r = ruta.trim().replace('\\', "/");
    r.trim_start_matches("./").to_string()
}

/// Nombre corto de una escena: el nombre del archivo sin extensión.
pub fn nombre(ruta: &str) -> &str {
    let f = ruta.rsplit('/').next().unwrap_or(ruta);
    f.strip_suffix(&format!(".{}", crate::project::SCENE_EXT)).unwrap_or(f)
}

/// Rutas de las escenas activas, en orden.
pub fn activas(lista: &[EntradaEscena]) -> Vec<String> {
    lista.iter().filter(|e| e.activa).map(|e| e.ruta.clone()).collect()
}

/// ¿Falta el archivo de la escena?
pub fn falta(raiz: &Path, ruta: &str) -> bool {
    !raiz.join(ruta).is_file()
}

/// Mueve la fila `de` a la posición `a` (índice de destino en la lista final).
pub fn mover(lista: &mut Vec<EntradaEscena>, de: usize, a: usize) {
    if de >= lista.len() {
        return;
    }
    let e = lista.remove(de);
    lista.insert(a.min(lista.len()), e);
}

/// Mueve la fila `de` para que quede justo antes de la fila `antes` actual
/// (`antes == lista.len()`: al final). Es lo que hace soltar una fila arrastrada.
pub fn mover_antes(lista: &mut Vec<EntradaEscena>, de: usize, antes: usize) {
    mover(lista, de, if de < antes { antes - 1 } else { antes });
}

/// Inserta `ruta` en `pos` (o la mueve allí si ya estaba) y la deja activa.
pub fn insertar(lista: &mut Vec<EntradaEscena>, ruta: &str, pos: usize) {
    let ruta = normalizar(ruta);
    let pos = match lista.iter().position(|e| e.ruta == ruta) {
        Some(i) => {
            let mut e = lista.remove(i);
            e.activa = true;
            let p = if i < pos { pos - 1 } else { pos };
            lista.insert(p.min(lista.len()), e);
            return;
        }
        None => pos.min(lista.len()),
    };
    lista.insert(pos, EntradaEscena::nueva(&ruta));
}

/// Pide un cambio de escena desde un script.
#[derive(Clone, Debug, PartialEq)]
pub enum Peticion {
    Nombre(String),
    Indice(i64),
    Siguiente,
    Anterior,
    Recargar,
}

/// Resuelve una petición contra las escenas activas. `actual` es la ruta de
/// la escena que se está jugando. Devuelve la ruta a cargar o el motivo por el
/// que no se hace nada (para avisar en la consola).
///
/// - Por nombre: ruta relativa (con o sin `.mscene`) o nombre del archivo,
///   primero exacto y luego sin distinguir mayúsculas. Solo escenas activas.
/// - Siguiente/anterior: en los extremos no hace nada (no da la vuelta).
/// - Fuera de la lista (Play de una escena que no está en el orden), siguiente
///   y anterior no hacen nada; por nombre e índice sí funcionan.
pub fn resolver(activas: &[String], actual: Option<&str>, p: &Peticion) -> Result<String, String> {
    let idx = actual.and_then(|a| indice(activas, a));
    match p {
        Peticion::Recargar => actual.map(str::to_string).ok_or_else(|| "no hay escena que recargar".into()),
        Peticion::Indice(i) => usize::try_from(*i).ok().and_then(|i| activas.get(i)).cloned().ok_or_else(|| format!("no hay escena con índice {i} (hay {})", activas.len())),
        Peticion::Siguiente | Peticion::Anterior => {
            let Some(i) = idx else { return Err("la escena actual no está en el orden de escenas".into()) };
            let j = if *p == Peticion::Siguiente { i.checked_add(1) } else { i.checked_sub(1) };
            j.and_then(|j| activas.get(j)).cloned().ok_or_else(|| if *p == Peticion::Siguiente { "ya es la última escena".into() } else { "ya es la primera escena".into() })
        }
        Peticion::Nombre(n) => buscar(activas, n).ok_or_else(|| format!("no hay ninguna escena activa llamada '{n}' en el orden de escenas")),
    }
}

/// Posición de `ruta` entre las activas.
pub fn indice(activas: &[String], ruta: &str) -> Option<usize> {
    let r = normalizar(ruta);
    activas.iter().position(|a| *a == r)
}

fn buscar(activas: &[String], n: &str) -> Option<String> {
    let n = normalizar(n);
    let con_ext = if n.ends_with(&format!(".{}", crate::project::SCENE_EXT)) { n.clone() } else { format!("{n}.{}", crate::project::SCENE_EXT) };
    let iguales = |f: &dyn Fn(&str, &str) -> bool| activas.iter().find(|a| f(a, &con_ext) || f(nombre(a), &n)).cloned();
    iguales(&|a, b| a == b).or_else(|| iguales(&|a, b| a.eq_ignore_ascii_case(b)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lista() -> Vec<String> {
        vec!["scenes/menu.mscene".into(), "scenes/Nivel1.mscene".into(), "niveles/Nivel2.mscene".into()]
    }

    #[test]
    fn resuelve_nombres_rutas_e_indices() {
        let l = lista();
        let a = Some("scenes/Nivel1.mscene");
        assert_eq!(resolver(&l, a, &Peticion::Nombre("Nivel2".into())).unwrap(), "niveles/Nivel2.mscene");
        assert_eq!(resolver(&l, a, &Peticion::Nombre("nivel2".into())).unwrap(), "niveles/Nivel2.mscene", "sin mayúsculas como segunda opción");
        assert_eq!(resolver(&l, a, &Peticion::Nombre("niveles/Nivel2".into())).unwrap(), "niveles/Nivel2.mscene");
        assert_eq!(resolver(&l, a, &Peticion::Nombre("./scenes/menu.mscene".into())).unwrap(), "scenes/menu.mscene");
        assert!(resolver(&l, a, &Peticion::Nombre("Nivel3".into())).is_err());
        assert_eq!(resolver(&l, a, &Peticion::Indice(0)).unwrap(), "scenes/menu.mscene");
        assert!(resolver(&l, a, &Peticion::Indice(3)).is_err());
        assert!(resolver(&l, a, &Peticion::Indice(-1)).is_err());
        assert_eq!(resolver(&l, a, &Peticion::Recargar).unwrap(), "scenes/Nivel1.mscene");
    }

    #[test]
    fn siguiente_y_anterior_no_dan_la_vuelta() {
        let l = lista();
        assert_eq!(resolver(&l, Some("scenes/menu.mscene"), &Peticion::Siguiente).unwrap(), "scenes/Nivel1.mscene");
        assert_eq!(resolver(&l, Some("scenes/Nivel1.mscene"), &Peticion::Anterior).unwrap(), "scenes/menu.mscene");
        assert!(resolver(&l, Some("niveles/Nivel2.mscene"), &Peticion::Siguiente).is_err());
        assert!(resolver(&l, Some("scenes/menu.mscene"), &Peticion::Anterior).is_err());
        assert!(resolver(&l, Some("scenes/otra.mscene"), &Peticion::Siguiente).is_err(), "fuera de la lista");
        assert_eq!(resolver(&l, Some("scenes/otra.mscene"), &Peticion::Indice(1)).unwrap(), "scenes/Nivel1.mscene");
    }

    #[test]
    fn mover_e_insertar() {
        let mut l: Vec<EntradaEscena> = ["a", "b", "c"].iter().map(|r| EntradaEscena::nueva(&format!("{r}.mscene"))).collect();
        mover(&mut l, 0, 2);
        assert_eq!(activas(&l), ["b.mscene", "c.mscene", "a.mscene"]);
        insertar(&mut l, "a.mscene", 0);
        assert_eq!(activas(&l), ["a.mscene", "b.mscene", "c.mscene"]);
        l[1].activa = false;
        insertar(&mut l, "b.mscene", 3);
        assert_eq!(activas(&l), ["a.mscene", "c.mscene", "b.mscene"], "insertar reactiva y mueve");
        insertar(&mut l, "d.mscene", 1);
        assert_eq!(activas(&l), ["a.mscene", "d.mscene", "c.mscene", "b.mscene"]);
        assert_eq!(nombre("x/Nivel2.mscene"), "Nivel2");
        mover_antes(&mut l, 0, 2);
        assert_eq!(activas(&l), ["d.mscene", "a.mscene", "c.mscene", "b.mscene"]);
        mover_antes(&mut l, 3, 0);
        assert_eq!(activas(&l), ["b.mscene", "d.mscene", "a.mscene", "c.mscene"]);
        mover_antes(&mut l, 0, 4);
        assert_eq!(activas(&l), ["d.mscene", "a.mscene", "c.mscene", "b.mscene"]);
    }
}
