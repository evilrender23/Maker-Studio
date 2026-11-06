//! Traducción de la salida JSONL de cada CLI a eventos comunes.
//!
//! Formatos verificados con grabaciones reales (en `fixtures/`):
//! - Claude Code: `-p --output-format stream-json --verbose`.
//! - Codex: `exec --json`.
//! - Antigravity: `-p … --output-format stream-json`.

use super::cli::Backend;
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TipoHerramienta {
    Comando,
    Lectura,
    Edicion,
    Busqueda,
    Otra,
}

/// Tokens y coste; los campos ausentes no se tocan al acumular.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Uso {
    /// Tokens de contexto ocupados tras la última llamada al modelo.
    pub contexto: Option<u64>,
    /// Tamaño de la ventana de contexto que informa la CLI.
    pub ventana: Option<u64>,
    pub entrada: Option<u64>,
    pub salida: Option<u64>,
    pub coste: Option<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Evento {
    /// Id nativo de la conversación (para reanudar).
    Sesion(String),
    /// Texto del asistente. `delta`: continúa el mensaje anterior.
    Texto { texto: String, delta: bool },
    HerramientaInicio { id: String, tipo: TipoHerramienta, titulo: String },
    HerramientaFin { id: String, salida: String, error: bool },
    Uso(Uso),
    Error(String),
    /// La CLI compactó la conversación (Claude `/compact`).
    Compactado,
    /// Fin del turno.
    Fin { ok: bool },
}

fn s(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(Value::as_str).map(String::from)
}

fn n(v: &Value, k: &str) -> u64 {
    v.get(k).and_then(Value::as_u64).unwrap_or(0)
}

fn recortar(t: &str, max: usize) -> String {
    let t = t.trim();
    let una = t.lines().next().unwrap_or("");
    let mut r: String = una.chars().take(max).collect();
    if r.len() < una.len() || t.lines().nth(1).is_some() {
        r.push('…');
    }
    r
}

/// Clasifica una herramienta por su nombre y saca un título corto de sus parámetros.
fn herramienta(nombre: &str, entrada: &Value) -> (TipoHerramienta, String) {
    let campo = |ks: &[&str]| ks.iter().find_map(|k| s(entrada, k)).unwrap_or_default();
    let l = nombre.to_ascii_lowercase();
    if ["bash", "run_command", "shell", "command"].iter().any(|k| l == *k) {
        return (TipoHerramienta::Comando, recortar(&campo(&["command", "CommandLine", "cmd"]), 160));
    }
    if ["read", "read_file", "view_file", "notebookread"].contains(&l.as_str()) {
        return (TipoHerramienta::Lectura, campo(&["file_path", "Path", "AbsolutePath", "path"]));
    }
    if ["write", "edit", "multiedit", "notebookedit", "write_to_file", "replace_file_content", "multi_replace_file_content", "edit_file", "create_file"].contains(&l.as_str()) {
        return (TipoHerramienta::Edicion, campo(&["file_path", "TargetFile", "Path", "path", "notebook_path"]));
    }
    if ["glob", "grep", "grep_search", "find_by_name", "list_dir", "ls", "codebase_search", "websearch", "webfetch"].contains(&l.as_str()) {
        let t = campo(&["pattern", "Query", "query", "SearchPath", "DirectoryPath", "path", "url"]);
        return (TipoHerramienta::Busqueda, format!("{nombre} {}", recortar(&t, 120)).trim().to_string());
    }
    (TipoHerramienta::Otra, nombre.to_string())
}

/// Contenido de un `tool_result` de Claude: texto o lista de bloques.
fn texto_de_bloques(v: &Value) -> String {
    match v {
        Value::String(t) => t.clone(),
        Value::Array(a) => a.iter().filter_map(|b| b.get("text").and_then(Value::as_str)).collect::<Vec<_>>().join("\n"),
        Value::Null => String::new(),
        o => o.to_string(),
    }
}

/// Intérprete con estado de la salida de una CLI (una línea JSON por llamada).
#[derive(Default)]
pub struct Interprete {
    backend: Option<Backend>,
    /// Herramientas abiertas (Codex/agy emiten el inicio y el fin por separado).
    abiertas: Vec<String>,
}

impl Interprete {
    pub fn new(backend: Backend) -> Self {
        Interprete { backend: Some(backend), abiertas: Vec::new() }
    }

    pub fn linea(&mut self, linea: &str) -> Vec<Evento> {
        let linea = linea.trim();
        if !linea.starts_with('{') {
            return Vec::new();
        }
        let Ok(v) = serde_json::from_str::<Value>(linea) else { return Vec::new() };
        match self.backend {
            Some(Backend::Claude) => claude(&v),
            Some(Backend::Codex) => self.codex(&v),
            Some(Backend::Antigravity) => self.agy(&v),
            None => Vec::new(),
        }
    }

    fn inicio(&mut self, id: &str, tipo: TipoHerramienta, titulo: String, out: &mut Vec<Evento>) {
        if !self.abiertas.iter().any(|a| a == id) {
            self.abiertas.push(id.to_string());
            out.push(Evento::HerramientaInicio { id: id.to_string(), tipo, titulo });
        }
    }

    fn fin(&mut self, id: &str, salida: String, error: bool, out: &mut Vec<Evento>) {
        self.abiertas.retain(|a| a != id);
        out.push(Evento::HerramientaFin { id: id.to_string(), salida, error });
    }

    fn codex(&mut self, v: &Value) -> Vec<Evento> {
        let mut out = Vec::new();
        match v.get("type").and_then(Value::as_str).unwrap_or("") {
            "thread.started" => out.extend(s(v, "thread_id").map(Evento::Sesion)),
            "item.started" | "item.updated" | "item.completed" => {
                let completo = v["type"] == "item.completed";
                let it = &v["item"];
                let id = s(it, "id").unwrap_or_default();
                match it.get("type").and_then(Value::as_str).unwrap_or("") {
                    "agent_message" if completo => out.push(Evento::Texto { texto: s(it, "text").unwrap_or_default(), delta: false }),
                    "command_execution" => {
                        self.inicio(&id, TipoHerramienta::Comando, recortar(&limpiar_shell(&s(it, "command").unwrap_or_default()), 160), &mut out);
                        if completo {
                            let codigo = it.get("exit_code").and_then(Value::as_i64).unwrap_or(0);
                            self.fin(&id, s(it, "aggregated_output").unwrap_or_default(), codigo != 0 || it["status"] == "failed", &mut out);
                        }
                    }
                    "file_change" => {
                        let rutas: Vec<String> = it["changes"].as_array().map(|c| c.iter().filter_map(|x| s(x, "path")).collect()).unwrap_or_default();
                        self.inicio(&id, TipoHerramienta::Edicion, rutas.join(", "), &mut out);
                        if completo {
                            self.fin(&id, String::new(), it["status"] == "failed", &mut out);
                        }
                    }
                    "mcp_tool_call" => {
                        let t = format!("{} {}", s(it, "server").unwrap_or_default(), s(it, "tool").unwrap_or_default());
                        self.inicio(&id, TipoHerramienta::Otra, t.trim().to_string(), &mut out);
                        if completo {
                            self.fin(&id, String::new(), it["status"] == "failed", &mut out);
                        }
                    }
                    "web_search" => {
                        self.inicio(&id, TipoHerramienta::Busqueda, format!("web {}", s(it, "query").unwrap_or_default()), &mut out);
                        if completo {
                            self.fin(&id, String::new(), false, &mut out);
                        }
                    }
                    "error" if completo => out.push(Evento::Error(s(it, "message").unwrap_or_default())),
                    _ => {}
                }
            }
            "turn.completed" => {
                let u = &v["usage"];
                let entrada = n(u, "input_tokens");
                let salida = n(u, "output_tokens");
                // El contexto ocupado lo da el registro de Codex (`cli::medida_codex`), no esta suma.
                out.push(Evento::Uso(Uso { entrada: Some(entrada), salida: Some(salida), ..Default::default() }));
                out.push(Evento::Fin { ok: true });
            }
            "turn.failed" => {
                out.push(Evento::Error(v["error"].get("message").and_then(Value::as_str).unwrap_or("el turno falló").to_string()));
                out.push(Evento::Fin { ok: false });
            }
            "error" => out.push(Evento::Error(s(v, "message").unwrap_or_else(|| v.to_string()))),
            _ => {}
        }
        out
    }

    fn agy(&mut self, v: &Value) -> Vec<Evento> {
        let mut out = Vec::new();
        match v.get("event").and_then(Value::as_str).unwrap_or("") {
            "init" => out.extend(s(v, "conversation_id").map(Evento::Sesion)),
            "step_update" => {
                let st = &v["step_update"];
                let id = format!("paso-{}", n(st, "step_index"));
                let hecho = st["state"] == "DONE";
                match st.get("step_type").and_then(Value::as_str).unwrap_or("") {
                    "agent_response" => {
                        if let Some(t) = s(st, "text_delta")
                            && !t.is_empty()
                        {
                            out.push(Evento::Texto { texto: t, delta: true });
                        }
                        if let Some(u) = st.get("usage") {
                            let (e, sa) = (n(u, "input_tokens") + n(u, "cache_read_tokens"), n(u, "output_tokens"));
                            out.push(Evento::Uso(Uso { contexto: Some(e + sa), ..Default::default() }));
                        }
                    }
                    "tool" => {
                        let info = &st["tool_info"];
                        let nombre = s(st, "tool_name").or_else(|| s(info, "name")).unwrap_or_default();
                        let (tipo, titulo) = herramienta(&nombre, &info["parameters"]);
                        self.inicio(&id, tipo, titulo, &mut out);
                        if hecho {
                            let salida = s(info, "output").unwrap_or_default();
                            let error = salida.starts_with("Command exited with code") && !salida.starts_with("Command exited with code 0");
                            self.fin(&id, salida, error, &mut out);
                        }
                    }
                    _ => {}
                }
            }
            "result" => {
                let r = &v["result"];
                let u = &r["usage"];
                out.push(Evento::Uso(Uso { entrada: Some(n(u, "input_tokens")), salida: Some(n(u, "output_tokens")), ..Default::default() }));
                let ok = r["status"] == "SUCCESS";
                if !ok {
                    out.push(Evento::Error(s(r, "error").filter(|e| !e.is_empty()).unwrap_or_else(|| "Antigravity terminó con error".into())));
                }
                out.push(Evento::Fin { ok });
            }
            _ => {}
        }
        out
    }
}

/// Codex envuelve las órdenes en `/usr/bin/bash -lc '…'`: se muestra solo lo de dentro.
fn limpiar_shell(c: &str) -> String {
    for p in ["/usr/bin/bash -lc ", "/bin/bash -lc ", "bash -lc ", "/bin/sh -c ", "/usr/bin/zsh -lc "] {
        if let Some(r) = c.strip_prefix(p) {
            let r = r.trim();
            let sin = r.strip_prefix('\'').and_then(|x| x.strip_suffix('\'')).or_else(|| r.strip_prefix('"').and_then(|x| x.strip_suffix('"')));
            return sin.unwrap_or(r).to_string();
        }
    }
    c.to_string()
}

fn claude(v: &Value) -> Vec<Evento> {
    let mut out = Vec::new();
    match v.get("type").and_then(Value::as_str).unwrap_or("") {
        "system" => match v.get("subtype").and_then(Value::as_str).unwrap_or("") {
            "init" => out.extend(s(v, "session_id").map(Evento::Sesion)),
            "compact_boundary" => out.push(Evento::Compactado),
            _ => {}
        },
        "assistant" => {
            let m = &v["message"];
            for b in m["content"].as_array().into_iter().flatten() {
                match b.get("type").and_then(Value::as_str).unwrap_or("") {
                    "text" => {
                        let t = s(b, "text").unwrap_or_default();
                        if !t.trim().is_empty() {
                            out.push(Evento::Texto { texto: t, delta: false });
                        }
                    }
                    "tool_use" => {
                        let (tipo, titulo) = herramienta(&s(b, "name").unwrap_or_default(), &b["input"]);
                        out.push(Evento::HerramientaInicio { id: s(b, "id").unwrap_or_default(), tipo, titulo });
                    }
                    _ => {}
                }
            }
            if let Some(u) = m.get("usage") {
                let c = n(u, "input_tokens") + n(u, "cache_creation_input_tokens") + n(u, "cache_read_input_tokens");
                out.push(Evento::Uso(Uso { contexto: Some(c), ..Default::default() }));
            }
        }
        "user" => {
            for b in v["message"]["content"].as_array().into_iter().flatten() {
                if b["type"] == "tool_result" {
                    out.push(Evento::HerramientaFin {
                        id: s(b, "tool_use_id").unwrap_or_default(),
                        salida: texto_de_bloques(&b["content"]),
                        error: b["is_error"].as_bool().unwrap_or(false),
                    });
                }
            }
        }
        "result" => {
            let u = &v["usage"];
            let ventana = v["modelUsage"].as_object().and_then(|m| m.values().filter_map(|x| x["contextWindow"].as_u64()).max());
            out.push(Evento::Uso(Uso {
                contexto: None,
                ventana,
                entrada: Some(n(u, "input_tokens") + n(u, "cache_creation_input_tokens") + n(u, "cache_read_input_tokens")),
                salida: Some(n(u, "output_tokens")),
                coste: v["total_cost_usd"].as_f64(),
            }));
            let ok = !v["is_error"].as_bool().unwrap_or(false) && v["subtype"] == "success";
            if !ok {
                let e = s(v, "result").or_else(|| v.get("errors").map(|e| e.to_string())).unwrap_or_else(|| format!("Claude terminó con error ({})", s(v, "subtype").unwrap_or_default()));
                out.push(Evento::Error(e));
            }
            out.push(Evento::Fin { ok });
        }
        _ => {}
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn todo(b: Backend, texto: &str) -> Vec<Evento> {
        let mut i = Interprete::new(b);
        texto.lines().flat_map(|l| i.linea(l)).collect()
    }

    fn textos(ev: &[Evento]) -> String {
        ev.iter().filter_map(|e| if let Evento::Texto { texto, .. } = e { Some(texto.as_str()) } else { None }).collect()
    }

    #[test]
    fn claude_con_herramientas() {
        let ev = todo(Backend::Claude, include_str!("fixtures/claude.jsonl"));
        assert_eq!(ev[0], Evento::Sesion("f843f023-d444-49c9-bc73-da46a54095c2".into()));
        let inicios: Vec<_> = ev.iter().filter_map(|e| if let Evento::HerramientaInicio { tipo, titulo, .. } = e { Some((*tipo, titulo.clone())) } else { None }).collect();
        assert_eq!(inicios, vec![(TipoHerramienta::Lectura, "/proyecto/w2/a.txt".into()), (TipoHerramienta::Comando, "ls".into()), (TipoHerramienta::Edicion, "/proyecto/w2/b.txt".into())]);
        assert!(ev.contains(&Evento::HerramientaFin { id: "toolu_018RqPbhuKApcW4LxEE5BwuD".into(), salida: "a.txt".into(), error: false }));
        assert_eq!(textos(&ev), "hecho");
        let usos: Vec<_> = ev.iter().filter_map(|e| if let Evento::Uso(u) = e { Some(u.clone()) } else { None }).collect();
        assert_eq!(usos.iter().rev().find_map(|u| u.contexto), Some(2 + 350 + 30716));
        let fin = usos.last().unwrap();
        assert_eq!(fin.ventana, Some(1_000_000));
        assert!(fin.coste.unwrap() > 0.0049);
        assert_eq!(ev.last(), Some(&Evento::Fin { ok: true }));
    }

    #[test]
    fn codex_con_ordenes() {
        let ev = todo(Backend::Codex, include_str!("fixtures/codex.jsonl"));
        assert_eq!(ev[0], Evento::Sesion("01a1b15b-ca7b-7362-83ee-164cf8b71f6a".into()));
        assert!(ev.contains(&Evento::HerramientaInicio { id: "item_1".into(), tipo: TipoHerramienta::Comando, titulo: "cat a.txt".into() }));
        assert!(ev.contains(&Evento::HerramientaFin { id: "item_2".into(), salida: "a.txt\n".into(), error: false }));
        assert_eq!(ev.iter().filter(|e| matches!(e, Evento::HerramientaInicio { .. })).count(), 3);
        assert!(textos(&ev).ends_with("hecho"));
        assert!(ev.contains(&Evento::Uso(Uso { entrada: Some(29045), salida: Some(124), ..Default::default() })));
        assert_eq!(ev.last(), Some(&Evento::Fin { ok: true }));
    }

    #[test]
    fn agy_texto_y_resultado() {
        let ev = todo(Backend::Antigravity, include_str!("fixtures/agy.jsonl"));
        assert_eq!(ev[0], Evento::Sesion("f204220f-58d7-4183-8f18-2b410d19a5de".into()));
        assert_eq!(textos(&ev), "hola\n");
        assert!(ev.iter().any(|e| matches!(e, Evento::Uso(Uso { contexto: Some(7067), .. }))));
        assert_eq!(ev.last(), Some(&Evento::Fin { ok: true }));
    }

    #[test]
    fn agy_herramientas_e_interrupcion() {
        let ev = todo(Backend::Antigravity, include_str!("fixtures/agy_herramientas.jsonl"));
        let inicios: Vec<_> = ev.iter().filter_map(|e| if let Evento::HerramientaInicio { tipo, titulo, .. } = e { Some((*tipo, titulo.clone())) } else { None }).collect();
        assert_eq!(inicios[0], (TipoHerramienta::Lectura, "/proyecto/w3/a.txt".into()));
        assert_eq!(inicios[1], (TipoHerramienta::Comando, "ls".into()));
        assert_eq!(inicios.len(), 3, "ACTIVE y DONE del mismo paso son una sola herramienta");
        assert!(ev.iter().any(|e| matches!(e, Evento::HerramientaFin { id, error: false, salida } if id == "paso-4" && salida.contains("a.txt"))));
        assert!(ev.contains(&Evento::Error("interrupted".into())));
        assert_eq!(ev.last(), Some(&Evento::Fin { ok: false }));
    }

    #[test]
    fn lineas_raras_se_ignoran() {
        let mut i = Interprete::new(Backend::Codex);
        assert!(i.linea("Reading additional input from stdin...").is_empty());
        assert!(i.linea("{roto").is_empty());
        assert_eq!(limpiar_shell("/usr/bin/bash -lc 'cat a.txt'"), "cat a.txt");
        assert_eq!(limpiar_shell("/usr/bin/bash -lc ls"), "ls");
    }
}
