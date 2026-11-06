//! Plantillas de interfaz (`ui/plantillas/*.mui`): un subárbol guardado como
//! asset que se instancia en escenas (como un *prefab* de Unity o una escena
//! instanciada de Godot).
//!
//! Las entidades instanciadas se guardan enteras en la escena (el juego no
//! necesita la plantilla para cargarla) con su id dentro de la plantilla
//! ([`OrigenPlantilla`]). La raíz lleva [`InstanciaUi`] con las propiedades
//! retocadas en esta instancia; al sincronizar, todo lo demás se pone al día
//! con la plantilla.

use super::{InstanciaUi, OrigenPlantilla};
use crate::components::EntityId;
use crate::scene::{EntityRecord, Scene};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlantillaUi {
    pub formato: String,
    pub version: u32,
    /// La raíz primero (sin padre); ids locales de la plantilla.
    pub entidades: Vec<EntityRecord>,
}

pub const FORMATO: &str = "maker-ui";

impl PlantillaUi {
    /// Subárbol de `raiz` con ids locales 1, 2, 3… en orden de la jerarquía.
    pub fn desde_escena(scene: &Scene, raiz: EntityId) -> Option<PlantillaUi> {
        let h = scene.hierarchy();
        let mut orden = vec![raiz];
        let mut i = 0;
        while i < orden.len() {
            orden.extend_from_slice(h.children_of(orden[i]));
            i += 1;
        }
        let mapa: HashMap<EntityId, EntityId> = orden.iter().enumerate().map(|(i, id)| (*id, EntityId(i as u64 + 1))).collect();
        let mut entidades = Vec::with_capacity(orden.len());
        for id in orden {
            let mut r = scene.record(id)?;
            r.id = mapa[&id];
            r.parent = if id == raiz { None } else { r.parent.and_then(|p| mapa.get(&p).copied()) };
            r.plantilla = None;
            r.origen = None;
            entidades.push(r);
        }
        Some(PlantillaUi { formato: FORMATO.into(), version: 1, entidades })
    }
    pub fn cargar(ruta: &Path) -> Result<PlantillaUi, String> {
        let t = std::fs::read_to_string(ruta).map_err(|e| format!("{}: {e}", ruta.display()))?;
        let p: PlantillaUi = serde_json::from_str(&t).map_err(|e| format!("{}: {e}", ruta.display()))?;
        if p.formato != FORMATO {
            return Err(format!("{}: formato desconocido {}", ruta.display(), p.formato));
        }
        if p.entidades.is_empty() {
            return Err(format!("{}: plantilla vacía", ruta.display()));
        }
        Ok(p)
    }
    pub fn guardar(&self, ruta: &Path) -> Result<(), String> {
        if let Some(d) = ruta.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        std::fs::write(ruta, serde_json::to_string_pretty(self).map_err(|e| e.to_string())?).map_err(|e| format!("{}: {e}", ruta.display()))
    }

    /// Crea una instancia bajo `padre`. Devuelve la raíz.
    pub fn instanciar(&self, scene: &mut Scene, ruta: &str, padre: Option<EntityId>) -> Option<EntityId> {
        let mut mapa: HashMap<EntityId, EntityId> = HashMap::new();
        for r in &self.entidades {
            mapa.insert(r.id, scene.new_id());
        }
        let mut raiz = None;
        for (i, r) in self.entidades.iter().enumerate() {
            let mut n = r.clone();
            n.id = mapa[&r.id];
            n.origen = Some(OrigenPlantilla(r.id.0));
            if i == 0 {
                n.parent = padre;
                n.plantilla = Some(InstanciaUi { plantilla: ruta.to_string(), overrides: BTreeSet::new() });
                n.sibling = padre.map(|p| scene.hierarchy().children_of(p).len() as i64).unwrap_or(scene.len() as i64);
                raiz = Some(n.id);
            } else {
                n.parent = r.parent.and_then(|p| mapa.get(&p).copied());
            }
            scene.spawn_from_record(&n);
        }
        scene.update_transforms();
        raiz
    }
}

/// Campos que no son propiedades editables (estructura e identidad).
const IGNORADOS: [&str; 5] = ["id", "parent", "sibling", "plantilla", "origen"];

/// Hojas del JSON de un registro: `campo.subcampo…` → valor.
pub fn hojas(r: &EntityRecord) -> BTreeMap<String, Value> {
    fn rec(prefijo: &str, v: &Value, out: &mut BTreeMap<String, Value>) {
        match v {
            Value::Object(m) if !m.is_empty() => {
                for (k, x) in m {
                    let p = if prefijo.is_empty() { k.clone() } else { format!("{prefijo}.{k}") };
                    rec(&p, x, out);
                }
            }
            _ => {
                out.insert(prefijo.to_string(), v.clone());
            }
        }
    }
    let mut out = BTreeMap::new();
    if let Value::Object(m) = serde_json::to_value(r).unwrap_or(Value::Null) {
        for (k, v) in m.iter().filter(|(k, _)| !IGNORADOS.contains(&k.as_str())) {
            rec(k, v, &mut out);
        }
    }
    out
}

fn poner(v: &mut Value, ruta: &str, x: Value) {
    let mut cur = v;
    let partes: Vec<&str> = ruta.split('.').collect();
    for (i, p) in partes.iter().enumerate() {
        if !cur.is_object() {
            *cur = Value::Object(Map::new());
        }
        let m = cur.as_object_mut().unwrap();
        if i + 1 == partes.len() {
            m.insert(p.to_string(), x);
            return;
        }
        cur = m.entry(p.to_string()).or_insert(Value::Object(Map::new()));
    }
}

fn quitar(v: &mut Value, ruta: &str) {
    let partes: Vec<&str> = ruta.split('.').collect();
    let mut cur = v;
    for (i, p) in partes.iter().enumerate() {
        let Some(m) = cur.as_object_mut() else { return };
        if i + 1 == partes.len() {
            m.remove(*p);
            return;
        }
        match m.get_mut(*p) {
            Some(x) => cur = x,
            None => return,
        }
    }
}

/// Entidades de la instancia por id local.
fn por_origen(scene: &Scene, raiz: EntityId) -> BTreeMap<u64, EntityId> {
    let h = scene.hierarchy();
    let mut out = BTreeMap::new();
    let mut pila = vec![raiz];
    while let Some(id) = pila.pop() {
        if let Some(o) = scene.get::<OrigenPlantilla>(id) {
            out.insert(o.0, id);
        }
        pila.extend_from_slice(h.children_of(id));
    }
    out
}

/// Propiedades de la instancia que difieren de la plantilla (`<local>/<campo>`).
pub fn diferencias(scene: &Scene, raiz: EntityId, p: &PlantillaUi) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let inst = por_origen(scene, raiz);
    for r in &p.entidades {
        let Some(&id) = inst.get(&r.id.0) else { continue };
        let Some(ri) = scene.record(id) else { continue };
        let (a, b) = (hojas(r), hojas(&ri));
        for k in a.keys().chain(b.keys()) {
            if a.get(k) != b.get(k) {
                out.insert(format!("{}/{k}", r.id.0));
            }
        }
    }
    out
}

/// Recalcula los retoques de una instancia (tras editarla en el editor).
pub fn actualizar_overrides(scene: &mut Scene, raiz: EntityId, p: &PlantillaUi) -> bool {
    let d = diferencias(scene, raiz, p);
    match scene.get_mut::<InstanciaUi>(raiz) {
        Some(mut i) if i.overrides != d => {
            i.overrides = d;
            true
        }
        _ => false,
    }
}

/// Pone la instancia al día con la plantilla, respetando sus retoques: crea
/// las entidades nuevas y quita las que la plantilla ya no tiene.
pub fn sincronizar(scene: &mut Scene, raiz: EntityId, p: &PlantillaUi) {
    let Some(inst_ui) = scene.get::<InstanciaUi>(raiz) else { return };
    let ov = &inst_ui.overrides;
    let mut inst = por_origen(scene, raiz);
    // Quitar las que ya no existen en la plantilla.
    let locales: BTreeSet<u64> = p.entidades.iter().map(|r| r.id.0).collect();
    for (local, id) in inst.clone() {
        if !locales.contains(&local) && id != raiz && scene.contains(id) {
            scene.despawn_recursive(id);
            inst.remove(&local);
        }
    }
    for r in &p.entidades {
        let local = r.id.0;
        match inst.get(&local).copied() {
            Some(id) => {
                let Some(ri) = scene.record(id) else { continue };
                let mut v = serde_json::to_value(&ri).unwrap_or(Value::Null);
                let (hp, hi) = (hojas(r), hojas(&ri));
                for (k, x) in &hp {
                    if !ov.contains(&format!("{local}/{k}")) && hi.get(k) != Some(x) {
                        poner(&mut v, k, x.clone());
                    }
                }
                for k in hi.keys() {
                    if !hp.contains_key(k) && !ov.contains(&format!("{local}/{k}")) {
                        quitar(&mut v, k);
                    }
                }
                if let Ok(nr) = serde_json::from_value::<EntityRecord>(v)
                    && nr != ri
                {
                    scene.reemplazar(&nr);
                }
            }
            None => {
                // Nueva en la plantilla: bajo su padre de la instancia.
                let padre = r.parent.and_then(|pl| inst.get(&pl.0).copied()).or(Some(raiz));
                let mut n = r.clone();
                n.id = scene.new_id();
                n.parent = padre;
                n.origen = Some(OrigenPlantilla(local));
                let id = scene.spawn_from_record(&n);
                inst.insert(local, id);
            }
        }
    }
    scene.update_transforms();
}

/// Sincroniza todas las instancias de la escena con sus plantillas.
pub fn sincronizar_todas(scene: &mut Scene, raiz_proyecto: &Path) -> usize {
    let raices: Vec<(EntityId, String)> = scene.world.query::<(&EntityId, &InstanciaUi)>().iter().map(|(id, i)| (*id, i.plantilla.clone())).collect();
    let mut n = 0;
    let mut cache: HashMap<String, Option<PlantillaUi>> = HashMap::new();
    for (id, ruta) in raices {
        let p = cache.entry(ruta.clone()).or_insert_with(|| match PlantillaUi::cargar(&raiz_proyecto.join(&ruta)) {
            Ok(p) => Some(p),
            Err(e) => {
                crate::log::warn("interfaz", e);
                None
            }
        });
        if let Some(p) = p {
            sincronizar(scene, id, p);
            n += 1;
        }
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::*;

    fn menu(s: &mut Scene) -> EntityId {
        let raiz = s.spawn("Menú");
        s.insert(raiz, RectUi::centrado(300.0, 200.0));
        s.insert(raiz, WidgetUi::nuevo(TipoWidget::Panel));
        let b = s.spawn("Botón");
        s.insert(b, RectUi::centrado(200.0, 50.0));
        s.insert(b, WidgetUi::nuevo(TipoWidget::Boton));
        s.set_parent(b, Some(raiz), false);
        raiz
    }

    #[test]
    fn guardar_instanciar_y_sincronizar_con_retoques() {
        let mut s = Scene::new();
        let lienzo = s.spawn("Lienzo");
        s.insert(lienzo, Lienzo::default());
        let raiz = menu(&mut s);
        let p = PlantillaUi::desde_escena(&s, raiz).unwrap();
        assert_eq!(p.entidades.len(), 2);
        assert_eq!(p.entidades[0].parent, None);
        assert_eq!(p.entidades[1].parent, Some(EntityId(1)));
        // Ida y vuelta por JSON.
        let txt = serde_json::to_string(&p).unwrap();
        assert_eq!(serde_json::from_str::<PlantillaUi>(&txt).unwrap(), p);

        let a = p.instanciar(&mut s, "ui/plantillas/menu.mui", Some(lienzo)).unwrap();
        let b = p.instanciar(&mut s, "ui/plantillas/menu.mui", Some(lienzo)).unwrap();
        assert_ne!(a, b);
        assert_eq!(s.parent(a), Some(lienzo));
        let hijo_a = s.hierarchy().children_of(a)[0];
        assert!(diferencias(&s, a, &p).is_empty());
        // Retoque por instancia: el texto del botón de «a».
        s.get_mut::<WidgetUi>(hijo_a).unwrap().clase = ClaseWidget::Boton(Boton { texto: "Salir".into() });
        assert!(actualizar_overrides(&mut s, a, &p));
        let ov = s.get::<InstanciaUi>(a).unwrap().overrides;
        assert_eq!(ov.iter().collect::<Vec<_>>(), vec!["2/widget.clase.Boton.texto"]);
        // La plantilla cambia: color local del botón y un hijo nuevo.
        let mut p2 = p.clone();
        p2.entidades[1].widget.as_mut().unwrap().clase = ClaseWidget::Boton(Boton { texto: "Jugar".into() });
        p2.entidades[1].widget.as_mut().unwrap().estilo = "grande".into();
        let mut nuevo = p2.entidades[1].clone();
        nuevo.id = EntityId(3);
        nuevo.name = "Otro".into();
        p2.entidades.push(nuevo);
        sincronizar(&mut s, a, &p2);
        sincronizar(&mut s, b, &p2);
        let w = s.get::<WidgetUi>(hijo_a).unwrap();
        assert_eq!(w.clase.texto(), Some("Salir"), "el retoque se respeta");
        assert_eq!(w.estilo, "grande", "lo no retocado se actualiza");
        let hijo_b = s.hierarchy().children_of(b)[0];
        assert_eq!(s.get::<WidgetUi>(hijo_b).unwrap().clase.texto(), Some("Jugar"));
        assert_eq!(s.hierarchy().children_of(a).len(), 2, "entidad nueva de la plantilla");
        // Y si la plantilla la quita, desaparece.
        sincronizar(&mut s, a, &p);
        assert_eq!(s.hierarchy().children_of(a).len(), 1);
        // La escena con instancias se guarda y carga sin la plantilla.
        let s2 = Scene::from_json(&s.to_json()).unwrap();
        assert_eq!(s2.get::<InstanciaUi>(a).unwrap().plantilla, "ui/plantillas/menu.mui");
    }
}
