//! Carga de bibliotecas de scripts (`cdylib`) con recarga en caliente.
//!
//! Cada carga copia la biblioteca a un nombre único en una carpeta temporal y
//! la abre desde ahí: el `.so` original queda libre para que `cargo build` lo
//! reescriba mientras el editor sigue abierto, y `dlopen` nunca devuelve una
//! copia en caché de la versión anterior.

use crate::components::PropVal;
use maker_api::{ABI_VERSION, ENTRY_SYMBOL, EntryFn, PropInfo, ScriptDesc, ScriptLibrary};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub struct ScriptLib {
    _lib: Option<libloading::Library>,
    lib: *const ScriptLibrary,
    pub source: Option<PathBuf>,
    pub loaded_at: std::time::SystemTime,
}

// La biblioteca es inmutable una vez cargada.
unsafe impl Send for ScriptLib {}
unsafe impl Sync for ScriptLib {}

static LOAD_COUNTER: AtomicU64 = AtomicU64::new(0);

impl ScriptLib {
    /// Biblioteca enlazada estáticamente (tests, reproductor con scripts integrados).
    pub fn from_static(lib: &'static ScriptLibrary) -> Result<Self, String> {
        if lib.abi_version != ABI_VERSION {
            return Err(format!("ABI {} distinta de la del motor ({ABI_VERSION})", lib.abi_version));
        }
        Ok(Self { _lib: None, lib, source: None, loaded_at: std::time::SystemTime::now() })
    }

    pub fn load(path: &Path) -> Result<Self, String> {
        if !path.exists() {
            return Err(format!("no existe {} (compila los scripts primero)", path.display()));
        }
        let dir = std::env::temp_dir().join("maker-studio-scripts");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let n = LOAD_COUNTER.fetch_add(1, Ordering::Relaxed);
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("so");
        let copy = dir.join(format!("scripts-{}-{n}.{ext}", std::process::id()));
        std::fs::copy(path, &copy).map_err(|e| format!("copiando {}: {e}", path.display()))?;
        // SAFETY: cargamos código del propio proyecto del usuario.
        let lib = unsafe { libloading::Library::new(&copy) }.map_err(|e| format!("cargando {}: {e}", path.display()));
        // En Unix la copia ya puede borrarse; en Windows queda bloqueada hasta descargarla.
        #[cfg(unix)]
        let _ = std::fs::remove_file(&copy);
        let lib = lib?;
        let ptr = unsafe {
            let entry: libloading::Symbol<EntryFn> =
                lib.get(ENTRY_SYMBOL).map_err(|e| format!("{} no exporta maker_scripts (¿falta export_scripts!?): {e}", path.display()))?;
            entry()
        };
        if ptr.is_null() {
            return Err("maker_scripts devolvió nulo".into());
        }
        let abi = unsafe { (*ptr).abi_version };
        if abi != ABI_VERSION {
            return Err(format!("la biblioteca usa la ABI {abi} y el motor la {ABI_VERSION}: recompila los scripts"));
        }
        Ok(Self { _lib: Some(lib), lib: ptr, source: Some(path.to_path_buf()), loaded_at: std::time::SystemTime::now() })
    }

    pub fn descs(&self) -> &[ScriptDesc] {
        unsafe { (*self.lib).descs() }
    }

    pub fn names(&self) -> Vec<String> {
        self.descs().iter().map(|d| unsafe { d.name() }.to_string()).collect()
    }

    pub fn index_of(&self, name: &str) -> Option<usize> {
        self.descs().iter().position(|d| unsafe { d.name() } == name)
    }

    /// Valores por defecto de las propiedades de un script (instancia temporal).
    pub fn default_props(&self, name: &str) -> Vec<(String, PropVal)> {
        let Some(i) = self.index_of(name) else { return Vec::new() };
        let d = &self.descs()[i];
        let inst = (d.create)();
        if inst.is_null() {
            return Vec::new();
        }
        let out = read_props(d, inst);
        (d.destroy)(inst);
        out
    }
}

/// Lee todas las propiedades de una instancia.
pub fn read_props(d: &ScriptDesc, inst: *mut std::ffi::c_void) -> Vec<(String, PropVal)> {
    let n = (d.prop_count)(inst);
    let mut out = Vec::with_capacity(n as usize);
    for i in 0..n {
        let mut info = PropInfo::default();
        if (d.prop_get)(inst, i, &mut info) && !info.name_ptr.is_null() {
            let name = unsafe { std::str::from_utf8(std::slice::from_raw_parts(info.name_ptr, info.name_len)) }.unwrap_or("?").to_string();
            if let Some(v) = PropVal::from_api(info.value) {
                out.push((name, v));
            }
        }
    }
    out
}

/// Escribe en la instancia los valores guardados en la escena (por nombre y tipo).
pub fn apply_props(d: &ScriptDesc, inst: *mut std::ffi::c_void, values: &std::collections::BTreeMap<String, PropVal>) {
    if values.is_empty() {
        return;
    }
    let n = (d.prop_count)(inst);
    for i in 0..n {
        let mut info = PropInfo::default();
        if !(d.prop_get)(inst, i, &mut info) || info.name_ptr.is_null() {
            continue;
        }
        let name = unsafe { std::str::from_utf8(std::slice::from_raw_parts(info.name_ptr, info.name_len)) }.unwrap_or("");
        if let Some(v) = values.get(name) {
            let api = v.to_api();
            if api.kind == info.value.kind {
                (d.prop_set)(inst, i, api);
            }
        }
    }
}

/// Extensión de biblioteca dinámica de la plataforma.
pub fn dylib_file_name(crate_name: &str) -> String {
    let n = crate_name.replace('-', "_");
    if cfg!(target_os = "windows") {
        format!("{n}.dll")
    } else if cfg!(target_os = "macos") {
        format!("lib{n}.dylib")
    } else {
        format!("lib{n}.so")
    }
}
