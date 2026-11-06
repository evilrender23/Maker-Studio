//! Modo Texturizado: puente con el motor de TextureCraft (MIT, github.com/storytold/texturecraft,
//! commit e01dac2). Ver `docs/TEXTURIZADO.md`.

pub mod cli;
pub mod glb;
pub mod render;
pub mod sesion;

pub use sesion::{Analisis, Opciones, Textura};
pub use texturecraft_engine as tc;
pub use texturecraft_render as tc_render;

#[cfg(test)]
mod tests {
    use super::sesion::{self, Opciones};
    use super::tc;
    use serde_json::json;
    use std::path::{Path, PathBuf};
    use tc::Session;

    fn asset(rel: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/biblioteca").join(rel)
    }

    fn temporal(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("maker-textura-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// Un asset de la biblioteca pasa por el flujo completo: importar, hornear, material
    /// inteligente y exportar los mapas PBR.
    #[test]
    fn caja_de_la_biblioteca_se_texturiza() {
        let bytes = std::fs::read(asset("scifi_esencial/Prop_Crate.glb")).unwrap();
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
        let mut s = Session::from_mesh_bytes("Prop_Barrel1.glb", &std::fs::read(asset("scifi_esencial/Prop_Barrel1.glb")).unwrap(), 256, "barril").unwrap();
        tc::execute(&mut s, "material.apply", &json!({"material": "steel_painted_worn"})).unwrap();
        while s.update() {}
        let dir = temporal("glb");
        let archivo = dir.join("barril.glb");
        std::fs::write(&archivo, s.mesh_glb_bytes().unwrap()).unwrap();
        let modelo = maker_engine::model::ModelData::load(&archivo).unwrap();
        let m = modelo.materials.iter().find(|m| m.texture.is_some()).expect("material con color base");
        assert!(m.normal.is_some(), "normal map");
        assert!(m.metallic_roughness.is_some(), "metal/rugosidad");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// El análisis detecta el solape de UV de los trim sheets y la piel de los personajes.
    #[test]
    fn analisis_de_uv_y_piel() {
        let caja = sesion::analizar(&asset("scifi_esencial/Prop_Crate.glb")).unwrap();
        assert!(caja.solapado() && !caja.con_piel, "{caja:?}");
        let barril = sesion::analizar(&asset("scifi_esencial/Prop_Barrel1.glb")).unwrap();
        assert!(!barril.solapado(), "{barril:?}");
        let personaje = sesion::analizar(&asset("personajes/Superhero_Male_FullBody.glb")).unwrap();
        assert!(personaje.con_piel, "{personaje:?}");
    }

    /// Abrir partiendo del original pone una capa «Original» por texture set; aplicar escribe
    /// las texturas glTF y una copia del modelo que Maker carga con todos los mapas y que
    /// conserva la piel del personaje. El `.tcraft` guardado se reabre.
    #[test]
    fn abrir_y_aplicar_conserva_piel_y_usa_los_mapas() {
        let raiz = temporal("aplicar");
        let ruta = asset("personajes/Superhero_Male_FullBody.glb");
        let mut t = sesion::abrir(&ruta, &raiz, Opciones { resolucion: 256, uv_unicos: true, desde_original: true }).unwrap();
        let originales = t.s.doc.project.layers.values().filter(|l| l.name == "Original").count();
        assert_eq!(originales, t.s.set_count(), "una capa Original por texture set (con piel no se despliegan los UV)");
        tc::execute(&mut t.s, "material.apply", &json!({"material": "steel_painted_worn"})).unwrap();
        let glb = sesion::aplicar(&mut t).unwrap();
        assert!(t.ruta_proyecto().is_file());
        let original = maker_engine::model::ModelData::load(&ruta).unwrap();
        let nuevo = maker_engine::model::ModelData::load(&glb).unwrap();
        assert_eq!(nuevo.anim.skeleton.joints.len(), original.anim.skeleton.joints.len());
        assert_eq!(nuevo.mesh.positions.len(), original.mesh.positions.len());
        let con_mapas = nuevo.materials.iter().filter(|m| m.texture.is_some() && m.metallic_roughness.is_some() && m.occlusion.is_some() && m.normal.is_some()).count();
        assert!(con_mapas >= t.s.set_count(), "{con_mapas} materiales con mapas de {}", t.s.set_count());
        let mut t2 = sesion::abrir(&t.ruta_proyecto(), &raiz, Opciones::default()).unwrap();
        assert_eq!(t2.s.set_count(), t.s.set_count());
        assert_eq!(sesion::aplicar(&mut t2).unwrap(), glb);
        let _ = std::fs::remove_dir_all(&raiz);
    }

    /// UV únicos en un prop: el despliegue nuevo no se solapa y el modelo aplicado lo usa.
    #[test]
    fn uv_unicos_en_un_prop() {
        let raiz = temporal("unicos");
        let mut t = sesion::abrir(&asset("scifi_esencial/Prop_Crate.glb"), &raiz, Opciones { resolucion: 512, uv_unicos: true, desde_original: true }).unwrap();
        let glb = sesion::aplicar(&mut t).unwrap();
        let a = sesion::analizar(&glb).unwrap();
        assert!(!a.solapado(), "{a:?}");
        let _ = std::fs::remove_dir_all(&raiz);
    }

    /// La CLI que usa el agente: abrir, info, ejecutar, lote, aplicar y comandos.
    #[test]
    fn cli_del_agente() {
        let raiz = temporal("cli");
        let asset = asset("scifi_esencial/Prop_Barrel1.glb").display().to_string();
        let a = |v: &[&str]| -> Vec<String> { v.iter().map(|s| s.to_string()).collect() };
        let prev = std::env::current_dir().unwrap();
        std::env::set_current_dir(&raiz).unwrap();
        let r = (|| -> Result<(), String> {
            let tcraft = super::cli::ejecutar(&a(&["abrir", &asset, "--resolucion", "256"]))?;
            assert!(Path::new(&tcraft).is_file(), "{tcraft}");
            assert!(super::cli::ejecutar(&a(&["info", &asset]))?.contains("solapeUv"));
            super::cli::ejecutar(&a(&["ejecutar", &tcraft, "material.apply", r#"{"material":"rust"}"#]))?;
            let pasos = raiz.join("pasos.json");
            std::fs::write(&pasos, r#"[{"id":"layer.add.paint","params":{"name":"Detalle"}}]"#).unwrap();
            super::cli::ejecutar(&a(&["lote", &tcraft, &pasos.display().to_string()]))?;
            let glb = super::cli::ejecutar(&a(&["aplicar", &tcraft]))?;
            assert!(Path::new(&glb).is_file(), "{glb}");
            assert!(super::cli::ejecutar(&a(&["comandos", "bake"]))?.contains("bake.run"));
            Ok(())
        })();
        std::env::set_current_dir(prev).unwrap();
        r.unwrap();
        let _ = std::fs::remove_dir_all(&raiz);
    }
}
