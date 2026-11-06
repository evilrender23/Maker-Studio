//! Modo Texturizado: puente con el motor de TextureCraft (MIT, github.com/storytold/texturecraft,
//! commit e01dac2). Ver `docs/TEXTURIZADO.md`.

pub use texturecraft_engine as tc;
pub use texturecraft_render as tc_render;

#[cfg(test)]
mod tests {
    use super::tc;
    use serde_json::json;
    use tc::Session;

    /// Un asset de la biblioteca pasa por el flujo completo: importar, hornear, material
    /// inteligente y exportar los mapas PBR.
    #[test]
    fn caja_de_la_biblioteca_se_texturiza() {
        let ruta = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/biblioteca/scifi_esencial/Prop_Crate.glb");
        let bytes = std::fs::read(ruta).unwrap();
        let mut s = Session::from_mesh_bytes("Prop_Crate.glb", &bytes, 512, "crate").unwrap();
        tc::execute(&mut s, "bake.run", &json!({"ts": "all", "resolution": 256})).unwrap();
        tc::execute(&mut s, "material.apply", &json!({"material": "steel_painted_worn"})).unwrap();
        while s.update() {}
        let opts = tc::file_cmds::export_options(&s, &json!({}), "prueba").unwrap();
        let nombres: Vec<String> = s.export_files(&[0], &opts).unwrap().into_iter().map(|f| f.name).collect();
        for canal in ["baseColor", "metallic", "roughness", "normal", "ambientOcclusion"] {
            assert!(nombres.iter().any(|n| n.contains(canal)), "falta {canal}: {nombres:?}");
        }
    }
}
