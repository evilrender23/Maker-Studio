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
    /// Tema de interfaz del proyecto (nombre de un `.mtema`); vacío = aspecto neutro del motor.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tema_ui: String,
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
            file: ProjectFile { name: name.into(), main_scene: "scenes/main.mscene".into(), engine_version: env!("CARGO_PKG_VERSION").into(), tema_ui: String::new() },
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
        let mut project = project;
        if demo {
            demo_ui(&mut project)?;
            library_scene().save(&project.root.join("scenes/biblioteca.mscene"))?;
        }
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
/// toro que gira y cámara que sigue al jugador. Mezcla modelos de sombreado:
/// suelo, jugador y cajas en toon, la esfera a plumilla y el anillo y el orbe en PBR.
pub fn demo_scene() -> Scene {
    let mut s = Scene::starter();
    let cube = s.find_by_name("Cubo").unwrap();
    s.despawn_recursive(cube);
    if let Some(ground) = s.find_by_name("Suelo")
        && let Some(mut mr) = s.get::<MeshRenderer>(ground)
    {
        mr.material.shading = ShadingModel::Toon;
        mr.material.style.outline_width = 0.0;
        mr.material.style.rim_strength = 0.0;
        s.insert(ground, mr);
    }

    let player = s.spawn("Jugador");
    s.set_transform(player, Transform::from_translation(Vec3::new(0.0, 1.5, 3.0)));
    let mut mr = MeshRenderer::primitive(PrimitiveShape::Capsule);
    mr.material = Material { base_color: [0.0, 0.33, 0.99, 1.0], roughness: 0.35, ..Material::toon([0.0; 3]) };
    mr.material.style.specular = 0.5;
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
            mr.material = Material::toon(colors[(i as usize) % colors.len()]);
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
    mr.material = Material { base_color: [0.96, 0.94, 0.88, 1.0], shading: ShadingModel::Hatching, ..Default::default() };
    mr.material.style.outline_color = [0.12, 0.13, 0.22];
    s.insert(sphere, mr);
    s.insert(sphere, RigidBody::dynamic());
    s.insert(sphere, Collider { restitution: 0.6, ..Collider::new(ColliderShape::fit(PrimitiveShape::Sphere)) });

    let cam = s.find_by_name("Cámara principal").unwrap();
    let mut slot = ScriptSlot { name: "CameraFollow".into(), enabled: true, ..Default::default() };
    slot.props.insert("offset".into(), PropVal::Vec3([0.0, 4.0, 8.0]));
    s.insert(cam, Scripts(vec![slot]));
    demo_hud(&mut s);
    s.update_transforms();
    s
}

/// Hoja de estilos, tema y fuente de la interfaz de la demo (tema `neon`).
fn demo_ui(p: &mut Project) -> Result<(), String> {
    use crate::ui::{DIR_UI, EXT_ESTILO, EXT_TEMA, estilo::Tema};
    write(&p.root.join(format!("{DIR_UI}/estilos/neon.{EXT_ESTILO}")), include_str!("ui/demo_neon.mestilo"))?;
    let mut tema = Tema::default();
    for (k, v) in [("Panel", "neon_panel"), ("Texto", "neon_texto"), ("Boton", "neon_boton"), ("Barra", "neon_barra"), ("Deslizador", "neon_deslizador"), ("Casilla", "neon_casilla")] {
        tema.tipos.insert(k.into(), v.into());
    }
    write(&p.root.join(format!("{DIR_UI}/temas/neon.{EXT_TEMA}")), &serde_json::to_string_pretty(&tema).map_err(|e| e.to_string())?)?;
    // Fuente propia del proyecto (Hack, licencia MIT + Bitstream Vera).
    let f = p.root.join(format!("{DIR_UI}/fuentes/Hack-Regular.ttf"));
    if let Some(d) = f.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    std::fs::write(&f, epaint_default_fonts::HACK_REGULAR).map_err(|e| format!("{}: {e}", f.display()))?;
    write(&p.root.join(format!("{DIR_UI}/fuentes/Hack-LICENSE.txt")), "Hack (https://github.com/source-foundry/Hack): MIT License + Bitstream Vera License.\n")?;
    p.file.tema_ui = "neon".into();
    p.save()
}

/// HUD de la demo: puntos, barra de vida, radar dibujado por script y menú
/// de pausa (Esc) con botones. Lo controla el script `Hud`.
fn demo_hud(s: &mut Scene) {
    use crate::ui::*;
    let hud = s.spawn("HUD");
    s.insert(hud, Lienzo::default());
    s.insert(hud, Scripts(vec![ScriptSlot { name: "Hud".into(), enabled: true, ..Default::default() }]));
    let hijo = |s: &mut Scene, padre: EntityId, nombre: &str, r: RectUi, w: Option<WidgetUi>| {
        let e = s.spawn(nombre);
        s.insert(e, r);
        if let Some(w) = w {
            s.insert(e, w);
        }
        s.set_parent(e, Some(padre), false);
        e
    };
    let texto = |t: &str, estilo: &str| WidgetUi { clase: ClaseWidget::Texto(Texto { texto: t.into(), ..Default::default() }), estilo: estilo.into(), ..Default::default() };

    hijo(s, hud, "Puntos", RectUi::anclado([0.0, 0.0], [0.0, 0.0], [40.0, 30.0], [520.0, 56.0]), Some(texto("PUNTOS 0000", "neon_puntos")));
    let vida = WidgetUi { clase: ClaseWidget::Barra(Barra { valor: 1.0, ..Default::default() }), ..Default::default() };
    hijo(s, hud, "Vida", RectUi::anclado([0.0, 0.0], [0.0, 0.0], [40.0, 96.0], [380.0, 26.0]), Some(vida));
    hijo(s, hud, "Radar", RectUi::anclado([1.0, 0.0], [1.0, 0.0], [-30.0, 30.0], [240.0, 240.0]), Some(WidgetUi::nuevo(TipoWidget::Personalizado)));
    let ayuda = texto("Esc: pausa", "");
    let mut ayuda_r = RectUi::anclado([0.0, 1.0], [0.0, 1.0], [40.0, -24.0], [400.0, 36.0]);
    ayuda_r.opacidad = 0.7;
    let a = hijo(s, hud, "Ayuda", ayuda_r, Some(ayuda));
    if let Some(mut w) = s.get_mut::<WidgetUi>(a) {
        w.local.estados.normal.tam_texto = Some(22.0);
    }

    // Pausa: velo a pantalla completa con el menú centrado.
    let mut velo_r = RectUi::estirado();
    velo_r.visible = false;
    let velo = hijo(s, hud, "Pausa", velo_r, Some(WidgetUi { estilo: "neon_velo".into(), ..WidgetUi::nuevo(TipoWidget::Panel) }));
    let menu = hijo(s, velo, "Menú", RectUi::centrado(460.0, 420.0), Some(WidgetUi::nuevo(TipoWidget::Panel)));
    s.insert(menu, Contenedor { tipo: TipoContenedor::Vertical, relleno: [36.0, 30.0, 36.0, 34.0], espacio: [0.0, 18.0], cruzado: Alinear::Estirar, ajustar: true, ..Default::default() });
    let mut t = texto("PAUSA", "neon_titulo");
    if let ClaseWidget::Texto(x) = &mut t.clase {
        x.alinear_h = AlinH::Centro;
    }
    hijo(s, menu, "Título", RectUi::centrado(380.0, 70.0), Some(t));
    let boton = |t: &str| WidgetUi { clase: ClaseWidget::Boton(Boton { texto: t.into() }), ..Default::default() };
    hijo(s, menu, "Continuar", RectUi::centrado(380.0, 60.0), Some(boton("Continuar")));
    hijo(s, menu, "Reiniciar", RectUi::centrado(380.0, 60.0), Some(boton("Reiniciar puntos")));
    let radar = WidgetUi { clase: ClaseWidget::Casilla(Casilla { activa: true, texto: "Radar".into() }), ..Default::default() };
    hijo(s, menu, "Ver radar", RectUi::centrado(380.0, 40.0), Some(radar));
}

/// Material para modelos de la biblioteca: el color sale de sus texturas
/// (tinte blanco) y la rugosidad alta evita brillos de plástico.
pub fn library_material() -> Material {
    Material { base_color: [1.0; 4], roughness: 0.85, ..Default::default() }
}

/// Escena de escaparate de la biblioteca integrada: esquina de una casa
/// medieval, vegetación con hojas recortadas, accesorios en toon y un
/// personaje con piel que camina (script `Paseante`).
pub fn library_scene() -> Scene {
    use crate::biblioteca::{DEFAULT_CLIPS, uri};
    let mut s = Scene::starter();
    let cube = s.find_by_name("Cubo").unwrap();
    s.despawn_recursive(cube);
    s.env.sky_top = [0.33, 0.55, 0.9];
    s.env.sky_horizon = [0.8, 0.88, 0.96];
    s.env.ground = [0.36, 0.42, 0.3];
    s.env.ambient_intensity = 0.7;
    s.env.fog_density = 0.012;
    s.env.fog_color = [0.78, 0.86, 0.95];
    if let Some(ground) = s.find_by_name("Suelo") {
        s.set_transform(ground, Transform::default().with_scale(Vec3::new(80.0, 1.0, 80.0)));
        if let Some(mut mr) = s.get::<MeshRenderer>(ground) {
            mr.material = Material { base_color: [0.38, 0.55, 0.25, 1.0], roughness: 0.95, ..Default::default() };
            s.insert(ground, mr);
        }
    }
    if let Some(sun) = s.find_by_name("Sol") {
        let mut t = s.transform(sun);
        t.set_euler_degrees(Vec3::new(-38.0, -40.0, 0.0));
        s.set_transform(sun, t);
        s.insert(sun, Light { intensity: 3.4, ..Light::directional() });
    }
    if let Some(cam) = s.find_by_name("Cámara principal") {
        let eye = Vec3::new(3.6, 3.3, 8.5);
        let target = Vec3::new(-1.6, 1.3, -3.2);
        let d = (target - eye).normalize();
        let mut t = Transform::from_translation(eye);
        t.rotation = Quat::from_euler(glam::EulerRot::YXZ, (-d.x).atan2(-d.z), d.y.asin(), 0.0);
        s.set_transform(cam, t);
        if let Some(mut c) = s.get::<Camera>(cam) {
            c.fov_y = 50.0;
            s.insert(cam, c);
        }
    }
    let add = |s: &mut Scene, name: &str, rel: &str, pos: [f32; 3], yaw: f32, scale: f32, toon: bool| -> EntityId {
        let e = s.spawn(name);
        s.set_transform(e, Transform::from_translation(Vec3::from(pos)).with_rotation(Quat::from_rotation_y(yaw.to_radians())).with_scale(Vec3::splat(scale)));
        let mut material = library_material();
        if toon {
            material.shading = ShadingModel::Toon;
            material.style.outline_width = 2.0;
            material.style.rim_strength = 0.35;
        }
        s.insert(e, MeshRenderer { mesh: MeshSource::Model(uri(rel)), material, cast_shadows: true });
        e
    };
    // Casa: dos paredes en esquina, suelo, tejado.
    let house = s.spawn("Casa");
    s.set_transform(house, Transform::from_translation(Vec3::new(-4.0, 0.0, -6.0)));
    let parts: [(&str, &str, [f32; 3], f32); 12] = [
        ("Pared puerta", "medieval/Wall_Plaster_Door_Round.glb", [-1.0, 0.0, 2.0], 0.0),
        ("Puerta", "medieval/Door_1_Round.glb", [-1.53, 0.0, 1.95], 0.0),
        ("Pared ventana", "medieval/Wall_Plaster_Window_Wide_Round.glb", [1.0, 0.0, 2.0], 0.0),
        ("Ventana", "medieval/Window_Wide_Round1.glb", [1.0, 0.0, 2.0], 0.0),
        ("Contraventanas", "medieval/WindowShutters_Wide_Round_Open.glb", [1.0, 0.0, 2.0], 0.0),
        ("Pared lateral", "medieval/Wall_Plaster_WoodGrid.glb", [2.0, 0.0, 1.0], 90.0),
        ("Pared lateral 2", "medieval/Wall_Plaster_Window_Wide_Round.glb", [2.0, 0.0, -1.0], 90.0),
        ("Ventana lateral", "medieval/Window_Wide_Round1.glb", [2.0, 0.0, -1.0], 90.0),
        ("Esquina", "medieval/Corner_Exterior_Wood.glb", [2.0, 0.0, 2.0], 0.0),
        ("Tejado", "medieval/Roof_RoundTiles_4x4.glb", [0.0, 3.1, 0.0], 0.0),
        ("Chimenea", "medieval/Prop_Chimney.glb", [-1.0, 4.4, -1.0], 0.0),
        ("Enredadera", "medieval/Prop_Vine1.glb", [1.6, 2.9, 2.12], 0.0),
    ];
    for (name, rel, pos, yaw) in parts {
        let e = add(&mut s, name, rel, pos, yaw, 1.0, false);
        s.set_parent(e, Some(house), false);
    }
    for (i, (x, z)) in [(-1.0, 1.0), (1.0, 1.0), (-1.0, -1.0), (1.0, -1.0)].into_iter().enumerate() {
        let e = add(&mut s, &format!("Suelo {}", i + 1), "medieval/Floor_WoodDark.glb", [x, 0.02, z], 0.0, 1.0, false);
        s.set_parent(e, Some(house), false);
    }
    // Vegetación (hojas con recorte alfa).
    let nature: [(&str, &str, [f32; 3], f32, f32); 14] = [
        ("Árbol", "naturaleza/CommonTree_1.glb", [4.5, 0.0, -7.5], 20.0, 1.0),
        ("Pino", "naturaleza/Pine_2.glb", [9.0, 0.0, -12.0], 0.0, 1.1),
        ("Pino 2", "naturaleza/Pine_4.glb", [-11.0, 0.0, -12.0], 40.0, 1.0),
        ("Árbol 2", "naturaleza/CommonTree_3.glb", [-9.5, 0.0, -3.0], 0.0, 0.9),
        ("Arbusto", "naturaleza/Bush_Common_Flowers.glb", [1.0, 0.0, -4.2], 0.0, 0.9),
        ("Arbusto 2", "naturaleza/Bush_Common.glb", [-6.9, 0.0, -2.6], 30.0, 1.0),
        ("Flores", "naturaleza/Flower_3_Group.glb", [2.7, 0.0, -1.4], 0.0, 0.8),
        ("Flores 2", "naturaleza/Flower_4_Group.glb", [-6.0, 0.0, 0.2], 60.0, 0.8),
        ("Helecho", "naturaleza/Fern_1.glb", [3.6, 0.0, -4.4], 10.0, 0.8),
        ("Hierba", "naturaleza/Grass_Common_Tall.glb", [1.8, 0.0, 0.6], 0.0, 0.8),
        ("Hierba 2", "naturaleza/Grass_Common_Short.glb", [-1.2, 0.0, 1.2], 0.0, 1.0),
        ("Hierba 3", "naturaleza/Grass_Wispy_Tall.glb", [-6.6, 0.0, -1.0], 0.0, 0.9),
        ("Roca", "naturaleza/Rock_Medium_1.glb", [6.5, 0.0, -3.5], 30.0, 0.6),
        ("Trébol", "naturaleza/Clover_1.glb", [0.4, 0.0, 2.6], 0.0, 0.9),
    ];
    for (name, rel, pos, yaw, scale) in nature {
        add(&mut s, name, rel, pos, yaw, scale, false);
    }
    for (i, (x, z)) in [(-5.0, -2.7), (-4.6, -0.7), (-3.9, 1.3), (-2.9, 3.2)].into_iter().enumerate() {
        add(&mut s, &format!("Camino {}", i + 1), "naturaleza/RockPath_Round_Wide.glb", [x, 0.0, z], i as f32 * 35.0, 0.9, false);
    }
    // Accesorios en toon.
    let props: [(&str, &str, [f32; 3], f32); 6] = [
        ("Barril", "props_fantasia/Barrel.glb", [-1.4, 0.0, -3.4], 0.0),
        ("Barril de manzanas", "props_fantasia/Barrel_Apples.glb", [-0.6, 0.0, -3.0], 0.0),
        ("Caja", "props_fantasia/Crate_Wooden.glb", [-1.3, 0.0, -2.4], 20.0),
        ("Banco", "props_fantasia/Bench.glb", [-3.0, 0.0, -3.2], 0.0),
        ("Carro", "props_fantasia/Stall_Cart_Empty.glb", [3.4, 0.0, -2.0], -60.0),
        ("Farol", "props_fantasia/Lantern_Wall.glb", [-4.0, 1.2, -3.95], 0.0),
    ];
    for (name, rel, pos, yaw) in props {
        add(&mut s, name, rel, pos, yaw, 1.0, true);
    }
    // Personajes con piel animados con la Universal Animation Library: un
    // explorador que pasea (script) y una maniquí sentada en el banco.
    let hero = add(&mut s, "Personaje", "atuendos/Male_Ranger.glb", [0.6, 0.0, 0.2], 200.0, 1.0, true);
    s.insert(hero, crate::components::Animator::new(DEFAULT_CLIPS, "Walk_Loop"));
    let sitter = add(&mut s, "Maniquí", "animaciones2/Mannequin_F.glb", [-3.0, 0.0, -2.95], 0.0, 1.0, false);
    s.insert(sitter, crate::components::Animator::new(DEFAULT_CLIPS, "Sitting_Idle_Loop"));
    let mut slot = ScriptSlot { name: "Paseante".into(), enabled: true, ..Default::default() };
    slot.props.insert("radius".into(), PropVal::Float(2.2));
    slot.props.insert("speed".into(), PropVal::Float(1.2));
    s.insert(hero, Scripts(vec![slot]));
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
            ctx.set_shading(ball, Shading::Toon);
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

/// Camina en círculo con el clip `Walk_Loop` del Animator y se para a
/// descansar (`Idle_Loop`) cada vuelta, con fundidos entre clips.
#[derive(Default)]
pub struct Paseante {
    radius: f32,
    speed: f32,
    center: Vec3,
    angle: f32,
    rest: f32,
}

impl Script for Paseante {
    fn props(&mut self, p: &mut Props) {
        p.float("radius", &mut self.radius);
        p.float("speed", &mut self.speed);
    }
    fn start(&mut self, ctx: &mut Ctx) {
        if self.radius == 0.0 {
            self.radius = 2.0;
        }
        if self.speed == 0.0 {
            self.speed = 1.2;
        }
        self.center = ctx.position() - Vec3::new(self.radius, 0.0, 0.0);
        let me = ctx.entity;
        ctx.play_animation(me, "Walk_Loop", 0.0);
    }
    fn update(&mut self, ctx: &mut Ctx) {
        let me = ctx.entity;
        if self.rest > 0.0 {
            self.rest -= ctx.dt;
            if self.rest <= 0.0 {
                ctx.play_animation(me, "Walk_Loop", 0.3);
            }
            return;
        }
        let before = self.angle;
        self.angle += self.speed / self.radius * ctx.dt;
        if (before / std::f32::consts::TAU).floor() != (self.angle / std::f32::consts::TAU).floor() {
            self.rest = 2.5;
            ctx.play_animation(me, "Idle_Loop", 0.3);
        }
        let (s, c) = self.angle.sin_cos();
        ctx.set_position(self.center + Vec3::new(c * self.radius, 0.0, -s * self.radius));
        // Mirar en la dirección de avance (tangente del círculo).
        ctx.set_rotation(Quat::from_euler(0.0, (-s).atan2(-c), 0.0));
    }
}

/// HUD de la demo: puntos por cajas derribadas, vida que baja al caer del
/// mapa, radar dibujado a mano y menú de pausa con Esc.
#[derive(Default)]
pub struct Hud {
    cajas: Vec<(Entity, Vec3)>,
    jugador: Option<Entity>,
    vida: f32,
    cayendo: bool,
    pausa: bool,
    base: u32,
}

impl Script for Hud {
    fn start(&mut self, ctx: &mut Ctx) {
        self.jugador = ctx.find("Jugador");
        self.vida = 1.0;
        for i in 1..=10 {
            if let Some(c) = ctx.find(&format!("Caja {i}")) {
                self.cajas.push((c, ctx.position_of(c)));
            }
        }
    }
    fn update(&mut self, ctx: &mut Ctx) {
        // Pausa: Esc o el botón Continuar.
        let continuar = ctx.ui("Continuar").is_some_and(|b| ctx.ui_clicked(b));
        if ctx.key_pressed(Key::Escape) || (self.pausa && continuar) {
            self.pausa = !self.pausa;
            if let Some(p) = ctx.ui("Pausa") {
                ctx.ui_set_visible(p, self.pausa);
            }
            ctx.set_time_scale(if self.pausa { 0.0 } else { 1.0 });
        }
        // Cajas fuera de su sitio = puntos.
        let movidas = self.cajas.iter().filter(|(c, p0)| ctx.exists(*c) && (ctx.position_of(*c) - *p0).length() > 0.5).count() as u32;
        if ctx.ui("Reiniciar").is_some_and(|b| ctx.ui_clicked(b)) {
            self.base = movidas;
        }
        if let Some(t) = ctx.ui("Puntos") {
            ctx.ui_set_text(t, &format!("PUNTOS {:04}", movidas.saturating_sub(self.base) * 100));
        }
        // Vida: cada caída del mapa quita un cuarto; se recupera despacio.
        if let Some(j) = self.jugador.filter(|j| ctx.exists(*j)) {
            let y = ctx.position_of(j).y;
            if y < -5.0 && !self.cayendo {
                self.vida = (self.vida - 0.25).max(0.0);
            }
            self.cayendo = y < -5.0;
        }
        self.vida = (self.vida + ctx.dt * 0.02).min(1.0);
        if let Some(v) = ctx.ui("Vida") {
            ctx.ui_set_value(v, self.vida);
        }
        self.radar(ctx);
    }
}

impl Hud {
    /// Radar: anillos, las cajas como puntos y el jugador en el centro.
    fn radar(&self, ctx: &mut Ctx) {
        let Some(r) = ctx.ui("Radar") else { return };
        let ver = ctx.ui("Ver radar").map(|c| ctx.ui_value(c) > 0.5).unwrap_or(true);
        ctx.ui_set_visible(r, ver);
        let Some(j) = self.jugador.filter(|j| ctx.exists(*j)) else { return };
        let centro = ctx.position_of(j);
        let rect = ctx.ui_rect(r);
        let (cx, cy, rad) = (rect.w * 0.5, rect.h * 0.5, rect.w.min(rect.h) * 0.5 - 6.0);
        let alcance = 12.0;
        let t = ctx.time;
        let puntos: Vec<(f32, f32)> = self
            .cajas
            .iter()
            .filter(|(c, _)| ctx.exists(*c))
            .map(|(c, _)| ctx.position_of(*c) - centro)
            .filter(|d| d.x * d.x + d.z * d.z < alcance * alcance)
            .map(|d| (cx + d.x / alcance * rad, cy + d.z / alcance * rad))
            .collect();
        ctx.ui_draw(r, |p| {
            p.circle(cx, cy, rad, [0.02, 0.03, 0.08, 0.85]);
            p.ring(cx, cy, rad, 2.0, [0.0, 0.95, 0.85, 0.9]);
            p.ring(cx, cy, rad * 0.5, 1.0, [0.0, 0.95, 0.85, 0.35]);
            p.line(cx - rad, cy, cx + rad, cy, 1.0, [0.0, 0.95, 0.85, 0.25]);
            p.line(cx, cy - rad, cx, cy + rad, 1.0, [0.0, 0.95, 0.85, 0.25]);
            // Barrido giratorio.
            let a = (t * 120.0) % 360.0;
            p.arc(cx, cy, rad * 0.75, a - 40.0, a, rad * 0.5, [0.0, 0.95, 0.85, 0.18]);
            for (x, y) in &puntos {
                p.circle(*x, *y, 5.0, [1.0, 0.78, 0.18, 1.0]);
            }
            p.circle(cx, cy, 7.0, [0.98, 0.25, 0.55, 1.0]);
            p.text_centered(0.0, rect.h - 30.0, rect.w, 24.0, 16.0, "RADAR", [0.0, 0.95, 0.85, 0.8]);
        });
    }
}

export_scripts!(Rotator, Bobber, PlayerController, CameraFollow, Paseante, Hud);
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
        let player = scene.find_by_name("Jugador").unwrap();
        assert_eq!(scene.get::<MeshRenderer>(player).unwrap().material.shading, ShadingModel::Toon);
        let sphere = scene.find_by_name("Esfera").unwrap();
        assert_eq!(scene.get::<MeshRenderer>(sphere).unwrap().material.shading, ShadingModel::Hatching);
        assert_eq!(p2.scenes(), vec!["scenes/biblioteca.mscene".to_string(), "scenes/main.mscene".to_string()]);
        // El escaparate usa la biblioteca integrada y un personaje animado.
        let lib = Scene::load(&p2.root.join("scenes/biblioteca.mscene")).unwrap();
        let hero = lib.find_by_name("Personaje").unwrap();
        assert_eq!(lib.get::<Animator>(hero).unwrap().clip, "Walk_Loop");
        let refs = crate::biblioteca::refs_in_scene(&lib.to_file());
        assert!(refs.len() > 15 && refs.iter().all(|r| r.starts_with("biblioteca://")), "{refs:?}");

        // HUD de la demo con el tema propio `neon` y la fuente del proyecto.
        assert_eq!(p2.file.tema_ui, "neon");
        let mut ui = crate::ui::SistemaUi::del_proyecto(&p2.root, &p2.file.tema_ui);
        let boton = scene.find_by_name("Continuar").unwrap();
        let w = scene.get::<crate::ui::WidgetUi>(boton).unwrap();
        let st = ui.recursos.resolver(&w, "", None, crate::ui::estilo::Estado::Encima);
        assert_eq!(st.fuente.as_deref(), Some("ui/fuentes/Hack-Regular.ttf"));
        assert_eq!(st.escala, Some(1.06));
        // Un título hereda de neon_texto (fuente) y redefine el tamaño.
        let titulo = scene.get::<crate::ui::WidgetUi>(scene.find_by_name("Título").unwrap()).unwrap();
        let st = ui.recursos.resolver(&titulo, "", None, crate::ui::estilo::Estado::Normal);
        assert_eq!((st.tam_texto, st.fuente.is_some()), (Some(54.0), true));
        ui.actualizar(&scene, [1920.0, 1080.0], 0.016);
        let (a, b) = ui.rect(scene.find_by_name("Radar").unwrap()).unwrap();
        assert_eq!((a, b), ([1650.0, 30.0], [1890.0, 270.0]));
        // La pausa empieza oculta: no hay nada suyo en la lista de dibujo bajo el centro.
        assert_ne!(ui.elemento_en([960.0, 540.0]), Some(boton));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
