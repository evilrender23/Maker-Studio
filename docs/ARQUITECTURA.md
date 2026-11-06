# Arquitectura

Maker Studio es un workspace de Cargo con cinco crates:

| Crate          | Qué es |
|----------------|--------|
| `maker_api`    | API de scripting: tipos C-ABI, el trait `Script`, `Ctx`, `Props` y la macro `export_scripts!`. Es lo único de lo que depende el crate de scripts de un juego. |
| `maker_engine` | Núcleo sin gráficos: ECS, escena y su formato, jerarquía de transforms, mallas (primitivas y glTF), modelos con materiales, esqueletos y clips (`model.rs`), animación (`animation.rs`), biblioteca integrada (`biblioteca.rs`), física, entrada, carga de scripts, bucle de juego, proyectos. Se prueba sin ventana. |
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
| Modelo de sombreado por material | Unreal (*shading models*), Godot (`render_mode`) | `Material::shading`: PBR, toon, sin luz o plumilla; un pipeline por modelo. |
| Biblioteca de assets integrada | Unreal (Starter Content / Fab), Unity (Asset Store) | `assets/biblioteca` va con el motor y se referencia como `biblioteca://pack/modelo.glb`; exportar copia solo lo usado. Ver [BIBLIOTECA.md](BIBLIOTECA.md). |
| Animación esquelética con reasignación por nombre de hueso | Unreal (IK Retargeter), Godot (`BoneMap`) | Componente `Animator`; skinning en GPU con una paleta de huesos por entidad y fotograma. |
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
- Modelos con varias submallas en un solo búfer de vértices; los dibujos se
  ordenan por pipeline, material y modelo para no repetir *binds*. Texturas
  cacheadas por ruta y compartidas entre modelos del mismo pack.
- Skinning en GPU: la paleta de huesos de cada entidad con piel se calcula en
  CPU y se sube una vez por fotograma a un único búfer de almacenamiento; la
  usan el pase de color, el de sombras y el contorno. Las listas de dibujo y
  los búferes de subida se reutilizan entre fotogramas.
- Física Rapier (SIMD, islas, *sleeping*); solo existe durante Play.
- Scripts nativos sin intérprete ni GC.
- El historial de deshacer guarda la escena completa solo al terminar una
  edición (al soltar el ratón), no en cada fotograma.

## Renderizador

Pasadas por fotograma: sombras (profundidad desde el sol, con recorte alfa y
piel) → cielo (triángulo a pantalla completa) → opacos y recortes agrupados por
pipeline (PBR y sin luz → toon → plumilla) → contornos de casco invertido de
los materiales toon y plumilla → transparentes de atrás adelante → contorno de
selección → rejilla infinita del editor (con desvanecimiento).

Cada submalla glTF tiene su material: textura de color base (sRGB, mipmaps,
trilineal + anisotropía ×16), factor, modo alfa (opaco, recorte con
`alphaCutoff`, mezcla) y doble cara. El color final es textura × factor ×
color de vértice × tinte del `MeshRenderer`, en todos los modelos de
sombreado. Las variantes (recorte, mezcla, piel, doble cara) son constantes
`override` de WGSL y pipelines creados bajo demanda.
Todo se dibuja en una textura fuera de pantalla `Rgba8UnormSrgb` (MSAA
resuelto) que el editor registra en egui y el reproductor pinta a pantalla
completa.

Cada modelo de sombreado (`ShadingModel`) es un *entry point* de fragmento
propio en `shaders.wgsl` (`fs_main`, `fs_toon`, `fs_hatch`), así el PBR no
paga ramas ni registros de los demás. Los dibujos se ordenan por pipeline, de
modo que hay como mucho un cambio por modelo y fotograma
(`FrameStats::pipeline_switches`). Los parámetros estilizados viven en el
mismo uniforme por objeto: caben en el paso de 256 B que ya imponía la
alineación del *offset* dinámico, así que el PBR no crece. El contorno usa
normales suavizadas (promediadas por posición) en un búfer de vértices aparte,
para que el casco no se abra en las aristas duras sin engordar el vértice
del PBR. Detalles de cada modelo en [SHADERS.md](SHADERS.md).

`crates/maker_render/src/renderer.rs` tiene tests que renderizan sin ventana y
comprueban píxeles: la escena inicial en PBR y una escena con todos los
modelos intercalados (agrupación por pipeline y color del contorno).

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
proyección, materiales (escenas antiguas, `unlit`, ida y vuelta), normales suavizadas, entrada, física (caída y reposo, raycast), scripts (props,
spawn, destroy, pánico atrapado, `set_shading`), proyectos, cámara del editor, gizmos
(arrastre, ajuste, anillo de rotación), resaltado de Rust, deshacer/rehacer,
render sin ventana, recarga en caliente de punta a punta, catálogo de la
biblioteca, orden de búsqueda y rutas `biblioteca://`, muestreo de animación
(lineal, escalón, slerp), paleta de huesos, reasignación por nombre de hueso y
mipmaps con cobertura alfa.
