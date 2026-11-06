//! Prueba de punta a punta del scripting: crea un proyecto, compila sus
//! scripts con cargo, los ejecuta, cambia el código, recompila y recarga en
//! caliente. Tarda ~20 s (compila un crate real), por eso va con `--ignored`:
//!
//! ```text
//! cargo test -p maker_engine --test hot_reload -- --ignored
//! ```

use maker_engine::components::*;
use maker_engine::{Input, Project, Runtime, Scene, ScriptLib};
use std::sync::Arc;

#[test]
#[ignore]
fn build_run_edit_rebuild_reload() {
    let dir = std::env::temp_dir().join(format!("maker-hot-reload-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let project = Project::create(&dir, "Recarga", false).unwrap();
    project.build_scripts_blocking().expect("los scripts de la plantilla compilan");

    let run = |speed_expect: f32| {
        let lib = Arc::new(ScriptLib::load(&project.script_lib_path()).unwrap());
        assert!(lib.names().contains(&"Rotator".to_string()));
        let mut scene = Scene::starter();
        let cube = scene.find_by_name("Cubo").unwrap();
        let mut slot = ScriptSlot { name: "Rotator".into(), enabled: true, ..Default::default() };
        slot.props.insert("speed".into(), PropVal::Float(90.0));
        scene.insert(cube, Scripts(vec![slot]));
        let mut rt = Runtime::start(&mut scene, Some(lib));
        let input = Input::default();
        for _ in 0..60 {
            rt.tick(&mut scene, &input, 1.0 / 60.0);
        }
        let deg = scene.transform(cube).euler_degrees().y;
        assert!((deg - speed_expect).abs() < 2.0, "giro esperado {speed_expect}°, obtenido {deg}°");
    };
    // 90 °/s durante 1 s.
    run(90.0);

    // Cambiar el código: el Rotator gira al doble.
    let lib_rs = project.scripts_dir().join("src/lib.rs");
    let src = std::fs::read_to_string(&lib_rs).unwrap();
    let edited = src.replace("ctx.rotate_y(self.speed.to_radians() * ctx.dt);", "ctx.rotate_y(self.speed.to_radians() * ctx.dt * 0.5);");
    assert_ne!(src, edited);
    std::fs::write(&lib_rs, edited).unwrap();
    project.build_scripts_blocking().expect("recompila");
    run(45.0);

    let _ = std::fs::remove_dir_all(&dir);
}
