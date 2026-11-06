# Arquitectura

Maker Studio es un workspace de Cargo con cinco crates:

| Crate          | Qué es |
|----------------|--------|
| `maker_api`    | API de scripting: tipos C-ABI, el trait `Script`, `Ctx`, `Props` y la macro `export_scripts!`. Es lo único de lo que depende el crate de scripts de un juego. |
| `maker_engine` | Núcleo sin gráficos: ECS, escena y su formato, jerarquía de transforms, mallas (primitivas y glTF), física, entrada, carga de scripts, bucle de juego, proyectos. Se prueba sin ventana. |
| `maker_render` | Renderizador de escena sobre wgpu (Vulkan, Metal, DX12 o GL según la plataforma). |
| `maker_editor` | El editor (`maker-studio`), con egui. |
| `maker_player` | El reproductor sin editor (`maker-player`), que es el ejecutable de un juego exportado. |

## Qué se toma de cada motor

| Práctica | De dónde | Cómo está aquí |
|----------|----------|----------------|
| Datos en componentes, lógica en sistemas; almacenamiento por arquetipos | Unity DOTS, Unreal Mass, Bevy | `hecs::World`: los componentes son structs de datos (`components.rs`); física, transforms y scripts son sistemas. La iteración recorre memoria contigua. |
| Árbol de escena padre/hijo con transform local y global | Godot (nodos), Unity (Transform) | `Parent` + `SiblingIndex` + `Transform` local; `GlobalTransform` calculado en O(n) cada fotograma. Re-emparentar conserva la posición de mundo, como en Unity. |
| Escenas en texto legible y diffeable | Godot (`.tscn`) | `.mscene` en JSON con ids estables (`EntityId`), apto para git. |
| Comportamientos como componentes, varios por entidad | Unity (`MonoBehaviour`), Unreal (`ActorComponent`) | `Scripts(Vec<ScriptSlot>)`; cada script en Rust nativo. |
| Propiedades editables expuestas desde el código | Unity `[SerializeField]`, Godot `@export`, Unreal `UPROPERTY` | `fn props(&mut self, p: &mut Props)`: el mismo visitante sirve para contar, leer y escribir. |
| Paso fijo de física separado del fotograma | Unity `FixedUpdate`, Godot `_physics_process`, Unreal substepping | 60 Hz con acumulador (máx. 5 pasos por fotograma para evitar la espiral de la muerte). |
| Destroy diferido | Unity | Las destrucciones se aplican al final del fotograma. |
| Play en el editor con restauración al parar | Unity, Unreal PIE | Instantánea de la escena al pulsar Play; Stop la restaura. Los cambios hechos jugando se descartan, como en Unity. |
| Recarga en caliente de código | Unreal Live Coding, Godot | Scripts en `cdylib` recargados al compilar; frontera C-ABI estable. |
| Materiales PBR metal-rugosidad | Unreal, Unity HDRP/URP, Godot | GGX + Smith + Schlick, ambiente hemisférico, reflejo aproximado del cielo, tonemap ACES. |
| Lanzar el juego aparte del editor | Unreal "Standalone Game" | Archivo → Ejecutar en ventana aparte (`maker-player`). |
| Exportar el juego | los tres | Archivo → Exportar juego: reproductor + escenas + assets + biblioteca de scripts en `export/<nombre>/`. |
| Project Hub | Unity Hub, gestor de proyectos de Godot | Pantalla inicial con recientes, nuevo (demo o básico) y abrir. |
| Controles de viewport | Unity / Unreal | Clic derecho + WASDQE para volar, Alt+clic para orbitar, botón central para desplazar, W/E/R para gizmos, F para encuadrar. |

## Rendimiento

- Perfil `release` con `opt-level = 3`, LTO *thin* y `codegen-units = 4`; en
  `dev` las dependencias se compilan optimizadas para que el editor sea fluido
  incluso en depuración.
- Renderizador: un único *buffer* uniforme con *offsets* dinámicos para todos
  los objetos (una escritura por fotograma), mallas en GPU cacheadas por origen,
  *frustum culling* por AABB, MSAA 4×, mapa de sombras de 2048² ajustado a
  texel para que no parpadee.
- Física Rapier (SIMD, islas, *sleeping*); solo existe durante Play.
- Scripts nativos sin intérprete ni GC.
- El historial de deshacer guarda la escena completa solo al terminar una
  edición (al soltar el ratón), no en cada fotograma.

## Renderizador

Pasadas por fotograma: sombras (profundidad desde el sol) → cielo (triángulo a
pantalla completa) → opacos PBR → contorno de selección (casco invertido) →
rejilla infinita del editor (con desvanecimiento). Todo se dibuja en una
textura fuera de pantalla `Rgba8UnormSrgb` (MSAA resuelto) que el editor
registra en egui y el reproductor pinta a pantalla completa.
`crates/maker_render/src/renderer.rs` tiene un test que renderiza la escena
inicial sin ventana y comprueba píxeles.

## Tema

El tema claro se extrajo de Hermes Agent Desktop (`app.asar` → CSS `:root`):
semillas `#0053fd` (primario) y `#17171a` (texto) y las mezclas `color-mix()`
resueltas a sRGB. La tabla completa de variables está en
`crates/maker_editor/src/theme.rs`. El editor de código usa la paleta Shiki
`github-light-default`, la misma que Hermes.

## Pruebas

```bash
cargo test --workspace --release
cargo test --release -p maker_engine --test hot_reload -- --ignored   # compila scripts reales
```

Cubren: ABI de propiedades, cuaterniones, escena (JSON ida y vuelta,
jerarquía, duplicar, reordenar), mallas (normales hacia fuera), rayos y
proyección, entrada, física (caída y reposo, raycast), scripts (props,
spawn, destroy, pánico atrapado), proyectos, cámara del editor, gizmos
(arrastre, ajuste, anillo de rotación), resaltado de Rust, deshacer/rehacer,
render sin ventana y recarga en caliente de punta a punta.
