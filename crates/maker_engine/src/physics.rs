//! Física con Rapier (paso fijo de 60 Hz, como FixedUpdate de Unity y el
//! physics tick de Godot/Unreal). Solo existe durante el modo Play.

use crate::components::{BodyKind, Collider, ColliderShape, EntityId, RigidBody};
use crate::scene::Scene;
use glam::{Mat4, Quat, Vec3};
use rapier3d::prelude::{ColliderBuilder, PhysicsWorld, Pose, QueryFilter, Ray, RigidBodyBuilder, RigidBodyHandle};
use std::collections::HashMap;

pub const FIXED_DT: f32 = 1.0 / 60.0;

pub struct Physics {
    pub world: PhysicsWorld,
    bodies: HashMap<EntityId, RigidBodyHandle>,
    /// Escala del mundo de cada entidad física (Rapier no tiene escala).
    scales: HashMap<EntityId, Vec3>,
    /// Raíz del proyecto, para cargar las mallas de los colisionadores convexos.
    pub raiz: Option<std::path::PathBuf>,
}

fn world_parts(m: Mat4) -> (Vec3, Quat, Vec3) {
    let (s, r, t) = m.to_scale_rotation_translation();
    (t, r.normalize(), s)
}

fn shape_builder(shape: &ColliderShape, s: Vec3, malla: Option<&crate::components::MeshSource>, raiz: Option<&std::path::Path>) -> ColliderBuilder {
    let s = s.abs().max(Vec3::splat(1e-4));
    match *shape {
        ColliderShape::Convex { complejidad } => {
            let puntos = malla.and_then(|m| crate::convexo::casco(m, raiz, complejidad)).map(|c| c.iter().map(|p| *p * s).collect::<Vec<Vec3>>());
            match puntos.and_then(|p| ColliderBuilder::convex_hull(&p)) {
                Some(b) => b,
                None => {
                    crate::log::warn("física", "colisionador convexo sin malla válida: se usa una caja");
                    ColliderBuilder::cuboid(0.5 * s.x, 0.5 * s.y, 0.5 * s.z)
                }
            }
        }
        ColliderShape::Box { half_extents } => {
            let h = half_extents * s;
            ColliderBuilder::cuboid(h.x, h.y, h.z)
        }
        ColliderShape::Sphere { radius } => ColliderBuilder::ball(radius * s.max_element()),
        ColliderShape::Capsule { half_height, radius } => ColliderBuilder::capsule_y(half_height * s.y, radius * s.x.max(s.z)),
        ColliderShape::Cylinder { half_height, radius } => ColliderBuilder::cylinder(half_height * s.y, radius * s.x.max(s.z)),
    }
}

impl Physics {
    pub fn new(gravity: Vec3) -> Self {
        let mut world = PhysicsWorld::new();
        world.gravity = gravity;
        world.integration_parameters.dt = FIXED_DT;
        Self { world, bodies: HashMap::new(), scales: HashMap::new(), raiz: None }
    }

    /// Crea cuerpos para todas las entidades activas con física.
    pub fn build(scene: &Scene) -> Self {
        Self::build_en(scene, None)
    }

    /// Como `build`, con la raíz del proyecto para los colisionadores convexos de modelos.
    pub fn build_en(scene: &Scene, raiz: Option<&std::path::Path>) -> Self {
        let mut p = Physics::new(Vec3::from(scene.env.gravity));
        p.raiz = raiz.map(std::path::Path::to_path_buf);
        let ids: Vec<EntityId> = {
            let mut q = scene.world.query::<(&EntityId, Option<&RigidBody>, Option<&Collider>)>();
            q.iter().filter(|(_, b, c)| b.is_some() || c.is_some()).map(|(id, _, _)| *id).collect()
        };
        for id in ids {
            if scene.is_active_in_hierarchy(id) {
                p.add_entity(scene, id);
            }
        }
        p
    }

    pub fn has(&self, id: EntityId) -> bool {
        self.bodies.contains_key(&id)
    }
    pub fn body_count(&self) -> usize {
        self.bodies.len()
    }

    pub fn add_entity(&mut self, scene: &Scene, id: EntityId) {
        if self.bodies.contains_key(&id) {
            return;
        }
        let rb = scene.get::<RigidBody>(id).unwrap_or_else(RigidBody::fixed);
        let (t, r, s) = world_parts(scene.global(id));
        let mut b = match rb.kind {
            BodyKind::Dynamic => RigidBodyBuilder::dynamic(),
            BodyKind::Fixed => RigidBodyBuilder::fixed(),
            BodyKind::Kinematic => RigidBodyBuilder::kinematic_position_based(),
        }
        .pose(Pose::from_parts(t, r))
        .linear_damping(rb.linear_damping)
        .angular_damping(rb.angular_damping)
        .gravity_scale(rb.gravity_scale)
        .ccd_enabled(rb.ccd)
        .user_data(id.0 as u128);
        if rb.lock_rotation {
            b = b.lock_rotations();
        }
        let handle = self.world.insert_body(b);
        if let Some(c) = scene.get::<Collider>(id) {
            let malla = scene.get::<crate::components::MeshRenderer>(id).map(|m| m.mesh);
            let cb = shape_builder(&c.shape, s, malla.as_ref(), self.raiz.as_deref())
                .friction(c.friction)
                .restitution(c.restitution)
                .sensor(c.is_trigger)
                .mass(rb.mass.max(1e-3))
                .user_data(id.0 as u128);
            self.world.colliders.insert_with_parent(cb, handle, &mut self.world.bodies);
        }
        self.bodies.insert(id, handle);
        self.scales.insert(id, s);
    }

    pub fn remove_entity(&mut self, id: EntityId) {
        if let Some(h) = self.bodies.remove(&id) {
            self.world.remove_body_with_colliders(h, true);
        }
        self.scales.remove(&id);
    }

    /// Antes del paso: los cinemáticos siguen a su transform.
    pub fn push_kinematic(&mut self, scene: &Scene) {
        for (id, h) in &self.bodies {
            if let Some(b) = self.world.bodies.get_mut(*h)
                && b.is_kinematic()
            {
                let (t, r, _) = world_parts(scene.global(*id));
                b.set_next_kinematic_position(Pose::from_parts(t, r));
            }
        }
    }

    pub fn step(&mut self) {
        self.world.step();
    }

    /// Tras el paso: los dinámicos escriben su pose en la escena.
    pub fn pull_dynamic(&self, scene: &mut Scene) {
        for (id, h) in &self.bodies {
            let Some(b) = self.world.bodies.get(*h) else { continue };
            if !b.is_dynamic() || b.is_sleeping() {
                continue;
            }
            let s = self.scales.get(id).copied().unwrap_or(Vec3::ONE);
            let m = Mat4::from_scale_rotation_translation(s, *b.rotation(), b.translation());
            scene.set_world_matrix(*id, m);
        }
    }

    /// Teletransporta un cuerpo (cuando un script cambia la posición).
    pub fn teleport(&mut self, id: EntityId, pos: Vec3, rot: Quat) {
        if let Some(b) = self.bodies.get(&id).and_then(|h| self.world.bodies.get_mut(*h)) {
            if b.is_kinematic() {
                b.set_next_kinematic_position(Pose::from_parts(pos, rot));
            } else {
                b.set_position(Pose::from_parts(pos, rot), true);
            }
        }
    }

    pub fn velocity(&self, id: EntityId) -> Vec3 {
        self.bodies.get(&id).and_then(|h| self.world.bodies.get(*h)).map(|b| b.linvel()).unwrap_or(Vec3::ZERO)
    }
    pub fn set_velocity(&mut self, id: EntityId, v: Vec3) {
        if let Some(b) = self.bodies.get(&id).and_then(|h| self.world.bodies.get_mut(*h)) {
            b.set_linvel(v, true);
        }
    }
    pub fn apply_impulse(&mut self, id: EntityId, i: Vec3) {
        if let Some(b) = self.bodies.get(&id).and_then(|h| self.world.bodies.get_mut(*h)) {
            b.apply_impulse(i, true);
        }
    }

    /// Rayo contra colisionadores. Devuelve (entidad, distancia, punto, normal).
    pub fn raycast(&self, origin: Vec3, dir: Vec3, max: f32) -> Option<(EntityId, f32, Vec3, Vec3)> {
        let d = dir.normalize_or_zero();
        if d == Vec3::ZERO {
            return None;
        }
        let ray = Ray::new(origin, d);
        let (ch, hit) = self.world.cast_ray_and_get_normal(&ray, max, true, QueryFilter::default())?;
        let c = self.world.colliders.get(ch)?;
        Some((EntityId(c.user_data as u64), hit.time_of_impact, origin + d * hit.time_of_impact, hit.normal))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::{PrimitiveShape, Transform};

    #[test]
    fn dynamic_box_falls_and_rests_on_ground() {
        let mut s = Scene::starter();
        let cube = s.find_by_name("Cubo").unwrap();
        s.set_transform(cube, Transform::from_translation(Vec3::new(0.0, 4.0, 0.0)));
        s.insert(cube, RigidBody::dynamic());
        s.insert(cube, Collider::new(ColliderShape::fit(PrimitiveShape::Cube)));
        s.update_transforms();
        let mut p = Physics::build(&s);
        assert_eq!(p.body_count(), 2);
        for _ in 0..240 {
            p.push_kinematic(&s);
            p.step();
            p.pull_dynamic(&mut s);
        }
        s.update_transforms();
        let y = s.global(cube).w_axis.y;
        assert!((y - 0.5).abs() < 0.05, "el cubo debería reposar en y≈0.5, está en {y}");
        let hit = p.raycast(Vec3::new(0.0, 10.0, 0.0), Vec3::NEG_Y, 100.0).unwrap();
        assert_eq!(hit.0, cube);
        assert!((hit.1 - 9.0).abs() < 0.06);
    }
}
