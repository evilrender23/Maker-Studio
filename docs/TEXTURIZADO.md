# Modo Texturizado

Objetivo: que Maker Studio texturice sus propios assets 3D (pintar PBR por capas sobre la malla,
hornear mapas, materiales inteligentes, exportar) sin salir del editor, como el modo Texture Paint
de Blender pero al nivel de un pintor de texturas dedicado. Primer paso hacia un motor
"todoterreno".

Referencia estudiada: **TextureCraft 0.2.0**, el flatpak `ai.storyteller.texturecraft` instalado en
este equipo.

## 1. Ingeniería inversa: qué se observó

| # | Pregunta | Evidencia | Resultado |
|---|---|---|---|
| 1 | ¿Qué es el artefacto? | `flatpak info`, `manifest.json`, `file` | Dos ELF x86-64 *stripped*: `texturecraft` (GUI, 52,9 MB, sha256 `2f7fa74b…`) y `texturecraft-cli` (16,8 MB, sha256 `e1ec871b…`). Runtime freedesktop 26.08, permisos solo `dri`, Wayland/X11, Documentos e Imágenes. |
| 2 | ¿Con qué se compiló? | REA `inspect_rust_binary` (evidencia `ev_b11addb3…`) | Sin metadatos `cargo-auditable`. Commit de rustc inferido `58cf8f9c35b3…`, el mismo que el rustc 1.99.1 de este equipo. |
| 3 | ¿Existe el código fuente de ese binario? | `--version` → `0.2.0 (e01dac2 2026-11-01)`; rutas `crates/*/src/*.rs` incrustadas en el binario | Sí: `github.com/storytold/texturecraft`, commit `e01dac2`. Las rutas del binario coinciden con los crates del repo (`core texel geom compose bake paint formats engine render automation ui-egui`), más `craft-ui`/`craft-window` (craft-libs). Licencia **MIT OR Apache-2.0**. |
| 4 | ¿Funciona con los assets de Maker? | Ejecución real de `texturecraft-cli` sobre `Prop_Crate.glb` | `new --sample` (horneado + material inteligente) 0,9 s; `export` → 6 PNG (baseColor, metallic, roughness, height, normal, AO); `render` 800×600 en 0,70 s en la RTX 4090. |
| 5 | ¿Cómo son los UV de la biblioteca? | `texturecraft-cli info` | Ver tabla abajo: casi todos los assets Quaternius **comparten UV** (trim sheets / atlas). |
| 6 | ¿Se puede incrustar en Maker? | Prueba real: crate `maker_textura` en el workspace | Compila: un solo `wgpu 30` en `Cargo.lock` (comparte `Device` con eframe); glam 0.30 convive con nuestro 0.33. El test carga `Prop_Crate.glb`, hornea, aplica `steel_painted_worn` y exporta 6 mapas (`cargo test -p maker_textura`). |

Límite: el análisis del binario es estático (identidad, compilador, rutas); el comportamiento se
comprobó ejecutando la CLI del flatpak y el motor desde el código fuente del mismo commit, no
desensamblando la GUI.

### UV de la biblioteca

| Asset | Triángulos | Texture sets | UV solapados | Fuera de 0..1 |
|---|---:|---:|---:|---:|
| `Prop_Crate` | 3 984 | 1 | 69,2 % | 112 |
| `Gun_Pistol` | 3 322 | 1 | 73,8 % | 0 |
| `Prop_Barrel1` | 2 352 | 1 | 0 % | 0 |
| `Female_Ranger` | 26 966 | 2 | 47,5 % | 0 |
| `Superhero_Male_FullBody` | 14 318 | 3 | 44,6 % | 156 |
| `Hair_Long` | 2 906 | 1 | 57,8 % | 0 |

Con UV solapados, una pincelada aparece en varios sitios a la vez. El modo necesita la opción
**"UV únicos"**: `texturecraft_geom::unwrap::auto_unwrap` (pública) genera un despliegue nuevo y el
asset texturizado se guarda como copia con sus propios UV, sin tocar el atlas original.

## 2. Cómo está hecho TextureCraft

```text
L0 core     modelo: Project, TextureSet, Layer (paint/fill/group/instance), Mask, Effect, 32 modos de fusión
L0 texel    TileImage: teselas 64×64 u16 dispersas y copy-on-write; padding UV; ruido
L0 geom     Mesh + MikkTSpace, BVH SAH, UvGBuffer, cámara orbital, auto-unwrap
L1 compose  compositor de la pila de capas en CPU (por tesela, rayon): la referencia de todo
L1 bake     AO, curvatura, grosor, normal/posición mundo, ID, normal/altura desde high-poly
L2 paint    trazo → sellos 3D (cruzan costuras UV), visibilidad desde cámara, commit
L2 formats  importación (glTF, FBX, DAE, USD, Alembic, OBJ, PLY, STL), exportación con presets, .tcraft
L3 engine   Session (documento + cachés + deshacer) y registro de comandos (135)
L3 render   renderer wgpu: PBR+IBL, vista UV, path tracing y horneado por GPU
L4 automation  MCP, JSON-lines, puente a la app
L5 ui-egui  la interfaz (intercambiable)
```

Ideas que importan para Maker:

- **G-buffer en espacio UV**: cada texel sabe qué triángulo y qué punto de la superficie es. Pintar,
  componer, máscaras procedurales, horneado y padding leen el mismo buffer.
- **Todo es un comando** (`engine::execute(session, id, json)`): menús, atajos, CLI y MCP usan los
  mismos ids. Nuestro agente puede texturizar con la misma API.
- **CPU como referencia, GPU para mostrar**: se recomponen solo las teselas sucias y se suben al
  renderer (`Session::update_for` + `take_uploads` + `Renderer::upload_tile`).
- **Deshacer = instantáneas**: copiar el documento cuesta un `Arc` por tesela.
- **La UI no tiene lógica**: `ui-egui` solo llama a comandos y a la API de trazos
  (`begin_stroke`/`continue_stroke`/`end_stroke`). Su viewport es un `egui_wgpu::CallbackTrait`
  que llama a `Renderer::prepare` y `Renderer::blit`.

Comandos por área: layer 26, tool 19, textureSet 12, viewport 11, file 10, mask 10, effect 9,
camera 8, bake 7, export 5, project 5, material 4, resource 3, edit 3.

## 3. Decisión

**Incrustar el motor de TextureCraft (L0–L3 + render) como dependencia y escribir nuestra propia
interfaz.**

| Opción | Pros | Contras |
|---|---|---|
| **A. Incrustar crates** (elegida) | ~35 000 líneas maduras (42 000 con tests); misma pila (Rust, wgpu 30, egui 0.37); licencia compatible; demostrado en el workspace | Dependencia git fijada a un commit; glam duplicado (convertir con arrays) |
| B. Reescribir desde cero | Control total | Meses de trabajo para alcanzar lo mismo |
| C. Abrir TextureCraft aparte | Cero código | No es un modo de Maker; ida y vuelta de archivos; flatpak con permisos de carpeta |

No usamos `texturecraft-ui-egui`: depende de forks de egui/winit (`[patch.crates-io]`) y de
`craft-ui`, y su aspecto no es el de Maker. La interfaz se escribe en `maker_editor` con nuestro
tema y nuestros iconos.

Licencia: TextureCraft se usa bajo MIT (a elegir entre MIT y Apache-2.0); hay que conservar su aviso
de copyright en los créditos de Maker.

## 4. El modo

### Abrir

- **Contenido** → clic derecho en un modelo (glTF, GLB, FBX, OBJ, USD…) → *Texturizar…*.
- **Inspector** de una entidad con modelo → *Malla* → *Texturizar…*. Al aplicar, la entidad pasa a
  usar el modelo texturizado.
- **Archivo → Texturizar asset…**, o doble clic en un `.tcraft` de Contenido (tipo
  *Texturizados*).

Antes de abrir un modelo se eligen:

| Opción | Qué hace |
|---|---|
| Resolución | Lado de las texturas de cada texture set (512–4096; luego se cambia por set). |
| UV únicos | Despliega los UV de nuevo, sin solapes. Se propone sola si más del 5 % de los UV se pisa (los trim sheets de la biblioteca). No está disponible en personajes con esqueleto: el modelo nuevo perdería la piel. |
| Partir de sus texturas | Una capa *Original* por texture set con el color base (o su color) y el normal map del asset, proyectados por sus UV; la capa de pintura queda encima. Sin sentido con UV únicos. |

Un modelo ya texturizado (dentro de `texturizados/`) retoma su proyecto.

### Distribución

Sustituye la del editor mientras está abierto (el panel del agente sigue disponible):

- **Barra superior**: deshacer/rehacer, vista 3D o UV, lo que muestra el visor (material, un
  canal, un mapa horneado, cuadrícula UV), *Hornear…*, *Exportar…*, *Guardar*,
  *Aplicar al asset* y *Salir*.
- **Izquierda, estante**: materiales, materiales inteligentes, máscaras inteligentes y alfas de
  pincel, con miniaturas y búsqueda. Clic aplica (el material encima de la capa seleccionada, la
  máscara a la capa, el alfa al pincel); los materiales y máscaras también se arrastran al visor
  o a una fila de la pila.
- **Centro, visor**: herramientas (pincel, borrador, proyección, relleno, difuminar, clonar,
  cuentagotas) y simetría X/Y/Z encima.
- **Derecha**: texture sets (visibilidad, resolución), pila de capas y propiedades.

### Visor

| Acción | Control |
|---|---|
| Pintar | Clic izquierdo y arrastrar (presión del lápiz si la hay) |
| Línea recta desde la última pincelada | Mayús + clic |
| Orbitar | Botón derecho, o Alt + izquierdo |
| Desplazar | Botón central, o Mayús + derecho |
| Acercar | Rueda |
| Tamaño del pincel | Ctrl + rueda, `[` y `]` |
| Origen del clonado | V + clic (herramienta Clonar) |
| Encuadrar | F |
| Herramientas | B pincel, E borrador, P relleno, I cuentagotas |
| Vista 3D / UV | Tab |
| Deshacer / rehacer / guardar | Ctrl+Z, Ctrl+Y (Ctrl+Mayús+Z), Ctrl+S |

En la vista UV se pinta sobre el despliegue; la rueda acerca y los botones central o derecho
desplazan.

### Capas

Filas de arriba abajo, con carpetas plegables. Ojo: visibilidad. Chip a la derecha: la máscara
(clic la selecciona para pintarla; Alt + clic la muestra sola en el visor). Doble clic renombra.
Arrastrar una fila sobre otra la coloca encima (dentro, si es una carpeta). Menú contextual:
renombrar, duplicar, agrupar, aplanar, subir/bajar, máscara (blanca, negra, inteligente,
generador, pintura, invertir, activar, quitar) y eliminar.

### Propiedades

- **Herramienta**: tamaño, flujo, opacidad, dureza, espaciado, ángulo, seguir trazo, solo caras
  visibles y ratón perezoso; con la capa (no la máscara) seleccionada, qué canales pinta y con
  qué valor. Pintando una máscara, el gris que deja.
- **Capa**: modo de fusión y opacidad por canal; en las de relleno, proyección y fuente de cada
  canal (uniforme o uno de los 22 procedurales, con escala y semilla); efectos del contenido y de
  la máscara con sus parámetros, visibilidad y opacidad; *+ Efecto* añade generadores (desgaste
  de bordes, suciedad, curvatura, oclusión…), filtros, niveles, rellenos o pintura.

Los deslizadores confirman al soltar: un arrastre es un solo paso de deshacer.

### Hornear y exportar

*Hornear…* calcula en otro hilo (trazado de rayos por GPU de TextureCraft) los mapas elegidos
(oclusión, curvatura, grosor, normal y posición del mundo, ID…) para todos los texture sets o el
activo; las máscaras de los materiales inteligentes los necesitan. *Exportar…* escribe texturas
con cualquier preset (glTF, Unreal, Unity, Godot, Blender…), formato y tamaño, o la malla en GLB.

### Aplicar al asset

Todo vive en `texturizados/<nombre>/` del proyecto:

1. guarda `<nombre>.tcraft` (escritura atómica) para seguir otro día;
2. exporta cada texture set con el preset *glTF PBR Metal Roughness* a `texturas/`
   (`BaseColor`, `OcclusionRoughnessMetallic`, `Normal` y, si hay emisión, `Emissive`);
3. escribe `<nombre>.glb`: el modelo de partida intacto (jerarquía, piel y animaciones) con cada
   material apuntando a esas texturas por nombre; con UV únicos, la malla desplegada;
4. si se abrió desde una entidad, le pone ese modelo (un paso de deshacer de la escena).

El renderer de Maker lee del glTF el color base, el normal, el metal/rugosidad, la oclusión y la
emisión ([SHADERS.md](SHADERS.md#mapas-pbr-del-gltf)), así que nada de lo pintado se pierde. La
biblioteca no se toca nunca.

### Sin ventana (agente y scripts)

`maker-studio textura <orden>` en la carpeta del proyecto:

| Orden | Qué hace |
|---|---|
| `info <asset>` | Triángulos, materiales, solape de UV, piel (JSON) |
| `comandos [filtro]` | Los comandos de TextureCraft con sus parámetros |
| `abrir <asset> [--resolucion N] [--uv-unicos] [--sin-original]` | Crea `texturizados/<nombre>/<nombre>.tcraft` |
| `ejecutar <x.tcraft> <id> [json]` | Un comando, y guarda |
| `lote <x.tcraft> <pasos.json>` | `[{"id", "params"}, …]`, y guarda |
| `aplicar <x.tcraft>` | Texturas y `<nombre>.glb` |
| `render <x.tcraft> <png> [--size AxB] [--yaw G]` | Vista previa |

La guía del modo agente lo explica, y al terminar un turno que toque `texturizados/` el editor
vuelve a leer los modelos y recarga el texturizado abierto si no tiene cambios propios.

## 5. Limitaciones

- Los UV únicos no se ofrecen en modelos con piel; esos se pintan con sus UV (si se solapan,
  lo pintado se repite).
- Con UV únicos se empieza desde cero: las texturas originales no encajan en el despliegue nuevo.
- El GLB aplicado conserva dentro las imágenes incrustadas del original aunque ya no las use.
- La proyección con plantilla (stencil), el trazado (Path), las calcomanías y las mallas de alta
  resolución para hornear están en los comandos (CLI y agente) pero aún no en la interfaz.
- La vista UV no dibuja el cursor del pincel.

## 6. Fases

- [x] **F0. Estudio y prueba de integración**: este documento y el crate `maker_textura`.
- [x] **F1. Renderer**: `metallicRoughnessTexture`, `occlusionTexture` y `emissiveTexture` en
      `maker_render` e importación glTF.
- [x] **F2. Sesión**: abrir un asset (UV únicos, capas originales), guardar/abrir `.tcraft` y
      aplicar (texturas + GLB que conserva piel y animaciones).
- [x] **F3. Viewport**: callback de egui con el `Renderer` de TextureCraft sobre el `Device` de
      eframe, sincronización de teselas con presupuesto por fotograma, vistas 3D y UV.
- [x] **F4. Pintura**: herramientas, pincel, canales, simetría, presión, deshacer.
- [x] **F5. Capas y materiales**: pila con máscaras y efectos, estante con miniaturas y arrastrar
      y soltar.
- [x] **F6. Horneado y exportación**: ventanas con progreso y presets.
- [x] **F7. Agente**: `maker-studio textura …` y la guía del modo agente.
