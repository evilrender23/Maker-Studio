//! Biblioteca integrada: modelos CC0 que vienen con el motor (no se copian a
//! cada proyecto). Las escenas los referencian como `biblioteca://<pack>/<modelo>.glb`
//! y la ruta se resuelve contra la carpeta de la biblioteca del motor:
//!
//! 1. la variable de entorno `MAKER_BIBLIOTECA`;
//! 2. `biblioteca/` junto al ejecutable (juegos exportados);
//! 3. la instalación: `<destino>/app/maker-studio` → `<destino>/codigo/assets/biblioteca`;
//! 4. la del repositorio con el que se compiló (`CARGO_MANIFEST_DIR/../../assets/biblioteca`).
//!
//! Al exportar un juego solo se copian los archivos que usan sus escenas.

use crate::components::MeshSource;
use crate::scene::SceneFile;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

pub const SCHEME: &str = "biblioteca://";
pub const ENV_VAR: &str = "MAKER_BIBLIOTECA";
/// Carpeta de la biblioteca junto al ejecutable (juegos exportados).
pub const DIR_NAME: &str = "biblioteca";
pub const CATALOG_FILE: &str = "catalogo.json";

/// ¿Es una ruta de la biblioteca?
pub fn is_library_path(p: &str) -> bool {
    p.starts_with(SCHEME)
}

/// Parte relativa de una ruta `biblioteca://…`.
pub fn relative(p: &str) -> Option<&str> {
    p.strip_prefix(SCHEME)
}

pub fn uri(rel: &str) -> String {
    format!("{SCHEME}{}", rel.trim_start_matches('/'))
}

/// Ruta de la biblioteca en el repositorio con el que se compiló el motor.
pub fn compiled_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/biblioteca")
}

/// Busca la carpeta de la biblioteca en el orden documentado arriba.
pub fn find_library_dir(env: Option<PathBuf>, exe_dir: Option<&Path>, compiled: &Path) -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(e) = env.filter(|e| !e.as_os_str().is_empty()) {
        candidates.push(e);
    }
    if let Some(d) = exe_dir {
        candidates.push(d.join(DIR_NAME));
        if let Some(dest) = d.parent() {
            candidates.push(dest.join("codigo").join("assets").join("biblioteca"));
        }
    }
    candidates.push(compiled.to_path_buf());
    candidates.into_iter().find(|c| c.is_dir()).map(|c| c.canonicalize().unwrap_or(c))
}

/// Carpeta de la biblioteca de este proceso (se busca una vez).
pub fn library_dir() -> Option<PathBuf> {
    static DIR: OnceLock<Option<PathBuf>> = OnceLock::new();
    DIR.get_or_init(|| {
        let exe_dir = std::env::current_exe().ok().and_then(|e| e.canonicalize().ok()).and_then(|e| e.parent().map(Path::to_path_buf));
        find_library_dir(std::env::var_os(ENV_VAR).map(PathBuf::from), exe_dir.as_deref(), &compiled_dir())
    })
    .clone()
}

/// Ruta en disco de un recurso: `biblioteca://…` contra la biblioteca, el
/// resto relativo a la raíz del proyecto.
pub fn resolve(path: &str, project_root: Option<&Path>) -> PathBuf {
    match relative(path) {
        Some(rel) => library_dir().unwrap_or_else(compiled_dir).join(rel),
        None => match project_root {
            Some(r) => r.join(path),
            None => PathBuf::from(path),
        },
    }
}

// ───────────────────────────── catálogo ─────────────────────────────

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PackInfo {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub license: String,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub models: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CatalogEntry {
    pub pack: String,
    pub category: String,
    pub name: String,
    /// Relativa a la biblioteca.
    pub path: String,
    #[serde(default)]
    pub thumbnail: String,
    #[serde(default)]
    pub triangles: u32,
    #[serde(default)]
    pub has_skin: bool,
    #[serde(default)]
    pub has_animations: bool,
    #[serde(default)]
    pub clips: Vec<String>,
    #[serde(default)]
    pub source: String,
}

impl CatalogEntry {
    pub fn uri(&self) -> String {
        uri(&self.path)
    }
    /// ¿Contiene todas las palabras de la búsqueda (sin mayúsculas ni tildes)?
    pub fn matches(&self, query: &str) -> bool {
        let hay = fold(&format!("{} {} {} {}", self.name, self.category, self.path, self.pack));
        fold(query).split_whitespace().all(|w| hay.contains(w))
    }
}

/// Minúsculas sin tildes, para buscar.
pub fn fold(s: &str) -> String {
    s.chars()
        .flat_map(|c| c.to_lowercase())
        .map(|c| match c {
            'á' | 'à' | 'ä' | 'â' => 'a',
            'é' | 'è' | 'ë' | 'ê' => 'e',
            'í' | 'ì' | 'ï' | 'î' => 'i',
            'ó' | 'ò' | 'ö' | 'ô' => 'o',
            'ú' | 'ù' | 'ü' | 'û' => 'u',
            '_' | '-' => ' ',
            c => c,
        })
        .collect()
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Catalog {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub packs: Vec<PackInfo>,
    #[serde(default)]
    pub models: Vec<CatalogEntry>,
}

impl Catalog {
    pub fn parse(text: &str) -> Result<Catalog, String> {
        serde_json::from_str(text).map_err(|e| format!("catálogo de la biblioteca inválido: {e}"))
    }
    pub fn load(dir: &Path) -> Result<Catalog, String> {
        let p = dir.join(CATALOG_FILE);
        let text = std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?;
        Catalog::parse(&text)
    }
    /// El de la biblioteca del motor.
    pub fn load_default() -> Result<Catalog, String> {
        Catalog::load(&library_dir().ok_or("no se encuentra la biblioteca (assets/biblioteca)")?)
    }
    pub fn entry(&self, path_or_uri: &str) -> Option<&CatalogEntry> {
        let rel = relative(path_or_uri).unwrap_or(path_or_uri);
        self.models.iter().find(|m| m.path == rel)
    }
    /// Categorías de un pack (o de todos) en orden de aparición.
    pub fn categories(&self, pack: Option<&str>) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for m in self.models.iter().filter(|m| pack.is_none_or(|p| m.pack == p)) {
            if !out.contains(&m.category) {
                out.push(m.category.clone());
            }
        }
        out
    }
    pub fn filter<'a>(&'a self, pack: Option<&'a str>, category: Option<&'a str>, query: &'a str) -> impl Iterator<Item = &'a CatalogEntry> + 'a {
        self.models
            .iter()
            .filter(move |m| pack.is_none_or(|p| m.pack == p) && category.is_none_or(|c| m.category == c) && (query.trim().is_empty() || m.matches(query)))
    }
    /// Archivos con clips de animación (para elegir la fuente de un Animator).
    pub fn animation_sources(&self) -> impl Iterator<Item = &CatalogEntry> {
        self.models.iter().filter(|m| m.has_animations && !m.clips.is_empty())
    }
}

/// Fuente de clips por defecto para personajes de la biblioteca.
pub const DEFAULT_CLIPS: &str = "biblioteca://animaciones/UAL1_Standard.glb";
pub const DEFAULT_IDLE: &str = "Idle_Loop";

// ───────────────────────────── exportación ─────────────────────────────

/// Rutas `biblioteca://` que usa una escena (mallas y fuentes de animación).
pub fn refs_in_scene(f: &SceneFile) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for r in &f.entities {
        if let Some(m) = &r.mesh
            && let MeshSource::Model(p) = &m.mesh
            && is_library_path(p)
        {
            out.insert(p.clone());
        }
        if let Some(a) = &r.animator
            && is_library_path(&a.source)
        {
            out.insert(a.source.clone());
        }
    }
    out
}

/// Archivos externos (texturas, .bin) que referencia un glTF/GLB.
pub fn dependencies(model: &Path) -> Vec<PathBuf> {
    let Ok(g) = gltf::Gltf::open(model) else { return Vec::new() };
    let dir = model.parent().unwrap_or(Path::new("."));
    let mut out = Vec::new();
    for b in g.document.buffers() {
        if let gltf::buffer::Source::Uri(u) = b.source()
            && !u.starts_with("data:")
        {
            out.push(dir.join(u));
        }
    }
    for i in g.document.images() {
        if let gltf::image::Source::Uri { uri, .. } = i.source()
            && !uri.starts_with("data:")
        {
            out.push(dir.join(uri));
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Copia a `out/biblioteca/` los recursos de la biblioteca que usan las
/// escenas (con sus texturas y la licencia de cada pack). Devuelve las rutas
/// relativas copiadas.
///
/// `extra`: otras rutas `biblioteca://` usadas (imágenes y fuentes de la interfaz).
pub fn export_used(scenes: &[SceneFile], extra: &BTreeSet<String>, out: &Path) -> Result<Vec<String>, String> {
    if extra.is_empty() && scenes.iter().all(|s| refs_in_scene(s).is_empty()) {
        return Ok(Vec::new());
    }
    let lib = library_dir().ok_or("no se encuentra la biblioteca del motor")?;
    export_used_from(&lib, scenes, extra, out)
}

pub fn export_used_from(lib: &Path, scenes: &[SceneFile], extra: &BTreeSet<String>, out: &Path) -> Result<Vec<String>, String> {
    let mut files: BTreeSet<String> = BTreeSet::new();
    let mut packs: BTreeSet<String> = BTreeSet::new();
    let todas = scenes.iter().flat_map(refs_in_scene).chain(extra.iter().filter(|r| is_library_path(r)).cloned()).collect::<BTreeSet<String>>();
    {
        for r in todas {
            let rel = relative(&r).unwrap_or(&r).to_string();
            let src = lib.join(&rel);
            if !src.exists() {
                return Err(format!("falta {} en la biblioteca", src.display()));
            }
            if let Some(p) = rel.split('/').next() {
                packs.insert(p.to_string());
            }
            let es_modelo = src.extension().is_some_and(|e| e == "glb" || e == "gltf");
            for d in if es_modelo { dependencies(&src) } else { Vec::new() } {
                if let Ok(r) = d.strip_prefix(lib) {
                    files.insert(r.to_string_lossy().replace('\\', "/"));
                }
            }
            files.insert(rel);
        }
    }
    for p in &packs {
        let lic = format!("LICENCIAS/{p}.txt");
        if lib.join(&lic).exists() {
            files.insert(lic);
        }
    }
    let dst_root = out.join(DIR_NAME);
    for f in &files {
        let dst = dst_root.join(f);
        if let Some(d) = dst.parent() {
            std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
        }
        std::fs::copy(lib.join(f), &dst).map_err(|e| format!("{f}: {e}"))?;
    }
    Ok(files.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::*;
    use crate::scene::Scene;

    const CATALOGO: &str = r#"{
      "version": 1,
      "packs": [{"id": "naturaleza", "name": "Stylized Nature MegaKit", "author": "Quaternius", "license": "CC0 1.0 Universal", "source": "https://quaternius.itch.io/stylized-nature-megakit", "models": 2}],
      "models": [
        {"pack": "naturaleza", "category": "Árboles", "name": "Common Tree 1", "path": "naturaleza/CommonTree_1.glb", "thumbnail": "naturaleza/miniaturas/CommonTree_1.png",
         "triangles": 1200, "has_skin": false, "has_animations": false, "clips": [], "source": "https://quaternius.itch.io/stylized-nature-megakit"},
        {"pack": "naturaleza", "category": "Rocas", "name": "Pebble Round 1", "path": "naturaleza/Pebble_Round_1.glb", "triangles": 80, "has_skin": false, "has_animations": false},
        {"pack": "animaciones", "category": "Maniquí", "name": "UAL1 Standard", "path": "animaciones/UAL1_Standard.glb", "triangles": 5000,
         "has_skin": true, "has_animations": true, "clips": ["Idle_Loop", "Walk_Loop"]}
      ]
    }"#;

    #[test]
    fn catalog_parses_and_filters() {
        let c = Catalog::parse(CATALOGO).unwrap();
        assert_eq!(c.packs[0].author, "Quaternius");
        assert_eq!(c.models.len(), 3);
        assert_eq!(c.models[1].thumbnail, "", "campos opcionales por defecto");
        assert_eq!(c.categories(Some("naturaleza")), vec!["Árboles".to_string(), "Rocas".to_string()]);
        assert_eq!(c.filter(Some("naturaleza"), None, "").count(), 2);
        // Búsqueda sin tildes ni mayúsculas, todas las palabras.
        assert_eq!(c.filter(None, None, "arboles tree").map(|m| m.name.as_str()).collect::<Vec<_>>(), vec!["Common Tree 1"]);
        assert_eq!(c.filter(None, Some("Rocas"), "").count(), 1);
        assert_eq!(c.filter(None, None, "inexistente").count(), 0);
        let e = c.entry("biblioteca://naturaleza/CommonTree_1.glb").unwrap();
        assert_eq!(e.uri(), "biblioteca://naturaleza/CommonTree_1.glb");
        assert_eq!(c.animation_sources().count(), 1);
        assert!(Catalog::parse("{nope").is_err());
    }

    #[test]
    fn library_dir_lookup_order() {
        let tmp = std::env::temp_dir().join(format!("maker-bib-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let env_dir = tmp.join("env");
        let exe_dir = tmp.join("dest/app");
        let install = tmp.join("dest/codigo/assets/biblioteca");
        let compiled = tmp.join("repo/assets/biblioteca");
        for d in [&env_dir, &exe_dir, &install, &compiled] {
            std::fs::create_dir_all(d).unwrap();
        }
        let canon = |p: &Path| p.canonicalize().unwrap();
        // 1. La variable de entorno manda.
        assert_eq!(find_library_dir(Some(env_dir.clone()), Some(&exe_dir), &compiled), Some(canon(&env_dir)));
        // Variable vacía o inexistente: se ignora.
        assert_eq!(find_library_dir(Some(tmp.join("no")), Some(&exe_dir), &compiled), Some(canon(&install)));
        // 3. Instalación: <destino>/codigo/assets/biblioteca.
        assert_eq!(find_library_dir(None, Some(&exe_dir), &compiled), Some(canon(&install)));
        // 2. Junto al ejecutable (juego exportado) gana a la instalación.
        std::fs::create_dir_all(exe_dir.join(DIR_NAME)).unwrap();
        assert_eq!(find_library_dir(None, Some(&exe_dir), &compiled), Some(canon(&exe_dir.join(DIR_NAME))));
        // 4. Sin nada más: la del repositorio.
        assert_eq!(find_library_dir(None, Some(&tmp.join("otro/bin")), &compiled), Some(canon(&compiled)));
        assert_eq!(find_library_dir(None, None, &tmp.join("nada")), None);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn resolves_paths_and_collects_scene_refs() {
        assert!(is_library_path("biblioteca://naturaleza/Fern_1.glb"));
        assert_eq!(relative("biblioteca://naturaleza/Fern_1.glb"), Some("naturaleza/Fern_1.glb"));
        assert_eq!(resolve("assets/models/a.glb", Some(Path::new("/p"))), PathBuf::from("/p/assets/models/a.glb"));
        assert!(resolve("biblioteca://naturaleza/Fern_1.glb", Some(Path::new("/p"))).ends_with("naturaleza/Fern_1.glb"));
        let mut s = Scene::starter();
        let a = s.spawn("Helecho");
        s.insert(a, MeshRenderer { mesh: MeshSource::Model("biblioteca://naturaleza/Fern_1.glb".into()), material: Material::default(), cast_shadows: true });
        let b = s.spawn("Chica");
        s.insert(b, MeshRenderer { mesh: MeshSource::Model("biblioteca://personajes/Superhero_Female_FullBody.glb".into()), material: Material::default(), cast_shadows: true });
        s.insert(b, Animator::new(DEFAULT_CLIPS, DEFAULT_IDLE));
        let c = s.spawn("Propio");
        s.insert(c, MeshRenderer { mesh: MeshSource::Model("assets/models/x.glb".into()), material: Material::default(), cast_shadows: true });
        let refs = refs_in_scene(&s.to_file());
        assert_eq!(
            refs.into_iter().collect::<Vec<_>>(),
            vec![DEFAULT_CLIPS.to_string(), "biblioteca://naturaleza/Fern_1.glb".into(), "biblioteca://personajes/Superhero_Female_FullBody.glb".into()]
        );
    }
}
