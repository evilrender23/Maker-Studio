//! Modo agente: conversa con Claude Code, Codex o Antigravity en modo sin cabeza sobre el
//! proyecto abierto. Cada turno es un proceso en segundo plano cuya salida JSONL llega por un
//! canal; al terminar se recargan la escena, el script abierto y la biblioteca de scripts.

pub mod cli;
pub mod contexto;
pub mod eventos;
pub mod panel;
pub mod sesion;

use crate::editor::Editor;
use crate::prefs::Preferencias;
use cli::{Backend, Catalogo, Ejecucion, Medida, Peticion};
use contexto::Chip;
use eventos::Evento;
use maker_engine::log;
use sesion::{Mensaje, Sesion};
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

const PEDIR_RESUMEN: &str = "Resume esta conversación para continuarla en una conversación nueva: objetivo, decisiones, archivos creados o cambiados, estado actual y tareas pendientes. Responde solo con el resumen, sin usar herramientas.";
const MAX_SALIDA: usize = 20_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Clase {
    Normal,
    /// `/compact` nativo de Claude.
    Compactar,
    /// Pedir un resumen para sembrar un hilo nuevo (Codex, Antigravity).
    Resumir,
}

/// Archivos del proyecto con su fecha y tamaño, para saber qué tocó el agente.
type Foto = HashMap<PathBuf, (Option<SystemTime>, u64)>;

fn foto(raiz: &Path) -> Foto {
    let mut f = Foto::new();
    maker_engine::project::walk(raiz, &mut |p| {
        if let Ok(m) = p.metadata() {
            f.insert(p.to_path_buf(), (m.modified().ok(), m.len()));
        }
    });
    f
}

fn cambiados(antes: &Foto, despues: &Foto) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = despues.iter().filter(|(p, m)| antes.get(*p) != Some(m)).map(|(p, _)| p.clone()).collect();
    v.extend(antes.keys().filter(|p| !despues.contains_key(*p)).cloned());
    v.sort();
    v
}

pub struct Turno {
    pub ej: Ejecucion,
    pub clase: Clase,
    antes: Foto,
    herramientas: usize,
    resumen: String,
    stderr: VecDeque<String>,
    fin: Option<bool>,
    pub parado: bool,
    /// La guía va en este turno: se marca como enviada cuando la CLI abre la conversación.
    guia: bool,
    semilla: Option<String>,
    sesion: String,
}

pub struct Agente {
    pub raiz: PathBuf,
    pub sesiones: Vec<Sesion>,
    pub actual: usize,
    pub turno: Option<Turno>,
    pub entrada: String,
    pub chips: Vec<Chip>,
    pub renombrando: Option<String>,
    pub catalogo: Catalogo,
    ctx: egui::Context,
    /// Último turno terminado (para la automatización).
    pub turnos_terminados: u32,
    /// Compactar en cuanto termine el turno en curso.
    compactar_luego: bool,
    /// Medición del contexto real en curso: (para qué se pidió, respuesta).
    medicion: Option<(String, std::sync::mpsc::Receiver<Option<Medida>>)>,
    /// Qué se midió por última vez (CLI, modelo, conversación, turnos): al cambiar, se vuelve a medir.
    medido: String,
}

impl Agente {
    pub fn new(raiz: &Path, prefs: &Preferencias, catalogo: Catalogo, ctx: &egui::Context) -> Agente {
        let mut a = Agente {
            raiz: raiz.to_path_buf(),
            sesiones: sesion::cargar_todas(raiz),
            actual: 0,
            turno: None,
            entrada: String::new(),
            chips: vec![Chip::Escena],
            renombrando: None,
            catalogo,
            ctx: ctx.clone(),
            turnos_terminados: 0,
            compactar_luego: false,
            medicion: None,
            medido: String::new(),
        };
        if a.sesiones.is_empty() {
            a.nueva_sesion(prefs);
        }
        a
    }

    pub fn sesion(&self) -> &Sesion {
        &self.sesiones[self.actual]
    }

    pub fn sesion_mut(&mut self) -> &mut Sesion {
        &mut self.sesiones[self.actual]
    }

    pub fn ocupado(&self) -> bool {
        self.turno.is_some()
    }

    /// Midiendo el contexto real (para que las capturas automáticas esperen).
    pub fn midiendo(&self) -> bool {
        self.medicion.is_some()
    }

    pub fn nueva_sesion(&mut self, prefs: &Preferencias) {
        if self.ocupado() {
            return;
        }
        // Una sesión vacía ya sirve de «nueva».
        if let Some(i) = self.sesiones.iter().position(|s| s.mensajes.is_empty()) {
            let s = &mut self.sesiones[i];
            let a = prefs.cli(prefs.cli);
            (s.backend, s.modelo, s.esfuerzo, s.permisos) = (prefs.cli, a.modelo.clone(), a.esfuerzo.clone(), prefs.permisos);
            self.actual = i;
            return;
        }
        let a = prefs.cli(prefs.cli);
        self.sesiones.insert(0, Sesion::nueva(prefs.cli, &a.modelo, &a.esfuerzo, prefs.permisos));
        self.actual = 0;
    }

    pub fn borrar_sesion(&mut self, prefs: &Preferencias) {
        if self.ocupado() {
            return;
        }
        let s = self.sesiones.remove(self.actual);
        sesion::borrar(&self.raiz, &s.id);
        self.actual = 0;
        if self.sesiones.is_empty() {
            self.nueva_sesion(prefs);
        }
    }

    pub fn guardar(&self) {
        let s = self.sesion();
        if s.mensajes.is_empty() {
            return;
        }
        if let Err(e) = sesion::guardar(&self.raiz, s) {
            log::error("agente", e);
        }
    }

    /// Cambia la CLI de la sesión (solo antes de abrir la conversación nativa).
    pub fn cambiar_backend(&mut self, b: Backend, prefs: &Preferencias) {
        let s = self.sesion_mut();
        if s.id_nativo.is_some() || s.backend == b {
            return;
        }
        let a = prefs.cli(b);
        (s.backend, s.modelo, s.esfuerzo) = (b, a.modelo.clone(), a.esfuerzo.clone());
        s.uso.ventana = 0;
        s.contexto_enviado = false;
    }

    /// Ventana real: la medida para esta conversación o la que la CLI informa del modelo. `None` si
    /// la CLI no la da (Antigravity): no se inventa.
    pub fn ventana(&self) -> Option<u64> {
        let s = self.sesion();
        if s.uso.ventana > 0 { Some(s.uso.ventana) } else { self.catalogo.ventana(s.backend, &s.modelo) }
    }

    /// Pide a la CLI el contexto real cuando cambia la CLI, el modelo, la conversación o acaba un
    /// turno. En segundo plano; se aplica en `medir_poll`.
    fn medir_si_cambio(&mut self, prefs: &Preferencias) {
        if self.turno.is_some() || self.medicion.is_some() {
            return;
        }
        let s = self.sesion();
        let clave = format!("{:?}|{}|{:?}|{}|{}", s.backend, s.modelo, s.id_nativo, s.id, self.turnos_terminados);
        if clave == self.medido {
            return;
        }
        let modelo_cambio = !self.medido.starts_with(&format!("{:?}|{}|", s.backend, s.modelo));
        let (b, modelo, id) = (s.backend, s.modelo.clone(), s.id_nativo.clone());
        self.medido = clave.clone();
        if modelo_cambio {
            // Lo medido era de otra CLI u otro modelo.
            let s = self.sesion_mut();
            s.uso.ventana = 0;
            if s.id_nativo.is_none() {
                s.uso.contexto = 0;
            }
        }
        let Ok(exe) = self.exe(prefs, b) else { return };
        let (tx, rx) = std::sync::mpsc::channel();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(cli::medir(b, &exe, &modelo, id.as_deref()));
            ctx.request_repaint();
        });
        self.medicion = Some((clave, rx));
    }

    fn medir_poll(&mut self) {
        let Some((clave, rx)) = &self.medicion else { return };
        let Ok(m) = rx.try_recv() else { return };
        let clave = clave.clone();
        self.medicion = None;
        let Some(m) = m else { return };
        // Si entretanto cambió la CLI, el modelo o la conversación, la medida ya no vale.
        if clave != self.medido {
            return;
        }
        {
            let s = self.sesion_mut();
            if let Some(v) = m.ventana {
                s.uso.ventana = v;
            }
            if let Some(u) = m.usado {
                s.uso.contexto = u;
            }
        }
    }

    fn exe(&self, prefs: &Preferencias, b: Backend) -> Result<PathBuf, String> {
        self.catalogo.info(b).ruta.or_else(|| cli::resolver(b, &prefs.cli(b).ruta)).ok_or_else(|| format!("no se encuentra {} ({}): instálalo o indica su ruta en Preferencias", b.nombre(), b.ejecutable()))
    }

    /// Envía un mensaje del usuario con los chips de contexto activos.
    pub fn enviar(&mut self, ed: &Editor, prefs: &Preferencias, texto: &str) {
        let texto = texto.trim().to_string();
        if texto.is_empty() || self.ocupado() {
            return;
        }
        let chips: Vec<Chip> = self.chips.iter().copied().filter(|c| c.disponible(ed).is_ok()).collect();
        let s = self.sesion().clone();
        let exe = match self.exe(prefs, s.backend) {
            Ok(e) => e,
            Err(e) => {
                self.sesion_mut().mensajes.push(Mensaje::Error { texto: e });
                return;
            }
        };
        // La guía va como instrucciones de sistema en Claude (salvo los shims .cmd de Windows,
        // donde un argumento de varias líneas no pasa por cmd.exe); en el resto, delante del mensaje.
        let mut previo = Vec::new();
        let mut sistema = None;
        let guia = !s.contexto_enviado;
        if guia {
            let g = contexto::guia(ed);
            let es_cmd = exe.extension().is_some_and(|e| e.eq_ignore_ascii_case("cmd") || e.eq_ignore_ascii_case("bat"));
            if s.backend == Backend::Claude && !es_cmd {
                sistema = Some(g);
            } else {
                previo.push(format!("<guia_maker_studio>\n{g}\n</guia_maker_studio>"));
            }
        }
        if let Some(sem) = &s.semilla {
            previo.push(format!("<resumen_de_la_conversacion_anterior>\n{sem}\n</resumen_de_la_conversacion_anterior>"));
        }
        let previo = (!previo.is_empty()).then(|| previo.join("\n\n"));
        let mensaje = contexto::componer(ed, &chips, &texto, previo.as_deref());
        let p = Peticion { mensaje, modelo: s.modelo.clone(), esfuerzo: s.esfuerzo.clone(), permisos: s.permisos, reanudar: s.id_nativo.clone(), sistema };
        {
            let s = self.sesion_mut();
            s.mensajes.push(Mensaje::Usuario { texto: texto.clone(), contexto: chips.iter().map(|c| c.nombre().to_string()).collect() });
            s.titular(&texto);
            s.actualizada = sesion::ahora();
        }
        self.lanzar(ed, &exe, p, Clase::Normal, guia);
        self.guardar();
    }

    fn lanzar(&mut self, ed: &Editor, exe: &Path, p: Peticion, clase: Clase, guia: bool) {
        let antes = foto(&ed.project.root);
        let b = self.sesion().backend;
        match cli::lanzar(b, exe, &ed.project.root, &p, Some(self.ctx.clone())) {
            Ok(ej) => {
                log::info("agente", format!("{} · {}{}", b.nombre(), if p.modelo.is_empty() { "modelo predeterminado" } else { &p.modelo }, if p.reanudar.is_some() { " · reanudando" } else { "" }));
                let semilla = if clase == Clase::Normal { self.sesion_mut().semilla.take() } else { None };
                self.turno = Some(Turno { ej, clase, antes, herramientas: 0, resumen: String::new(), stderr: VecDeque::new(), fin: None, parado: false, guia, semilla, sesion: self.sesion().id.clone() });
            }
            Err(e) => {
                log::error("agente", e.clone());
                self.sesion_mut().mensajes.push(Mensaje::Error { texto: e });
            }
        }
    }

    /// Compacta la conversación: `/compact` en Claude; en las demás, resumen + hilo nuevo.
    pub fn compactar(&mut self, ed: &Editor, prefs: &Preferencias) {
        if self.ocupado() {
            return;
        }
        let s = self.sesion().clone();
        let Some(id) = s.id_nativo.clone() else { return };
        let exe = match self.exe(prefs, s.backend) {
            Ok(e) => e,
            Err(e) => {
                self.sesion_mut().mensajes.push(Mensaje::Error { texto: e });
                return;
            }
        };
        let (mensaje, clase) = match s.backend {
            Backend::Claude => ("/compact".to_string(), Clase::Compactar),
            _ => (PEDIR_RESUMEN.to_string(), Clase::Resumir),
        };
        self.sesion_mut().mensajes.push(Mensaje::Nota { texto: "Compactando la conversación…".into() });
        let p = Peticion { mensaje, modelo: s.modelo.clone(), esfuerzo: s.esfuerzo.clone(), permisos: if clase == Clase::Compactar { s.permisos } else { cli::Permisos::Lectura }, reanudar: Some(id), sistema: None };
        self.lanzar(ed, &exe, p, clase, false);
    }

    pub fn parar(&mut self) {
        if let Some(t) = &mut self.turno
            && !t.parado
        {
            t.parado = true;
            let pid = t.ej.pid;
            std::thread::spawn(move || cli::matar_arbol(pid));
        }
    }

    /// Procesa los eventos pendientes; al terminar un turno integra los cambios en el editor.
    pub fn actualizar(&mut self, ed: &mut Editor, prefs: &Preferencias) {
        self.medir_poll();
        self.medir_si_cambio(prefs);
        let Some(t) = &mut self.turno else { return };
        let Some(idx) = self.sesiones.iter().position(|s| s.id == t.sesion) else {
            self.turno = None;
            return;
        };
        let s = &mut self.sesiones[idx];
        let mut salida = None;
        while let Ok(m) = t.ej.rx.try_recv() {
            match m {
                cli::Mensaje::Evento(ev) => aplicar(s, t, ev),
                cli::Mensaje::Stderr(l) => {
                    if t.stderr.len() >= 40 {
                        t.stderr.pop_front();
                    }
                    t.stderr.push_back(l);
                }
                cli::Mensaje::Salida(c) => salida = Some(c),
            }
        }
        let Some(codigo) = salida else { return };
        let t = self.turno.take().unwrap();
        let s = &mut self.sesiones[idx];
        // Herramientas que quedaron abiertas (turno detenido o roto).
        for m in s.mensajes.iter_mut() {
            if let Mensaje::Herramienta { terminada, .. } = m {
                *terminada = true;
            }
        }
        let ok = t.fin == Some(true) && !t.parado;
        if t.parado {
            s.mensajes.push(Mensaje::Nota { texto: "Detenido".into() });
        } else if t.fin.is_none() {
            let cola: Vec<&str> = t.stderr.iter().map(String::as_str).filter(|l| !l.trim().is_empty() && !l.starts_with("Reading additional input")).collect();
            let detalle = cola[cola.len().saturating_sub(8)..].join("\n");
            s.mensajes.push(Mensaje::Error { texto: format!("{} terminó sin respuesta (código {}){}", s.backend.nombre(), codigo.map(|c| c.to_string()).unwrap_or("?".into()), if detalle.is_empty() { String::new() } else { format!(":\n{detalle}") }) });
        }
        if t.semilla.is_some() && s.id_nativo.is_none() {
            // No llegó a abrirse la conversación nueva: el resumen se usará la próxima vez.
            s.semilla = t.semilla.clone();
        }
        match t.clase {
            Clase::Compactar if ok => {
                s.uso.contexto = 0;
                // Tras compactar, Claude vuelve a registrar las instrucciones de sistema.
                s.contexto_enviado = false;
                s.mensajes.push(Mensaje::Nota { texto: "Conversación compactada".into() });
            }
            Clase::Resumir if ok && !t.resumen.trim().is_empty() => {
                s.semilla = Some(t.resumen.trim().to_string());
                s.id_nativo = None;
                s.contexto_enviado = false;
                s.uso.contexto = 0;
                s.mensajes.push(Mensaje::Nota { texto: "Conversación compactada: la próxima petición abre un hilo nuevo con el resumen".into() });
            }
            Clase::Compactar | Clase::Resumir => s.mensajes.push(Mensaje::Error { texto: "No se pudo compactar".into() }),
            Clase::Normal => {}
        }
        s.actualizada = sesion::ahora();
        let (contexto, ventana) = (s.uso.contexto, if s.uso.ventana > 0 { s.uso.ventana } else { self.catalogo.ventana(s.backend, &s.modelo).unwrap_or(0) });
        let secs = t.ej.inicio.elapsed().as_secs_f32();
        let herramientas = t.herramientas;
        self.actual = idx;
        self.guardar();
        self.turnos_terminados += 1;
        let resumen = self.integrar(ed, prefs, &t.antes);
        log::info("agente", format!("turno terminado en {secs:.0} s · {herramientas} herramienta(s){resumen}"));
        if !ok && !t.parado {
            log::error("agente", "el turno terminó con error (mira el panel del agente)");
        }
        let lleno = prefs.auto_compactar > 0 && ventana > 0 && contexto * 100 >= ventana * prefs.auto_compactar as u64;
        if (self.compactar_luego || (t.clase == Clase::Normal && ok && lleno)) && self.sesion().id_nativo.is_some() {
            self.compactar_luego = false;
            if lleno {
                log::info("agente", format!("contexto al {} %: compactando", contexto * 100 / ventana));
            }
            self.compactar(ed, prefs);
        }
    }

    pub fn compactar_al_terminar(&mut self) {
        self.compactar_luego = true;
    }

    /// Recarga lo que el agente cambió en disco. Devuelve un resumen para la consola.
    fn integrar(&mut self, ed: &mut Editor, prefs: &Preferencias, antes: &Foto) -> String {
        let despues = foto(&ed.project.root);
        let cambios = cambiados(antes, &despues);
        if cambios.is_empty() {
            return String::new();
        }
        let rel: Vec<String> = cambios.iter().map(|p| ed.project.relative(p)).collect();
        let mut notas = Vec::new();
        // Escena abierta.
        if cambios.contains(&ed.scene_path) {
            if ed.is_playing() || ed.dirty {
                let t = "El agente cambió la escena abierta, pero el editor tiene cambios sin guardar: no se recarga (guarda o vuelve a abrirla)".to_string();
                log::warn("agente", t.clone());
                notas.push(t);
            } else if ed.scene_path.exists() {
                let sel = ed.selection;
                let r = ed.project.relative(&ed.scene_path);
                ed.open_scene(&r);
                ed.selection = sel.filter(|s| ed.scene.contains(*s));
                notas.push(format!("Escena {r} recargada"));
            }
        }
        // Script abierto en el editor de código.
        let mut script_sucio_cambiado = false;
        if let Some(sc) = &mut ed.script
            && cambios.contains(&sc.path)
        {
            if sc.text == sc.saved {
                if let Ok(t) = std::fs::read_to_string(&sc.path) {
                    sc.text = t.clone();
                    sc.saved = t;
                    notas.push(format!("Script {} recargado", sc.path.file_name().unwrap_or_default().to_string_lossy()));
                }
            } else {
                script_sucio_cambiado = true;
                let t = format!("El agente cambió {}, que tiene cambios sin guardar en el editor: no se recarga", sc.path.display());
                log::warn("agente", t.clone());
                notas.push(t);
            }
        }
        // Scripts: compilar y recargar en caliente.
        let src = ed.project.scripts_dir().join("src");
        let scripts = cambios.iter().any(|p| p.starts_with(&src));
        // Con la compilación automática activada, ya compila el editor al ver el cambio.
        if scripts && prefs.recompilar && !prefs.compilar_auto {
            if script_sucio_cambiado {
                log::warn("agente", "no se recompila: guardar el script abierto pisaría los cambios del agente");
            } else {
                ed.build_scripts();
                notas.push("Recompilando scripts".into());
            }
        }
        if !notas.is_empty() {
            let s = self.sesion_mut();
            s.mensajes.push(Mensaje::Nota { texto: notas.join(" · ") });
            self.guardar();
        }
        format!(" · cambios: {}", rel.join(", "))
    }
}

/// Vuelca la sesión actual a stdout (automatización y pruebas).
pub fn imprimir_sesion(g: &Agente) {
    let s = g.sesion();
    println!("agente: sesión {} «{}» · {} · modelo {} · esfuerzo {} · {:?} · id nativo {}", s.id, s.titulo, s.backend.nombre(), if s.modelo.is_empty() { "-" } else { &s.modelo }, if s.esfuerzo.is_empty() { "-" } else { &s.esfuerzo }, s.permisos, s.id_nativo.as_deref().unwrap_or("-"));
    let corto = |t: &str| -> String {
        let t = t.replace('\n', " ⏎ ");
        if t.chars().count() > 300 { format!("{}…", t.chars().take(300).collect::<String>()) } else { t }
    };
    for m in &s.mensajes {
        match m {
            Mensaje::Usuario { texto, contexto } => println!("  [usuario] {} {:?}", corto(texto), contexto),
            Mensaje::Asistente { texto } => println!("  [asistente] {}", corto(texto)),
            Mensaje::Herramienta { clase, titulo, salida, error, .. } => println!("  [{clase:?}{}] {} → {}", if *error { " ERROR" } else { "" }, corto(titulo), corto(salida)),
            Mensaje::Error { texto } => println!("  [error] {}", corto(texto)),
            Mensaje::Nota { texto } => println!("  [nota] {}", corto(texto)),
        }
    }
    let ventana = g.ventana().map(|v| v.to_string()).unwrap_or_else(|| "?".into());
    println!("agente: contexto {} / {ventana} · entrada {} · salida {} · coste {:.5} $", s.uso.contexto, s.uso.entrada, s.uso.salida, s.uso.coste);
}

fn recortar_salida(mut s: String) -> String {
    if s.len() > MAX_SALIDA {
        let mut corte = MAX_SALIDA;
        while !s.is_char_boundary(corte) {
            corte -= 1;
        }
        s.truncate(corte);
        s.push_str("\n…");
    }
    s
}

/// Aplica un evento a la sesión.
fn aplicar(s: &mut Sesion, t: &mut Turno, ev: Evento) {
    match ev {
        Evento::Sesion(id) => {
            if t.clase != Clase::Resumir {
                s.id_nativo = Some(id);
                if t.guia {
                    s.contexto_enviado = true;
                }
                t.semilla = None;
            }
        }
        Evento::Texto { texto, delta } => {
            if t.clase == Clase::Resumir {
                t.resumen.push_str(&texto);
                if !delta {
                    t.resumen.push('\n');
                }
                return;
            }
            match s.mensajes.last_mut() {
                Some(Mensaje::Asistente { texto: prev }) if delta => prev.push_str(&texto),
                Some(Mensaje::Asistente { texto: prev }) => {
                    prev.push_str("\n\n");
                    prev.push_str(&texto);
                }
                _ => s.mensajes.push(Mensaje::Asistente { texto }),
            }
        }
        Evento::HerramientaInicio { id, tipo, titulo } => {
            t.herramientas += 1;
            s.mensajes.push(Mensaje::Herramienta { id, clase: tipo, titulo, salida: String::new(), error: false, terminada: false });
        }
        Evento::HerramientaFin { id, salida, error } => {
            let hallada = s.mensajes.iter_mut().rev().find_map(|m| match m {
                Mensaje::Herramienta { id: i, salida: so, error: e, terminada, .. } if *i == id => Some((so, e, terminada)),
                _ => None,
            });
            match hallada {
                Some((so, e, term)) => {
                    *so = recortar_salida(salida);
                    *e = error;
                    *term = true;
                }
                None => s.mensajes.push(Mensaje::Herramienta { id, clase: eventos::TipoHerramienta::Otra, titulo: "herramienta".into(), salida: recortar_salida(salida), error, terminada: true }),
            }
        }
        Evento::Uso(u) => {
            if let Some(c) = u.contexto {
                s.uso.contexto = c;
            }
            if let Some(v) = u.ventana {
                s.uso.ventana = v;
            }
            s.uso.entrada += u.entrada.unwrap_or(0);
            s.uso.salida += u.salida.unwrap_or(0);
            s.uso.coste += u.coste.unwrap_or(0.0);
        }
        Evento::Error(e) => s.mensajes.push(Mensaje::Error { texto: e }),
        Evento::Compactado => {}
        Evento::Fin { ok } => t.fin = Some(ok && t.fin != Some(false)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detecta_archivos_cambiados() {
        let dir = std::env::temp_dir().join(format!("maker-foto-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("scripts/src")).unwrap();
        std::fs::write(dir.join("scripts/src/lib.rs"), "a").unwrap();
        std::fs::write(dir.join("borrar.txt"), "x").unwrap();
        let a = foto(&dir);
        std::fs::write(dir.join("scripts/src/lib.rs"), "abc").unwrap();
        std::fs::remove_file(dir.join("borrar.txt")).unwrap();
        std::fs::write(dir.join("nuevo.txt"), "n").unwrap();
        let c = cambiados(&a, &foto(&dir));
        assert_eq!(c, vec![dir.join("borrar.txt"), dir.join("nuevo.txt"), dir.join("scripts/src/lib.rs")]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
