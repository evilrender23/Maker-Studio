//! # maker_api — scripting en Rust para Maker Studio
//!
//! Los scripts de un proyecto son un crate `cdylib` que depende de este crate.
//! Cada script es un tipo que implementa [`Script`] (como un `MonoBehaviour` de
//! Unity, un script de nodo de Godot o un `ActorComponent` de Unreal) y se
//! exporta con [`export_scripts!`]:
//!
//! ```ignore
//! use maker_api::prelude::*;
//!
//! #[derive(Default)]
//! pub struct Rotator { speed: f32 }
//!
//! impl Script for Rotator {
//!     fn props(&mut self, p: &mut Props) { p.float("speed", &mut self.speed); }
//!     fn update(&mut self, ctx: &mut Ctx) { ctx.rotate_y(self.speed * ctx.dt); }
//! }
//!
//! export_scripts!(Rotator);
//! ```
//!
//! La frontera entre el motor y la biblioteca es **C-ABI** (`#[repr(C)]` y
//! `extern "C"`), así que la recarga en caliente no depende de la ABI inestable
//! de Rust. Los pánicos de un script se atrapan dentro de la biblioteca y se
//! informan en la consola; nunca cruzan la frontera.

use std::ffi::c_void;
use std::ops::{Add, AddAssign, Mul, Neg, Sub, SubAssign};
use std::panic::{AssertUnwindSafe, catch_unwind};

/// Versión de la ABI. El motor rechaza bibliotecas con otra versión.
pub const ABI_VERSION: u32 = 2;
/// Símbolo que exporta cada biblioteca de scripts.
pub const ENTRY_SYMBOL: &[u8] = b"maker_scripts\0";
/// Firma del símbolo de entrada.
pub type EntryFn = unsafe extern "C" fn() -> *const ScriptLibrary;

// ───────────────────────────── Matemática C-ABI ─────────────────────────────

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec2 {
    pub x: f32,
    pub y: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Vec3 {
    pub const ZERO: Vec3 = Vec3::new(0.0, 0.0, 0.0);
    pub const ONE: Vec3 = Vec3::new(1.0, 1.0, 1.0);
    pub const X: Vec3 = Vec3::new(1.0, 0.0, 0.0);
    pub const Y: Vec3 = Vec3::new(0.0, 1.0, 0.0);
    pub const Z: Vec3 = Vec3::new(0.0, 0.0, 1.0);
    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }
    pub fn dot(self, o: Vec3) -> f32 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }
    pub fn cross(self, o: Vec3) -> Vec3 {
        Vec3::new(self.y * o.z - self.z * o.y, self.z * o.x - self.x * o.z, self.x * o.y - self.y * o.x)
    }
    pub fn length(self) -> f32 {
        self.dot(self).sqrt()
    }
    pub fn normalized(self) -> Vec3 {
        let l = self.length();
        if l > 1e-8 { self * (1.0 / l) } else { Vec3::ZERO }
    }
    pub fn lerp(self, o: Vec3, t: f32) -> Vec3 {
        self + (o - self) * t
    }
    pub fn distance(self, o: Vec3) -> f32 {
        (self - o).length()
    }
}

impl Add for Vec3 {
    type Output = Vec3;
    fn add(self, o: Vec3) -> Vec3 {
        Vec3::new(self.x + o.x, self.y + o.y, self.z + o.z)
    }
}
impl Sub for Vec3 {
    type Output = Vec3;
    fn sub(self, o: Vec3) -> Vec3 {
        Vec3::new(self.x - o.x, self.y - o.y, self.z - o.z)
    }
}
impl Mul<f32> for Vec3 {
    type Output = Vec3;
    fn mul(self, s: f32) -> Vec3 {
        Vec3::new(self.x * s, self.y * s, self.z * s)
    }
}
impl Neg for Vec3 {
    type Output = Vec3;
    fn neg(self) -> Vec3 {
        Vec3::new(-self.x, -self.y, -self.z)
    }
}
impl AddAssign for Vec3 {
    fn add_assign(&mut self, o: Vec3) {
        *self = *self + o;
    }
}
impl SubAssign for Vec3 {
    fn sub_assign(&mut self, o: Vec3) {
        *self = *self - o;
    }
}

/// Cuaternión unitario (x, y, z, w).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quat {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

impl Default for Quat {
    fn default() -> Self {
        Quat::IDENTITY
    }
}

impl Quat {
    pub const IDENTITY: Quat = Quat { x: 0.0, y: 0.0, z: 0.0, w: 1.0 };
    pub fn from_axis_angle(axis: Vec3, radians: f32) -> Quat {
        let a = axis.normalized();
        let (s, c) = (radians * 0.5).sin_cos();
        Quat { x: a.x * s, y: a.y * s, z: a.z * s, w: c }
    }
    /// Ángulos de Euler en radianes, orden YXZ (yaw, pitch, roll) como Unity.
    pub fn from_euler(pitch: f32, yaw: f32, roll: f32) -> Quat {
        Quat::from_axis_angle(Vec3::Y, yaw) * Quat::from_axis_angle(Vec3::X, pitch) * Quat::from_axis_angle(Vec3::Z, roll)
    }
    pub fn rotate(self, v: Vec3) -> Vec3 {
        let q = Vec3::new(self.x, self.y, self.z);
        let t = q.cross(v) * 2.0;
        v + t * self.w + q.cross(t)
    }
    pub fn forward(self) -> Vec3 {
        self.rotate(-Vec3::Z)
    }
    pub fn right(self) -> Vec3 {
        self.rotate(Vec3::X)
    }
    pub fn up(self) -> Vec3 {
        self.rotate(Vec3::Y)
    }
}

impl Mul for Quat {
    type Output = Quat;
    fn mul(self, b: Quat) -> Quat {
        let a = self;
        Quat {
            w: a.w * b.w - a.x * b.x - a.y * b.y - a.z * b.z,
            x: a.w * b.x + a.x * b.w + a.y * b.z - a.z * b.y,
            y: a.w * b.y - a.x * b.z + a.y * b.w + a.z * b.x,
            z: a.w * b.z + a.x * b.y - a.y * b.x + a.z * b.w,
        }
    }
}

/// Teclas que entiende el motor. Los códigos son estables (forman parte de la ABI).
#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    A = 0, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y, Z,
    Num0 = 26, Num1, Num2, Num3, Num4, Num5, Num6, Num7, Num8, Num9,
    Space = 36, Enter, Escape, Tab, Backspace, Shift, Ctrl, Alt,
    Up = 44, Down, Left, Right,
    F1 = 48, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11, F12,
}

impl Key {
    pub const COUNT: u32 = 60;
}

/// Botón del ratón: 0 izquierdo, 1 derecho, 2 central.
#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
    Left = 0,
    Right = 1,
    Middle = 2,
}

/// Primitivas que puede crear un script en tiempo de ejecución.
#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Primitive {
    Cube = 0,
    Sphere = 1,
    Plane = 2,
    Cylinder = 3,
    Cone = 4,
    Capsule = 5,
}

/// Modelo de sombreado de un material (ver `Ctx::set_shading`).
#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shading {
    /// PBR metal-rugosidad (por defecto).
    Pbr = 0,
    /// Cel / toon: bandas duras, sombra tintada, borde y contorno.
    Toon = 1,
    /// Color plano sin luz.
    Unlit = 2,
    /// Rayado a plumilla.
    Hatching = 3,
}

/// Opciones de `spawn`.
pub mod spawn_flags {
    /// Añade cuerpo rígido dinámico y colisionador ajustado a la forma.
    pub const DYNAMIC_BODY: u32 = 1;
    /// Añade colisionador estático.
    pub const STATIC_BODY: u32 = 2;
}

/// Identificador estable de entidad en la escena (0 = ninguna).
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct Entity(pub u64);

impl Entity {
    pub const NONE: Entity = Entity(0);
    pub fn is_none(self) -> bool {
        self.0 == 0
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct RayHit {
    pub entity: Entity,
    pub distance: f32,
    pub point: Vec3,
    pub normal: Vec3,
}

/// Color RGBA lineal-sRGB (0..1), C-ABI.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rgba {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct FrameInfo {
    pub dt: f32,
    pub time: f32,
    pub frame: u64,
}

// ───────────────────────────────── ABI del host ─────────────────────────────

/// Tabla de funciones que el motor ofrece a los scripts. `ctx` es opaco.
#[repr(C)]
pub struct HostApi {
    pub ctx: *mut c_void,
    pub log: unsafe extern "C" fn(*mut c_void, u32, *const u8, usize),
    pub get_position: unsafe extern "C" fn(*mut c_void, Entity) -> Vec3,
    pub set_position: unsafe extern "C" fn(*mut c_void, Entity, Vec3),
    pub get_rotation: unsafe extern "C" fn(*mut c_void, Entity) -> Quat,
    pub set_rotation: unsafe extern "C" fn(*mut c_void, Entity, Quat),
    pub get_scale: unsafe extern "C" fn(*mut c_void, Entity) -> Vec3,
    pub set_scale: unsafe extern "C" fn(*mut c_void, Entity, Vec3),
    pub key_down: unsafe extern "C" fn(*mut c_void, u32) -> bool,
    pub key_pressed: unsafe extern "C" fn(*mut c_void, u32) -> bool,
    pub mouse_down: unsafe extern "C" fn(*mut c_void, u32) -> bool,
    pub mouse_delta: unsafe extern "C" fn(*mut c_void) -> Vec2,
    pub find_entity: unsafe extern "C" fn(*mut c_void, *const u8, usize) -> Entity,
    pub get_velocity: unsafe extern "C" fn(*mut c_void, Entity) -> Vec3,
    pub set_velocity: unsafe extern "C" fn(*mut c_void, Entity, Vec3),
    pub apply_impulse: unsafe extern "C" fn(*mut c_void, Entity, Vec3),
    pub spawn: unsafe extern "C" fn(*mut c_void, u32, Vec3, u32) -> Entity,
    pub destroy: unsafe extern "C" fn(*mut c_void, Entity),
    pub raycast: unsafe extern "C" fn(*mut c_void, Vec3, Vec3, f32, *mut RayHit) -> bool,
    pub set_color: unsafe extern "C" fn(*mut c_void, Entity, Rgba),
    pub exists: unsafe extern "C" fn(*mut c_void, Entity) -> bool,
    // Añadir solo al final: el orden de los campos es parte de la ABI.
    pub set_shading: unsafe extern "C" fn(*mut c_void, Entity, u32),
    /// Reproduce un clip (nombre UTF-8) con un fundido de N segundos. `false`
    /// si la entidad no tiene Animator.
    pub play_animation: unsafe extern "C" fn(*mut c_void, Entity, *const u8, usize, f32) -> bool,
    pub set_animation_speed: unsafe extern "C" fn(*mut c_void, Entity, f32),
}

/// Tipos de propiedad exportada (como `[SerializeField]` / `@export`).
pub mod prop_kind {
    pub const FLOAT: u32 = 0;
    pub const INT: u32 = 1;
    pub const BOOL: u32 = 2;
    pub const VEC3: u32 = 3;
    pub const COLOR: u32 = 4;
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PropValue {
    pub kind: u32,
    pub v: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct PropInfo {
    pub name_ptr: *const u8,
    pub name_len: usize,
    pub value: PropValue,
}

impl Default for PropInfo {
    fn default() -> Self {
        Self { name_ptr: std::ptr::null(), name_len: 0, value: PropValue::default() }
    }
}

pub type LifecycleFn = extern "C" fn(*mut c_void, *const HostApi, Entity, *const FrameInfo) -> bool;

/// Descriptor de un tipo de script dentro de la biblioteca.
#[repr(C)]
pub struct ScriptDesc {
    pub name_ptr: *const u8,
    pub name_len: usize,
    pub create: extern "C" fn() -> *mut c_void,
    pub destroy: extern "C" fn(*mut c_void),
    pub start: LifecycleFn,
    pub update: LifecycleFn,
    pub fixed_update: LifecycleFn,
    pub prop_count: extern "C" fn(*mut c_void) -> u32,
    pub prop_get: extern "C" fn(*mut c_void, u32, *mut PropInfo) -> bool,
    pub prop_set: extern "C" fn(*mut c_void, u32, PropValue) -> bool,
}

// SAFETY: los punteros apuntan a datos `'static` inmutables.
unsafe impl Sync for ScriptDesc {}
unsafe impl Send for ScriptDesc {}

impl ScriptDesc {
    pub const fn of<T: Script>(name: &'static str) -> ScriptDesc {
        ScriptDesc {
            name_ptr: name.as_ptr(),
            name_len: name.len(),
            create: ffi_create::<T>,
            destroy: ffi_destroy::<T>,
            start: ffi_start::<T>,
            update: ffi_update::<T>,
            fixed_update: ffi_fixed_update::<T>,
            prop_count: ffi_prop_count::<T>,
            prop_get: ffi_prop_get::<T>,
            prop_set: ffi_prop_set::<T>,
        }
    }

    /// Nombre del script (host).
    ///
    /// # Safety
    /// El descriptor debe venir de una biblioteca cargada y válida.
    pub unsafe fn name(&self) -> &str {
        unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(self.name_ptr, self.name_len)) }
    }
}

#[repr(C)]
pub struct ScriptLibrary {
    pub abi_version: u32,
    pub count: u32,
    pub scripts: *const ScriptDesc,
}

unsafe impl Sync for ScriptLibrary {}
unsafe impl Send for ScriptLibrary {}

impl ScriptLibrary {
    /// # Safety
    /// `scripts` debe apuntar a `count` descriptores válidos.
    pub unsafe fn descs(&self) -> &[ScriptDesc] {
        if self.count == 0 { &[] } else { unsafe { std::slice::from_raw_parts(self.scripts, self.count as usize) } }
    }
}

// ───────────────────────────── SDK seguro (lado script) ─────────────────────

/// Comportamiento adjuntable a una entidad.
pub trait Script: Default + 'static {
    /// Una vez, antes del primer `update`.
    fn start(&mut self, _ctx: &mut Ctx) {}
    /// Cada fotograma (paso variable).
    fn update(&mut self, _ctx: &mut Ctx) {}
    /// A paso fijo, antes de cada paso de física (60 Hz).
    fn fixed_update(&mut self, _ctx: &mut Ctx) {}
    /// Propiedades editables desde el Inspector.
    fn props(&mut self, _p: &mut Props) {}
}

/// Contexto de ejecución de un script: la entidad dueña, el tiempo y el motor.
pub struct Ctx<'a> {
    api: &'a HostApi,
    pub entity: Entity,
    pub dt: f32,
    pub time: f32,
    pub frame: u64,
}

impl<'a> Ctx<'a> {
    /// # Safety
    /// `api` debe ser la tabla válida que entrega el motor durante la llamada.
    pub unsafe fn from_raw(api: &'a HostApi, entity: Entity, frame: &FrameInfo) -> Self {
        Ctx { api, entity, dt: frame.dt, time: frame.time, frame: frame.frame }
    }

    pub fn log(&self, msg: impl AsRef<str>) {
        let m = msg.as_ref();
        unsafe { (self.api.log)(self.api.ctx, 0, m.as_ptr(), m.len()) }
    }
    pub fn warn(&self, msg: impl AsRef<str>) {
        let m = msg.as_ref();
        unsafe { (self.api.log)(self.api.ctx, 1, m.as_ptr(), m.len()) }
    }
    pub fn error(&self, msg: impl AsRef<str>) {
        let m = msg.as_ref();
        unsafe { (self.api.log)(self.api.ctx, 2, m.as_ptr(), m.len()) }
    }

    // — transform propio —
    pub fn position(&self) -> Vec3 {
        self.position_of(self.entity)
    }
    pub fn set_position(&mut self, p: Vec3) {
        self.set_position_of(self.entity, p)
    }
    pub fn translate(&mut self, d: Vec3) {
        let p = self.position();
        self.set_position(p + d);
    }
    pub fn rotation(&self) -> Quat {
        self.rotation_of(self.entity)
    }
    pub fn set_rotation(&mut self, q: Quat) {
        self.set_rotation_of(self.entity, q)
    }
    /// Gira alrededor de Y global (radianes).
    pub fn rotate_y(&mut self, radians: f32) {
        let q = Quat::from_axis_angle(Vec3::Y, radians) * self.rotation();
        self.set_rotation(q);
    }
    /// Gira alrededor de un eje local (radianes).
    pub fn rotate_local(&mut self, axis: Vec3, radians: f32) {
        let q = self.rotation() * Quat::from_axis_angle(axis, radians);
        self.set_rotation(q);
    }
    pub fn scale(&self) -> Vec3 {
        unsafe { (self.api.get_scale)(self.api.ctx, self.entity) }
    }
    pub fn set_scale(&mut self, s: Vec3) {
        unsafe { (self.api.set_scale)(self.api.ctx, self.entity, s) }
    }

    // — otras entidades —
    pub fn find(&self, name: &str) -> Option<Entity> {
        let e = unsafe { (self.api.find_entity)(self.api.ctx, name.as_ptr(), name.len()) };
        (!e.is_none()).then_some(e)
    }
    pub fn exists(&self, e: Entity) -> bool {
        unsafe { (self.api.exists)(self.api.ctx, e) }
    }
    pub fn position_of(&self, e: Entity) -> Vec3 {
        unsafe { (self.api.get_position)(self.api.ctx, e) }
    }
    pub fn set_position_of(&mut self, e: Entity, p: Vec3) {
        unsafe { (self.api.set_position)(self.api.ctx, e, p) }
    }
    pub fn rotation_of(&self, e: Entity) -> Quat {
        unsafe { (self.api.get_rotation)(self.api.ctx, e) }
    }
    pub fn set_rotation_of(&mut self, e: Entity, q: Quat) {
        unsafe { (self.api.set_rotation)(self.api.ctx, e, q) }
    }
    pub fn set_color(&mut self, e: Entity, rgba: [f32; 4]) {
        let c = Rgba { r: rgba[0], g: rgba[1], b: rgba[2], a: rgba[3] };
        unsafe { (self.api.set_color)(self.api.ctx, e, c) }
    }
    /// Cambia el modelo de sombreado del material de `e` (toon, PBR…).
    pub fn set_shading(&mut self, e: Entity, shading: Shading) {
        unsafe { (self.api.set_shading)(self.api.ctx, e, shading as u32) }
    }
    /// Reproduce el clip `name` del Animator de `e`, fundiéndolo con el
    /// anterior durante `crossfade` segundos. Devuelve `false` si `e` no tiene
    /// Animator.
    pub fn play_animation(&mut self, e: Entity, name: &str, crossfade: f32) -> bool {
        unsafe { (self.api.play_animation)(self.api.ctx, e, name.as_ptr(), name.len(), crossfade) }
    }
    /// Velocidad de reproducción del Animator de `e` (1 = normal, negativa = hacia atrás).
    pub fn set_animation_speed(&mut self, e: Entity, speed: f32) {
        unsafe { (self.api.set_animation_speed)(self.api.ctx, e, speed) }
    }
    pub fn spawn(&mut self, shape: Primitive, at: Vec3, flags: u32) -> Entity {
        unsafe { (self.api.spawn)(self.api.ctx, shape as u32, at, flags) }
    }
    pub fn destroy(&mut self, e: Entity) {
        unsafe { (self.api.destroy)(self.api.ctx, e) }
    }

    // — entrada —
    pub fn key_down(&self, k: Key) -> bool {
        unsafe { (self.api.key_down)(self.api.ctx, k as u32) }
    }
    pub fn key_pressed(&self, k: Key) -> bool {
        unsafe { (self.api.key_pressed)(self.api.ctx, k as u32) }
    }
    pub fn mouse_down(&self, b: MouseButton) -> bool {
        unsafe { (self.api.mouse_down)(self.api.ctx, b as u32) }
    }
    pub fn mouse_delta(&self) -> Vec2 {
        unsafe { (self.api.mouse_delta)(self.api.ctx) }
    }
    /// Eje -1..1 a partir de dos teclas.
    pub fn axis(&self, neg: Key, pos: Key) -> f32 {
        (self.key_down(pos) as i32 - self.key_down(neg) as i32) as f32
    }

    // — física —
    pub fn velocity(&self) -> Vec3 {
        unsafe { (self.api.get_velocity)(self.api.ctx, self.entity) }
    }
    pub fn set_velocity(&mut self, v: Vec3) {
        unsafe { (self.api.set_velocity)(self.api.ctx, self.entity, v) }
    }
    pub fn apply_impulse(&mut self, i: Vec3) {
        unsafe { (self.api.apply_impulse)(self.api.ctx, self.entity, i) }
    }
    pub fn apply_impulse_to(&mut self, e: Entity, i: Vec3) {
        unsafe { (self.api.apply_impulse)(self.api.ctx, e, i) }
    }
    pub fn raycast(&self, origin: Vec3, dir: Vec3, max_distance: f32) -> Option<RayHit> {
        let mut hit = RayHit::default();
        let ok = unsafe { (self.api.raycast)(self.api.ctx, origin, dir, max_distance, &mut hit) };
        ok.then_some(hit)
    }
}

enum PropMode {
    Count,
    Get(u32, PropInfo, bool),
    Set(u32, PropValue, bool),
}

/// Visitante de propiedades: el mismo código sirve para contar, leer y escribir.
pub struct Props {
    mode: PropMode,
    index: u32,
}

impl Props {
    fn visit(&mut self, name: &'static str, kind: u32, get: impl FnOnce() -> [f32; 4], set: impl FnOnce([f32; 4])) {
        let i = self.index;
        self.index += 1;
        match &mut self.mode {
            PropMode::Count => {}
            PropMode::Get(want, info, found) if *want == i => {
                *info = PropInfo { name_ptr: name.as_ptr(), name_len: name.len(), value: PropValue { kind, v: get() } };
                *found = true;
            }
            PropMode::Set(want, val, found) if *want == i && val.kind == kind => {
                set(val.v);
                *found = true;
            }
            _ => {}
        }
    }
    pub fn float(&mut self, name: &'static str, v: &mut f32) {
        let cur = *v;
        self.visit(name, prop_kind::FLOAT, || [cur, 0.0, 0.0, 0.0], |n| *v = n[0]);
    }
    pub fn int(&mut self, name: &'static str, v: &mut i32) {
        let cur = *v;
        self.visit(name, prop_kind::INT, || [cur as f32, 0.0, 0.0, 0.0], |n| *v = n[0].round() as i32);
    }
    pub fn bool(&mut self, name: &'static str, v: &mut bool) {
        let cur = *v;
        self.visit(name, prop_kind::BOOL, || [if cur { 1.0 } else { 0.0 }, 0.0, 0.0, 0.0], |n| *v = n[0] > 0.5);
    }
    pub fn vec3(&mut self, name: &'static str, v: &mut Vec3) {
        let cur = *v;
        self.visit(name, prop_kind::VEC3, || [cur.x, cur.y, cur.z, 0.0], |n| *v = Vec3::new(n[0], n[1], n[2]));
    }
    pub fn color(&mut self, name: &'static str, v: &mut [f32; 4]) {
        let cur = *v;
        self.visit(name, prop_kind::COLOR, || cur, |n| *v = n);
    }
}

// ─────────────── Puentes extern "C" genéricos (lado biblioteca) ───────────────

fn panic_message(p: Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = p.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = p.downcast_ref::<String>() {
        s.clone()
    } else {
        "pánico sin mensaje".into()
    }
}

extern "C" fn ffi_create<T: Script>() -> *mut c_void {
    match catch_unwind(T::default) {
        Ok(v) => Box::into_raw(Box::new(v)) as *mut c_void,
        Err(_) => std::ptr::null_mut(),
    }
}

extern "C" fn ffi_destroy<T: Script>(this: *mut c_void) {
    if !this.is_null() {
        let _ = catch_unwind(AssertUnwindSafe(|| drop(unsafe { Box::from_raw(this as *mut T) })));
    }
}

fn run_lifecycle<T: Script>(
    this: *mut c_void,
    api: *const HostApi,
    entity: Entity,
    frame: *const FrameInfo,
    stage: &str,
    f: fn(&mut T, &mut Ctx),
) -> bool {
    if this.is_null() || api.is_null() || frame.is_null() {
        return false;
    }
    let api = unsafe { &*api };
    let frame = unsafe { &*frame };
    let res = catch_unwind(AssertUnwindSafe(|| {
        let s = unsafe { &mut *(this as *mut T) };
        let mut ctx = unsafe { Ctx::from_raw(api, entity, frame) };
        f(s, &mut ctx);
    }));
    match res {
        Ok(()) => true,
        Err(p) => {
            let msg = format!("pánico en {}::{}: {}", std::any::type_name::<T>(), stage, panic_message(p));
            unsafe { (api.log)(api.ctx, 2, msg.as_ptr(), msg.len()) };
            false
        }
    }
}

extern "C" fn ffi_start<T: Script>(this: *mut c_void, api: *const HostApi, e: Entity, f: *const FrameInfo) -> bool {
    run_lifecycle::<T>(this, api, e, f, "start", T::start)
}
extern "C" fn ffi_update<T: Script>(this: *mut c_void, api: *const HostApi, e: Entity, f: *const FrameInfo) -> bool {
    run_lifecycle::<T>(this, api, e, f, "update", T::update)
}
extern "C" fn ffi_fixed_update<T: Script>(this: *mut c_void, api: *const HostApi, e: Entity, f: *const FrameInfo) -> bool {
    run_lifecycle::<T>(this, api, e, f, "fixed_update", T::fixed_update)
}

extern "C" fn ffi_prop_count<T: Script>(this: *mut c_void) -> u32 {
    if this.is_null() {
        return 0;
    }
    let s = unsafe { &mut *(this as *mut T) };
    let mut p = Props { mode: PropMode::Count, index: 0 };
    match catch_unwind(AssertUnwindSafe(|| s.props(&mut p))) {
        Ok(()) => p.index,
        Err(_) => 0,
    }
}

extern "C" fn ffi_prop_get<T: Script>(this: *mut c_void, idx: u32, out: *mut PropInfo) -> bool {
    if this.is_null() || out.is_null() {
        return false;
    }
    let s = unsafe { &mut *(this as *mut T) };
    let mut p = Props { mode: PropMode::Get(idx, PropInfo::default(), false), index: 0 };
    if catch_unwind(AssertUnwindSafe(|| s.props(&mut p))).is_err() {
        return false;
    }
    if let PropMode::Get(_, info, true) = p.mode {
        unsafe { *out = info };
        true
    } else {
        false
    }
}

extern "C" fn ffi_prop_set<T: Script>(this: *mut c_void, idx: u32, val: PropValue) -> bool {
    if this.is_null() {
        return false;
    }
    let s = unsafe { &mut *(this as *mut T) };
    let mut p = Props { mode: PropMode::Set(idx, val, false), index: 0 };
    if catch_unwind(AssertUnwindSafe(|| s.props(&mut p))).is_err() {
        return false;
    }
    matches!(p.mode, PropMode::Set(_, _, true))
}

/// Declara una `static` [`ScriptLibrary`] con los tipos dados (sin exportar símbolo).
#[macro_export]
macro_rules! script_library {
    ($vis:vis $name:ident; $($t:ident),* $(,)?) => {
        $vis static $name: $crate::ScriptLibrary = {
            const DESCS: &[$crate::ScriptDesc] = &[$($crate::ScriptDesc::of::<$t>(stringify!($t))),*];
            $crate::ScriptLibrary { abi_version: $crate::ABI_VERSION, count: DESCS.len() as u32, scripts: DESCS.as_ptr() }
        };
    };
}

/// Exporta los scripts del crate para que Maker Studio los cargue.
#[macro_export]
macro_rules! export_scripts {
    ($($t:ident),* $(,)?) => {
        $crate::script_library!(__MAKER_SCRIPT_LIBRARY; $($t),*);
        #[unsafe(no_mangle)]
        pub extern "C" fn maker_scripts() -> *const $crate::ScriptLibrary {
            &__MAKER_SCRIPT_LIBRARY
        }
    };
}

pub mod prelude {
    pub use crate::{Ctx, Entity, Key, MouseButton, Primitive, Props, Quat, RayHit, Script, Shading, Vec2, Vec3, export_scripts, spawn_flags};
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Mover {
        speed: f32,
        on: bool,
        dir: Vec3,
    }
    impl Script for Mover {
        fn props(&mut self, p: &mut Props) {
            p.float("speed", &mut self.speed);
            p.bool("on", &mut self.on);
            p.vec3("dir", &mut self.dir);
        }
    }
    script_library!(LIB; Mover);

    #[test]
    fn props_roundtrip_through_abi() {
        let d = unsafe { &LIB.descs()[0] };
        assert_eq!(unsafe { d.name() }, "Mover");
        let inst = (d.create)();
        assert_eq!((d.prop_count)(inst), 3);
        assert!((d.prop_set)(inst, 0, PropValue { kind: prop_kind::FLOAT, v: [4.5, 0.0, 0.0, 0.0] }));
        // tipo equivocado → rechazado
        assert!(!(d.prop_set)(inst, 1, PropValue { kind: prop_kind::FLOAT, v: [1.0; 4] }));
        let mut info = PropInfo::default();
        assert!((d.prop_get)(inst, 0, &mut info));
        assert_eq!(info.value.v[0], 4.5);
        assert!((d.prop_get)(inst, 2, &mut info));
        let name = unsafe { std::str::from_utf8(std::slice::from_raw_parts(info.name_ptr, info.name_len)).unwrap() };
        assert_eq!(name, "dir");
        (d.destroy)(inst);
    }

    #[test]
    fn quat_rotates() {
        let q = Quat::from_axis_angle(Vec3::Y, std::f32::consts::FRAC_PI_2);
        let v = q.rotate(Vec3::X);
        assert!((v.z + 1.0).abs() < 1e-5, "{v:?}");
    }
}
