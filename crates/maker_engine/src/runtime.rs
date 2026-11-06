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
use crate::ui::{self, SistemaUi, WidgetUi};
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
    /// Interfaz: layout, entrada, eventos y lista de dibujo.
    pub ui: SistemaUi,
    /// Entrada que ven los scripts (sin lo que consumió la interfaz).
    entrada: Input,
    /// Escala del tiempo del juego (la cambian los scripts; 0 = pausa).
    pub time_scale: f32,
    /// Último modo de cursor pedido por un script (`None`: el del proyecto).
    pub cursor_mode: Option<maker_api::CursorMode>,
}

/// Estado que ven las funciones del host durante una llamada a un script.
struct HostCtx<'a> {
    scene: &'a mut Scene,
    physics: &'a mut Physics,
    input: &'a Input,
    ui: &'a mut SistemaUi,
    time_scale: f32,
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
        let ui = match raiz {
            Some(r) => SistemaUi::del_proyecto(r, &crate::project::Project::open(r).map(|p| p.file.tema_ui).unwrap_or_default()),
            None => SistemaUi::default(),
        };
        let mut rt = Runtime {
            physics,
            lib,
            instances: Vec::new(),
            time: 0.0,
            frame: 0,
            accumulator: 0.0,
            paused: false,
            animation: Animation::default(),
            project_root: raiz.map(std::path::Path::to_path_buf),
            ui,
            entrada: Input::default(),
            time_scale: 1.0,
            cursor_mode: None,
        };
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
        let dt_real = dt.clamp(0.0, 0.1);
        let dt = dt_real * self.time_scale;
        let lib = self.lib.clone();
        // La interfaz ve la entrada primero y se queda con lo que consume.
        self.ui.procesar_entrada(scene, input);
        let mut entrada = std::mem::take(&mut self.entrada);
        self.ui.filtrar(input, &mut entrada);
        let mut ctx = HostCtx { scene, physics: &mut self.physics, input: &entrada, ui: &mut self.ui, time_scale: self.time_scale, destroyed: Vec::new(), source: String::new(), cursor_mode: None };
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
                ctx.ui.quitar_dibujo(*id);
            }
            ctx.ui.marcar_sucio();
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
        let pantalla = [input.pantalla.x, input.pantalla.y];
        self.time_scale = ctx.time_scale;
        self.ui.actualizar(ctx.scene, pantalla, dt_real);
        self.entrada = entrada;
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
        ui_set_text: h_ui_set_text,
        ui_get_text: h_ui_get_text,
        ui_set_value: h_ui_set_value,
        ui_get_value: h_ui_get_value,
        ui_set_visible: h_ui_set_visible,
        ui_is_visible: h_ui_is_visible,
        ui_set_interactable: h_ui_set_interactable,
        ui_set_style: h_ui_set_style,
        ui_set_image: h_ui_set_image,
        ui_set_color: h_ui_set_color,
        ui_set_opacity: h_ui_set_opacity,
        ui_event: h_ui_event,
        ui_instantiate: h_ui_instantiate,
        ui_draw: h_ui_draw,
        ui_rect: h_ui_rect,
        ui_set_position: h_ui_set_position,
        ui_set_size: h_ui_set_size,
        ui_pointer_over: h_ui_pointer_over,
        ui_set_theme: h_ui_set_theme,
        set_time_scale: h_set_time_scale,
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

// ─────────────────────────── interfaz ───────────────────────────

unsafe fn texto<'a>(p: *const u8, n: usize) -> std::borrow::Cow<'a, str> {
    if p.is_null() || n == 0 {
        return std::borrow::Cow::Borrowed("");
    }
    String::from_utf8_lossy(unsafe { std::slice::from_raw_parts(p, n) })
}

unsafe extern "C" fn h_ui_set_text(c: *mut c_void, e: Entity, p: *const u8, n: usize) {
    let c = unsafe { cx(c) };
    let t = unsafe { texto(p, n) };
    c.ui.fijar_texto(c.scene, id(e), &t);
}
unsafe extern "C" fn h_ui_get_text(c: *mut c_void, e: Entity, buf: *mut u8, cap: usize) -> usize {
    let c = unsafe { cx(c) };
    let Some(w) = c.scene.get_mut::<WidgetUi>(id(e)) else { return 0 };
    let t = w.clase.texto().unwrap_or("").as_bytes();
    if !buf.is_null() {
        let k = t.len().min(cap);
        unsafe { std::ptr::copy_nonoverlapping(t.as_ptr(), buf, k) };
    }
    t.len()
}
unsafe extern "C" fn h_ui_set_value(c: *mut c_void, e: Entity, v: f32) {
    let c = unsafe { cx(c) };
    c.ui.cambiar_valor(c.scene, id(e), v);
}
unsafe extern "C" fn h_ui_get_value(c: *mut c_void, e: Entity) -> f32 {
    let c = unsafe { cx(c) };
    c.ui.valor(c.scene, id(e))
}
unsafe extern "C" fn h_ui_set_visible(c: *mut c_void, e: Entity, v: bool) {
    let c = unsafe { cx(c) };
    c.ui.editar_rect(c.scene, id(e), |r| r.visible = v);
}
unsafe extern "C" fn h_ui_is_visible(c: *mut c_void, e: Entity) -> bool {
    let c = unsafe { cx(c) };
    c.scene.get_mut::<ui::RectUi>(id(e)).is_some_and(|r| r.visible)
}
unsafe extern "C" fn h_ui_set_interactable(c: *mut c_void, e: Entity, v: bool) {
    let c = unsafe { cx(c) };
    c.ui.editar_rect(c.scene, id(e), |r| r.interactuable = v);
}
unsafe extern "C" fn h_ui_set_opacity(c: *mut c_void, e: Entity, v: f32) {
    let c = unsafe { cx(c) };
    c.ui.editar_rect(c.scene, id(e), |r| r.opacidad = v.clamp(0.0, 1.0));
}
unsafe extern "C" fn h_ui_set_style(c: *mut c_void, e: Entity, p: *const u8, n: usize) {
    let c = unsafe { cx(c) };
    let t = unsafe { texto(p, n) }.into_owned();
    if !t.is_empty() && c.ui.recursos.estilo(&t).is_none() {
        log::warn("script", format!("ui_set_style: no existe el estilo '{t}'"));
    }
    c.ui.editar_widget(c.scene, id(e), |w| w.estilo = t);
}
unsafe extern "C" fn h_ui_set_image(c: *mut c_void, e: Entity, p: *const u8, n: usize) {
    let c = unsafe { cx(c) };
    let t = unsafe { texto(p, n) }.into_owned();
    c.ui.editar_widget(c.scene, id(e), |w| match &mut w.clase {
        ui::ClaseWidget::Imagen(im) => im.imagen = t,
        _ => w.local.estados.normal.imagen = Some(t),
    });
}
unsafe extern "C" fn h_ui_set_color(c: *mut c_void, e: Entity, rgba: maker_api::Rgba) {
    let c = unsafe { cx(c) };
    let col = [rgba.r, rgba.g, rgba.b, rgba.a];
    c.ui.editar_widget(c.scene, id(e), |w| match &mut w.clase {
        ui::ClaseWidget::Texto(_) => w.local.estados.normal.color_texto = Some(col),
        ui::ClaseWidget::Imagen(im) => im.tinte = col,
        _ => w.local.estados.normal.fondo = Some(ui::estilo::Relleno::Color(col)),
    });
}
unsafe extern "C" fn h_ui_event(c: *mut c_void, e: Entity, kind: u32) -> bool {
    let c = unsafe { cx(c) };
    kind < 32 && c.ui.evento(id(e), 1 << kind)
}
unsafe extern "C" fn h_ui_instantiate(c: *mut c_void, p: *const u8, n: usize, parent: Entity) -> Entity {
    let c = unsafe { cx(c) };
    let ruta = unsafe { texto(p, n) }.into_owned();
    let Some(raiz) = c.ui.raiz().map(std::path::Path::to_path_buf) else {
        log::warn("script", "ui_instantiate: sin proyecto");
        return Entity::NONE;
    };
    let padre = if parent.is_none() {
        let mut q = c.scene.world.query::<(&EntityId, &ui::Lienzo)>();
        q.iter().map(|(i, _)| *i).min_by_key(|i| i.0)
    } else {
        Some(id(parent))
    };
    match ui::plantilla::PlantillaUi::cargar(&raiz.join(&ruta)) {
        Ok(pl) => {
            let r = pl.instanciar(c.scene, &ruta, padre);
            c.ui.marcar_sucio();
            r.map(|r| Entity(r.0)).unwrap_or(Entity::NONE)
        }
        Err(e) => {
            log::warn("script", format!("ui_instantiate: {e}"));
            Entity::NONE
        }
    }
}
unsafe extern "C" fn h_ui_draw(c: *mut c_void, e: Entity, cmds: *const maker_api::UiDrawCmd, n: usize) {
    let c = unsafe { cx(c) };
    let cmds = if cmds.is_null() || n == 0 { &[][..] } else { unsafe { std::slice::from_raw_parts(cmds, n) } };
    c.ui.fijar_dibujo(id(e), cmds);
}
unsafe extern "C" fn h_ui_rect(c: *mut c_void, e: Entity) -> maker_api::UiRect {
    let c = unsafe { cx(c) };
    match c.ui.rect(id(e)) {
        Some((a, b)) => maker_api::UiRect { x: a[0], y: a[1], w: b[0] - a[0], h: b[1] - a[1] },
        None => maker_api::UiRect::default(),
    }
}
unsafe extern "C" fn h_ui_set_position(c: *mut c_void, e: Entity, p: maker_api::Vec2) {
    let c = unsafe { cx(c) };
    c.ui.editar_rect(c.scene, id(e), |r| r.fijar_posicion([p.x, p.y]));
}
unsafe extern "C" fn h_ui_set_size(c: *mut c_void, e: Entity, s: maker_api::Vec2) {
    let c = unsafe { cx(c) };
    c.ui.editar_rect(c.scene, id(e), |r| r.fijar_tam([s.x, s.y]));
}
unsafe extern "C" fn h_ui_pointer_over(c: *mut c_void) -> bool {
    unsafe { cx(c) }.ui.puntero_sobre_ui()
}
unsafe extern "C" fn h_set_time_scale(c: *mut c_void, s: f32) {
    unsafe { cx(c) }.time_scale = if s.is_finite() { s.clamp(0.0, 100.0) } else { 1.0 };
}
unsafe extern "C" fn h_ui_set_theme(c: *mut c_void, p: *const u8, n: usize) {
    let c = unsafe { cx(c) };
    let t = unsafe { texto(p, n) }.into_owned();
    if !t.is_empty() && !c.ui.recursos.temas.contains_key(&t) {
        log::warn("script", format!("ui_set_theme: no existe el tema '{t}'"));
    }
    c.ui.tema = Some(t);
    c.ui.marcar_sucio();
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
    struct Hud {
        clics: i32,
        juego_vio_clic: bool,
    }
    impl Script for Hud {
        fn update(&mut self, ctx: &mut Ctx) {
            let (Some(t), Some(b), Some(barra), Some(radar)) = (ctx.ui("Puntos"), ctx.ui("Botón"), ctx.ui("Vida"), ctx.ui("Radar")) else {
                panic!("faltan elementos");
            };
            if ctx.ui_clicked(b) {
                self.clics += 1;
            }
            ctx.ui_set_text(t, &format!("Puntos: {}", self.clics));
            assert_eq!(ctx.ui_text(t), format!("Puntos: {}", self.clics));
            ctx.ui_set_value(barra, 0.25);
            assert_eq!(ctx.ui_value(barra), 0.25);
            ctx.ui_draw(radar, |p| {
                p.circle(10.0, 10.0, 5.0, [1.0, 0.0, 0.0, 1.0]);
                p.text(0.0, 0.0, 0.0, 0.0, 12.0, "N", [1.0; 4]);
            });
            self.juego_vio_clic |= ctx.mouse_down(MouseButton::Left);
            if self.clics == 1 && !ctx.ui_visible(t) {
                panic!("visible");
            }
            if self.clics >= 1 {
                ctx.ui_set_style(b, "");
                ctx.ui_set_visible(radar, false);
                let r = ctx.ui_rect(b);
                assert_eq!((r.w, r.h), (200.0, 50.0));
                ctx.log(format!("hud: clics={} juego_vio_clic={}", self.clics, self.juego_vio_clic));
            }
        }
    }


    #[test]
    fn scripts_use_the_ui_through_abi_and_ui_consumes_clicks() {
        use crate::ui::*;
        let lib = Arc::new(ScriptLib::from_static(&TEST_LIB).unwrap());
        let mut s = Scene::starter();
        let l = s.spawn("Lienzo");
        s.insert(l, Lienzo { resolucion: [800.0, 600.0], ..Default::default() });
        let hijo = |s: &mut Scene, n: &str, t: TipoWidget, r: RectUi| {
            let e = s.spawn(n);
            s.insert(e, r);
            s.insert(e, WidgetUi::nuevo(t));
            s.set_parent(e, Some(l), false);
            e
        };
        let txt = hijo(&mut s, "Puntos", TipoWidget::Texto, RectUi::anclado([0.0, 0.0], [0.0, 0.0], [10.0, 10.0], [200.0, 40.0]));
        hijo(&mut s, "Botón", TipoWidget::Boton, RectUi::centrado(200.0, 50.0));
        let barra = hijo(&mut s, "Vida", TipoWidget::Barra, RectUi::anclado([0.0, 1.0], [0.0, 1.0], [10.0, -40.0], [300.0, 20.0]));
        let radar = hijo(&mut s, "Radar", TipoWidget::Personalizado, RectUi::anclado([1.0, 0.0], [1.0, 0.0], [-110.0, 10.0], [100.0, 100.0]));
        s.insert(l, Scripts(vec![ScriptSlot { name: "Hud".into(), enabled: true, ..Default::default() }]));
        let mut rt = Runtime::start(&mut s, Some(lib));
        let mut input = Input::default();
        input.pantalla = glam::Vec2::new(800.0, 600.0);
        rt.tick(&mut s, &input, 1.0 / 60.0);
        assert!(rt.ui.lista.quads.len() > 3, "la interfaz se dibuja tras el primer fotograma");
        // Clic en el botón (centro de la pantalla).
        input.mouse_pos = glam::Vec2::new(400.0, 300.0);
        rt.tick(&mut s, &input, 1.0 / 60.0);
        input.mouse_buttons[0] = true;
        rt.tick(&mut s, &input, 1.0 / 60.0);
        input.mouse_buttons[0] = false;
        rt.tick(&mut s, &input, 1.0 / 60.0);
        assert_eq!(s.get::<WidgetUi>(txt).unwrap().clase.texto(), Some("Puntos: 1"));
        assert_eq!(s.get::<WidgetUi>(barra).unwrap().clase.valor(), 0.25);
        assert!(!s.get::<RectUi>(radar).unwrap().visible);
        assert!(log::since(0).iter().any(|e| e.text.contains("hud: clics=1 juego_vio_clic=false")), "el clic sobre la interfaz no llega al juego");
        // Fuera de la interfaz el juego sí ve el ratón.
        input.mouse_pos = glam::Vec2::new(400.0, 500.0);
        input.mouse_buttons[0] = true;
        rt.tick(&mut s, &input, 1.0 / 60.0);
        assert!(log::since(0).iter().any(|e| e.text.contains("juego_vio_clic=true")));
    }

    #[derive(Default)]
    struct MenuCursor;
    impl Script for MenuCursor {
        fn update(&mut self, ctx: &mut Ctx) {
            ctx.set_cursor_mode(if ctx.frame == 0 { CursorMode::Libre } else { CursorMode::Bloqueado });
        }
    }

    maker_api::script_library!(TEST_LIB; Spinner, Spawner, Panicker, Toonify, Dancer, MenuCursor, GlobalToon, Hud);

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
