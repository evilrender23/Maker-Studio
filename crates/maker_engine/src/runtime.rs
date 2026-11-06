//! Sesión de juego (modo Play): scripts + física + tiempo.
//!
//! Orden de cada fotograma (el de Unity, que comparten Godot y Unreal):
//! `start` pendientes → N × (`fixed_update` → física) → `update` →
//! destrucciones diferidas → propagación de transforms.

use crate::animation::Animation;
use crate::components::*;
use crate::input::Input;
use crate::log;
use crate::physics::{FIXED_DT, Physics};
use crate::scene::Scene;
use crate::scripting::{ScriptLib, apply_props};
use glam::{Mat4, Quat, Vec3};
use maker_api::{Entity, FrameInfo, HostApi, RayHit};
use std::ffi::c_void;
use std::sync::Arc;

struct Instance {
    entity: EntityId,
    desc: usize,
    ptr: *mut c_void,
    started: bool,
    failed: bool,
}

pub struct Runtime {
    pub physics: Physics,
    lib: Option<Arc<ScriptLib>>,
    instances: Vec<Instance>,
    pub time: f32,
    pub frame: u64,
    accumulator: f32,
    pub paused: bool,
    pub animation: Animation,
    /// Raíz del proyecto (para resolver rutas de modelos y clips).
    pub project_root: Option<std::path::PathBuf>,
    /// Último modo de cursor pedido por un script (`None`: el del proyecto).
    pub cursor_mode: Option<maker_api::CursorMode>,
}

/// Estado que ven las funciones del host durante una llamada a un script.
struct HostCtx<'a> {
    scene: &'a mut Scene,
    physics: &'a mut Physics,
    input: &'a Input,
    destroyed: Vec<EntityId>,
    source: String,
    cursor_mode: Option<maker_api::CursorMode>,
}

impl Runtime {
    /// Empieza a jugar la escena: crea la física e instancia los scripts.
    pub fn start(scene: &mut Scene, lib: Option<Arc<ScriptLib>>) -> Runtime {
        Self::start_en(scene, lib, None)
    }

    /// Como `start`, con la raíz del proyecto (modelos, colisionadores convexos).
    pub fn start_en(scene: &mut Scene, lib: Option<Arc<ScriptLib>>, raiz: Option<&std::path::Path>) -> Runtime {
        scene.update_transforms();
        let physics = Physics::build_en(scene, raiz);
        let mut rt = Runtime { physics, lib, instances: Vec::new(), time: 0.0, frame: 0, accumulator: 0.0, paused: false, animation: Animation::default(), project_root: raiz.map(std::path::Path::to_path_buf), cursor_mode: None };
        let mut slots: Vec<(EntityId, Scripts)> = {
            let mut q = scene.world.query::<(&EntityId, &Scripts)>();
            q.iter().map(|(id, s)| (*id, s.clone())).collect()
        };
        slots.sort_by_key(|(id, _)| id.0);
        for (id, scripts) in slots {
            if !scene.is_active_in_hierarchy(id) {
                continue;
            }
            for slot in scripts.0.iter().filter(|s| s.enabled) {
                rt.instantiate(id, slot);
            }
        }
        log::info("motor", format!("▶ Play: {} entidades, {} cuerpos físicos, {} scripts", scene.len(), rt.physics.body_count(), rt.instances.len()));
        rt
    }

    fn instantiate(&mut self, id: EntityId, slot: &ScriptSlot) {
        let Some(lib) = &self.lib else {
            log::warn("scripts", format!("'{}' usa el script {} pero no hay biblioteca de scripts cargada", id.0, slot.name));
            return;
        };
        let Some(di) = lib.index_of(&slot.name) else {
            log::warn("scripts", format!("script desconocido: {} (¿lo exportaste con export_scripts!?)", slot.name));
            return;
        };
        let d = &lib.descs()[di];
        let ptr = (d.create)();
        if ptr.is_null() {
            log::error("scripts", format!("{}: falló la creación", slot.name));
            return;
        }
        apply_props(d, ptr, &slot.props);
        self.instances.push(Instance { entity: id, desc: di, ptr, started: false, failed: false });
    }

    pub fn script_count(&self) -> usize {
        self.instances.len()
    }

    /// Avanza `dt` segundos.
    pub fn tick(&mut self, scene: &mut Scene, input: &Input, dt: f32) {
        if self.paused {
            return;
        }
        let dt = dt.clamp(0.0, 0.1);
        let lib = self.lib.clone();
        let mut ctx = HostCtx { scene, physics: &mut self.physics, input, destroyed: Vec::new(), source: String::new(), cursor_mode: None };
        let api = host_api(&mut ctx);
        // Todo acceso posterior pasa por el mismo puntero que ven los scripts.
        let ctx = unsafe { &mut *(api.ctx as *mut HostCtx) };

        // start
        if let Some(lib) = &lib {
            let info = FrameInfo { dt, time: self.time, frame: self.frame };
            for inst in self.instances.iter_mut().filter(|i| !i.started && !i.failed) {
                inst.started = true;
                let d = &lib.descs()[inst.desc];
                if !(d.start)(inst.ptr, &api, Entity(inst.entity.0), &info) {
                    inst.failed = true;
                }
            }
        }

        // paso fijo
        self.accumulator += dt;
        let mut steps = 0;
        while self.accumulator >= FIXED_DT && steps < 5 {
            if let Some(lib) = &lib {
                let info = FrameInfo { dt: FIXED_DT, time: self.time, frame: self.frame };
                for inst in self.instances.iter_mut().filter(|i| i.started && !i.failed) {
                    let d = &lib.descs()[inst.desc];
                    if !(d.fixed_update)(inst.ptr, &api, Entity(inst.entity.0), &info) {
                        inst.failed = true;
                    }
                }
            }
            ctx.physics.push_kinematic(ctx.scene);
            ctx.physics.step();
            ctx.physics.pull_dynamic(ctx.scene);
            ctx.scene.update_transforms();
            self.accumulator -= FIXED_DT;
            steps += 1;
        }
        if steps == 5 {
            self.accumulator = 0.0;
        }

        // update
        if let Some(lib) = &lib {
            let info = FrameInfo { dt, time: self.time, frame: self.frame };
            for inst in self.instances.iter_mut().filter(|i| i.started && !i.failed) {
                let d = &lib.descs()[inst.desc];
                if !(d.update)(inst.ptr, &api, Entity(inst.entity.0), &info) {
                    inst.failed = true;
                }
            }
        }

        // destrucciones diferidas (como Destroy() de Unity)
        let destroyed = std::mem::take(&mut ctx.destroyed);
        if !destroyed.is_empty() {
            let h = ctx.scene.hierarchy();
            let mut all = Vec::new();
            for id in destroyed {
                let mut stack = vec![id];
                while let Some(c) = stack.pop() {
                    stack.extend_from_slice(h.children_of(c));
                    all.push(c);
                }
            }
            for id in &all {
                ctx.physics.remove_entity(*id);
            }
            ctx.scene.despawn_recursive_many(&all);
            if let Some(lib) = &lib {
                self.instances.retain(|i| {
                    if all.contains(&i.entity) {
                        (lib.descs()[i.desc].destroy)(i.ptr);
                        false
                    } else {
                        true
                    }
                });
            }
        }
        ctx.scene.update_transforms();
        if let Some(m) = ctx.cursor_mode {
            self.cursor_mode = Some(m);
        }
        // Animación después de los scripts (que pueden cambiar de clip).
        self.animation.update(ctx.scene, self.project_root.as_deref(), dt, |_| true);
        self.time += dt;
        self.frame += 1;
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        if let Some(lib) = &self.lib {
            for i in self.instances.drain(..) {
                (lib.descs()[i.desc].destroy)(i.ptr);
            }
        }
    }
}

impl Scene {
    fn despawn_recursive_many(&mut self, ids: &[EntityId]) {
        for id in ids {
            if self.contains(*id) {
                self.despawn_recursive(*id);
            }
        }
    }

    /// Recalcula solo la matriz de mundo de `id` (los hijos se ponen al día al
    /// final del fotograma).
    fn refresh_global(&mut self, id: EntityId) {
        let local = self.transform(id).matrix();
        let m = match self.parent(id) {
            Some(p) => self.global(p) * local,
            None => local,
        };
        if let Some(mut g) = self.get_mut::<GlobalTransform>(id) {
            g.0 = m;
        }
    }
}

// ─────────────────────────── funciones del host ───────────────────────────

fn host_api(ctx: &mut HostCtx) -> HostApi {
    HostApi {
        ctx: ctx as *mut HostCtx as *mut c_void,
        log: h_log,
        get_position: h_get_position,
        set_position: h_set_position,
        get_rotation: h_get_rotation,
        set_rotation: h_set_rotation,
        get_scale: h_get_scale,
        set_scale: h_set_scale,
        key_down: h_key_down,
        key_pressed: h_key_pressed,
        mouse_down: h_mouse_down,
        mouse_delta: h_mouse_delta,
        find_entity: h_find,
        get_velocity: h_get_velocity,
        set_velocity: h_set_velocity,
        apply_impulse: h_apply_impulse,
        spawn: h_spawn,
        destroy: h_destroy,
        raycast: h_raycast,
        set_color: h_set_color,
        exists: h_exists,
        set_shading: h_set_shading,
        play_animation: h_play_animation,
        set_animation_speed: h_set_animation_speed,
        set_cursor_mode: h_set_cursor_mode,
        set_global_shading: h_set_global_shading,
    }
}

unsafe fn cx<'a>(p: *mut c_void) -> &'a mut HostCtx<'a> {
    unsafe { &mut *(p as *mut HostCtx) }
}
fn v3(v: Vec3) -> maker_api::Vec3 {
    maker_api::Vec3::new(v.x, v.y, v.z)
}
fn gv3(v: maker_api::Vec3) -> Vec3 {
    Vec3::new(v.x, v.y, v.z)
}
fn id(e: Entity) -> EntityId {
    EntityId(e.0)
}

unsafe extern "C" fn h_log(c: *mut c_void, level: u32, p: *const u8, n: usize) {
    let c = unsafe { cx(c) };
    let msg = String::from_utf8_lossy(unsafe { std::slice::from_raw_parts(p, n) }).into_owned();
    let src = if c.source.is_empty() { "script" } else { c.source.as_str() };
    match level {
        0 => log::info(src, msg),
        1 => log::warn(src, msg),
        _ => log::error(src, msg),
    }
}

unsafe extern "C" fn h_get_position(c: *mut c_void, e: Entity) -> maker_api::Vec3 {
    let c = unsafe { cx(c) };
    v3(c.scene.global(id(e)).w_axis.truncate())
}

fn set_world_trs(c: &mut HostCtx, e: EntityId, pos: Option<Vec3>, rot: Option<Quat>) {
    if !c.scene.contains(e) {
        return;
    }
    let (s, r, t) = c.scene.global(e).to_scale_rotation_translation();
    let (t, r) = (pos.unwrap_or(t), rot.unwrap_or(r).normalize());
    c.scene.set_world_matrix(e, Mat4::from_scale_rotation_translation(s, r, t));
    c.scene.refresh_global(e);
    c.physics.teleport(e, t, r);
}

unsafe extern "C" fn h_set_position(c: *mut c_void, e: Entity, p: maker_api::Vec3) {
    let c = unsafe { cx(c) };
    set_world_trs(c, id(e), Some(gv3(p)), None);
}
unsafe extern "C" fn h_get_rotation(c: *mut c_void, e: Entity) -> maker_api::Quat {
    let c = unsafe { cx(c) };
    let (_, r, _) = c.scene.global(id(e)).to_scale_rotation_translation();
    maker_api::Quat { x: r.x, y: r.y, z: r.z, w: r.w }
}
unsafe extern "C" fn h_set_rotation(c: *mut c_void, e: Entity, q: maker_api::Quat) {
    let c = unsafe { cx(c) };
    set_world_trs(c, id(e), None, Some(Quat::from_xyzw(q.x, q.y, q.z, q.w)));
}
unsafe extern "C" fn h_get_scale(c: *mut c_void, e: Entity) -> maker_api::Vec3 {
    let c = unsafe { cx(c) };
    v3(c.scene.transform(id(e)).scale)
}
unsafe extern "C" fn h_set_scale(c: *mut c_void, e: Entity, s: maker_api::Vec3) {
    let c = unsafe { cx(c) };
    let e = id(e);
    let mut t = c.scene.transform(e);
    t.scale = gv3(s);
    c.scene.set_transform(e, t);
    c.scene.refresh_global(e);
}
unsafe extern "C" fn h_key_down(c: *mut c_void, k: u32) -> bool {
    unsafe { cx(c) }.input.key_down(k)
}
unsafe extern "C" fn h_key_pressed(c: *mut c_void, k: u32) -> bool {
    unsafe { cx(c) }.input.key_pressed(k)
}
unsafe extern "C" fn h_mouse_down(c: *mut c_void, b: u32) -> bool {
    unsafe { cx(c) }.input.mouse_buttons.get(b as usize).copied().unwrap_or(false)
}
unsafe extern "C" fn h_mouse_delta(c: *mut c_void) -> maker_api::Vec2 {
    let d = unsafe { cx(c) }.input.mouse_delta;
    maker_api::Vec2 { x: d.x, y: d.y }
}
unsafe extern "C" fn h_find(c: *mut c_void, p: *const u8, n: usize) -> Entity {
    let c = unsafe { cx(c) };
    let name = String::from_utf8_lossy(unsafe { std::slice::from_raw_parts(p, n) });
    c.scene.find_by_name(&name).map(|i| Entity(i.0)).unwrap_or(Entity::NONE)
}
unsafe extern "C" fn h_exists(c: *mut c_void, e: Entity) -> bool {
    unsafe { cx(c) }.scene.contains(id(e))
}
unsafe extern "C" fn h_get_velocity(c: *mut c_void, e: Entity) -> maker_api::Vec3 {
    v3(unsafe { cx(c) }.physics.velocity(id(e)))
}
unsafe extern "C" fn h_set_velocity(c: *mut c_void, e: Entity, v: maker_api::Vec3) {
    unsafe { cx(c) }.physics.set_velocity(id(e), gv3(v))
}
unsafe extern "C" fn h_apply_impulse(c: *mut c_void, e: Entity, v: maker_api::Vec3) {
    unsafe { cx(c) }.physics.apply_impulse(id(e), gv3(v))
}
unsafe extern "C" fn h_spawn(c: *mut c_void, shape: u32, at: maker_api::Vec3, flags: u32) -> Entity {
    let c = unsafe { cx(c) };
    let Some(shape) = PrimitiveShape::from_api(shape) else { return Entity::NONE };
    let e = c.scene.spawn(shape.label());
    c.scene.set_transform(e, Transform::from_translation(gv3(at)));
    c.scene.refresh_global(e);
    c.scene.insert(e, MeshRenderer::primitive(shape));
    let dynamic = flags & maker_api::spawn_flags::DYNAMIC_BODY != 0;
    if dynamic || flags & maker_api::spawn_flags::STATIC_BODY != 0 {
        c.scene.insert(e, if dynamic { RigidBody::dynamic() } else { RigidBody::fixed() });
        c.scene.insert(e, Collider::new(ColliderShape::fit(shape)));
        c.physics.add_entity(c.scene, e);
    }
    Entity(e.0)
}
unsafe extern "C" fn h_destroy(c: *mut c_void, e: Entity) {
    let c = unsafe { cx(c) };
    if !c.destroyed.contains(&id(e)) {
        c.destroyed.push(id(e));
    }
}
unsafe extern "C" fn h_raycast(c: *mut c_void, o: maker_api::Vec3, d: maker_api::Vec3, max: f32, out: *mut RayHit) -> bool {
    let c = unsafe { cx(c) };
    match c.physics.raycast(gv3(o), gv3(d), max) {
        Some((e, dist, p, n)) => {
            if !out.is_null() {
                unsafe { *out = RayHit { entity: Entity(e.0), distance: dist, point: v3(p), normal: v3(n) } };
            }
            true
        }
        None => false,
    }
}
unsafe extern "C" fn h_set_color(c: *mut c_void, e: Entity, rgba: maker_api::Rgba) {
    let c = unsafe { cx(c) };
    if let Some(mut m) = c.scene.get_mut::<MeshRenderer>(id(e)) {
        m.material.base_color = [rgba.r, rgba.g, rgba.b, rgba.a];
    }
}
unsafe extern "C" fn h_set_global_shading(c: *mut c_void, code: u32) {
    let c = unsafe { cx(c) };
    if code == u32::MAX {
        c.scene.env.global_shading.model = None;
        return;
    }
    match ShadingModel::from_api(code) {
        Some(m) => c.scene.env.global_shading.model = Some(m),
        None => log::warn("script", format!("set_global_shading: modelo desconocido {code}")),
    }
}
unsafe extern "C" fn h_set_shading(c: *mut c_void, e: Entity, code: u32) {
    let c = unsafe { cx(c) };
    let Some(model) = ShadingModel::from_api(code) else {
        log::warn("script", format!("set_shading: modelo desconocido {code}"));
        return;
    };
    if let Some(mut m) = c.scene.get_mut::<MeshRenderer>(id(e)) {
        m.material.shading = model;
    }
}

unsafe extern "C" fn h_play_animation(c: *mut c_void, e: Entity, p: *const u8, n: usize, fade: f32) -> bool {
    let c = unsafe { cx(c) };
    let name = String::from_utf8_lossy(unsafe { std::slice::from_raw_parts(p, n) }).into_owned();
    match c.scene.get_mut::<Animator>(id(e)) {
        Some(mut a) => {
            a.play(&name, fade.max(0.0));
            true
        }
        None => {
            log::warn("script", format!("play_animation: la entidad {} no tiene Animator", e.0));
            false
        }
    }
}
unsafe extern "C" fn h_set_animation_speed(c: *mut c_void, e: Entity, speed: f32) {
    let c = unsafe { cx(c) };
    if let Some(mut a) = c.scene.get_mut::<Animator>(id(e)) {
        a.speed = speed;
    }
}

unsafe extern "C" fn h_set_cursor_mode(c: *mut c_void, code: u32) {
    let c = unsafe { cx(c) };
    match maker_api::CursorMode::from_u32(code) {
        Some(m) => c.cursor_mode = Some(m),
        None => log::warn("script", format!("set_cursor_mode: modo desconocido {code}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use maker_api::prelude::*;

    #[derive(Default)]
    struct Spinner {
        speed: f32,
    }
    impl Script for Spinner {
        fn props(&mut self, p: &mut Props) {
            p.float("speed", &mut self.speed);
        }
        fn update(&mut self, ctx: &mut Ctx) {
            ctx.translate(maker_api::Vec3::new(self.speed * ctx.dt, 0.0, 0.0));
        }
    }

    #[derive(Default)]
    struct Spawner {
        done: bool,
    }
    impl Script for Spawner {
        fn update(&mut self, ctx: &mut Ctx) {
            if !self.done {
                self.done = true;
                let e = ctx.spawn(Primitive::Sphere, maker_api::Vec3::new(0.0, 5.0, 0.0), spawn_flags::DYNAMIC_BODY);
                ctx.log(format!("creada {}", e.0));
                let me = ctx.entity;
                ctx.destroy(me);
            }
        }
    }

    #[derive(Default)]
    struct Panicker;
    impl Script for Panicker {
        fn update(&mut self, _ctx: &mut Ctx) {
            panic!("fallo a propósito");
        }
    }

    #[derive(Default)]
    struct Toonify;
    impl Script for Toonify {
        fn start(&mut self, ctx: &mut Ctx) {
            let me = ctx.entity;
            ctx.set_shading(me, Shading::Toon);
        }
    }

    #[derive(Default)]
    struct GlobalToon;
    impl Script for GlobalToon {
        fn update(&mut self, ctx: &mut Ctx) {
            ctx.set_global_shading(if ctx.frame == 0 { Some(Shading::Hatching) } else { None });
        }
    }

    #[derive(Default)]
    struct Dancer;
    impl Script for Dancer {
        fn start(&mut self, ctx: &mut Ctx) {
            let me = ctx.entity;
            assert!(ctx.play_animation(me, "Dance_Loop", 0.25));
            ctx.set_animation_speed(me, 1.5);
            let cam = ctx.find("Cámara principal").unwrap();
            assert!(!ctx.play_animation(cam, "Dance_Loop", 0.0), "sin Animator");
        }
    }

    #[derive(Default)]
    struct MenuCursor;
    impl Script for MenuCursor {
        fn update(&mut self, ctx: &mut Ctx) {
            ctx.set_cursor_mode(if ctx.frame == 0 { CursorMode::Libre } else { CursorMode::Bloqueado });
        }
    }

    maker_api::script_library!(TEST_LIB; Spinner, Spawner, Panicker, Toonify, Dancer, MenuCursor, GlobalToon);

    #[test]
    fn scripts_request_cursor_mode() {
        let lib = Arc::new(ScriptLib::from_static(&TEST_LIB).unwrap());
        let mut s = Scene::starter();
        let cube = s.find_by_name("Cubo").unwrap();
        let mut rt = Runtime::start(&mut s, Some(lib.clone()));
        rt.tick(&mut s, &Input::default(), 1.0 / 60.0);
        assert_eq!(rt.cursor_mode, None, "sin scripts que lo pidan, manda el proyecto");
        s.insert(cube, Scripts(vec![ScriptSlot { name: "MenuCursor".into(), enabled: true, ..Default::default() }]));
        let mut rt = Runtime::start(&mut s, Some(lib));
        rt.tick(&mut s, &Input::default(), 1.0 / 60.0);
        assert_eq!(rt.cursor_mode, Some(CursorMode::Libre));
        rt.tick(&mut s, &Input::default(), 1.0 / 60.0);
        assert_eq!(rt.cursor_mode, Some(CursorMode::Bloqueado));
    }

    #[test]
    fn scripts_drive_animator_through_abi() {
        let lib = Arc::new(ScriptLib::from_static(&TEST_LIB).unwrap());
        let mut s = Scene::starter();
        let cube = s.find_by_name("Cubo").unwrap();
        let mut a = Animator::new("", "Idle_Loop");
        a.state.current = "Idle_Loop".into();
        s.insert(cube, a);
        s.insert(cube, Scripts(vec![ScriptSlot { name: "Dancer".into(), enabled: true, ..Default::default() }]));
        let mut rt = Runtime::start(&mut s, Some(lib));
        rt.tick(&mut s, &Input::default(), 1.0 / 60.0);
        let a = s.get::<Animator>(cube).unwrap();
        assert_eq!((a.clip.as_str(), a.speed, a.playing), ("Dance_Loop", 1.5, true));
        assert_eq!(a.state.from.as_ref().map(|f| f.0.as_str()), Some("Idle_Loop"));
    }

    #[test]
    fn scripts_can_change_shading_model() {
        let lib = Arc::new(ScriptLib::from_static(&TEST_LIB).unwrap());
        let mut s = Scene::starter();
        let cube = s.find_by_name("Cubo").unwrap();
        assert_eq!(s.get::<MeshRenderer>(cube).unwrap().material.shading, ShadingModel::Pbr);
        s.insert(cube, Scripts(vec![ScriptSlot { name: "Toonify".into(), enabled: true, ..Default::default() }]));
        let mut rt = Runtime::start(&mut s, Some(lib));
        rt.tick(&mut s, &Input::default(), 1.0 / 60.0);
        assert_eq!(s.get::<MeshRenderer>(cube).unwrap().material.shading, ShadingModel::Toon);
    }

    #[test]
    fn scripts_set_global_shading() {
        let lib = Arc::new(ScriptLib::from_static(&TEST_LIB).unwrap());
        let mut s = Scene::starter();
        let cube = s.find_by_name("Cubo").unwrap();
        s.insert(cube, Scripts(vec![ScriptSlot { name: "GlobalToon".into(), enabled: true, ..Default::default() }]));
        let mut rt = Runtime::start(&mut s, Some(lib));
        rt.tick(&mut s, &Input::default(), 1.0 / 60.0);
        assert_eq!(s.env.global_shading.model, Some(ShadingModel::Hatching));
        rt.tick(&mut s, &Input::default(), 1.0 / 60.0);
        assert_eq!(s.env.global_shading.model, None);
    }

    #[test]
    fn scripts_run_props_apply_spawn_and_destroy() {
        let lib = Arc::new(ScriptLib::from_static(&TEST_LIB).unwrap());
        assert_eq!(lib.default_props("Spinner").len(), 1);
        let mut s = Scene::starter();
        let cube = s.find_by_name("Cubo").unwrap();
        let mut slot = ScriptSlot { name: "Spinner".into(), enabled: true, ..Default::default() };
        slot.props.insert("speed".into(), PropVal::Float(2.0));
        s.insert(cube, Scripts(vec![slot]));
        let sp = s.spawn("Spawner");
        s.insert(sp, Scripts(vec![ScriptSlot { name: "Spawner".into(), enabled: true, ..Default::default() }]));
        let pn = s.spawn("Panicker");
        s.insert(pn, Scripts(vec![ScriptSlot { name: "Panicker".into(), enabled: true, ..Default::default() }]));
        let before = s.len();
        let mut rt = Runtime::start(&mut s, Some(lib));
        assert_eq!(rt.script_count(), 3);
        let input = Input::default();
        for _ in 0..60 {
            rt.tick(&mut s, &input, 1.0 / 60.0);
        }
        let x = s.global(cube).w_axis.x;
        assert!((x - 2.0).abs() < 0.05, "Spinner debía mover 2 m en 1 s, x={x}");
        // Spawner se destruyó y creó una esfera dinámica que cae
        assert!(!s.contains(sp));
        assert_eq!(s.len(), before);
        let sphere = s.find_by_name("Esfera").unwrap();
        assert!(s.global(sphere).w_axis.y < 5.0);
        // el pánico no tumbó nada y quedó registrado
        assert!(log::since(0).iter().any(|e| e.text.contains("fallo a propósito")));
        assert_eq!(rt.script_count(), 2);
    }
}
