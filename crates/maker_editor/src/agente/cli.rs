//! Las tres CLIs de agentes: localizarlas, sus modelos y esfuerzos, la orden de cada turno y
//! su ejecución en segundo plano (sin TTY, matando el árbol de procesos al parar).

use super::eventos::{Evento, Interprete};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Backend {
    Claude,
    Codex,
    Antigravity,
}

impl Backend {
    pub const TODOS: [Backend; 3] = [Backend::Claude, Backend::Codex, Backend::Antigravity];

    pub fn nombre(self) -> &'static str {
        match self {
            Backend::Claude => "Claude Code",
            Backend::Codex => "Codex",
            Backend::Antigravity => "Antigravity",
        }
    }

    pub fn ejecutable(self) -> &'static str {
        match self {
            Backend::Claude => "claude",
            Backend::Codex => "codex",
            Backend::Antigravity => "agy",
        }
    }

    pub fn clave(self) -> &'static str {
        match self {
            Backend::Claude => "claude",
            Backend::Codex => "codex",
            Backend::Antigravity => "agy",
        }
    }

    pub fn desde_clave(s: &str) -> Option<Backend> {
        Backend::TODOS.into_iter().find(|b| b.clave() == s || b.nombre().eq_ignore_ascii_case(s))
    }

    /// Esfuerzos que acepta la CLI (para Codex dependen del modelo: ver `Catalogo`).
    pub fn esfuerzos(self) -> &'static [&'static str] {
        match self {
            Backend::Claude | Backend::Antigravity => &["low", "medium", "high", "xhigh", "max"],
            Backend::Codex => &["low", "medium", "high", "xhigh", "max", "ultra"],
        }
    }

    /// Ventana de contexto supuesta cuando la CLI no la informa.
    pub fn ventana_por_defecto(self, modelo: &str) -> u64 {
        let m = modelo.to_ascii_lowercase();
        match self {
            Backend::Claude => 200_000,
            Backend::Codex => 272_000,
            Backend::Antigravity if m.starts_with("claude") || m.starts_with("gpt") => 200_000,
            Backend::Antigravity => 1_000_000,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Permisos {
    /// Solo lectura / planificar.
    Lectura,
    /// Editar archivos.
    #[default]
    Edicion,
    /// Acceso total, sin preguntar.
    Total,
}

impl Permisos {
    pub const TODOS: [Permisos; 3] = [Permisos::Lectura, Permisos::Edicion, Permisos::Total];

    pub fn nombre(self) -> &'static str {
        match self {
            Permisos::Lectura => "Solo lectura",
            Permisos::Edicion => "Editar archivos",
            Permisos::Total => "Acceso total",
        }
    }

    pub fn ayuda(self, b: Backend) -> String {
        match (b, self) {
            (Backend::Claude, Permisos::Lectura) => "--permission-mode plan",
            (Backend::Claude, Permisos::Edicion) => "--permission-mode acceptEdits",
            (Backend::Claude, Permisos::Total) => "--permission-mode bypassPermissions",
            (Backend::Codex, Permisos::Lectura) => "sandbox read-only",
            (Backend::Codex, Permisos::Edicion) => "sandbox workspace-write",
            (Backend::Codex, Permisos::Total) => "sandbox danger-full-access",
            (Backend::Antigravity, Permisos::Lectura) => "--mode plan",
            (Backend::Antigravity, Permisos::Edicion) => "--mode accept-edits",
            (Backend::Antigravity, Permisos::Total) => "--dangerously-skip-permissions",
        }
        .into()
    }

    pub fn desde_clave(s: &str) -> Option<Permisos> {
        match s {
            "read" | "lectura" | "plan" | "read-only" => Some(Permisos::Lectura),
            "edit" | "edicion" | "edición" => Some(Permisos::Edicion),
            "full" | "total" => Some(Permisos::Total),
            _ => None,
        }
    }
}

// ───────────────────────────── localizar ejecutables ─────────────────────────────

fn extensiones() -> Vec<String> {
    if cfg!(windows) {
        let pe = std::env::var("PATHEXT").unwrap_or_else(|_| ".EXE;.CMD;.BAT".into());
        let mut v: Vec<String> = pe.split(';').filter(|e| !e.is_empty()).map(|e| e.to_ascii_lowercase()).collect();
        // `.exe` antes que los shims de npm (`.cmd`).
        v.sort_by_key(|e| match e.as_str() {
            ".exe" => 0,
            ".cmd" => 1,
            ".bat" => 2,
            _ => 3,
        });
        v
    } else {
        vec![String::new()]
    }
}

/// Carpetas donde suelen instalarse las CLIs aunque el editor se abra sin el PATH de la shell.
fn carpetas_extra() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(h) = std::env::var_os("HOME").map(PathBuf::from) {
        for d in [".local/bin", ".npm-global/bin", ".bun/bin", ".volta/bin", ".cargo/bin", "bin"] {
            v.push(h.join(d));
        }
    }
    if let Some(a) = std::env::var_os("APPDATA") {
        v.push(PathBuf::from(a).join("npm"));
    }
    if let Some(l) = std::env::var_os("LOCALAPPDATA") {
        let l = PathBuf::from(l);
        v.push(l.join("Programs").join("Antigravity").join("bin"));
        v.push(l.join("Microsoft").join("WinGet").join("Links"));
    }
    if let Some(u) = std::env::var_os("USERPROFILE") {
        v.push(PathBuf::from(u).join(".local").join("bin"));
    }
    v.push(PathBuf::from("/usr/local/bin"));
    v.push(PathBuf::from("/opt/homebrew/bin"));
    v
}

fn es_ejecutable(p: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        p.metadata().is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        p.is_file()
    }
}

/// Busca `nombre` en el PATH (y carpetas habituales), resolviendo `.exe`/`.cmd` en Windows.
pub fn buscar_en_path(nombre: &str) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    dirs.extend(carpetas_extra());
    let exts = extensiones();
    for d in dirs {
        for e in &exts {
            let p = d.join(format!("{nombre}{e}"));
            if es_ejecutable(&p) {
                return Some(p);
            }
        }
    }
    None
}

/// Ruta efectiva: la personalizada (si existe) o la del PATH.
pub fn resolver(b: Backend, personalizada: &str) -> Option<PathBuf> {
    let p = personalizada.trim();
    if !p.is_empty() {
        let pb = PathBuf::from(p);
        return if pb.is_file() { Some(pb) } else { buscar_en_path(p) };
    }
    buscar_en_path(b.ejecutable())
}

/// Orden base para un ejecutable: los `.cmd`/`.bat` de Windows van por `cmd /C`.
/// Sin consola visible y sin TTY: stdin se conecta a una tubería o a nada.
pub fn orden(exe: &Path) -> Command {
    let ext = exe.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    let mut c = if cfg!(windows) && (ext == "cmd" || ext == "bat") {
        let mut c = Command::new("cmd");
        c.arg("/C").arg(exe);
        c
    } else {
        Command::new(exe)
    };
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(CREATE_NO_WINDOW);
    }
    c.stdin(Stdio::null());
    c
}

/// Ejecuta una orden corta y devuelve su stdout (con límite de tiempo).
fn salida_corta(mut c: Command, limite: Duration) -> Option<String> {
    let mut h = c.stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
    let out = h.stdout.take()?;
    let lector = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = std::io::Read::read_to_string(&mut BufReader::new(out), &mut s);
        s
    });
    let inicio = Instant::now();
    loop {
        if h.try_wait().ok().flatten().is_some() {
            break;
        }
        if inicio.elapsed() > limite {
            let _ = h.kill();
            let _ = h.wait();
            return None;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    lector.join().ok()
}

// ───────────────────────────── detección y catálogo ─────────────────────────────

#[derive(Clone, Debug, Default)]
pub struct InfoCli {
    pub ruta: Option<PathBuf>,
    pub version: Option<String>,
    /// Modelos descubiertos: (id, nombre visible, ventana, esfuerzos válidos).
    pub modelos: Vec<Modelo>,
    pub comprobado: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Modelo {
    pub id: String,
    pub nombre: String,
    pub ventana: Option<u64>,
    pub esfuerzos: Vec<String>,
}

/// Estado compartido de la detección (se rellena en un hilo).
#[derive(Clone, Default)]
pub struct Catalogo {
    datos: Arc<Mutex<std::collections::HashMap<Backend, InfoCli>>>,
}

impl Catalogo {
    pub fn info(&self, b: Backend) -> InfoCli {
        self.datos.lock().unwrap().get(&b).cloned().unwrap_or_default()
    }

    /// Vuelve a buscar las CLIs (rutas, versiones y modelos) en segundo plano.
    pub fn detectar(&self, rutas: [String; 3], ctx: Option<egui::Context>) {
        for (b, personal) in Backend::TODOS.into_iter().zip(rutas) {
            let datos = self.datos.clone();
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                let info = detectar_una(b, &personal);
                datos.lock().unwrap().insert(b, info);
                if let Some(c) = ctx {
                    c.request_repaint();
                }
            });
        }
    }

    /// Esfuerzos válidos para un modelo (Codex los informa por modelo).
    pub fn esfuerzos(&self, b: Backend, modelo: &str) -> Vec<String> {
        let info = self.info(b);
        if let Some(m) = info.modelos.iter().find(|m| m.id == modelo)
            && !m.esfuerzos.is_empty()
        {
            return m.esfuerzos.clone();
        }
        b.esfuerzos().iter().map(|s| s.to_string()).collect()
    }

    pub fn ventana(&self, b: Backend, modelo: &str) -> u64 {
        self.info(b).modelos.iter().find(|m| m.id == modelo).and_then(|m| m.ventana).unwrap_or_else(|| b.ventana_por_defecto(modelo))
    }
}

fn detectar_una(b: Backend, personal: &str) -> InfoCli {
    let Some(ruta) = resolver(b, personal) else { return InfoCli { comprobado: true, ..Default::default() } };
    let mut c = orden(&ruta);
    c.arg("--version");
    let version = salida_corta(c, Duration::from_secs(20)).map(|s| s.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim().to_string()).filter(|s| !s.is_empty());
    let modelos = match b {
        Backend::Claude => ["opus", "sonnet", "haiku"].iter().map(|m| Modelo { id: m.to_string(), nombre: m.to_string(), ..Default::default() }).collect(),
        Backend::Codex => {
            let mut c = orden(&ruta);
            c.args(["debug", "models"]);
            salida_corta(c, Duration::from_secs(30)).map(|s| modelos_codex(&s)).unwrap_or_default()
        }
        Backend::Antigravity => {
            let mut c = orden(&ruta);
            c.arg("models");
            salida_corta(c, Duration::from_secs(60)).map(|s| modelos_agy(&s)).unwrap_or_default()
        }
    };
    InfoCli { ruta: Some(ruta), version, modelos, comprobado: true }
}

/// `codex debug models`: JSON con `models[{slug, display_name, visibility, context_window, supported_reasoning_levels}]`.
pub fn modelos_codex(json: &str) -> Vec<Modelo> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json.trim()) else { return Vec::new() };
    v["models"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|m| m["visibility"].as_str().is_none_or(|x| x == "list"))
        .filter_map(|m| {
            let id = m["slug"].as_str()?.to_string();
            Some(Modelo {
                nombre: m["display_name"].as_str().unwrap_or(&id).to_string(),
                ventana: m["context_window"].as_u64(),
                esfuerzos: m["supported_reasoning_levels"].as_array().into_iter().flatten().filter_map(|e| e["effort"].as_str().map(String::from)).collect(),
                id,
            })
        })
        .collect()
}

/// `agy models`: líneas `slug<TAB>Nombre` (tras «Fetching available models...»).
pub fn modelos_agy(texto: &str) -> Vec<Modelo> {
    texto
        .lines()
        .filter_map(|l| {
            let (id, nombre) = l.split_once('\t')?;
            let id = id.trim();
            (!id.is_empty() && !id.contains(' ')).then(|| Modelo { id: id.into(), nombre: nombre.trim().into(), ..Default::default() })
        })
        .collect()
}

// ───────────────────────────── un turno ─────────────────────────────

/// Todo lo necesario para lanzar un turno.
#[derive(Clone, Debug, Default)]
pub struct Peticion {
    pub mensaje: String,
    pub modelo: String,
    pub esfuerzo: String,
    pub permisos: Permisos,
    /// Id nativo para reanudar.
    pub reanudar: Option<String>,
    /// Instrucciones de sistema (solo Claude, por `--append-system-prompt`).
    pub sistema: Option<String>,
}

/// Argumentos de la orden (sin el ejecutable) y el texto que va por stdin.
pub fn argumentos(b: Backend, p: &Peticion) -> (Vec<String>, Option<String>) {
    let mut a: Vec<String> = Vec::new();
    let mut add = |xs: &[&str]| a.extend(xs.iter().map(|s| s.to_string()));
    let modelo = p.modelo.trim();
    let esfuerzo = p.esfuerzo.trim();
    match b {
        Backend::Claude => {
            add(&["-p", "--output-format", "stream-json", "--verbose"]);
            if !modelo.is_empty() {
                add(&["--model", modelo]);
            }
            if !esfuerzo.is_empty() {
                add(&["--effort", esfuerzo]);
            }
            let modo = match p.permisos {
                Permisos::Lectura => "plan",
                Permisos::Edicion => "acceptEdits",
                Permisos::Total => "bypassPermissions",
            };
            add(&["--permission-mode", modo]);
            if let Some(id) = &p.reanudar {
                add(&["--resume", id]);
            }
            if let Some(s) = &p.sistema {
                add(&["--append-system-prompt", s]);
            }
            (a, Some(p.mensaje.clone()))
        }
        Backend::Codex => {
            let sandbox = match p.permisos {
                Permisos::Lectura => "read-only",
                Permisos::Edicion => "workspace-write",
                Permisos::Total => "danger-full-access",
            };
            add(&["exec"]);
            if let Some(id) = &p.reanudar {
                add(&["resume", id]);
            }
            add(&["--json", "--skip-git-repo-check"]);
            // `exec resume` no acepta `-s`; la clave de configuración vale en ambos casos.
            a.push("-c".into());
            a.push(format!("sandbox_mode=\"{sandbox}\""));
            if !modelo.is_empty() {
                a.push("-m".into());
                a.push(modelo.into());
            }
            if !esfuerzo.is_empty() {
                a.push("-c".into());
                a.push(format!("model_reasoning_effort=\"{esfuerzo}\""));
            }
            a.push("-".into());
            (a, Some(p.mensaje.clone()))
        }
        Backend::Antigravity => {
            add(&["-p", &p.mensaje, "--output-format", "stream-json", "--print-timeout", "0"]);
            if !modelo.is_empty() {
                add(&["--model", modelo]);
            }
            if !esfuerzo.is_empty() {
                add(&["--effort", esfuerzo]);
            }
            match p.permisos {
                Permisos::Lectura => add(&["--mode", "plan"]),
                Permisos::Edicion => add(&["--mode", "accept-edits"]),
                Permisos::Total => add(&["--dangerously-skip-permissions"]),
            }
            if let Some(id) = &p.reanudar {
                add(&["--conversation", id]);
            }
            (a, None)
        }
    }
}

pub enum Mensaje {
    Evento(Evento),
    /// Línea de stderr (se guarda la cola para explicar fallos).
    Stderr(String),
    /// El proceso terminó (código de salida).
    Salida(Option<i32>),
}

/// Un turno en marcha.
pub struct Ejecucion {
    pub rx: Receiver<Mensaje>,
    pub pid: u32,
    pub inicio: Instant,
}

pub fn matar_arbol(pid: u32) {
    #[cfg(windows)]
    {
        let mut c = Command::new("taskkill");
        c.args(["/T", "/F", "/PID", &pid.to_string()]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        use std::os::windows::process::CommandExt;
        c.creation_flags(CREATE_NO_WINDOW);
        let _ = c.status();
    }
    #[cfg(not(windows))]
    {
        // El hijo es líder de su propio grupo (`process_group(0)`): se mata el grupo entero.
        for señal in ["-TERM", "-KILL"] {
            let _ = Command::new("kill").args([señal, "--", &format!("-{pid}")]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status();
            std::thread::sleep(Duration::from_millis(300));
        }
    }
}

/// Lanza la CLI en `cwd` y lee su salida en hilos; la interfaz recibe eventos por el canal.
pub fn lanzar(b: Backend, exe: &Path, cwd: &Path, p: &Peticion, repintar: Option<egui::Context>) -> Result<Ejecucion, String> {
    let (args, stdin) = argumentos(b, p);
    let mut c = orden(exe);
    c.args(&args).current_dir(cwd).stdout(Stdio::piped()).stderr(Stdio::piped());
    c.stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() });
    // Que ninguna CLI se quede esperando a un terminal o abra un paginador.
    c.env("CI", "1").env("NO_COLOR", "1").env("TERM", "dumb").env("GIT_TERMINAL_PROMPT", "0");
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        c.process_group(0);
    }
    let mut hijo = c.spawn().map_err(|e| format!("no se pudo lanzar {}: {e}", exe.display()))?;
    if let Some(texto) = stdin
        && let Some(mut entrada) = hijo.stdin.take()
    {
        // En un hilo: un mensaje largo no debe bloquear la interfaz; al soltarlo se cierra stdin.
        std::thread::spawn(move || {
            let _ = entrada.write_all(texto.as_bytes());
        });
    }
    let pid = hijo.id();
    let (tx, rx) = channel();
    let aviso = move |tx: &Sender<Mensaje>, m: Mensaje, ctx: &Option<egui::Context>| {
        let _ = tx.send(m);
        if let Some(c) = ctx {
            c.request_repaint();
        }
    };
    let err = hijo.stderr.take();
    let (tx2, ctx2) = (tx.clone(), repintar.clone());
    let hilo_err = std::thread::spawn(move || {
        if let Some(e) = err {
            for l in BufReader::new(e).lines().map_while(Result::ok) {
                aviso(&tx2, Mensaje::Stderr(l), &ctx2);
            }
        }
    });
    let out = hijo.stdout.take();
    std::thread::spawn(move || {
        let mut interprete = Interprete::new(b);
        if let Some(o) = out {
            for l in BufReader::new(o).lines().map_while(Result::ok) {
                for ev in interprete.linea(&l) {
                    aviso(&tx, Mensaje::Evento(ev), &repintar);
                }
            }
        }
        let _ = hilo_err.join();
        let codigo = hijo.wait().ok().and_then(|s| s.code());
        aviso(&tx, Mensaje::Salida(codigo), &repintar);
    });
    Ok(Ejecucion { rx, pid, inicio: Instant::now() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordenes_por_cli() {
        let p = Peticion { mensaje: "hola".into(), modelo: "haiku".into(), esfuerzo: "low".into(), permisos: Permisos::Edicion, reanudar: Some("abc".into()), sistema: Some("primer".into()) };
        let (a, stdin) = argumentos(Backend::Claude, &p);
        assert_eq!(stdin.as_deref(), Some("hola"));
        for x in ["--model haiku", "--effort low", "--permission-mode acceptEdits", "--resume abc", "--append-system-prompt primer"] {
            assert!(a.join(" ").contains(x), "{x}");
        }
        let p = Peticion { permisos: Permisos::Lectura, sistema: None, modelo: String::new(), ..p };
        let (a, _) = argumentos(Backend::Codex, &p);
        assert_eq!(&a[..3], ["exec", "resume", "abc"]);
        assert!(a.contains(&"sandbox_mode=\"read-only\"".to_string()));
        assert!(!a.contains(&"-s".to_string()) && !a.contains(&"-m".to_string()));
        assert_eq!(a.last().map(String::as_str), Some("-"));
        let (a, stdin) = argumentos(Backend::Antigravity, &Peticion { permisos: Permisos::Total, ..p });
        assert!(stdin.is_none());
        assert_eq!(&a[..2], ["-p", "hola"]);
        assert!(a.contains(&"--dangerously-skip-permissions".to_string()));
        assert!(a.join(" ").contains("--conversation abc"));
    }

    #[cfg(unix)]
    #[test]
    fn parar_mata_el_grupo_entero() {
        use std::os::unix::process::CommandExt;
        let mut c = Command::new("sh");
        c.args(["-c", "sleep 30 & sleep 30"]).stdin(Stdio::null()).process_group(0);
        let mut h = c.spawn().unwrap();
        let pid = h.id();
        std::thread::sleep(Duration::from_millis(200));
        let inicio = Instant::now();
        matar_arbol(pid);
        h.wait().unwrap();
        assert!(inicio.elapsed() < Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(200));
        // Ningún proceso queda en el grupo (incluido el `sleep` en segundo plano).
        let vivos = Command::new("pgrep").args(["-g", &pid.to_string()]).output().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
        assert!(vivos.is_empty(), "quedan procesos: {vivos}");
    }

    #[test]
    fn listas_de_modelos() {
        let j = r#"{"models":[{"slug":"gpt-6-sol","display_name":"GPT-6 Sol","visibility":"list","context_window":272000,"supported_reasoning_levels":[{"effort":"low","description":"x"},{"effort":"high","description":"y"}]},{"slug":"oculto","visibility":"hide"}]}"#;
        let m = modelos_codex(j);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].ventana, Some(272000));
        assert_eq!(m[0].esfuerzos, vec!["low", "high"]);
        let a = modelos_agy("Fetching available models...\ngemini-3.8-flash-high\tGemini 3.8 Flash (High)\nclaude-sonnet-4-6\tClaude Sonnet 4.6 (Thinking)\n");
        assert_eq!(a.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["gemini-3.8-flash-high", "claude-sonnet-4-6"]);
        assert_eq!(Backend::Antigravity.ventana_por_defecto("gemini-3.8-flash-low"), 1_000_000);
    }
}
