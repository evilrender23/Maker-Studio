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

    /// El GLB texturizado que escribe TextureCraft se carga en Maker con todos sus mapas PBR.
    #[test]
    fn glb_texturizado_se_carga_con_sus_mapas() {
        let ruta = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/biblioteca/scifi_esencial/Prop_Barrel1.glb");
        let mut s = Session::from_mesh_bytes("Prop_Barrel1.glb", &std::fs::read(ruta).unwrap(), 256, "barril").unwrap();
        tc::execute(&mut s, "material.apply", &json!({"material": "steel_painted_worn"})).unwrap();
        while s.update() {}
        let glb = s.mesh_glb_bytes().unwrap();
        let dir = std::env::temp_dir().join(format!("maker-textura-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let archivo = dir.join("barril.glb");
        std::fs::write(&archivo, glb).unwrap();
        let modelo = maker_engine::model::ModelData::load(&archivo).unwrap();
        let m = modelo.materials.iter().find(|m| m.texture.is_some()).expect("material con color base");
        assert!(m.normal.is_some(), "normal map");
        assert!(m.metallic_roughness.is_some(), "metal/rugosidad");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
