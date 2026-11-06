//! Actualizaciones automáticas sin releases: cada máquina compila Maker Studio desde la rama
//! `main` de GitHub con el instalador de la raíz del repo (`instalar.sh` / `instalar.ps1`).
//!
//! - El instalador deja `<destino>/app` (maker-studio, maker-player, version.json) y el clon en
//!   `<destino>/codigo`. `version.json`, junto a este ejecutable, dice de qué commit y rama salió.
//!   Sin él es una compilación de desarrollo (`cargo run`, `target/release`): nunca se actualiza sola.
//! - Para saber si hay algo nuevo se hace `git fetch` en el clon, así que los repos privados
//!   funcionan con las credenciales que ya tenga git.
//! - Para actualizar se lanza el instalador de la rama con `--actualizar --esperar-pid <nosotros>
//!   --relanzar` en su propio grupo de procesos. Compila en `app.nuevo`, escribe `LISTO <commit>` en
//!   `registro/actualizacion.log` y espera a que el editor se cierre para cambiar las carpetas y
//!   volver a abrirlo. Aquí se sigue ese registro para mostrar el progreso.

use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const RAMA: &str = "main";
const INSTALADOR: &str = if cfg!(windows) { "instalar.ps1" } else { "instalar.sh" };
const LIMITE_GIT: Duration = Duration::from_secs(45);
const PRIMERA_COMPROBACION: Duration = Duration::from_secs(60);
const CADA: Duration = Duration::from_secs(6 * 60 * 60);
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
#[cfg(windows)]
const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;

#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub struct VersionInstalada {
    pub commit: String,
    #[serde(default = "rama_por_defecto")]
    pub rama: String,
    #[serde(default)]
    pub fecha: Option<String>,
}

fn rama_por_defecto() -> String {
    RAMA.into()
}

#[derive(Debug, Clone, PartialEq)]
pub struct Commit {
    pub sha: String,
    pub mensaje: Option<String>,
    pub fecha: Option<String>,
}

/// En qué punto está la actualización; la interfaz lo lee cada fotograma.
#[derive(Debug, Clone, PartialEq)]
pub enum Estado {
    Inactiva,
    Buscando,
    AlDia,
    /// Hay un commit más nuevo (solo se muestra con la actualización automática apagada).
    Nueva { ultimo: Commit },
    Compilando { paso: u32, total: u32, texto: String },
    Lista { commit: String },
    Error(String),
}

#[derive(Debug, PartialEq)]
enum Linea {
    Paso { paso: u32, total: u32, texto: String },
    Listo(String),
    Error(String),
}

/// Las líneas del registro del instalador que importan aquí; el resto se ignora.
fn interpretar_linea(linea: &str) -> Option<Linea> {
    let linea = linea.trim_start_matches('\u{feff}').trim();
    let (clave, resto) = linea.split_once(' ').unwrap_or((linea, ""));
    match clave {
        "PASO" => {
            let (cuenta, texto) = resto.split_once(' ').unwrap_or((resto, ""));
            let (paso, total) = cuenta.split_once('/')?;
            Some(Linea::Paso { paso: paso.parse().ok()?, total: total.parse().ok()?, texto: texto.into() })
        }
        "LISTO" if !resto.is_empty() => Some(Linea::Listo(resto.into())),
        "ERROR" => Some(Linea::Error(resto.into())),
        _ => None,
    }
}

pub fn corto(commit: &str) -> String {
    commit.chars().take(7).collect()
}

/// Un hash corto y uno largo son el mismo commit si uno empieza por el otro.
fn mismo_commit(a: &str, b: &str) -> bool {
    let (a, b) = (a.to_ascii_lowercase(), b.to_ascii_lowercase());
    a.len() >= 7 && b.len() >= 7 && (a.starts_with(&b) || b.starts_with(&a))
}

fn interpretar_version(texto: &str) -> Option<VersionInstalada> {
    serde_json::from_str(texto.trim_start_matches('\u{feff}')).ok()
}

/// Salida de `git log -1 --format=%H%x00%s%x00%cI`.
fn interpretar_log(salida: &str) -> Option<Commit> {
    let mut campos = salida.trim().splitn(3, '\0');
    let sha = campos.next().filter(|s| !s.is_empty())?.to_string();
    let mensaje = campos.next().filter(|s| !s.is_empty()).map(String::from);
    let fecha = campos.next().filter(|s| !s.is_empty()).map(String::from);
    Some(Commit { sha, mensaje, fecha })
}

fn explicar_error_git(error: &str) -> String {
    let e = error.to_lowercase();
    if ["authentication failed", "repository not found", "could not read username", "terminal prompts disabled", "403", "401"].iter().any(|m| e.contains(m)) {
        "sin acceso al repositorio: hace falta permiso para verlo y haber iniciado sesión en GitHub con Git".into()
    } else if ["could not resolve host", "unable to access", "timed out"].iter().any(|m| e.contains(m)) {
        "sin conexión con el repositorio".into()
    } else {
        format!("git falló: {}", error.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("sin detalles"))
    }
}

fn orden_oculta(programa: impl AsRef<std::ffi::OsStr>) -> Command {
    let mut orden = Command::new(programa);
    orden.stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        orden.creation_flags(CREATE_NO_WINDOW);
    }
    orden
}

/// `git -C <carpeta> …` con límite de tiempo. Nunca pide credenciales: falla en vez de esperar.
fn git(carpeta: &Path, argumentos: &[&str]) -> Result<String, String> {
    let mut hijo = orden_oculta("git")
        .arg("-C")
        .arg(carpeta)
        .args(argumentos)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GCM_INTERACTIVE", "never")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("no se pudo ejecutar git ({e}): ¿está instalado?"))?;
    let inicio = Instant::now();
    while hijo.try_wait().map_err(|e| e.to_string())?.is_none() {
        if inicio.elapsed() > LIMITE_GIT {
            let _ = hijo.kill();
            return Err(format!("git {} no respondió en {} s", argumentos[0], LIMITE_GIT.as_secs()));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let salida = hijo.wait_with_output().map_err(|e| e.to_string())?;
    if !salida.status.success() {
        return Err(explicar_error_git(&String::from_utf8_lossy(&salida.stderr)));
    }
    Ok(String::from_utf8_lossy(&salida.stdout).into_owned())
}

/// `<destino>/app`, donde está este ejecutable.
fn carpeta_app() -> Option<PathBuf> {
    std::env::current_exe().ok()?.canonicalize().ok()?.parent().map(Path::to_path_buf)
}

/// La versión instalada, o `None` en una compilación de desarrollo.
pub fn version_instalada() -> Option<VersionInstalada> {
    interpretar_version(&fs::read_to_string(carpeta_app()?.join("version.json")).ok()?)
}

/// `<destino>`: la carpeta con `app/`, `codigo/` y `registro/`.
fn destino() -> Option<PathBuf> {
    carpeta_app()?.parent().map(Path::to_path_buf)
}

fn registro(destino: &Path) -> PathBuf {
    destino.join("registro").join("actualizacion.log")
}

/// Trae la rama y compara su último commit con el instalado.
/// `Ok(None)`: al día. `Err` también en compilaciones de desarrollo.
pub fn buscar() -> Result<Option<Commit>, String> {
    let version = version_instalada().ok_or("esta copia no se instaló con el instalador: no se actualiza sola")?;
    let clon = destino().ok_or("no se encuentra la carpeta de Maker Studio")?.join("codigo");
    if !clon.join(".git").exists() {
        return Err(format!("falta el código en {}: vuelve a instalar Maker Studio", clon.display()));
    }
    git(&clon, &["fetch", "--quiet", "origin", &version.rama])?;
    let remota = format!("origin/{}", version.rama);
    let ultimo = interpretar_log(&git(&clon, &["log", "-1", "--format=%H%x00%s%x00%cI", &remota])?).ok_or(format!("no se pudo leer el último commit de {remota}"))?;
    Ok((!mismo_commit(&version.commit, &ultimo.sha)).then_some(ultimo))
}

/// El instalador de la punta de la rama (traída por `buscar`), copiado a `registro/`. Así un
/// arreglo del instalador se aplica desde la actualización que lo trae, y bash nunca ejecuta un
/// archivo que `git reset` cambia por debajo.
fn instalador_de_la_rama(destino: &Path, rama: &str) -> Result<PathBuf, String> {
    let codigo = destino.join("codigo");
    let texto = git(&codigo, &["show", &format!("origin/{rama}:{INSTALADOR}")]).or_else(|_| fs::read_to_string(codigo.join(INSTALADOR)).map_err(|e| e.to_string()))?;
    if !texto.starts_with('#') {
        return Err(format!("{INSTALADOR} no parece un instalador"));
    }
    let carpeta = destino.join("registro");
    fs::create_dir_all(&carpeta).map_err(|e| e.to_string())?;
    let (ruta, nuevo) = (carpeta.join(INSTALADOR), carpeta.join(format!("{INSTALADOR}.nuevo")));
    fs::write(&nuevo, texto).map_err(|e| e.to_string())?;
    fs::rename(&nuevo, &ruta).map_err(|e| e.to_string())?;
    Ok(ruta)
}

/// Lee lo añadido al registro desde `leidos` (vuelve a 0 si el archivo se recreó).
fn leer_desde(ruta: &Path, leidos: &mut u64) -> String {
    let Ok(mut f) = fs::File::open(ruta) else { return String::new() };
    let largo = f.metadata().map(|m| m.len()).unwrap_or(0);
    if largo < *leidos {
        *leidos = 0;
    }
    let mut nuevo = Vec::new();
    if f.seek(SeekFrom::Start(*leidos)).is_ok() && f.read_to_end(&mut nuevo).is_ok() {
        *leidos += nuevo.len() as u64;
    }
    String::from_utf8_lossy(&nuevo).into_owned()
}

// ------------------------------------------------------------------ ajustes

fn ajustes() -> Option<PathBuf> {
    Some(crate::prefs::carpeta_config()?.join("actualizar.txt"))
}

/// La actualización automática está activada salvo que el usuario la apague.
pub fn automatico() -> bool {
    ajustes().and_then(|f| fs::read_to_string(f).ok()).map(|t| t.trim() != "no").unwrap_or(true)
}

pub fn cambiar_automatico(si: bool) {
    if let Some(f) = ajustes() {
        if let Some(d) = f.parent() {
            let _ = fs::create_dir_all(d);
        }
        let _ = fs::write(f, if si { "si" } else { "no" });
    }
}

// ------------------------------------------------------------------ el actualizador

/// Compartido entre el editor (hilo de la interfaz) y los hilos en segundo plano.
#[derive(Clone)]
pub struct Actualizador {
    estado: Arc<Mutex<Estado>>,
    repintar: Arc<Mutex<Option<egui::Context>>>,
    /// El editor se cierra para aplicar la actualización: el instalador debe reabrirlo.
    aplicando: Arc<AtomicBool>,
}

impl Default for Actualizador {
    fn default() -> Self {
        Actualizador { estado: Arc::new(Mutex::new(Estado::Inactiva)), repintar: Arc::new(Mutex::new(None)), aplicando: Arc::default() }
    }
}

impl Actualizador {
    pub fn estado(&self) -> Estado {
        self.estado.lock().unwrap().clone()
    }

    fn poner(&self, e: Estado) {
        *self.estado.lock().unwrap() = e;
        if let Some(ctx) = self.repintar.lock().unwrap().as_ref() {
            ctx.request_repaint();
        }
    }

    fn ocupado(&self) -> bool {
        matches!(self.estado(), Estado::Buscando | Estado::Compilando { .. } | Estado::Lista { .. })
    }

    /// Un minuto después de arrancar y cada 6 horas: si hay versión nueva y la actualización
    /// automática está activada, la compila en segundo plano.
    pub fn vigilar(&self, ctx: &egui::Context) {
        *self.repintar.lock().unwrap() = Some(ctx.clone());
        if version_instalada().is_none() {
            return;
        }
        let a = self.clone();
        std::thread::spawn(move || {
            let mut espera = PRIMERA_COMPROBACION;
            loop {
                std::thread::sleep(espera);
                espera = CADA;
                if automatico() && !a.ocupado() {
                    a.buscar_e_instalar(true);
                }
            }
        });
    }

    /// «Buscar actualizaciones» (menú): busca en segundo plano y compila la versión nueva enseguida
    /// si `instalar` (o la actualización automática) está activado.
    pub fn buscar_ahora(&self, instalar: bool) {
        if self.ocupado() {
            return;
        }
        let a = self.clone();
        std::thread::spawn(move || a.buscar_e_instalar(instalar));
    }

    fn buscar_e_instalar(&self, instalar: bool) {
        self.poner(Estado::Buscando);
        match buscar() {
            Err(e) => self.poner(Estado::Error(e)),
            Ok(None) => self.poner(Estado::AlDia),
            Ok(Some(ultimo)) if instalar => {
                eprintln!("[maker-studio] versión nueva {}: compilando", corto(&ultimo.sha));
                if let Err(e) = self.instalar() {
                    self.poner(Estado::Error(e));
                }
            }
            Ok(Some(ultimo)) => self.poner(Estado::Nueva { ultimo }),
        }
    }

    /// Lanza el instalador en modo actualización y sigue su registro hasta `LISTO` o `ERROR`.
    pub fn instalar(&self) -> Result<(), String> {
        let version = version_instalada().ok_or("esta copia no se instaló con el instalador")?;
        let destino = destino().ok_or("no se encuentra la carpeta de Maker Studio")?;
        let script = instalador_de_la_rama(&destino, &version.rama)?;
        // Un registro viejo no debe leerse como el de esta vez.
        let _ = fs::remove_file(registro(&destino));
        let _ = fs::remove_file(destino.join("registro").join("no-relanzar"));
        let pid = std::process::id().to_string();
        #[cfg(windows)]
        let mut orden = {
            use std::os::windows::process::CommandExt;
            let mut o = orden_oculta("powershell.exe");
            o.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
                .arg(&script)
                .args(["-Actualizar", "-Destino"])
                .arg(&destino)
                .args(["-Rama", &version.rama, "-EsperarPid", &pid, "-Relanzar"]);
            // Su propio grupo: sigue vivo cuando el editor se cierra, que es cuando cambia las carpetas.
            o.creation_flags(CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP);
            o
        };
        #[cfg(not(windows))]
        let mut orden = {
            use std::os::unix::process::CommandExt;
            let mut o = orden_oculta("/bin/bash");
            o.arg(&script).args(["--actualizar", "--destino"]).arg(&destino).args(["--rama", &version.rama, "--esperar-pid", &pid, "--relanzar"]);
            o.process_group(0);
            o
        };
        orden.stdout(Stdio::null()).stderr(Stdio::null());
        let hijo = orden.spawn().map_err(|e| format!("no se pudo lanzar {INSTALADOR}: {e}"))?;
        self.poner(Estado::Compilando { paso: 0, total: 0, texto: "Empezando".into() });
        let a = self.clone();
        let log = registro(&destino);
        std::thread::spawn(move || a.seguir_registro(&log, hijo));
        Ok(())
    }

    fn seguir_registro(&self, ruta: &Path, mut instalador: Child) {
        let (mut leidos, mut pendiente) = (0, String::new());
        loop {
            std::thread::sleep(Duration::from_millis(500));
            let terminado = instalador.try_wait().ok().flatten();
            pendiente.push_str(&leer_desde(ruta, &mut leidos));
            while let Some(fin) = pendiente.find('\n') {
                let linea: String = pendiente.drain(..=fin).collect();
                match interpretar_linea(&linea) {
                    Some(Linea::Paso { paso, total, texto }) => self.poner(Estado::Compilando { paso, total, texto }),
                    Some(Linea::Listo(commit)) => {
                        eprintln!("[maker-studio] actualización {} lista", corto(&commit));
                        return self.poner(Estado::Lista { commit: corto(&commit) });
                    }
                    Some(Linea::Error(texto)) => return self.poner(Estado::Error(texto)),
                    None => {}
                }
            }
            if let Some(codigo) = terminado {
                return self.poner(Estado::Error(format!("el instalador terminó sin acabar ({codigo}); mira {}", ruta.display())));
            }
        }
    }

    /// El editor va a cerrarse para aplicar una actualización lista: el instalador lo reabrirá.
    pub fn aplicar(&self) {
        self.aplicando.store(true, Ordering::SeqCst);
    }

    /// Al cerrar sin aplicar una actualización pendiente: el instalador cambia las carpetas igualmente
    /// cuando nos vamos (el siguiente arranque ya es la versión nueva), pero no debe reabrir el editor.
    pub fn al_salir(&self) {
        let pendiente = matches!(self.estado(), Estado::Compilando { .. } | Estado::Lista { .. });
        if pendiente
            && !self.aplicando.load(Ordering::SeqCst)
            && let Some(d) = destino()
        {
            let _ = fs::write(d.join("registro").join("no-relanzar"), "");
        }
    }

    /// Para `maker-studio --actualizar`: busca, compila y espera a `LISTO` sin ventana, mostrando el
    /// progreso. Al volver, el editor sale y el instalador aplica la actualización.
    pub fn sin_ventana(&self) -> Result<(), String> {
        let v = version_instalada().ok_or("esta copia no se instaló con el instalador: no se actualiza sola")?;
        println!("instalada: {} ({})", corto(&v.commit), v.rama);
        match buscar()? {
            None => {
                println!("está al día");
                return Ok(());
            }
            Some(u) => println!("nueva: {} {}", corto(&u.sha), u.mensaje.unwrap_or_default()),
        }
        self.aplicar();
        self.instalar()?;
        let mut ultimo = String::new();
        loop {
            std::thread::sleep(Duration::from_millis(300));
            match self.estado() {
                Estado::Compilando { paso, total, texto } => {
                    let t = format!("[{paso}/{total}] {texto}");
                    if t != ultimo && total > 0 {
                        println!("{t}");
                        ultimo = t;
                    }
                }
                Estado::Lista { commit } => {
                    println!("versión {commit} lista: se aplica al cerrar");
                    return Ok(());
                }
                Estado::Error(e) => return Err(e),
                _ => {}
            }
        }
    }
}

/// Texto para el menú Ayuda y la barra de actualización.
pub fn descripcion_version() -> String {
    match version_instalada() {
        Some(v) => format!("{} ({}{})", corto(&v.commit), v.rama, v.fecha.map(|f| format!(", {}", &f[..f.len().min(10)])).unwrap_or_default()),
        None => "compilación de desarrollo".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lineas_del_registro() {
        assert_eq!(interpretar_linea("PASO 3/6 Compilando"), Some(Linea::Paso { paso: 3, total: 6, texto: "Compilando".into() }));
        assert_eq!(interpretar_linea("\u{feff}LISTO abc1234def\r"), Some(Linea::Listo("abc1234def".into())));
        assert_eq!(interpretar_linea("ERROR sin conexión"), Some(Linea::Error("sin conexión".into())));
        assert_eq!(interpretar_linea("APLICADO abc"), None);
        assert_eq!(interpretar_linea("PASO x"), None);
    }

    #[test]
    fn versiones_y_commits() {
        let v = interpretar_version("\u{feff}{ \"commit\": \"1e648a248962\", \"fecha\": \"2026-11-05T17:15:12+01:00\", \"rama\": \"main\", \"repo\": \"x\" }").unwrap();
        assert_eq!(v.commit, "1e648a248962");
        assert_eq!(interpretar_version("{ \"commit\": \"abcdef12\" }").unwrap().rama, "main");
        assert!(interpretar_version("{}").is_none());
        assert!(mismo_commit("1e648a2", "1E648A248962af"));
        assert!(!mismo_commit("1e648a2", "1e648a3ffff"));
        assert!(!mismo_commit("1e6", "1e6"), "demasiado corto para comparar");
        let c = interpretar_log("abc123456\0Arreglar zoom\x002026-11-05T10:00:00+01:00\n").unwrap();
        assert_eq!(c.mensaje.as_deref(), Some("Arreglar zoom"));
        assert_eq!(c.fecha.as_deref(), Some("2026-11-05T10:00:00+01:00"));
    }

    #[test]
    fn errores_de_git_explicados() {
        assert!(explicar_error_git("fatal: Authentication failed for 'https://…'").starts_with("sin acceso"));
        assert!(explicar_error_git("fatal: unable to access: Could not resolve host").starts_with("sin conexión"));
        assert!(explicar_error_git("\n  fatal: algo malo\n").contains("algo malo"));
    }

    #[test]
    fn lee_solo_lo_nuevo() {
        let f = std::env::temp_dir().join(format!("maker-upd-{}.log", std::process::id()));
        fs::write(&f, "PASO 1/6 a\n").unwrap();
        let mut n = 0;
        assert_eq!(leer_desde(&f, &mut n), "PASO 1/6 a\n");
        assert_eq!(leer_desde(&f, &mut n), "");
        fs::write(&f, "X\n").unwrap();
        assert_eq!(leer_desde(&f, &mut n), "X\n");
        let _ = fs::remove_file(f);
    }
}
