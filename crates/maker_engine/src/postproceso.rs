//! Sombreado global y pila de posprocesado de la escena. Solo datos y
//! utilidades sin GPU (el renderizado vive en `maker_render::postproceso`).
//!
//! - [`GlobalShading`]: fuerza un modelo de sombreado y sus parámetros en todos
//!   los materiales (salvo los que lo ignoran); cada material conserva su
//!   color y textura.
//! - [`PostEffect`]: un paso de la pila ordenada de `Environment::post`.
//!   Efectos integrados y efectos WGSL del proyecto (`shaders/post/*.wgsl`)
//!   con parámetros declarados en la cabecera del archivo.

use crate::components::{Material, ShadingModel, StyleParams};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Sombreado global de la escena (`Environment::global_shading`).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GlobalShading {
    /// `None` = cada material usa su propio modelo.
    pub model: Option<ShadingModel>,
    pub style: StyleParams,
    pub metallic: f32,
    pub roughness: f32,
}

impl Default for GlobalShading {
    fn default() -> Self {
        let m = Material::default();
        Self { model: None, style: StyleParams::default(), metallic: m.metallic, roughness: m.roughness }
    }
}

impl GlobalShading {
    /// Material con el que se dibuja `m`: el modelo y los parámetros globales
    /// con el color, la textura y la emisión del propio material.
    pub fn apply(&self, m: &Material) -> Material {
        match self.model {
            Some(model) if !m.ignore_global => Material { shading: model, style: self.style, metallic: self.metallic, roughness: self.roughness, ..m.clone() },
            _ => m.clone(),
        }
    }

    /// Lo mismo que `apply` sin copiar el material (para el renderizador, en
    /// cada fotograma): modelo, estilo, metálico y rugosidad efectivos.
    pub fn resolve<'a>(&'a self, m: &'a Material) -> Shade<'a> {
        match self.model {
            Some(model) if !m.ignore_global => Shade { shading: model, style: &self.style, metallic: self.metallic, roughness: self.roughness },
            _ => Shade { shading: m.shading, style: &m.style, metallic: m.metallic, roughness: m.roughness },
        }
    }
}

/// Parámetros de sombreado efectivos de un material (ver `GlobalShading::resolve`).
#[derive(Clone, Copy, Debug)]
pub struct Shade<'a> {
    pub shading: ShadingModel,
    pub style: &'a StyleParams,
    pub metallic: f32,
    pub roughness: f32,
}

fn yes() -> bool {
    true
}
fn one() -> f32 {
    1.0
}

/// Un efecto de la pila de posprocesado.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PostEffect {
    #[serde(default = "yes")]
    pub enabled: bool,
    /// Fuerza del efecto (0 = sin efecto; se omite como si estuviera apagado).
    #[serde(default = "one")]
    pub intensity: f32,
    pub kind: PostKind,
}

impl PostEffect {
    pub fn new(kind: PostKind) -> Self {
        let intensity = match kind {
            PostKind::Bloom(_) => 0.6,
            PostKind::FilmGrain(_) => 0.35,
            _ => 1.0,
        };
        Self { enabled: true, intensity, kind }
    }
    /// ¿Se ejecuta? (encendido y con fuerza; el tonemap no usa la fuerza).
    pub fn active(&self) -> bool {
        self.enabled && (matches!(self.kind, PostKind::Tonemap(_)) || self.intensity > 0.0)
    }
}

/// Operador de tonemapping.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Tonemapper {
    #[default]
    Aces,
    Agx,
    /// Sin curva: recorta a 0..1.
    None,
}

impl Tonemapper {
    pub const ALL: [Tonemapper; 3] = [Tonemapper::Aces, Tonemapper::Agx, Tonemapper::None];
    pub fn label(self) -> &'static str {
        match self {
            Tonemapper::Aces => "ACES",
            Tonemapper::Agx => "AgX",
            Tonemapper::None => "Ninguno",
        }
    }
    /// Código del shader (`tonemap()` en WGSL). 3 = pasar HDR sin curva.
    pub fn code(self) -> u32 {
        match self {
            Tonemapper::Aces => 0,
            Tonemapper::Agx => 1,
            Tonemapper::None => 2,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Tonemap {
    pub op: Tonemapper,
    /// Corrección de exposición en pasos (EV) sobre la de la escena.
    pub exposure_ev: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ColorGrading {
    pub contrast: f32,
    pub saturation: f32,
    /// -1 (frío) .. 1 (cálido).
    pub temperature: f32,
    /// -1 (verde) .. 1 (magenta).
    pub tint: f32,
    pub lift: [f32; 3],
    pub gamma: [f32; 3],
    pub gain: [f32; 3],
    /// Imagen LUT en tira (N²×N, p. ej. 256×16) relativa al proyecto; vacía = sin LUT.
    pub lut: String,
}

impl Default for ColorGrading {
    fn default() -> Self {
        Self { contrast: 1.0, saturation: 1.0, temperature: 0.0, tint: 0.0, lift: [0.0; 3], gamma: [1.0; 3], gain: [1.0; 3], lut: String::new() }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Bloom {
    /// Luminancia (HDR) a partir de la que brilla.
    pub threshold: f32,
    /// 0..1: niveles de la cadena de reducción que se usan.
    pub radius: f32,
}

impl Default for Bloom {
    fn default() -> Self {
        Self { threshold: 1.0, radius: 0.7 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Vignette {
    pub radius: f32,
    pub softness: f32,
    pub color: [f32; 3],
}

impl Default for Vignette {
    fn default() -> Self {
        Self { radius: 0.75, softness: 0.45, color: [0.0; 3] }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ChromaticAberration {
    /// Desplazamiento en píxeles (a 1080 p) en el borde de la imagen.
    pub amount: f32,
}

impl Default for ChromaticAberration {
    fn default() -> Self {
        Self { amount: 4.0 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FilmGrain {
    /// Tamaño del grano en píxeles.
    pub size: f32,
}

impl Default for FilmGrain {
    fn default() -> Self {
        Self { size: 1.5 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Retro {
    /// Tamaño del píxel (1 = sin pixelado).
    pub pixel_size: f32,
    /// Niveles por canal (0 = sin posterizar).
    pub levels: u32,
    /// Tramado Bayer 4×4 antes de posterizar (0 = sin tramado, 1 = un escalón).
    pub dither: f32,
}

impl Default for Retro {
    fn default() -> Self {
        Self { pixel_size: 4.0, levels: 8, dither: 0.0 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Sharpen {
    /// Distancia de las muestras en píxeles.
    pub radius: f32,
}

impl Default for Sharpen {
    fn default() -> Self {
        Self { radius: 1.0 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EdgeOutline {
    pub color: [f32; 3],
    /// Grosor en píxeles.
    pub thickness: f32,
    /// Salto de distancia relativo (0..1) que cuenta como borde.
    pub depth_threshold: f32,
    /// Diferencia de normales (0..2) que cuenta como borde (pliegues).
    pub normal_threshold: f32,
}

impl Default for EdgeOutline {
    fn default() -> Self {
        Self { color: [0.05, 0.05, 0.07], thickness: 1.0, depth_threshold: 0.08, normal_threshold: 0.5 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HeightFog {
    pub color: [f32; 3],
    pub density: f32,
    /// Altura (m) a la que la niebla tiene la densidad indicada.
    pub height: f32,
    /// Cuánto se aclara por metro de altura.
    pub falloff: f32,
}

impl Default for HeightFog {
    fn default() -> Self {
        Self { color: [0.8, 0.85, 0.92], density: 0.06, height: 0.0, falloff: 0.4 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DepthOfField {
    /// Distancia enfocada (m).
    pub focus: f32,
    /// Profundidad nítida alrededor del foco (m).
    pub range: f32,
    /// Desenfoque máximo en píxeles (a 1080 p).
    pub max_blur: f32,
}

impl Default for DepthOfField {
    fn default() -> Self {
        Self { focus: 8.0, range: 4.0, max_blur: 8.0 }
    }
}

/// Efecto WGSL del proyecto.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CustomEffect {
    /// Ruta relativa al proyecto (`shaders/post/crt.wgsl`).
    pub path: String,
    /// Valores por nombre de parámetro (los que faltan toman el de la cabecera).
    pub values: BTreeMap<String, [f32; 4]>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum PostKind {
    Tonemap(Tonemap),
    ColorGrading(ColorGrading),
    Bloom(Bloom),
    Vignette(Vignette),
    ChromaticAberration(ChromaticAberration),
    FilmGrain(FilmGrain),
    Retro(Retro),
    Sharpen(Sharpen),
    Fxaa,
    EdgeOutline(EdgeOutline),
    HeightFog(HeightFog),
    DepthOfField(DepthOfField),
    Custom(CustomEffect),
}

impl PostKind {
    /// Los efectos integrados con sus valores por defecto (menú «Añadir»).
    pub fn builtins() -> Vec<PostKind> {
        vec![
            PostKind::Tonemap(Tonemap::default()),
            PostKind::ColorGrading(ColorGrading::default()),
            PostKind::Bloom(Bloom::default()),
            PostKind::Vignette(Vignette::default()),
            PostKind::ChromaticAberration(ChromaticAberration::default()),
            PostKind::FilmGrain(FilmGrain::default()),
            PostKind::Retro(Retro::default()),
            PostKind::Sharpen(Sharpen::default()),
            PostKind::Fxaa,
            PostKind::EdgeOutline(EdgeOutline::default()),
            PostKind::HeightFog(HeightFog::default()),
            PostKind::DepthOfField(DepthOfField::default()),
        ]
    }
    pub fn label(&self) -> String {
        match self {
            PostKind::Tonemap(_) => "Tonemapping".into(),
            PostKind::ColorGrading(_) => "Corrección de color".into(),
            PostKind::Bloom(_) => "Bloom".into(),
            PostKind::Vignette(_) => "Viñeta".into(),
            PostKind::ChromaticAberration(_) => "Aberración cromática".into(),
            PostKind::FilmGrain(_) => "Grano".into(),
            PostKind::Retro(_) => "Retro (píxel y posterizado)".into(),
            PostKind::Sharpen(_) => "Enfoque".into(),
            PostKind::Fxaa => "FXAA".into(),
            PostKind::EdgeOutline(_) => "Contorno en pantalla".into(),
            PostKind::HeightFog(_) => "Niebla de altura".into(),
            PostKind::DepthOfField(_) => "Profundidad de campo".into(),
            PostKind::Custom(c) => std::path::Path::new(&c.path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Efecto".into()),
        }
    }
    /// ¿Trabaja sobre la luz HDR, antes del tonemap implícito? (El resto
    /// recibe la imagen ya en 0..1.)
    pub fn wants_hdr(&self) -> bool {
        matches!(self, PostKind::Bloom(_) | PostKind::HeightFog(_) | PostKind::DepthOfField(_))
    }
}

/// Paso del plan de posprocesado de un fotograma.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Step {
    Tonemap { op: Tonemapper, exposure_ev: f32 },
    /// Índice en la pila.
    Effect(usize),
}

/// Plan de un fotograma: los pasos a ejecutar tras la escena y el tonemap que
/// hace el propio shader de la escena (si va primero se fusiona y no cuesta
/// un pase).
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    /// `Some`: la escena sale ya con esta curva. `None`: sale en HDR lineal.
    pub scene_tonemap: Option<(Tonemapper, f32)>,
    pub steps: Vec<Step>,
}

/// Ordena la pila: omite lo apagado (y los efectos propios que no compilan,
/// según `usable`) y coloca el tonemap. Sin tonemap explícito se añade un ACES
/// antes del primer efecto que trabaja en 0..1. Si el tonemap queda el primero
/// se fusiona con la escena. Una pila vacía (o solo con tonemap) no añade
/// ningún pase.
pub fn plan(effects: &[PostEffect], usable: impl Fn(usize, &PostEffect) -> bool) -> Plan {
    let mut steps = Vec::new();
    let mut tonemapped = false;
    for (i, e) in effects.iter().enumerate() {
        if !e.active() || !usable(i, e) {
            continue;
        }
        match &e.kind {
            PostKind::Tonemap(t) => {
                if !tonemapped {
                    steps.push(Step::Tonemap { op: t.op, exposure_ev: t.exposure_ev });
                    tonemapped = true;
                }
            }
            k => {
                if !tonemapped && !k.wants_hdr() {
                    steps.push(Step::Tonemap { op: Tonemapper::Aces, exposure_ev: 0.0 });
                    tonemapped = true;
                }
                steps.push(Step::Effect(i));
            }
        }
    }
    if !tonemapped {
        steps.push(Step::Tonemap { op: Tonemapper::Aces, exposure_ev: 0.0 });
    }
    let scene_tonemap = match steps.first() {
        Some(Step::Tonemap { op, exposure_ev }) => Some((*op, *exposure_ev)),
        _ => None,
    };
    if scene_tonemap.is_some() {
        steps.remove(0);
    }
    Plan { scene_tonemap, steps }
}

// ───────────────────────── Efectos WGSL del proyecto ─────────────────────────

/// Carpeta de los efectos del proyecto.
pub const CUSTOM_DIR: &str = "shaders/post";
/// Parámetros declarables por efecto (cada uno ocupa un `vec4`).
pub const MAX_CUSTOM_PARAMS: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ParamKind {
    Float { min: f32, max: f32 },
    /// Color sRGB en el Inspector; llega al shader en lineal.
    Color,
}

/// Parámetro declarado en la cabecera: `// @param nombre float 0.5 0 1` o
/// `// @param tinte color 1 0.5 0`.
#[derive(Clone, Debug, PartialEq)]
pub struct CustomParam {
    pub name: String,
    pub kind: ParamKind,
    pub default: [f32; 4],
}

fn ident_ok(s: &str) -> bool {
    let mut c = s.chars();
    c.next().is_some_and(|f| f.is_ascii_alphabetic() || f == '_') && c.all(|ch| ch.is_ascii_alphanumeric() || ch == '_') && s != "_"
}

/// Lee los `// @param` del archivo. Error con el número de línea si alguno
/// está mal escrito.
pub fn parse_params(src: &str) -> Result<Vec<CustomParam>, String> {
    let mut out: Vec<CustomParam> = Vec::new();
    for (ln, line) in src.lines().enumerate() {
        let Some(rest) = line.trim_start().strip_prefix("//").map(str::trim_start).and_then(|r| r.strip_prefix("@param")) else { continue };
        let at = |m: String| format!("línea {}: {m}", ln + 1);
        let words: Vec<&str> = rest.split_whitespace().collect();
        let (Some(name), Some(kind)) = (words.first(), words.get(1)) else {
            return Err(at("se esperaba `@param nombre float|color valores…`".into()));
        };
        if !ident_ok(name) {
            return Err(at(format!("«{name}» no es un identificador WGSL válido")));
        }
        if out.iter().any(|p| p.name == *name) {
            return Err(at(format!("parámetro «{name}» repetido")));
        }
        let nums: Result<Vec<f32>, _> = words[2..].iter().map(|w| w.parse::<f32>()).collect();
        let nums = nums.map_err(|_| at("los valores deben ser números".into()))?;
        let p = match *kind {
            "float" => {
                let d = nums.first().copied().unwrap_or(0.0);
                let (min, max) = match (nums.get(1), nums.get(2)) {
                    (Some(a), Some(b)) if a < b => (*a, *b),
                    (Some(_), Some(_)) => return Err(at("el mínimo debe ser menor que el máximo".into())),
                    _ => (d.min(0.0), d.max(1.0)),
                };
                CustomParam { name: name.to_string(), kind: ParamKind::Float { min, max }, default: [d, 0.0, 0.0, 0.0] }
            }
            "color" => {
                let c = |i: usize| nums.get(i).copied().unwrap_or(1.0);
                CustomParam { name: name.to_string(), kind: ParamKind::Color, default: [c(0), c(1), c(2), 1.0] }
            }
            k => return Err(at(format!("tipo «{k}» desconocido (float o color)"))),
        };
        out.push(p);
    }
    if out.len() > MAX_CUSTOM_PARAMS {
        return Err(format!("{} parámetros; el máximo es {MAX_CUSTOM_PARAMS}", out.len()));
    }
    Ok(out)
}

/// Plantilla de «Nuevo efecto».
pub const CUSTOM_TEMPLATE: &str = include_str!("post_ejemplos/plantilla.wgsl");
/// Efectos de ejemplo del proyecto de demostración: (archivo, código).
pub const EXAMPLES: [(&str, &str); 2] = [("crt.wgsl", include_str!("post_ejemplos/crt.wgsl")), ("tramado.wgsl", include_str!("post_ejemplos/tramado.wgsl"))];

/// Nombre de archivo libre en `dir` a partir de `base` (`efecto.wgsl`,
/// `efecto_2.wgsl`…).
pub fn free_name(dir: &std::path::Path, base: &str) -> String {
    let stem: String = base.trim().chars().map(|c| if c.is_alphanumeric() || c == '_' || c == '-' { c } else { '_' }).collect();
    let stem = if stem.is_empty() { "efecto".to_string() } else { stem };
    let mut name = format!("{stem}.wgsl");
    let mut n = 2;
    while dir.join(&name).exists() {
        name = format!("{stem}_{n}.wgsl");
        n += 1;
    }
    name
}

/// Archivos del proyecto que usa el posprocesado de las escenas (efectos
/// propios y LUT), para exportarlos.
pub fn post_files_used<'a>(envs: impl IntoIterator<Item = &'a crate::scene::Environment>) -> Vec<String> {
    let mut v: Vec<String> = envs
        .into_iter()
        .flat_map(|e| e.post.iter())
        .filter_map(|p| match &p.kind {
            PostKind::Custom(c) => Some(c.path.clone()),
            PostKind::ColorGrading(g) => Some(g.lut.clone()),
            _ => None,
        })
        .filter(|p| !p.is_empty())
        .collect();
    v.sort();
    v.dedup();
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fx(kind: PostKind) -> PostEffect {
        PostEffect::new(kind)
    }

    #[test]
    fn global_shading_keeps_color_and_respects_exception() {
        let mut g = GlobalShading { model: Some(ShadingModel::Toon), ..Default::default() };
        g.style.bands = 3;
        let mut m = Material::color([0.9, 0.1, 0.2]);
        m.emissive_strength = 2.0;
        let r = g.apply(&m);
        assert_eq!((r.shading, r.style.bands, r.base_color, r.emissive_strength), (ShadingModel::Toon, 3, m.base_color, 2.0));
        m.ignore_global = true;
        assert_eq!(g.apply(&m), m);
        g.model = None;
        m.ignore_global = false;
        assert_eq!(g.apply(&m), m);
    }

    #[test]
    fn empty_stack_is_free() {
        let p = plan(&[], |_, _| true);
        assert_eq!(p, Plan { scene_tonemap: Some((Tonemapper::Aces, 0.0)), steps: vec![] });
        // Solo un tonemap: se fusiona con la escena.
        let t = fx(PostKind::Tonemap(Tonemap { op: Tonemapper::Agx, exposure_ev: 1.0 }));
        assert_eq!(plan(&[t], |_, _| true), Plan { scene_tonemap: Some((Tonemapper::Agx, 1.0)), steps: vec![] });
    }

    #[test]
    fn implicit_tonemap_goes_after_hdr_effects() {
        let stack = [fx(PostKind::Bloom(Bloom::default())), fx(PostKind::Vignette(Vignette::default())), fx(PostKind::Fxaa)];
        let p = plan(&stack, |_, _| true);
        assert_eq!(p.scene_tonemap, None);
        assert_eq!(p.steps, vec![Step::Effect(0), Step::Tonemap { op: Tonemapper::Aces, exposure_ev: 0.0 }, Step::Effect(1), Step::Effect(2)]);
        // Solo efectos en 0..1: el tonemap se fusiona con la escena.
        let p = plan(&stack[1..], |_, _| true);
        assert_eq!(p.scene_tonemap, Some((Tonemapper::Aces, 0.0)));
        assert_eq!(p.steps, vec![Step::Effect(0), Step::Effect(1)]);
    }

    #[test]
    fn disabled_zero_and_broken_effects_are_skipped() {
        let mut off = fx(PostKind::Vignette(Vignette::default()));
        off.enabled = false;
        let mut zero = fx(PostKind::Sharpen(Sharpen::default()));
        zero.intensity = 0.0;
        let broken = fx(PostKind::Custom(CustomEffect { path: "shaders/post/x.wgsl".into(), ..Default::default() }));
        let p = plan(&[off, zero, broken], |_, e| !matches!(e.kind, PostKind::Custom(_)));
        assert_eq!(p, plan(&[], |_, _| true));
        // Un segundo tonemap no se aplica dos veces.
        let t = fx(PostKind::Tonemap(Tonemap::default()));
        let p = plan(&[fx(PostKind::Bloom(Bloom::default())), t.clone(), t], |_, _| true);
        assert_eq!(p.steps.len(), 2);
    }

    #[test]
    fn parses_header_params() {
        let src = "// Mi efecto\n// @param curvatura float 0.2 0 0.5\n//@param lineas color 0.1 0.2 0.3\nfn x() {}\n";
        let p = parse_params(src).unwrap();
        assert_eq!(p.len(), 2);
        assert_eq!(p[0], CustomParam { name: "curvatura".into(), kind: ParamKind::Float { min: 0.0, max: 0.5 }, default: [0.2, 0.0, 0.0, 0.0] });
        assert_eq!(p[1].kind, ParamKind::Color);
        assert_eq!(p[1].default, [0.1, 0.2, 0.3, 1.0]);
        assert!(parse_params("// @param 1x float 0").unwrap_err().contains("línea 1"));
        assert!(parse_params("// @param a float 0\n// @param a float 1").unwrap_err().contains("repetido"));
        assert!(parse_params("// @param a vec2 0").is_err());
        let many: String = (0..17).map(|i| format!("// @param p{i} float 0\n")).collect();
        assert!(parse_params(&many).is_err());
        for (_, src) in EXAMPLES {
            assert!(!parse_params(src).unwrap().is_empty());
        }
        assert!(parse_params(CUSTOM_TEMPLATE).is_ok());
    }

    #[test]
    fn stack_round_trips_and_old_scenes_load() {
        let mut env = crate::scene::Environment::default();
        env.post.push(fx(PostKind::Bloom(Bloom { threshold: 2.0, radius: 0.5 })));
        env.post.push(fx(PostKind::Fxaa));
        let mut values = BTreeMap::new();
        values.insert("curvatura".to_string(), [0.3, 0.0, 0.0, 0.0]);
        env.post.push(fx(PostKind::Custom(CustomEffect { path: "shaders/post/crt.wgsl".into(), values })));
        env.global_shading.model = Some(ShadingModel::Hatching);
        let json = serde_json::to_string(&env).unwrap();
        let back: crate::scene::Environment = serde_json::from_str(&json).unwrap();
        assert_eq!(back, env);
        assert_eq!(post_files_used([&env, &back]), vec!["shaders/post/crt.wgsl".to_string()]);
        env.post.push(fx(PostKind::ColorGrading(ColorGrading { lut: "assets/lut.png".into(), ..Default::default() })));
        assert_eq!(post_files_used([&env]), vec!["assets/lut.png".to_string(), "shaders/post/crt.wgsl".to_string()]);
        // Escena anterior: sin pila ni sombreado global.
        let old = r#"{"sky_top":[0.1,0.2,0.3],"sky_horizon":[1,1,1],"ground":[0,0,0],"ambient_intensity":0.5,"fog_color":[1,1,1],"fog_density":0.0,"exposure":1.2,"gravity":[0,-9.81,0]}"#;
        let e: crate::scene::Environment = serde_json::from_str(old).unwrap();
        assert!(e.post.is_empty() && e.global_shading.model.is_none());
        assert_eq!(e.exposure, 1.2);
        // Campos nuevos de un efecto toman su valor por defecto.
        let k: PostKind = serde_json::from_str(r#"{"Bloom":{"threshold":3.0}}"#).unwrap();
        assert_eq!(k, PostKind::Bloom(Bloom { threshold: 3.0, ..Default::default() }));
    }

    #[test]
    fn free_names_do_not_clash() {
        let d = std::env::temp_dir().join(format!("maker-post-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        assert_eq!(free_name(&d, "mi efecto"), "mi_efecto.wgsl");
        std::fs::write(d.join("mi_efecto.wgsl"), "").unwrap();
        assert_eq!(free_name(&d, "mi efecto"), "mi_efecto_2.wgsl");
        let _ = std::fs::remove_dir_all(&d);
    }
}
