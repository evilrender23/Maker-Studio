# Arquitectura

Maker Studio es un workspace de Cargo con seis crates:

| Crate          | Qué es |
|----------------|--------|
| `maker_api`    | API de scripting: tipos C-ABI, el trait `Script`, `Ctx`, `Props` y la macro `export_scripts!`. Es lo único de lo que depende el crate de scripts de un juego. |
| `maker_engine` | Núcleo sin gráficos: ECS, interfaz de juego (`ui/`: layout, estilos, fuentes, plantillas, entrada y lista de dibujo), escena y su formato, jerarquía de transforms, mallas (primitivas y glTF), modelos con materiales, esqueletos y clips (`model.rs`), animación (`animation.rs`), biblioteca integrada (`biblioteca.rs`), física, entrada, carga de scripts, bucle de juego, proyectos. Se prueba sin ventana. |
| `maker_render` | Renderizador de escena sobre wgpu (Vulkan, Metal, DX12 o GL según la plataforma). |
| `maker_sculpt` | Esculpido digital sin interfaz: malla esculpible, BVH, trazo, pinceles, caída, simetría, máscara, deshacer por trazo y escritura `.glb`. Ver [ESCULPIR.md](ESCULPIR.md). |
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
| Iluminación global dinámica híbrida (pantalla → escena → caché de radiancia) | Unreal (Lumen), NVIDIA VXGI, Godot (VoxelGI / SDFGI) | Clipmap de vóxeles en cascadas centrado en la cámara, inyección de luz con realimentación, recolección por píxel (trazado en pantalla, conos, cielo), acumulación temporal y à-trous. Ver [ILUMINACION.md](ILUMINACION.md). |
| Modelo de sombreado por material | Unreal (*shading models*), Godot (`render_mode`) | `Material::shading`: PBR, toon, sin luz o plumilla; un pipeline por modelo. |
| Biblioteca de assets integrada | Unreal (Starter Content / Fab), Unity (Asset Store) | `assets/biblioteca` va con el motor y se referencia como `biblioteca://pack/modelo.glb`; exportar copia solo lo usado. Ver [BIBLIOTECA.md](BIBLIOTECA.md). |
| Animación esquelética con reasignación por nombre de hueso | Unreal (IK Retargeter), Godot (`BoneMap`) | Componente `Animator`; skinning en GPU con una paleta de huesos por entidad y fotograma. |
| Interfaz como entidades con rectángulo anclado, contenedores y estilos | Unity UGUI (`RectTransform`, `CanvasScaler`, `CanvasGroup`), Godot (`Control`, `Container`, `Theme`), Unreal UMG | Componentes `Lienzo`, `RectUi`, `Contenedor` y `WidgetUi` en el ECS; estilos y temas JSON del proyecto con herencia y retoques locales; plantillas `.mui` como prefabs. Ver [INTERFAZ.md](INTERFAZ.md). |
| Lanzar el juego aparte del editor | Unreal "Standalone Game" | Archivo → Ejecutar en ventana aparte (`maker-player`). |
| Orden de escenas y cambio de escena | Unity (*Scenes In Build*, `SceneManager.LoadScene`), Godot (`change_scene_to_file`) | `escenas` en `project.maker` (`escenas.rs`); la primera activa arranca. Los scripts piden el cambio y el host lo aplica al acabar el fotograma (`Runtime::aplicar_cambio_escena`): escena, física y scripts nuevos con la misma biblioteca, sin reentrada. |
| Exportar el juego | los tres | Archivo → Exportar juego: reproductor + escenas activas del orden + assets + biblioteca de scripts en `export/<nombre>/`. |
| Project Hub | Unity Hub, gestor de proyectos de Godot | Pantalla inicial con recientes, nuevo (demo o básico) y abrir. |
| Controles de viewport | Unity / Unreal | Clic derecho + WASDQE para volar, Alt+clic para orbitar, botón central para desplazar, W/E/R para gizmos, F para encuadrar. |
| Modos del visor sobre la misma escena | Blender (*Object Mode* / *Sculpt Mode*) | Tab cambia entre Objeto y Esculpir; al salir, la escultura se hornea a un `.glb` del proyecto y la entidad lo referencia con un paso normal del historial. |

## Rendimiento

- Interfaz: layout y estilos resueltos solo cuando algo cambia; lista de
  dibujo reutilizada sin reservas en régimen estable; un pipeline de quads
  instanciados con SDF y una llamada de dibujo por lote (imagen + recorte);
  atlas de glifos que sube solo las filas nuevas.

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
- Iluminación global: recursos creados solo con la GI activa (y liberados al
  apagarla), cascadas de vóxeles en caché que solo se revoxelizan cuando cambia
  lo que contienen, grupos de enlace precreados para el doble búfer: ninguna
  reserva por fotograma.
- Física Rapier (SIMD, islas, *sleeping*); solo existe durante Play.
- Scripts nativos sin intérprete ni GC.
- El historial de deshacer guarda la escena completa solo al terminar una
  edición (al soltar el ratón), no en cada fotograma.

## Esculpido (`maker_sculpt`)

Malla en estructura de arrays (posiciones, normales, máscara), con los vértices repetidos en
las costuras soldados y un mapa a los vértices de render (UV, colores, tangentes, submallas y
materiales) para guardar el modelo tal como era. Un BVH de hojas de 128 a 256 triángulos da
los rayos y las consultas de esfera y de cilindro; los vértices se renumeran para que los de
cada hoja sean contiguos, así una hoja es a la vez la unidad de trabajo en paralelo (rayon),
de deshacer (se guardan solo las hojas tocadas) y de subida a la GPU. Tras cada fotograma solo
se recalculan las normales y las cajas de las hojas tocadas y sus vecinas, y el renderizador
reescribe con `queue.write_buffer` solo esos rangos de un búfer de vértices dinámico (la
máscara oscurece el color de vértice). Los búferes de trabajo tienen el tamaño de la malla y
se reutilizan: un toque no reserva memoria. Pruebas: dirección de cada pincel, inversión,
máscara, simetría exacta, copias radiales, Capa sin acumular, volumen con Taubin, deshacer
exacto, rayos y consultas del BVH, ida y vuelta por `.glb`, rangos de subida y una prueba de
rendimiento sobre un millón de vértices; en `maker_render`, una prueba sin ventana de la subida
parcial.

## Renderizador

Pasadas por fotograma: sombras (profundidad desde el sol, con recorte alfa y
piel) → iluminación global si la escena la tiene (voxelizar lo que cambió,
inyectar la luz, prepase de normales, recolección, temporal y filtro; ver
[ILUMINACION.md](ILUMINACION.md)) → cielo (triángulo a pantalla completa) → opacos y recortes agrupados por
pipeline (PBR y sin luz → toon → plumilla) → contornos de casco invertido de
los materiales toon y plumilla → transparentes de atrás adelante → contorno de
selección → rejilla infinita del editor (con desvanecimiento) → posprocesado
(solo si la escena tiene efectos activos; ver [SHADERS.md](SHADERS.md)) →
interfaz del juego (`maker_render::ui`, sobre la imagen final y sin
posprocesar; ver [INTERFAZ.md](INTERFAZ.md)).

Cada submalla glTF tiene su material: textura de color base (sRGB, mipmaps,
trilineal + anisotropía ×16), factor, modo alfa (opaco, recorte con
`alphaCutoff`, mezcla) y doble cara. El color final es textura × factor ×
color de vértice × tinte del `MeshRenderer`, en todos los modelos de
sombreado. Las variantes (recorte, mezcla, piel, doble cara) son constantes
`override` de WGSL y pipelines creados bajo demanda.
Todo se dibuja en una textura fuera de pantalla `Rgba8UnormSrgb` (MSAA
resuelto) que el editor registra en egui y el reproductor pinta a pantalla
completa. Con posprocesado, la escena va antes a un objetivo `Rgba16Float` y
la cadena de efectos (`maker_render::postproceso`, objetivos creados solo al
cambiar de tamaño) escribe esa misma textura sRGB; el sombreado global
sustituye el material de cada objeto al rellenar su uniforme.

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
comprueban píxeles (y `gi/tests.rs`, los de la iluminación global): la escena inicial en PBR y una escena con todos los
modelos intercalados (agrupación por pipeline y color del contorno), la pila
de posprocesado vacía (píxeles idénticos, cero pases), la cadena HDR frente al
tonemap en la escena, todos los efectos integrados, los efectos WGSL propios
(recarga, errores) y el sombreado global con excepciones.

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
