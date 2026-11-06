//! La demo cambia de escena con su script compilado de verdad (frontera C-ABI
//! completa: `load_scene_index`, `next_scene`, `scene_index`…). Compila el
//! crate de scripts de la demo, por eso va con `--ignored`:
//!
//! ```text
//! cargo test -p maker_engine --test demo_escenas -- --ignored
//! ```

use maker_engine::{Input, Key, Project, Runtime, Scene, ScriptLib, log};
use std::sync::Arc;

#[test]
#[ignore]
fn demo_switches_scenes_with_n() {
    let dir = std::env::temp_dir().join(format!("maker-demo-escenas-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let project = Project::create(&dir, "DemoEscenas", true).unwrap();
    project.build_scripts_blocking().expect("los scripts de la demo compilan");
    let lib = Arc::new(ScriptLib::load(&project.script_lib_path()).unwrap());
    assert!(lib.names().contains(&"SceneSwitcher".to_string()));

    let mut scene = Scene::load(&project.main_scene_path()).unwrap();
    let mut rt = Runtime::start_en(&mut scene, Some(lib), Some(&project.root));
    rt.fijar_escena(&project.escena_inicial());
    let mut input = Input::default();
    let mut pulsar_n = |rt: &mut Runtime, scene: &mut Scene| {
        rt.tick(scene, &input, 1.0 / 60.0);
        input.set_key(Key::N, true);
        rt.tick(scene, &input, 1.0 / 60.0);
        input.end_frame();
        input.set_key(Key::N, false);
        input.end_frame();
        rt.aplicar_cambio_escena(scene)
    };
    assert_eq!(pulsar_n(&mut rt, &mut scene).as_deref(), Some("scenes/biblioteca.mscene"));
    assert_eq!(rt.escena.as_deref(), Some("scenes/biblioteca.mscene"));
    // En la última, el script vuelve a la primera con load_scene_index(0).
    assert_eq!(pulsar_n(&mut rt, &mut scene).as_deref(), Some("scenes/main.mscene"));
    rt.tick(&mut scene, &input, 1.0 / 60.0);
    let textos: Vec<String> = log::since(0).into_iter().map(|e| e.text).collect();
    assert!(textos.iter().any(|t| t == "Escena 2 de 2: biblioteca (N para cambiar)"), "{textos:?}");
    assert!(textos.iter().any(|t| t == "Escena 1 de 2: main (N para cambiar)"), "{textos:?}");
    let _ = std::fs::remove_dir_all(&dir);
}
