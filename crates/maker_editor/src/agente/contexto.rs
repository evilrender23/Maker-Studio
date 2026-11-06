//! Contexto que se envía al agente: la guía fija de Maker Studio y los «chips» del editor.

use crate::editor::Editor;
use maker_engine::components::*;
use maker_engine::log;

/// Guía del proyecto (una vez por conversación nativa).
pub fn guia(ed: &Editor) -> String {
    let p = &ed.project;
    format!(
        r#"Trabajas dentro de un proyecto de Maker Studio, un motor y editor 3D en Rust (estilo Unity/Godot). El usuario tiene el editor abierto; responde en español y de forma concisa.

Proyecto: «{nombre}» en {raiz} (directorio de trabajo actual).
Estructura:
- project.maker: JSON {{name, main_scene, engine_version, escenas:[{{ruta, activa}}]}}. `escenas` es el orden de escenas: la primera activa es la de arranque (main_scene la copia). Escena de arranque: {principal}.
- scenes/*.mscene: escenas en JSON legible: {{"format":"maker-scene","version":1,"environment":{{sky_top, sky_horizon, ground, ambient_intensity, fog_color, fog_density, exposure, gravity}},"entities":[{{"id","name","parent"?,"sibling","active","transform":{{translation,rotation(cuaternión xyzw),scale}},"mesh"?,"light"?,"camera"?,"rigid_body"?,"collider"?,"scripts"?:[{{"name","props":{{..}},"enabled"}}]}}]}}. Mantén ids únicos y el JSON válido.
- assets/: modelos glTF (assets/models) y materiales.
- scripts/: crate Rust `cdylib` que depende de `maker_api` (ruta en scripts/Cargo.toml). Código en scripts/src/lib.rs. NO edites scripts/target.

Scripts (maker_api):
```rust
use maker_api::prelude::*;
#[derive(Default)]
pub struct Rotator {{ speed: f32 }}
impl Script for Rotator {{
    fn props(&mut self, p: &mut Props) {{ p.float("speed", &mut self.speed); }} // editables en el Inspector
    fn start(&mut self, ctx: &mut Ctx) {{}}
    fn fixed_update(&mut self, ctx: &mut Ctx) {{}} // 60 Hz, antes de la física
    fn update(&mut self, ctx: &mut Ctx) {{ ctx.rotate_y(self.speed.to_radians() * ctx.dt); }}
}}
export_scripts!(Rotator, /* todos los tipos */);
```
Ctx: dt, time, frame, entity; log/warn/error; position/set_position/translate/rotation/set_rotation/rotate_y/rotate_local/scale/set_scale; find(nombre), exists, position_of, set_position_of, rotation_of, set_rotation_of, set_color, spawn(Primitive, pos, spawn_flags::DYNAMIC_BODY|STATIC_BODY), destroy; key_down/key_pressed/axis(Key, Key)/mouse_down/mouse_delta; velocity/set_velocity/apply_impulse/apply_impulse_to/raycast; load_scene(nombre o ruta)/load_scene_index(i)/next_scene/previous_scene/reload_scene (se aplican al acabar el fotograma), scene_index/scene_count/scene_name. Props: float, int, bool, vec3, color.
Compilar: `cargo build --release --manifest-path scripts/Cargo.toml` (el editor lo hace solo tras tus cambios en scripts/src y recarga la biblioteca en caliente). Un tipo nuevo debe añadirse a export_scripts! para aparecer en Inspector → Añadir componente → Script.
Si cambias una escena, el editor la recarga si no tiene cambios sin guardar.

Texturizado (pintar texturas PBR de un modelo con el motor de TextureCraft): `"{exe}" textura <orden>` en esta carpeta. Órdenes: `info <asset>` (solape de UV, piel), `comandos [filtro]` (los ~135 comandos: capas, máscaras, materiales, horneado, exportación), `abrir <asset> [--resolucion N] [--uv-unicos] [--sin-original]` (crea texturizados/<nombre>/<nombre>.tcraft), `ejecutar <x.tcraft> <id> [json]`, `lote <x.tcraft> <pasos.json>` con `[{{"id":…,"params":{{…}}}}]`, `aplicar <x.tcraft>` (texturas glTF + texturizados/<nombre>/<nombre>.glb; ponlo en el `mesh` de la entidad como {{"Model":"texturizados/<nombre>/<nombre>.glb"}}) y `render <x.tcraft> <png>` para ver el resultado. Flujo típico: abrir → `bake.run {{"ts":"all"}}` → `material.apply {{"material":"rust"}}` (ids en `material.list`) → máscaras/efectos → aplicar → render."#,
        exe = std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_else(|_| "maker-studio".into()),
        nombre = p.name(),
        raiz = p.root.display(),
        principal = p.escena_inicial(),
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Chip {
    Escena,
    Seleccion,
    Script,
    Errores,
}

impl Chip {
    pub const TODOS: [Chip; 4] = [Chip::Escena, Chip::Seleccion, Chip::Script, Chip::Errores];

    pub fn nombre(self) -> &'static str {
        match self {
            Chip::Escena => "Escena",
            Chip::Seleccion => "Selección",
            Chip::Script => "Script",
            Chip::Errores => "Errores",
        }
    }

    pub fn desde_clave(s: &str) -> Option<Chip> {
        match s {
            "escena" => Some(Chip::Escena),
            "seleccion" | "selección" => Some(Chip::Seleccion),
            "script" => Some(Chip::Script),
            "errores" => Some(Chip::Errores),
            _ => None,
        }
    }

    /// Si hay algo que enviar (y si no, por qué).
    pub fn disponible(self, ed: &Editor) -> Result<(), &'static str> {
        match self {
            Chip::Escena => Ok(()),
            Chip::Seleccion if ed.selection.is_none() => Err("No hay nada seleccionado"),
            Chip::Script if ed.script.is_none() => Err("No hay ningún script abierto"),
            Chip::Errores if errores_recientes().is_empty() => Err("La consola no tiene errores"),
            _ => Ok(()),
        }
    }

    pub fn ayuda(self, ed: &Editor) -> String {
        match self {
            Chip::Escena => format!("{} y su lista de entidades", ed.project.relative(&ed.scene_path)),
            Chip::Seleccion => ed.selection.map(|s| format!("{} con sus componentes", ed.scene.name(s))).unwrap_or_default(),
            Chip::Script => ed.script.as_ref().map(|s| ed.project.relative(&s.path)).unwrap_or_default(),
            Chip::Errores => format!("{} errores y avisos recientes de la consola", errores_recientes().len()),
        }
    }

    pub fn texto(self, ed: &Editor) -> String {
        match self {
            Chip::Escena => escena(ed),
            Chip::Seleccion => seleccion(ed),
            Chip::Script => script(ed),
            Chip::Errores => errores_recientes().join("\n"),
        }
    }
}

fn componentes(ed: &Editor, id: EntityId) -> Vec<String> {
    let s = &ed.scene;
    let mut v = Vec::new();
    if let Some(m) = s.get::<MeshRenderer>(id) {
        v.push(match m.mesh {
            MeshSource::Primitive(p) => format!("Malla {}", p.label()),
            MeshSource::Model(r) => format!("Modelo {r}"),
        });
    }
    if s.has::<Light>(id) {
        v.push("Luz".into());
    }
    if s.has::<Camera>(id) {
        v.push("Cámara".into());
    }
    if s.has::<RigidBody>(id) {
        v.push("Cuerpo rígido".into());
    }
    if s.has::<Collider>(id) {
        v.push("Colisionador".into());
    }
    if let Some(sc) = s.get::<Scripts>(id) {
        v.extend(sc.0.iter().map(|x| format!("Script {}", x.name)));
    }
    v
}

fn escena(ed: &Editor) -> String {
    let mut t = format!("Escena abierta: {}{}\nEntidades:\n", ed.project.relative(&ed.scene_path), if ed.dirty { " (con cambios sin guardar en el editor)" } else { "" });
    for (id, nivel) in ed.scene.hierarchy().depth_first() {
        let c = componentes(ed, id);
        t.push_str(&format!("{}- {} [id {}]{}\n", "  ".repeat(nivel), ed.scene.name(id), id.0, if c.is_empty() { String::new() } else { format!(": {}", c.join(", ")) }));
    }
    t
}

fn seleccion(ed: &Editor) -> String {
    let Some(id) = ed.selection else { return String::new() };
    match ed.scene.record(id) {
        Some(r) => format!("Entidad seleccionada:\n```json\n{}\n```", serde_json::to_string_pretty(&r).unwrap_or_default()),
        None => String::new(),
    }
}

fn script(ed: &Editor) -> String {
    let Some(s) = &ed.script else { return String::new() };
    let rel = ed.project.relative(&s.path);
    let sucio = s.text != s.saved;
    const MAX: usize = 24_000;
    if s.text.len() > MAX {
        return format!("Script abierto en el editor: {rel}{}", if sucio { " (con cambios sin guardar)" } else { "" });
    }
    format!("Script abierto en el editor: {rel}{}\n```rust\n{}\n```", if sucio { " (contenido sin guardar, distinto del archivo)" } else { "" }, s.text)
}

/// Últimos errores y avisos de la consola.
pub fn errores_recientes() -> Vec<String> {
    let mut v: Vec<String> = log::since(0).into_iter().filter(|e| e.level != log::Level::Info && e.source != "agente").map(|e| format!("[{:?}] [{}] {}", e.level, e.source, e.text)).collect();
    let n = v.len();
    v.drain(..n.saturating_sub(20));
    v
}

/// Mensaje final: contexto pedido + texto del usuario.
pub fn componer(ed: &Editor, chips: &[Chip], texto: &str, previo: Option<&str>) -> String {
    let mut partes: Vec<String> = Vec::new();
    if let Some(p) = previo {
        partes.push(p.to_string());
    }
    let ctx: Vec<String> = chips.iter().filter(|c| c.disponible(ed).is_ok()).map(|c| c.texto(ed)).filter(|t| !t.trim().is_empty()).collect();
    if !ctx.is_empty() {
        partes.push(format!("<contexto_del_editor>\n{}\n</contexto_del_editor>", ctx.join("\n\n")));
    }
    partes.push(texto.to_string());
    partes.join("\n\n")
}
