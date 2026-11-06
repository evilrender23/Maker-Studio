//! Escena: un `hecs::World` (ECS por arquetipos, iteración contigua en memoria)
//! más un índice de ids estables y el entorno. Formato de disco: JSON legible
//! (`.mscene`) apto para control de versiones, como `.tscn` de Godot.

use crate::components::*;
use crate::ui;
use glam::{Mat4, Quat, Vec3};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// Cielo, luz ambiente, niebla, exposición, gravedad, sombreado global y
/// posprocesado. Los campos que faltan en escenas antiguas toman su valor por
/// defecto.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Environment {
    pub sky_top: [f32; 3],
    pub sky_horizon: [f32; 3],
    pub ground: [f32; 3],
    pub ambient_intensity: f32,
    pub fog_color: [f32; 3],
    pub fog_density: f32,
    pub exposure: f32,
    pub gravity: [f32; 3],
    /// Modelo de sombreado forzado en todos los materiales.
    pub global_shading: crate::postproceso::GlobalShading,
    /// Pila ordenada de efectos de posprocesado.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub post: Vec<crate::postproceso::PostEffect>,
    /// Iluminación global dinámica (desactivada en escenas antiguas).
    #[serde(skip_serializing_if = "crate::iluminacion::IluminacionGlobal::es_defecto")]
    pub iluminacion_global: crate::iluminacion::IluminacionGlobal,
}

impl Default for Environment {
    fn default() -> Self {
        Self {
            sky_top: [0.36, 0.55, 0.92],
            sky_horizon: [0.84, 0.89, 0.97],
            ground: [0.42, 0.40, 0.38],
            ambient_intensity: 0.55,
            fog_color: [0.84, 0.89, 0.97],
            fog_density: 0.004,
            exposure: 1.0,
            gravity: [0.0, -9.81, 0.0],
            global_shading: Default::default(),
            post: Vec::new(),
            iluminacion_global: Default::default(),
        }
    }
}

/// Registro de una entidad en disco.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EntityRecord {
    pub id: EntityId,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<EntityId>,
    #[serde(default)]
    pub sibling: i64,
    #[serde(default = "yes")]
    pub active: bool,
    pub transform: Transform,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mesh: Option<MeshRenderer>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub light: Option<Light>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub camera: Option<Camera>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rigid_body: Option<RigidBody>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collider: Option<Collider>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scripts: Option<Scripts>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub animator: Option<Animator>,
    // Interfaz.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lienzo: Option<ui::Lienzo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rect_ui: Option<ui::RectUi>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contenedor: Option<ui::Contenedor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub widget: Option<ui::WidgetUi>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plantilla: Option<ui::InstanciaUi>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origen: Option<ui::OrigenPlantilla>,
}

fn yes() -> bool {
    true
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SceneFile {
    pub format: String,
    pub version: u32,
    pub environment: Environment,
    pub entities: Vec<EntityRecord>,
}

/// Padres → hijos ordenados. Se calcula una vez y se reutiliza (O(n)).
#[derive(Clone, Debug, Default)]
pub struct Hierarchy {
    pub roots: Vec<EntityId>,
    pub children: HashMap<EntityId, Vec<EntityId>>,
}

impl Hierarchy {
    pub fn children_of(&self, id: EntityId) -> &[EntityId] {
        self.children.get(&id).map(|v| v.as_slice()).unwrap_or(&[])
    }
    /// Recorrido en profundidad (orden de la jerarquía).
    pub fn depth_first(&self) -> Vec<(EntityId, usize)> {
        let mut out = Vec::new();
        let mut stack: Vec<(EntityId, usize)> = self.roots.iter().rev().map(|&r| (r, 0)).collect();
        while let Some((id, d)) = stack.pop() {
            out.push((id, d));
            for &c in self.children_of(id).iter().rev() {
                stack.push((c, d + 1));
            }
        }
        out
    }
}

pub struct Scene {
    pub world: hecs::World,
    pub env: Environment,
    index: HashMap<EntityId, hecs::Entity>,
    next_id: u64,
}

impl Default for Scene {
    fn default() -> Self {
        Self::new()
    }
}

impl Scene {
    pub fn new() -> Self {
        Self { world: hecs::World::new(), env: Environment::default(), index: HashMap::new(), next_id: 1 }
    }

    /// Escena inicial: cámara, sol, suelo y un cubo.
    pub fn starter() -> Self {
        let mut s = Scene::new();
        let cam = s.spawn("Cámara principal");
        s.set_transform(cam, Transform::from_translation(Vec3::new(0.0, 3.0, 8.0)).with_rotation(Quat::from_rotation_x(-0.3)));
        s.insert(cam, Camera::default());
        let sun = s.spawn("Sol");
        let mut t = Transform::from_translation(Vec3::new(0.0, 10.0, 0.0));
        t.set_euler_degrees(Vec3::new(-50.0, -35.0, 0.0));
        s.set_transform(sun, t);
        s.insert(sun, Light::directional());
        let ground = s.spawn("Suelo");
        s.set_transform(ground, Transform::default().with_scale(Vec3::new(20.0, 1.0, 20.0)));
        let mut mr = MeshRenderer::primitive(PrimitiveShape::Plane);
        mr.material = Material::color([0.62, 0.64, 0.68]);
        s.insert(ground, mr);
        s.insert(ground, RigidBody::fixed());
        s.insert(ground, Collider::new(ColliderShape::fit(PrimitiveShape::Plane)));
        let cube = s.spawn("Cubo");
        s.set_transform(cube, Transform::from_translation(Vec3::new(0.0, 0.5, 0.0)));
        let mut mr = MeshRenderer::primitive(PrimitiveShape::Cube);
        mr.material = Material::color([0.0, 0.33, 0.99]);
        s.insert(cube, mr);
        s.update_transforms();
        s
    }

    pub fn len(&self) -> usize {
        self.index.len()
    }
    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    fn alloc_id(&mut self) -> EntityId {
        let id = EntityId(self.next_id);
        self.next_id += 1;
        id
    }

    pub fn spawn(&mut self, name: &str) -> EntityId {
        let id = self.alloc_id();
        let sib = self.index.len() as i64;
        let e = self.world.spawn((id, Name(name.to_string()), Transform::default(), GlobalTransform::default(), SiblingIndex(sib)));
        self.index.insert(id, e);
        id
    }

    pub fn entity(&self, id: EntityId) -> Option<hecs::Entity> {
        self.index.get(&id).copied()
    }
    pub fn contains(&self, id: EntityId) -> bool {
        self.index.contains_key(&id)
    }
    pub fn ids(&self) -> impl Iterator<Item = EntityId> + '_ {
        self.index.keys().copied()
    }

    pub fn insert<C: hecs::Component>(&mut self, id: EntityId, c: C) {
        if let Some(e) = self.entity(id) {
            let _ = self.world.insert_one(e, c);
        }
    }
    pub fn remove<C: hecs::Component>(&mut self, id: EntityId) {
        if let Some(e) = self.entity(id) {
            let _ = self.world.remove_one::<C>(e);
        }
    }
    pub fn has<C: hecs::Component>(&self, id: EntityId) -> bool {
        self.entity(id).is_some_and(|e| self.world.satisfies::<&C>(e))
    }
    pub fn get<C: hecs::Component + Clone>(&self, id: EntityId) -> Option<C> {
        let e = self.entity(id)?;
        self.world.get::<&C>(e).ok().map(|c| (*c).clone())
    }
    pub fn get_mut<C: hecs::Component>(&self, id: EntityId) -> Option<hecs::RefMut<'_, C>> {
        let e = self.entity(id)?;
        self.world.get::<&mut C>(e).ok()
    }

    pub fn name(&self, id: EntityId) -> String {
        self.get::<Name>(id).map(|n| n.0).unwrap_or_default()
    }
    pub fn set_name(&mut self, id: EntityId, name: &str) {
        if let Some(mut n) = self.get_mut::<Name>(id) {
            n.0 = name.to_string();
        }
    }
    pub fn transform(&self, id: EntityId) -> Transform {
        self.get::<Transform>(id).unwrap_or_default()
    }
    pub fn set_transform(&mut self, id: EntityId, t: Transform) {
        if let Some(mut c) = self.get_mut::<Transform>(id) {
            *c = t;
        }
    }
    pub fn parent(&self, id: EntityId) -> Option<EntityId> {
        self.get::<Parent>(id).map(|p| p.0)
    }
    pub fn is_active(&self, id: EntityId) -> bool {
        !self.has::<Inactive>(id)
    }
    /// Activo en la jerarquía (él y todos sus ancestros).
    pub fn is_active_in_hierarchy(&self, id: EntityId) -> bool {
        let mut cur = Some(id);
        while let Some(c) = cur {
            if !self.is_active(c) {
                return false;
            }
            cur = self.parent(c);
        }
        true
    }
    pub fn global(&self, id: EntityId) -> Mat4 {
        self.get::<GlobalTransform>(id).map(|g| g.0).unwrap_or(Mat4::IDENTITY)
    }

    pub fn find_by_name(&self, name: &str) -> Option<EntityId> {
        let mut q = self.world.query::<(&EntityId, &Name)>();
        let mut best: Option<EntityId> = None;
        for (id, n) in q.iter() {
            if n.0 == name && best.is_none_or(|b| id.0 < b.0) {
                best = Some(*id);
            }
        }
        best
    }

    /// ¿`ancestor` es antepasado de `id`?
    pub fn is_ancestor(&self, ancestor: EntityId, id: EntityId) -> bool {
        let mut cur = self.parent(id);
        let mut guard = 0;
        while let Some(c) = cur {
            if c == ancestor {
                return true;
            }
            cur = self.parent(c);
            guard += 1;
            if guard > 100_000 {
                return false;
            }
        }
        false
    }

    pub fn hierarchy(&self) -> Hierarchy {
        let mut items: Vec<(EntityId, Option<EntityId>, i64)> = Vec::with_capacity(self.index.len());
        let mut q = self.world.query::<(&EntityId, Option<&Parent>, Option<&SiblingIndex>)>();
        for (id, p, s) in q.iter() {
            let parent = p.map(|p| p.0).filter(|p| self.index.contains_key(p));
            items.push((*id, parent, s.map(|s| s.0).unwrap_or(0)));
        }
        items.sort_by_key(|&(id, _, s)| (s, id.0));
        let mut h = Hierarchy::default();
        for (id, parent, _) in items {
            match parent {
                Some(p) => h.children.entry(p).or_default().push(id),
                None => h.roots.push(id),
            }
        }
        h
    }

    /// Cambia el padre. Con `keep_world` conserva la posición en el mundo
    /// (comportamiento por defecto al arrastrar en la jerarquía de Unity).
    pub fn set_parent(&mut self, child: EntityId, parent: Option<EntityId>, keep_world: bool) -> bool {
        if let Some(p) = parent
            && (p == child || self.is_ancestor(child, p) || !self.contains(p))
        {
            return false;
        }
        self.update_transforms();
        let world = self.global(child);
        match parent {
            Some(p) => self.insert(child, Parent(p)),
            None => self.remove::<Parent>(child),
        }
        let last = self.hierarchy_siblings(parent).iter().filter(|&&s| s != child).map(|&s| self.sibling(s)).max().unwrap_or(-1);
        self.insert(child, SiblingIndex(last + 1));
        if keep_world {
            let pm = parent.map(|p| self.global(p)).unwrap_or(Mat4::IDENTITY);
            let local = pm.inverse() * world;
            let t = Transform::from_matrix(local);
            self.set_transform(child, t);
        }
        self.update_transforms();
        true
    }

    fn sibling(&self, id: EntityId) -> i64 {
        self.get::<SiblingIndex>(id).map(|s| s.0).unwrap_or(0)
    }

    fn hierarchy_siblings(&self, parent: Option<EntityId>) -> Vec<EntityId> {
        let h = self.hierarchy();
        match parent {
            Some(p) => h.children_of(p).to_vec(),
            None => h.roots,
        }
    }

    /// Mueve `id` justo antes de `before` (mismo padre que `before`).
    pub fn move_before(&mut self, id: EntityId, before: EntityId) -> bool {
        if id == before || self.is_ancestor(id, before) {
            return false;
        }
        let parent = self.parent(before);
        if self.parent(id) != parent && !self.set_parent(id, parent, true) {
            return false;
        }
        let mut sibs: Vec<EntityId> = self.hierarchy_siblings(parent).into_iter().filter(|&s| s != id).collect();
        let pos = sibs.iter().position(|&s| s == before).unwrap_or(sibs.len());
        sibs.insert(pos, id);
        for (i, s) in sibs.into_iter().enumerate() {
            self.insert(s, SiblingIndex(i as i64));
        }
        true
    }

    pub fn despawn_recursive(&mut self, id: EntityId) {
        let h = self.hierarchy();
        let mut stack = vec![id];
        while let Some(c) = stack.pop() {
            stack.extend_from_slice(h.children_of(c));
            if let Some(e) = self.index.remove(&c) {
                let _ = self.world.despawn(e);
            }
        }
    }

    /// Calcula `GlobalTransform` de todo el árbol en O(n).
    pub fn update_transforms(&mut self) {
        let h = self.hierarchy();
        let mut globals: HashMap<EntityId, Mat4> = HashMap::with_capacity(self.index.len());
        for (id, _) in h.depth_first() {
            let local = self.transform(id).matrix();
            let m = match self.parent(id).and_then(|p| globals.get(&p)) {
                Some(pm) => *pm * local,
                None => local,
            };
            globals.insert(id, m);
        }
        for (id, g) in self.world.query_mut::<(&EntityId, &mut GlobalTransform)>() {
            if let Some(m) = globals.get(id) {
                g.0 = *m;
            }
        }
    }

    /// Transform local a partir de una matriz de mundo deseada.
    pub fn set_world_matrix(&mut self, id: EntityId, world: Mat4) {
        let pm = self.parent(id).map(|p| self.global(p)).unwrap_or(Mat4::IDENTITY);
        self.set_transform(id, Transform::from_matrix(pm.inverse() * world));
    }

    pub fn record(&self, id: EntityId) -> Option<EntityRecord> {
        self.entity(id)?;
        Some(EntityRecord {
            id,
            name: self.name(id),
            parent: self.parent(id),
            sibling: self.sibling(id),
            active: self.is_active(id),
            transform: self.transform(id),
            mesh: self.get::<MeshRenderer>(id),
            light: self.get::<Light>(id),
            camera: self.get::<Camera>(id),
            rigid_body: self.get::<RigidBody>(id),
            collider: self.get::<Collider>(id),
            scripts: self.get::<Scripts>(id),
            animator: self.get::<Animator>(id),
            lienzo: self.get::<ui::Lienzo>(id),
            rect_ui: self.get::<ui::RectUi>(id),
            contenedor: self.get::<ui::Contenedor>(id),
            widget: self.get::<ui::WidgetUi>(id),
            plantilla: self.get::<ui::InstanciaUi>(id),
            origen: self.get::<ui::OrigenPlantilla>(id),
        })
    }

    fn spawn_record(&mut self, r: &EntityRecord) {
        let mut b = hecs::EntityBuilder::new();
        b.add(r.id).add(Name(r.name.clone())).add(r.transform).add(GlobalTransform::default()).add(SiblingIndex(r.sibling));
        if let Some(p) = r.parent {
            b.add(Parent(p));
        }
        if !r.active {
            b.add(Inactive);
        }
        if let Some(c) = &r.mesh {
            b.add(c.clone());
        }
        if let Some(c) = r.light {
            b.add(c);
        }
        if let Some(c) = r.camera {
            b.add(c);
        }
        if let Some(c) = r.rigid_body {
            b.add(c);
        }
        if let Some(c) = r.collider {
            b.add(c);
        }
        if let Some(c) = &r.scripts {
            b.add(c.clone());
        }
        if let Some(c) = &r.animator {
            b.add(c.clone());
        }
        if let Some(c) = &r.lienzo {
            b.add(c.clone());
        }
        if let Some(c) = &r.rect_ui {
            b.add(c.clone());
        }
        if let Some(c) = &r.contenedor {
            b.add(c.clone());
        }
        if let Some(c) = &r.widget {
            b.add(c.clone());
        }
        if let Some(c) = &r.plantilla {
            b.add(c.clone());
        }
        if let Some(c) = r.origen {
            b.add(c);
        }
        let e = self.world.spawn(b.build());
        self.index.insert(r.id, e);
        self.next_id = self.next_id.max(r.id.0 + 1);
    }

    /// Sustituye todos los componentes de una entidad por los de `r` (mismo id).
    pub fn reemplazar(&mut self, r: &EntityRecord) {
        if let Some(e) = self.index.remove(&r.id) {
            let _ = self.world.despawn(e);
        }
        self.spawn_record(r);
    }

    /// Crea una entidad desde un registro (el id debe estar libre; si no, se le da uno nuevo).
    pub fn spawn_from_record(&mut self, r: &EntityRecord) -> EntityId {
        let mut r = r.clone();
        if self.contains(r.id) || r.id.0 == 0 {
            r.id = self.alloc_id();
        }
        self.spawn_record(&r);
        r.id
    }

    /// Reserva un id nuevo sin crear la entidad.
    pub fn new_id(&mut self) -> EntityId {
        self.alloc_id()
    }

    pub fn to_file(&self) -> SceneFile {
        let h = self.hierarchy();
        let entities = h.depth_first().into_iter().filter_map(|(id, _)| self.record(id)).collect();
        SceneFile { format: "maker-scene".into(), version: 1, environment: self.env.clone(), entities }
    }

    pub fn from_file(f: &SceneFile) -> Scene {
        let mut s = Scene::new();
        s.env = f.environment.clone();
        for r in &f.entities {
            if !s.contains(r.id) {
                s.spawn_record(r);
            }
        }
        s.update_transforms();
        s
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(&self.to_file()).expect("la escena siempre es serializable")
    }
    pub fn from_json(s: &str) -> Result<Scene, String> {
        let f: SceneFile = serde_json::from_str(s).map_err(|e| format!("escena inválida: {e}"))?;
        if f.format != "maker-scene" {
            return Err(format!("formato desconocido: {}", f.format));
        }
        Ok(Scene::from_file(&f))
    }
    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        std::fs::write(path, self.to_json()).map_err(|e| format!("{}: {e}", path.display()))
    }
    pub fn load(path: &Path) -> Result<Scene, String> {
        let s = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Scene::from_json(&s)
    }

    /// Duplica una entidad con todos sus hijos. Devuelve la raíz nueva.
    pub fn duplicate(&mut self, id: EntityId) -> Option<EntityId> {
        let h = self.hierarchy();
        let mut order = vec![id];
        let mut i = 0;
        while i < order.len() {
            order.extend_from_slice(h.children_of(order[i]));
            i += 1;
        }
        let mut map: HashMap<EntityId, EntityId> = HashMap::new();
        let records: Vec<EntityRecord> = order.iter().filter_map(|&o| self.record(o)).collect();
        for r in &records {
            let new_id = self.alloc_id();
            map.insert(r.id, new_id);
        }
        for r in records {
            let mut n = r.clone();
            n.id = map[&r.id];
            n.parent = r.parent.map(|p| *map.get(&p).unwrap_or(&p));
            if r.id == id {
                n.name = next_copy_name(&r.name);
                n.sibling = r.sibling + 1;
            }
            self.spawn_record(&n);
        }
        self.update_transforms();
        map.get(&id).copied()
    }

    /// Entidad con cámara principal activa.
    pub fn primary_camera(&self) -> Option<(EntityId, Camera)> {
        let mut q = self.world.query::<(&EntityId, &Camera, Option<&Inactive>)>();
        let mut best = None;
        for (id, cam, inactive) in q.iter() {
            if inactive.is_some() {
                continue;
            }
            if cam.primary {
                return Some((*id, *cam));
            }
            best.get_or_insert((*id, *cam));
        }
        best
    }
}

fn next_copy_name(name: &str) -> String {
    if let Some((base, n)) = name.rsplit_once(" (")
        && let Some(num) = n.strip_suffix(')').and_then(|n| n.parse::<u32>().ok())
    {
        return format!("{base} ({})", num + 1);
    }
    format!("{name} (1)")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_roundtrip_keeps_everything() {
        let s = Scene::starter();
        let json = s.to_json();
        let s2 = Scene::from_json(&json).unwrap();
        assert_eq!(s.to_file(), s2.to_file());
        assert_eq!(s2.len(), 4);
    }

    #[test]
    fn hierarchy_propagates_and_reparent_keeps_world() {
        let mut s = Scene::new();
        let a = s.spawn("A");
        let b = s.spawn("B");
        s.set_transform(a, Transform::from_translation(Vec3::new(5.0, 0.0, 0.0)));
        s.set_transform(b, Transform::from_translation(Vec3::new(1.0, 2.0, 0.0)));
        assert!(s.set_parent(b, Some(a), true));
        let wp = s.global(b).w_axis.truncate();
        assert!((wp - Vec3::new(1.0, 2.0, 0.0)).length() < 1e-4, "{wp}");
        assert!((s.transform(b).translation - Vec3::new(-4.0, 2.0, 0.0)).length() < 1e-4);
        // ciclo prohibido
        assert!(!s.set_parent(a, Some(b), true));
        // mover el padre mueve al hijo
        s.set_transform(a, Transform::from_translation(Vec3::new(0.0, 0.0, 0.0)));
        s.update_transforms();
        assert!((s.global(b).w_axis.truncate() - Vec3::new(-4.0, 2.0, 0.0)).length() < 1e-4);
    }

    #[test]
    fn duplicate_copies_children_and_names() {
        let mut s = Scene::new();
        let a = s.spawn("Torre");
        let b = s.spawn("Hijo");
        s.set_parent(b, Some(a), false);
        let d = s.duplicate(a).unwrap();
        assert_eq!(s.name(d), "Torre (1)");
        assert_eq!(s.hierarchy().children_of(d).len(), 1);
        assert_eq!(s.len(), 4);
        s.despawn_recursive(d);
        assert_eq!(s.len(), 2);
    }

    #[test]
    fn move_before_reorders() {
        let mut s = Scene::new();
        let a = s.spawn("A");
        let b = s.spawn("B");
        let c = s.spawn("C");
        assert!(s.move_before(c, a));
        assert_eq!(s.hierarchy().roots, vec![c, a, b]);
    }
}
