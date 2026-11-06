//! Proyectos: carpeta con `project.maker`, escenas, assets y un crate de
//! scripts en Rust que se compila a biblioteca dinámica.

use crate::components::*;
use crate::scene::Scene;
use crate::scripting::dylib_file_name;
use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub const PROJECT_FILE: &str = "project.maker";
pub const SCENE_EXT: &str = "mscene";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ProjectFile {
    pub name: String,
    pub main_scene: String,
    #[serde(default)]
    pub engine_version: String,
}

#[derive(Clone, Debug)]
pub struct Project {
    pub root: PathBuf,
    pub file: ProjectFile,
}

/// Ruta del crate `maker_api` con el que se compilan los scripts.
pub fn maker_api_path() -> PathBuf {
    if let Ok(p) = std::env::var("MAKER_API_PATH") {
        return PathBuf::from(p);
    }
    let compiled = Path::new(env!("CARGO_MANIFEST_DIR")).join("../maker_api");
    compiled.canonicalize().unwrap_or(compiled)
}

pub fn sanitize_crate_name(name: &str) -> String {
    let mut s: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '_' })
        .collect::<String>()
        .split('_')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("_");
    if s.is_empty() || s.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        s = format!("game_{s}");
    }
    format!("{s}_scripts")
}

impl Project {
    pub fn open(root: &Path) -> Result<Project, String> {
        let root = if root.file_name().is_some_and(|n| n == PROJECT_FILE) { root.parent().unwrap_or(root) } else { root };
        let pf = root.join(PROJECT_FILE);
        let txt = std::fs::read_to_string(&pf).map_err(|e| format!("{}: {e}", pf.display()))?;
        let file: ProjectFile = serde_json::from_str(&txt).map_err(|e| format!("{}: {e}", pf.display()))?;
        Ok(Project { root: root.canonicalize().unwrap_or(root.to_path_buf()), file })
    }

    pub fn save(&self) -> Result<(), String> {
        let txt = serde_json::to_string_pretty(&self.file).map_err(|e| e.to_string())?;
        std::fs::write(self.root.join(PROJECT_FILE), txt).map_err(|e| e.to_string())
    }

    pub fn name(&self) -> &str {
        &self.file.name
    }
    pub fn main_scene_path(&self) -> PathBuf {
        self.root.join(&self.file.main_scene)
    }
    pub fn scripts_dir(&self) -> PathBuf {
        self.root.join("scripts")
    }
    pub fn scripts_manifest(&self) -> PathBuf {
        self.scripts_dir().join("Cargo.toml")
    }
    pub fn scripts_crate_name(&self) -> String {
        // Lee el nombre real del Cargo.toml (el usuario puede cambiarlo).
        if let Ok(t) = std::fs::read_to_string(self.scripts_manifest()) {
            for line in t.lines() {
                let l = line.trim();
                if let Some(rest) = l.strip_prefix("name") {
                    let v = rest.trim_start().trim_start_matches('=').trim().trim_matches('"');
                    if !v.is_empty() {
                        return v.to_string();
                    }
                }
            }
        }
        sanitize_crate_name(&self.file.name)
    }
    /// Biblioteca compilada en release.
    pub fn script_lib_path(&self) -> PathBuf {
        self.scripts_dir().join("target").join("release").join(dylib_file_name(&self.scripts_crate_name()))
    }
    pub fn relative(&self, p: &Path) -> String {
        p.strip_prefix(&self.root).unwrap_or(p).to_string_lossy().replace('\\', "/")
    }

    /// Crea un proyecto nuevo. Con `demo` añade una escena jugable de ejemplo.
    pub fn create(root: &Path, name: &str, demo: bool) -> Result<Project, String> {
        if root.join(PROJECT_FILE).exists() {
            return Err(format!("ya hay un proyecto en {}", root.display()));
        }
        for d in ["scenes", "assets/models", "assets/materials", "scripts/src"] {
            std::fs::create_dir_all(root.join(d)).map_err(|e| format!("{}: {e}", root.join(d).display()))?;
        }
        let project = Project {
            root: root.canonicalize().unwrap_or(root.to_path_buf()),
            file: ProjectFile { name: name.into(), main_scene: "scenes/main.mscene".into(), engine_version: env!("CARGO_PKG_VERSION").into() },
        };
        project.save()?;
        let crate_name = sanitize_crate_name(name);
        let api = maker_api_path().to_string_lossy().replace('\\', "/");
        let cargo = SCRIPTS_CARGO_TOML.replace("{name}", &crate_name).replace("{api}", &api);
        write(&project.scripts_manifest(), &cargo)?;
        write(&project.scripts_dir().join("src/lib.rs"), SCRIPTS_LIB_RS)?;
        write(&project.scripts_dir().join(".gitignore"), "target/\n")?;
        write(&project.root.join(".gitignore"), "scripts/target/\nexport/\n")?;
        let scene = if demo { demo_scene() } else { Scene::starter() };
        scene.save(&project.main_scene_path())?;
        Ok(project)
    }

    /// Lanza `cargo build --release` de los scripts. Devuelve el proceso para
    /// leer su salida (el editor la vuelca a la consola).
    pub fn spawn_script_build(&self) -> Result<std::process::Child, String> {
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
        Command::new(cargo)
            .args(["build", "--release", "--color", "never", "--message-format", "short", "--manifest-path"])
            .arg(self.scripts_manifest())
            .env_remove("CARGO_TARGET_DIR")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("no se pudo ejecutar cargo: {e}"))
    }

    /// Compila los scripts de forma síncrona (CLI / tests).
    pub fn build_scripts_blocking(&self) -> Result<String, String> {
        let out = self.spawn_script_build()?.wait_with_output().map_err(|e| e.to_string())?;
        let text = String::from_utf8_lossy(&out.stderr).into_owned();
        if out.status.success() { Ok(text) } else { Err(text) }
    }

    /// Escenas del proyecto (rutas relativas).
    pub fn scenes(&self) -> Vec<String> {
        let mut v = Vec::new();
        walk(&self.root, &mut |p| {
            if p.extension().is_some_and(|e| e == SCENE_EXT) {
                v.push(self.relative(p));
            }
        });
        v.sort();
        v
    }
}

fn write(p: &Path, s: &str) -> Result<(), String> {
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    std::fs::write(p, s).map_err(|e| format!("{}: {e}", p.display()))
}

/// Recorre la carpeta del proyecto saltando `target`, `export` y ocultos.
pub fn walk(dir: &Path, f: &mut dyn FnMut(&Path)) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = rd.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let p = e.path();
        let name = e.file_name();
        let n = name.to_string_lossy();
        if n.starts_with('.') || n == "target" || n == "export" {
            continue;
        }
        if p.is_dir() {
            walk(&p, f);
        } else {
            f(&p);
        }
    }
}

/// Escena de demostración: personaje físico, pila de cajas, luz que flota,
/// toro que gira y cámara que sigue al jugador.
pub fn demo_scene() -> Scene {
    let mut s = Scene::starter();
    let cube = s.find_by_name("Cubo").unwrap();
    s.despawn_recursive(cube);

    let player = s.spawn("Jugador");
    s.set_transform(player, Transform::from_translation(Vec3::new(0.0, 1.5, 3.0)));
    let mut mr = MeshRenderer::primitive(PrimitiveShape::Capsule);
    mr.material = Material { base_color: [0.0, 0.33, 0.99, 1.0], roughness: 0.35, ..Default::default() };
    s.insert(player, mr);
    s.insert(player, RigidBody { lock_rotation: true, ..RigidBody::dynamic() });
    s.insert(player, Collider { friction: 0.0, ..Collider::new(ColliderShape::fit(PrimitiveShape::Capsule)) });
    let mut slot = ScriptSlot { name: "PlayerController".into(), enabled: true, ..Default::default() };
    slot.props.insert("speed".into(), PropVal::Float(5.0));
    slot.props.insert("jump".into(), PropVal::Float(6.0));
    s.insert(player, Scripts(vec![slot]));

    let colors = [[0.81, 0.18, 0.34], [0.86, 0.44, 0.29], [0.75, 0.52, 0.20], [0.12, 0.54, 0.40]];
    let mut i = 0;
    for row in 0..4 {
        for col in 0..(4 - row) {
            let c = s.spawn(&format!("Caja {}", i + 1));
            let x = -3.0 + col as f32 * 1.02 + row as f32 * 0.51;
            s.set_transform(c, Transform::from_translation(Vec3::new(x, 0.5 + row as f32 * 1.0, -2.0)));
            let mut mr = MeshRenderer::primitive(PrimitiveShape::Cube);
            mr.material = Material::color(colors[(i as usize) % colors.len()]);
            s.insert(c, mr);
            s.insert(c, RigidBody::dynamic());
            s.insert(c, Collider::new(ColliderShape::fit(PrimitiveShape::Cube)));
            i += 1;
        }
    }

    let torus = s.spawn("Anillo");
    s.set_transform(torus, Transform::from_translation(Vec3::new(4.0, 2.0, -1.0)).with_scale(Vec3::splat(2.5)).with_rotation(Quat::from_rotation_x(1.2)));
    let mut mr = MeshRenderer::primitive(PrimitiveShape::Torus);
    mr.material = Material { base_color: [0.95, 0.78, 0.35, 1.0], metallic: 1.0, roughness: 0.25, ..Default::default() };
    s.insert(torus, mr);
    let mut slot = ScriptSlot { name: "Rotator".into(), enabled: true, ..Default::default() };
    slot.props.insert("speed".into(), PropVal::Float(45.0));
    s.insert(torus, Scripts(vec![slot]));

    let orb = s.spawn("Orbe de luz");
    s.set_transform(orb, Transform::from_translation(Vec3::new(-2.0, 2.5, 1.5)).with_scale(Vec3::splat(0.3)));
    let mut mr = MeshRenderer::primitive(PrimitiveShape::Sphere);
    mr.material = Material { base_color: [1.0, 0.8, 0.5, 1.0], emissive: [1.0, 0.75, 0.45], emissive_strength: 4.0, ..Default::default() };
    mr.cast_shadows = false;
    s.insert(orb, mr);
    s.insert(orb, Light::point());
    s.insert(orb, Scripts(vec![ScriptSlot { name: "Bobber".into(), enabled: true, ..Default::default() }]));

    let sphere = s.spawn("Esfera");
    s.set_transform(sphere, Transform::from_translation(Vec3::new(2.0, 0.75, 1.0)).with_scale(Vec3::splat(1.5)));
    let mut mr = MeshRenderer::primitive(PrimitiveShape::Sphere);
    mr.material = Material { base_color: [0.9, 0.9, 0.92, 1.0], metallic: 0.0, roughness: 0.15, ..Default::default() };
    s.insert(sphere, mr);
    s.insert(sphere, RigidBody::dynamic());
    s.insert(sphere, Collider { restitution: 0.6, ..Collider::new(ColliderShape::fit(PrimitiveShape::Sphere)) });

    let cam = s.find_by_name("Cámara principal").unwrap();
    let mut slot = ScriptSlot { name: "CameraFollow".into(), enabled: true, ..Default::default() };
    slot.props.insert("offset".into(), PropVal::Vec3([0.0, 4.0, 8.0]));
    s.insert(cam, Scripts(vec![slot]));
    s.update_transforms();
    s
}

const SCRIPTS_CARGO_TOML: &str = r#"# Scripts del juego. Maker Studio los compila con
#   cargo build --release
# y carga la biblioteca resultante en caliente.
[package]
name = "{name}"
version = "0.1.0"
edition = "2024"

[lib]
crate-type = ["cdylib"]

[dependencies]
maker_api = { path = "{api}" }

[profile.release]
opt-level = 3
lto = "thin"
codegen-units = 4
# Necesario: el motor atrapa los pánicos de cada script.
panic = "unwind"

# Crate independiente (no forma parte de otro workspace).
[workspace]
"#;

pub const SCRIPTS_LIB_RS: &str = r#"//! Scripts del juego (Rust). Cada tipo que implementa `Script` aparece en el
//! Inspector de Maker Studio tras compilar (F7). Las propiedades declaradas en
//! `props` se pueden editar desde el editor y se guardan en la escena.

use maker_api::prelude::*;

/// Gira la entidad alrededor del eje Y global.
#[derive(Default)]
pub struct Rotator {
    /// Grados por segundo.
    speed: f32,
}

impl Script for Rotator {
    fn props(&mut self, p: &mut Props) {
        p.float("speed", &mut self.speed);
    }
    fn start(&mut self, _ctx: &mut Ctx) {
        if self.speed == 0.0 {
            self.speed = 90.0;
        }
    }
    fn update(&mut self, ctx: &mut Ctx) {
        ctx.rotate_y(self.speed.to_radians() * ctx.dt);
    }
}

/// Sube y baja suavemente (onda seno).
#[derive(Default)]
pub struct Bobber {
    amplitude: f32,
    frequency: f32,
    base: Option<Vec3>,
}

impl Script for Bobber {
    fn props(&mut self, p: &mut Props) {
        p.float("amplitude", &mut self.amplitude);
        p.float("frequency", &mut self.frequency);
    }
    fn start(&mut self, ctx: &mut Ctx) {
        if self.amplitude == 0.0 {
            self.amplitude = 0.5;
        }
        if self.frequency == 0.0 {
            self.frequency = 0.5;
        }
        self.base = Some(ctx.position());
    }
    fn update(&mut self, ctx: &mut Ctx) {
        let base = self.base.unwrap_or_default();
        let y = (ctx.time * self.frequency * std::f32::consts::TAU).sin() * self.amplitude;
        ctx.set_position(base + Vec3::new(0.0, y, 0.0));
    }
}

/// Personaje con cuerpo rígido: WASD/flechas para moverse, Espacio para saltar,
/// F para lanzar una bola.
#[derive(Default)]
pub struct PlayerController {
    speed: f32,
    jump: f32,
}

impl Script for PlayerController {
    fn props(&mut self, p: &mut Props) {
        p.float("speed", &mut self.speed);
        p.float("jump", &mut self.jump);
    }
    fn start(&mut self, ctx: &mut Ctx) {
        ctx.log("PlayerController listo: WASD para moverte, Espacio para saltar, F para lanzar");
    }
    fn fixed_update(&mut self, ctx: &mut Ctx) {
        let x = ctx.axis(Key::A, Key::D) + ctx.axis(Key::Left, Key::Right);
        let z = ctx.axis(Key::W, Key::S) + ctx.axis(Key::Up, Key::Down);
        let mut v = ctx.velocity();
        let dir = Vec3::new(x, 0.0, z).normalized();
        v.x = dir.x * self.speed;
        v.z = dir.z * self.speed;
        ctx.set_velocity(v);
    }
    fn update(&mut self, ctx: &mut Ctx) {
        let pos = ctx.position();
        if ctx.key_pressed(Key::Space) {
            // ¿Toca el suelo? Rayo hacia abajo desde los pies.
            let feet = pos - Vec3::new(0.0, 1.05, 0.0);
            if ctx.raycast(feet, -Vec3::Y, 0.2).is_some() {
                let mut v = ctx.velocity();
                v.y = self.jump;
                ctx.set_velocity(v);
            }
        }
        if ctx.key_pressed(Key::F) {
            let ball = ctx.spawn(Primitive::Sphere, pos + Vec3::new(0.0, 0.5, -1.2), spawn_flags::DYNAMIC_BODY);
            ctx.set_color(ball, [1.0, 0.4, 0.2, 1.0]);
            ctx.apply_impulse_to(ball, Vec3::new(0.0, 2.0, -12.0));
        }
        if pos.y < -20.0 {
            ctx.set_position(Vec3::new(0.0, 2.0, 3.0));
        }
    }
}

/// Cámara que sigue a una entidad con suavizado.
#[derive(Default)]
pub struct CameraFollow {
    offset: Vec3,
    smoothing: f32,
    target: Option<Entity>,
}

impl Script for CameraFollow {
    fn props(&mut self, p: &mut Props) {
        p.vec3("offset", &mut self.offset);
        p.float("smoothing", &mut self.smoothing);
    }
    fn start(&mut self, ctx: &mut Ctx) {
        self.target = ctx.find("Jugador");
        if self.smoothing == 0.0 {
            self.smoothing = 6.0;
        }
        if self.target.is_none() {
            ctx.warn("CameraFollow: no hay ninguna entidad llamada 'Jugador'");
        }
    }
    fn update(&mut self, ctx: &mut Ctx) {
        let Some(t) = self.target.filter(|t| ctx.exists(*t)) else { return };
        let target = ctx.position_of(t);
        let want = target + self.offset;
        let k = 1.0 - (-self.smoothing * ctx.dt).exp();
        let p = ctx.position().lerp(want, k);
        ctx.set_position(p);
        // Mirar al objetivo (yaw + pitch).
        let d = (target - p).normalized();
        let yaw = (-d.x).atan2(-d.z);
        let pitch = d.y.asin();
        ctx.set_rotation(Quat::from_euler(pitch, yaw, 0.0));
    }
}

export_scripts!(Rotator, Bobber, PlayerController, CameraFollow);
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crate_names_are_valid() {
        assert_eq!(sanitize_crate_name("Mi Juego 3D!"), "mi_juego_3d_scripts");
        assert_eq!(sanitize_crate_name("3D"), "game_3d_scripts");
    }

    #[test]
    fn create_and_open_project() {
        let dir = std::env::temp_dir().join(format!("maker-proj-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let p = Project::create(&dir, "Prueba", true).unwrap();
        let p2 = Project::open(&dir).unwrap();
        assert_eq!(p2.file, p.file);
        assert_eq!(p2.scripts_crate_name(), "prueba_scripts");
        let scene = Scene::load(&p2.main_scene_path()).unwrap();
        assert!(scene.find_by_name("Jugador").is_some());
        assert_eq!(p2.scenes(), vec!["scenes/main.mscene".to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
