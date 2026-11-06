# SculptForge: ingeniería inversa para el modo Esculpir

Estudio de **SculptForge 0.11.8** (la AppImage instalada en `~/.local/bin`) para diseñar el
modo de esculpido de Maker Studio. Objetivo a largo plazo: que Maker Studio sea un «todo en
uno» del 3D, como Blender, con modos (Objeto, Esculpir, …) sobre la misma escena.

> **Licencia.** SculptForge es software libre **GPLv3 o posterior** (lo dice su «Acerca de» y
> su guía). Maker Studio es **MIT**. Aquí no se copia código, texto ni imágenes de SculptForge:
> se documenta *qué hace* y *cómo se organiza* para reimplementarlo desde cero, apoyándose en
> los algoritmos publicados que el propio programa cita (Taubin, Kelvinlets de de Goes y James,
> subdivisión de Loop, etc.). Las ideas y los algoritmos no están sujetos a copyright; el código
> y los textos sí.

## Cómo se ha estudiado

| Evidencia | Qué se sacó |
|---|---|
| `file`, `readelf -p .comment` | ELF x86-64 **no stripped**, C++ compilado con GCC 11.4 (Ubuntu 22.04). |
| Cadenas del binario | Dear ImGui (`imgui_impl_sdl3`, `imgui_impl_opengl3`), SDL 3.2, OpenGL 3.3; libcurl para red. |
| Tabla de símbolos (`nm -C`, 38 421 símbolos) | Clases y métodos del espacio `sf::`: la arquitectura completa. |
| Guía de usuario embebida (`sf::guide_data::kFiles`, 163 archivos: 51 capítulos `.md` + imágenes) | Comportamiento exacto de cada herramienta, rangos y valores por defecto. Se extrajo leyendo la tabla `kFiles` (nombre, puntero, tamaño; 24 B por entrada) con las relocalizaciones `R_X86_64_RELATIVE`. |
| Registro y ajustes del usuario (`~/.local/share/SculptForge`) | Perfil de hardware, `project.json`, carpetas de biblioteca (`~/Documentos/SculptForge`). |

No se ha ejecutado código de SculptForge para este estudio (análisis estático). Lo que la guía
describe se toma como comportamiento declarado, no observado.

## Arquitectura (por símbolos)

Número de símbolos por clase, como medida de peso:

| Módulo | Símbolos | Qué es |
|---|---|---|
| `sf::App` | 1567 | La aplicación: UI ImGui, paneles, entrada, órdenes. |
| `sf::ai`, `sf::link`, `sf::connect` | ~900 | Asistente de IA, enlace con el móvil y puentes (Blender, Maya). |
| `sf::Renderer`, `sf::MicroRenderer` | ~140 | Render OpenGL 3.3 del modelo y de los «micro niveles». |
| **`sf::Sculptor`** | 100 | El motor de pinceles: trazo, *dabs*, Kelvinlets, VDM, máscara, capas. |
| **`sf::Mesh`** | 87 | Malla editable: atributos (máscara, color, grupos, ocultos), capas, quads emparejados, normales por bloques sucios. |
| **`sf::DynamicTopology`** | 59 | Dyntopo: dividir, colapsar y girar aristas, relajar; espejo de triángulos. |
| **`sf::BVH`** | 49 | Árbol de volúmenes para consultas bajo el pincel y para lanzar rayos, con reajuste incremental. |
| `sf::CommandRegistry`, `sf::ActionRegistry` | ~100 | Todo es una orden con nombre y parámetros: la paleta (F3), los scripts y la CLI usan lo mismo. |
| `sf::Timeline` | 56 | Historial visual del esculpido. |
| `sf::UndoStack`, `sf::UndoRecord`, `*Undo` | ~60 | Deshacer por *Part* con presupuesto de memoria (~1 GB), registros por tipo. |
| `sf::Subdiv*`, `loopSubdivide`, `MultiresStack` | ~45 | Niveles de subdivisión (multires) y reconstrucción. |
| `sf::voxelRemesh`, `makeVoxelGrid`, `qremesh` | — | Remallado por vóxeles y remallado a quads con guías. |
| `sf::MicroStroke`, `sf::MicroStore` | ~50 | «Micro niveles»: detalle por encima de la malla base (cientos de millones de polígonos). |
| `sf::TexturePainter`, `sf::Uv*`, `PaintStack` | ~80 | Pintura de textura y PBR, editor UV, capas de pintura. |
| `sf::BrushRecipe`, `sf::BrushAlpha`, `sf::BrushSettingsStore` | ~70 | Pinceles como datos (recetas JSON), alfas generadas y ajustes por pincel. |
| `sf::ThreadPool` | 14 | Paralelismo de los *dabs* y las operaciones de malla completa. |

### El trazo (`sf::Sculptor`)

`beginStroke` → muchos `dab` → `endStroke`. Detalles que se ven en los nombres:

- **Dabs por lotes** (`batchable`, `flushBatch`, `endBatch`): varios *dabs* se acumulan y se
  aplican de una vez sobre la región tocada.
- **Normales y BVH perezosos** (`addLazyLeaves`, `flushLazy`, `catchUpNormals`,
  `updateDabNormals`): solo se recalcula lo que el pincel tocó, por hojas del BVH.
- **Grab** (`beginGrab`, `grabMove`, `grabFinish`): Move y Tendril no son *dabs*; arrastran los
  vértices capturados al empezar.
- **Kelvinlets** (`kelvinletStep`, `KelvinletKernel::{profile,envelope,cutoff,grabTerms}`,
  `KelvinFarField`): cálculo cercano exacto y un «campo lejano» agrupado y diferido.
- **Espejos** (`mirrors`): cada *dab* se replica por simetría X/Y/Z, radial y local.
- **Capas y objetivos de morph** (`prepareLayer`, `morphTarget`).
- **Remallado durante el trazo** (`remeshDab`, `followRemesh`, `mergeTopologyUndo`): dyntopo
  se aplica dentro del trazo y su deshacer se fusiona con el del trazo.
- **Suavizado Taubin** (`taubinShrink`, `sf::taubinSmooth`).

### La malla (`sf::Mesh` + `sf::BVH`)

- Triángulos con **quads emparejados** (`pairQuad`, `quadDiagonal`, `quadNeighbors`): el
  suavizado sigue las aristas del quad y no la diagonal.
- **Atributos opcionales** creados al usarse (`ensureMask`, `ensureColors`, `ensureFaceGroups`,
  `ensureHidden`).
- **Normales por bloques sucios** (`markDirtyAround`, `growDirtyBlocks`, `updateNormalsAround`):
  la GPU solo recibe los rangos cambiados.
- **BVH editable** (`insertTriangle`, `removeTriangle`, `splitLeaf`, `refitChanged`,
  `queryVerticesCapsule/Cylinder`): el BVH no se reconstruye con dyntopo, se edita.
- **Capas de esculpido** (`addLayer`, `setLayerWeight`, `layerTarget`): desplazamientos por capa
  con peso, exportables como *blend shapes*.

## Funciones (resumen de la guía)

### Pinceles

19 de esculpido más Pintar. Los diez primeros van en las teclas 1–0:

| Pincel | Qué hace | Con Alt |
|---|---|---|
| Standard (1) | Levanta la superficie | Hunde |
| Clay (2) | Añade arcilla que rellena huecos primero, tope algo plano | Quita |
| Smooth (3) | Relaja; también con Mayús | — |
| Move (4) | Arrastra paralelo a la pantalla | Mueve a lo largo de la normal |
| Inflate (5) | Hincha a lo largo de las normales | Desinfla |
| Crease (6) | Pliegue con lados pellizcados | Cresta |
| Flatten (7) | Aplana hacia un plano de la zona | Aleja del plano |
| Pinch (8) | Atrae hacia el trazo | Separa |
| Sharp Crease (9) | Pliegue estrecho en V | Cresta afilada |
| Clay Block (0) | Arcilla cuadrada de tope plano | Talla plano |
| Tendril | Estira cuernos y tentáculos que se afinan | — |
| Trim | Rasura por encima de un plano (superficie dura) | Rellena hasta el plano |
| Layer | Sube hasta una altura fija por trazo, sin acumular | Baja |
| Nudge | Desliza de lado sin levantar | — |
| VDM | Estampa un mapa de desplazamiento vectorial (con voladizos) | Lo hunde |
| KelvinGrab / Twist / Scale, Elastic Tendril | Deformaciones elásticas (Kelvinlets regularizados) | Sentido contrario |
| DetailClone | Repite un detalle capturado a lo largo del trazo | Lo hunde |
| Morph | Devuelve hacia un objetivo guardado | Aleja |

Además hay **recetas**, pinceles definidos como datos JSON (`BrushRecipe`): Wrinkle, Palette
Knife, Dragon Scales, Stone Chisel, Skin Pores, Clay Slab, Edge Wear.

### Ajustes de cada pincel

- **Tamaño**: de 2 a 800 px en pantalla, 60 por defecto. Hay un modo «tamaño en el mundo».
- **Intensidad**: de 1 a 100, 50 por defecto.
- **Caída**: de −100 a 100. Por debajo de 0 la punta se ensancha y se vuelve dura; por encima
  se estrecha y se suaviza.
- **Intensidad de color**.
- Cada pincel recuerda su propio tamaño, intensidad, caída, trazo, alfa y curva de presión.
- **Smooth** tiene una intensidad propia, y además un modo que conserva el volumen (Taubin).

**Opciones de caída**
- **Solo conectado**: no salta a piezas cercanas, por ejemplo al dedo de al lado.
- **Caída de vista**: un cilindro a lo largo de la vista que atraviesa el modelo.
- **Solo caras frontales**: no toca la cara de atrás en partes finas.

**Tipos de trazo**
- **Dots**: el normal, con espaciado.
- **Drag Stamp**: el arrastre fija el tamaño y el giro.
- **Slide Stamp**: el sello se desliza bajo el cursor.
- **Spray**, con dispersión.
- **Lazy / Steady Stroke**: «cuerda» con radio.
- **Continuous**: 30 *dabs* por segundo con el cursor quieto.

**Precisión**
- Línea recta con Mayús a mitad del trazo, en pasos de 45°.
- **Arrastre de precisión**: el pincel va a ¼ de la velocidad.
- Tamaño de hasta 0,5 px.
- Filtrado ligero del punto de la superficie (`PickSmoother`).

**Alfas**: 15 generadas por código (Round, Square, Noise, Pores, Cracks, Scales, Rivets…) o
cargadas desde una imagen. Opciones: seguir la dirección del trazo, ángulo, variación aleatoria
del ángulo y repetición en mosaico.

**Simetría**
- **Espejo**: X (activa por defecto, tecla X), Y y Z, combinables entre sí.
- **Radial**: de 2 a 64 copias alrededor de un eje.
- **Local**: el centro de la pieza en lugar del origen.
- El cursor marca dónde cae cada copia.

### Teclas y modificadores

| Tecla | Qué hace |
|---|---|
| Alt | Invertir |
| Mayús (antes del trazo) | Suavizar |
| Mayús (durante el trazo) | Línea recta |
| Ctrl | Pintar máscara |
| Ctrl+Alt | Borrar máscara |
| Ctrl+clic | Difuminar la máscara |
| Ctrl+Mayús | Gestos de visibilidad |
| [ / ] | Tamaño ±10 % |
| S, U, I, O | Deslizador en el cursor para tamaño, intensidad, color y caída |
| Espacio o clic derecho | Menú rápido |
| T, Y, J | Menús radiales |
| B | Selector de pinceles por letras |
| X | Simetría X |
| F3 | Paleta de órdenes |

### Geometría y herramientas

**Dyntopo**
- Detalle de 0 a 100. El tamaño de los triángulos sigue al pincel, o es constante si se activa
  «detalle constante».
- Bajo el pincel divide las aristas largas, colapsa las cortas y relaja.
- Al suavizar, simplifica.
- Respeta la simetría (triángulos espejados), la máscara, los bordes y los límites de grupo.

**Niveles de subdivisión** (multires)
- Dividir, subir y bajar de nivel, borrar los niveles inferiores o superiores, y reconstruir.
- Lo esculpido en un nivel se propaga a los demás.

**Micro niveles**: detalle por encima del nivel más alto, con render propio.

**Remallado y limpieza**
- Remallado por vóxeles y remallado rápido.
- Remallado a quads (Quad Remesh) con guías.
- Cerrar agujeros, diezmar y LODs.
- Espejar y soldar.

**Organización de la malla**
- **Máscara**: pincel, lazo, rectángulo, invertir, difuminar y afilar, máscara por cavidad,
  grupos a partir de la máscara.
- **Grupos de caras y visibilidad**: aislar, ocultar y mostrar.
- **Parts**: piezas independientes, cada una con su historial; se pueden unir, separar y extraer
  una cáscara.

**Capas de esculpido** con peso, exportables como *blend shapes*.

**Deformaciones de toda la pieza**: Polish, Relax, Smooth y Balloon.

**Gizmo** para mover, rotar, escalar y posar, con caída desde el borde de la máscara.

**Pintura**: pintura de vértices y de textura o PBR, editor UV y horneado de mapas.

### Automatización

- Todo es una orden con nombre:
  `stroke brush=Crease points="…" symmetry=x radial=8`, `deform_smooth amount=30 keep_volume=true`,
  `vdm_stamp …`, `dyntopo smooth_remesh=0`.
- Las mismas órdenes funcionan desde la paleta F3, los scripts, la CLI (`sculptforge-cli -c '…'`)
  y la IA.

### Rendimiento declarado

- Un *dab* que mueve medio millón de vértices en una pieza de 10 M tarda unos 10 ms.
- Al orbitar escaneos muy densos se dibuja una versión reducida y el detalle completo vuelve al
  parar la vista.
- Al arrancar se calibra un perfil de hardware: en la RTX 4090 de esta máquina, «Workstation,
  14 651 M triángulos/s».

## Cómo se traslada a Maker Studio

Maker Studio ya tiene buena parte de lo que rodea al esculpido:
- escena ECS, render wgpu con PBR y toon, Lumen y normal maps;
- deshacer, importación glTF, biblioteca y exportación;
- scripting en Rust y modo agente.

Falta el núcleo de esculpido. Propuesta:

### Modos del editor, como en Blender

La barra de herramientas del visor cambia de **Objeto** a **Esculpir** (más adelante Pintar,
UV, Animar…). Al entrar en Esculpir:
- el objeto seleccionado (una malla del proyecto o una primitiva) se convierte en una malla
  editable;
- el visor pasa a las herramientas de esculpido;
- los paneles cambian a pinceles y ajustes.

Al salir, la malla se guarda como asset del proyecto (`.glb`) y la entidad la referencia.

### Crate nuevo `maker_sculpt`, puro Rust y sin UI

| Pieza | Diseño |
|---|---|
| Malla | Estructura de arrays: posiciones, normales, máscara (`f32`), color y grupos opcionales. Índices de triángulos y adyacencia vértice→triángulos. |
| BVH | Árbol de AABB por hojas de unos 256 triángulos. Rayo, consulta de esfera y cilindro, reajuste solo de las hojas sucias. |
| Trazo | Espaciado de *dabs* (porcentaje del radio), suavizado del punto, línea recta con Mayús y modo *lazy*. |
| Pinceles | Rasgo `Pincel` con `dab(&mut Malla, &Dab)`. Standard, Clay, Clay Block, Smooth (Laplaciano y Taubin), Move (grab), Inflate, Crease, Sharp Crease, Flatten, Pinch, Trim, Layer, Nudge; Kelvinlets en una fase posterior. |
| Caída | Curva con el parámetro de −100 a 100 y alfas multiplicadas, con giro y mosaico. |
| Simetría | Espejos X/Y/Z, radial con N copias y centro local, aplicados a cada *dab*. |
| Máscara | Pincel de máscara, invertir, limpiar, difuminar y afilar. |
| Deshacer | Registro por trazo con las posiciones previas de los vértices tocados (dispersas) y presupuesto de memoria. |
| Paralelismo | `rayon` por hojas del BVH (ya llega a través de las dependencias, o se añade). |
| GPU | Subir solo los rangos de vértices sucios a un buffer de la malla esculpida (sin reconstruir el modelo). |

### Fases

1. **Base**:
   - malla, BVH, trazo y caída;
   - los 10 pinceles básicos más Trim, Layer y Nudge;
   - simetría X/Y/Z y radial;
   - máscara y deshacer;
   - modo Esculpir en el editor con estante de pinceles y atajos al estilo SculptForge;
   - guardar como `.glb`.
2. **Topología**: dyntopo (dividir, colapsar, girar, relajar), subdivisión de Loop con niveles,
   remallado por vóxeles (SDF y *marching cubes*) y diezmado.
3. **Detalle**: alfas generadas y por imagen, sellos (*drag* y *slide*), VDM, Kelvinlets, capas
   de esculpido y grupos de caras con visibilidad.
4. **Pintura**: pintura de vértices, de textura y PBR, UVs y horneado de normal maps de la malla
   alta a la baja.
5. **Automatización**: órdenes con nombre (`stroke`, `deform_*`) expuestas a los scripts y al
   modo agente.

### Qué *no* se trae

- Los textos, las imágenes y el código de SculptForge.
- Su IA, el enlace con el móvil, los *splats* y los puentes con Maya: no forman parte del
  esculpido.
