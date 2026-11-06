//! Binary glTF: leer (también un `.gltf` con sus búferes, que queda autocontenido) y escribir.

use serde_json::Value;
use std::path::Path;

const MAGIC: u32 = 0x4654_6C67; // "glTF"
const JSON: u32 = 0x4E4F_534A;
const BIN: u32 = 0x004E_4942;

/// Un glTF en memoria: el JSON y el búfer binario único.
#[derive(Clone, Debug)]
pub struct Glb {
    pub json: Value,
    pub bin: Vec<u8>,
}

fn u32_at(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(i..i + 4)?.try_into().ok()?))
}

impl Glb {
    /// Lee un GLB desde bytes.
    pub fn parse(bytes: &[u8]) -> Result<Glb, String> {
        if u32_at(bytes, 0) != Some(MAGIC) {
            return Err("no es un GLB".into());
        }
        let total = (u32_at(bytes, 8).ok_or("GLB truncado")? as usize).min(bytes.len());
        let mut i = 12;
        let (mut json, mut bin) = (None, Vec::new());
        while i + 8 <= total {
            let len = u32_at(bytes, i).ok_or("GLB truncado")? as usize;
            let kind = u32_at(bytes, i + 4).ok_or("GLB truncado")?;
            let data = bytes.get(i + 8..i + 8 + len).ok_or("bloque del GLB fuera de rango")?;
            match kind {
                JSON => json = Some(serde_json::from_slice(data).map_err(|e| format!("JSON del GLB: {e}"))?),
                BIN if bin.is_empty() => bin = data.to_vec(),
                _ => {}
            }
            i += 8 + len.div_ceil(4) * 4;
        }
        Ok(Glb { json: json.ok_or("el GLB no tiene JSON")?, bin })
    }

    /// Lee un `.glb` o un `.gltf`; los búferes de un `.gltf` (archivos o `data:`) se juntan en
    /// el binario único y las imágenes con ruta relativa pasan a ruta absoluta.
    pub fn load(path: &Path) -> Result<Glb, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        if u32_at(&bytes, 0) == Some(MAGIC) {
            return Glb::parse(&bytes);
        }
        let mut json: Value = serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        let dir = path.parent().unwrap_or(Path::new("."));
        let mut bin = Vec::new();
        let mut base = Vec::new();
        for b in json["buffers"].as_array().cloned().unwrap_or_default() {
            let data = match b["uri"].as_str() {
                Some(u) if u.starts_with("data:") => base64(u.split_once(',').map(|x| x.1).unwrap_or(""))?,
                Some(u) => std::fs::read(dir.join(percent_decode(u))).map_err(|e| format!("búfer {u}: {e}"))?,
                None => Vec::new(),
            };
            while bin.len() % 4 != 0 {
                bin.push(0);
            }
            base.push(bin.len());
            bin.extend(data);
        }
        if let Some(views) = json["bufferViews"].as_array_mut() {
            for v in views {
                let b = v["buffer"].as_u64().unwrap_or(0) as usize;
                let off = v["byteOffset"].as_u64().unwrap_or(0) as usize + base.get(b).copied().unwrap_or(0);
                v["buffer"] = 0.into();
                v["byteOffset"] = off.into();
            }
        }
        if let Some(images) = json["images"].as_array_mut() {
            for img in images {
                if let Some(u) = img["uri"].as_str()
                    && !u.starts_with("data:")
                {
                    img["uri"] = dir.join(percent_decode(u)).to_string_lossy().into_owned().into();
                }
            }
        }
        json["buffers"] = serde_json::json!([{ "byteLength": bin.len() }]);
        Ok(Glb { json, bin })
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut json = self.json.clone();
        if !self.bin.is_empty() {
            json["buffers"] = serde_json::json!([{ "byteLength": self.bin.len() }]);
        } else if let Some(o) = json.as_object_mut() {
            o.remove("buffers");
        }
        let mut j = serde_json::to_vec(&json).unwrap_or_default();
        while j.len() % 4 != 0 {
            j.push(b' ');
        }
        let mut b = self.bin.clone();
        while b.len() % 4 != 0 {
            b.push(0);
        }
        let total = 12 + 8 + j.len() + if b.is_empty() { 0 } else { 8 + b.len() };
        let mut out = Vec::with_capacity(total);
        out.extend(MAGIC.to_le_bytes());
        out.extend(2u32.to_le_bytes());
        out.extend((total as u32).to_le_bytes());
        out.extend((j.len() as u32).to_le_bytes());
        out.extend(JSON.to_le_bytes());
        out.extend(j);
        if !b.is_empty() {
            out.extend((b.len() as u32).to_le_bytes());
            out.extend(BIN.to_le_bytes());
            out.extend(b);
        }
        out
    }

    /// ¿Tiene pieles (personajes animados)?
    pub fn skinned(&self) -> bool {
        self.json["skins"].as_array().is_some_and(|s| !s.is_empty())
    }

    /// Nombre con que TextureCraft llama al material `i` (y a su texture set).
    pub fn material_name(&self, i: usize) -> String {
        self.json["materials"][i]["name"].as_str().map(str::to_string).unwrap_or_else(|| format!("Material_{i}"))
    }
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && let Some(v) = s.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(v);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn base64(s: &str) -> Result<Vec<u8>, String> {
    let val = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return None,
        } as u32)
    };
    let mut out = Vec::new();
    let (mut acc, mut n) = (0u32, 0);
    for c in s.bytes().filter(|c| !c.is_ascii_whitespace() && *c != b'=') {
        acc = acc << 6 | val(c).ok_or("base64 no válido")?;
        n += 6;
        if n >= 8 {
            n -= 8;
            out.push((acc >> n) as u8);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ida_y_vuelta() {
        let g = Glb { json: serde_json::json!({"asset": {"version": "2.0"}}), bin: vec![1, 2, 3] };
        let b = g.to_bytes();
        assert_eq!(b.len() % 4, 0);
        let r = Glb::parse(&b).unwrap();
        assert_eq!(r.json["asset"]["version"], "2.0");
        assert_eq!(&r.bin[..3], &[1, 2, 3]);
        assert_eq!(r.json["buffers"][0]["byteLength"], 3);
    }

    #[test]
    fn base64_y_porcentajes() {
        assert_eq!(base64("aGVsbG8=").unwrap(), b"hello");
        assert_eq!(percent_decode("a%20b.png"), "a b.png");
    }
}
