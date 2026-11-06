//! `maker-studio textura …`: el modo Texturizado sin ventana, para el agente y los scripts.
//! Trabaja en la carpeta actual como raíz del proyecto.

use crate::sesion::{self, Opciones, Textura};
use crate::tc;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

pub const AYUDA: &str = "maker-studio textura <orden> …  (en la carpeta del proyecto)

  info <asset>                       triángulos, materiales, solape de UV, piel (JSON)
  comandos [filtro]                  comandos de TextureCraft (id, nombre, parámetros)
  abrir <asset> [--resolucion N] [--uv-unicos] [--sin-original]
                                     crea texturizados/<nombre>/<nombre>.tcraft
  ejecutar <x.tcraft> <id> [json]    ejecuta un comando y guarda
  lote <x.tcraft> <pasos.json>       [{\"id\": …, \"params\": {…}}, …] y guarda
  aplicar <x.tcraft>                 texturas glTF + texturizados/<nombre>/<nombre>.glb
  render <x.tcraft> <png> [--size ANCHOxALTO] [--yaw GRADOS]
                                     vista previa en PNG";

fn abrir_tcraft(p: &str) -> Result<Textura, String> {
    let raiz = std::env::current_dir().map_err(|e| e.to_string())?;
    sesion::abrir(Path::new(p), &raiz, Opciones::default())
}

fn valor(args: &[String], k: &str) -> Option<String> {
    args.iter().position(|a| a == k).and_then(|i| args.get(i + 1)).cloned()
}

fn pos(args: &[String], i: usize, que: &str) -> Result<String, String> {
    args.iter().filter(|a| !a.starts_with("--")).nth(i).cloned().ok_or_else(|| format!("falta {que}\n\n{AYUDA}"))
}

/// Ejecuta una orden; devuelve lo que se imprime.
pub fn ejecutar(args: &[String]) -> Result<String, String> {
    let orden = args.first().map(String::as_str).unwrap_or("ayuda");
    let rest = args.get(1..).unwrap_or_default();
    let pretty = |v: &Value| serde_json::to_string_pretty(v).unwrap_or_default();
    match orden {
        "info" => {
            let a = sesion::analizar(Path::new(&pos(rest, 0, "<asset>")?))?;
            Ok(pretty(&json!({
                "triangulos": a.triangulos, "materiales": a.materiales, "solapeUv": a.solape,
                "uvFueraDeRango": a.fuera_de_rango, "sinUv": a.sin_uv, "conPiel": a.con_piel,
                "recomiendaUvUnicos": a.solapado() && !a.con_piel,
            })))
        }
        "comandos" => {
            let filtro = rest.first().map(|f| f.to_lowercase());
            let v: Vec<Value> = tc::command_specs()
                .iter()
                .filter(|c| filtro.as_ref().is_none_or(|f| c.id.to_lowercase().contains(f) || c.label.to_lowercase().contains(f)))
                .map(|c| json!({ "id": c.id, "label": c.label, "params": c.params }))
                .collect();
            Ok(pretty(&Value::Array(v)))
        }
        "abrir" => {
            let raiz = std::env::current_dir().map_err(|e| e.to_string())?;
            let op = Opciones {
                resolucion: valor(rest, "--resolucion").and_then(|v| v.parse().ok()).unwrap_or(2048),
                uv_unicos: rest.iter().any(|a| a == "--uv-unicos"),
                desde_original: !rest.iter().any(|a| a == "--sin-original"),
            };
            let mut t = sesion::abrir(Path::new(&pos(rest, 0, "<asset>")?), &raiz, op)?;
            let p = sesion::guardar(&mut t)?;
            Ok(p.display().to_string())
        }
        "ejecutar" => {
            let mut t = abrir_tcraft(&pos(rest, 0, "<x.tcraft>")?)?;
            let id = pos(rest, 1, "<id>")?;
            let params: Value = match rest.iter().filter(|a| !a.starts_with("--")).nth(2) {
                Some(j) => serde_json::from_str(j).map_err(|e| format!("parámetros JSON: {e}"))?,
                None => json!({}),
            };
            let r = tc::execute(&mut t.s, &id, &params).map_err(|e| e.to_string())?;
            sesion::guardar(&mut t)?;
            Ok(pretty(&r))
        }
        "lote" => {
            let mut t = abrir_tcraft(&pos(rest, 0, "<x.tcraft>")?)?;
            let texto = std::fs::read_to_string(pos(rest, 1, "<pasos.json>")?).map_err(|e| e.to_string())?;
            let pasos: Value = serde_json::from_str(&texto).map_err(|e| format!("pasos: {e}"))?;
            let mut res = Vec::new();
            for (i, p) in pasos.as_array().ok_or("los pasos son una lista")?.iter().enumerate() {
                let id = p["id"].as_str().ok_or_else(|| format!("paso {i}: falta \"id\""))?;
                let r = tc::execute(&mut t.s, id, p.get("params").unwrap_or(&json!({}))).map_err(|e| format!("paso {i} ({id}): {e}"))?;
                res.push(r);
            }
            sesion::guardar(&mut t)?;
            Ok(pretty(&Value::Array(res)))
        }
        "aplicar" => {
            let mut t = abrir_tcraft(&pos(rest, 0, "<x.tcraft>")?)?;
            Ok(sesion::aplicar(&mut t)?.display().to_string())
        }
        "render" => {
            let mut t = abrir_tcraft(&pos(rest, 0, "<x.tcraft>")?)?;
            let out = PathBuf::from(pos(rest, 1, "<png>")?);
            let (w, h) = valor(rest, "--size").and_then(|s| s.split_once('x').and_then(|(a, b)| Some((a.parse().ok()?, b.parse().ok()?)))).unwrap_or((800, 600));
            let png = crate::render::png(&mut t.s, w, h, valor(rest, "--yaw").and_then(|v| v.parse().ok()))?;
            std::fs::write(&out, png).map_err(|e| e.to_string())?;
            Ok(out.display().to_string())
        }
        _ => Ok(AYUDA.to_string()),
    }
}
