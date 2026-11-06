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

## 4. Diseño del modo

**Entrada**: en Contenido o en el inspector de una entidad con malla, *Texturizar*. Abre el modo
con el `.glb` del asset; si los UV se solapan más de un 5 %, propone UV únicos.

**Distribución** (mismo esqueleto de paneles que el resto del editor):

- izquierda: materiales, materiales inteligentes, máscaras inteligentes y alfas de pincel;
- centro: viewport 3D (renderer de TextureCraft dentro de un callback de egui) y vista UV 2D;
- derecha: texture sets, pila de capas (pintura/relleno/carpeta, máscaras, efectos) y
  propiedades de la herramienta o capa;
- barra de herramientas: pincel, borrador, proyección, relleno de polígono, difuminar, clonar,
  cuentagotas, simetría.

**Salida** (*Aplicar al asset*):

1. guarda `<asset>.tcraft` junto al asset para reeditar;
2. exporta con el preset *glTF PBR Metal Roughness* a `texturas/<asset>_*.png`;
3. actualiza el material de Maker (color base, normal y, cuando el renderer los admita,
   ORM y emisión) y recarga la miniatura.

**Hueco en el renderer de Maker**: hoy solo lee `base_tex` y `normal_tex`; metal y rugosidad son
escalares por material. Sin un mapa ORM (oclusión/rugosidad/metal) y otro de emisión se pierde lo
pintado en esos canales.

## 5. Fases

- [x] **F0. Estudio y prueba de integración**: este documento y el crate `maker_textura`.
- [ ] **F1. Renderer**: `metallicRoughnessTexture`, `occlusionTexture` y `emissiveTexture` en
      `maker_render` e importación glTF.
- [ ] **F2. Sesión**: `maker_textura` abre un asset (con UV únicos opcionales), guarda/carga
      `.tcraft` y aplica el resultado a la biblioteca.
- [ ] **F3. Viewport**: callback de egui con el `Renderer` de TextureCraft compartiendo el
      `Device` de eframe; sincronización de teselas por frame; navegación igual que el viewport
      de Maker.
- [ ] **F4. Pintura**: pincel/borrador con tamaño, flujo, dureza y espaciado; deshacer; selector de
      canales.
- [ ] **F5. Capas y materiales**: pila de capas, máscaras, estantería de materiales y
      materiales inteligentes con arrastrar y soltar.
- [ ] **F6. Horneado y exportación**: panel de horneado y presets de exportación.
- [ ] **F7. Agente**: exponer los comandos de TextureCraft a la herramienta del agente de Maker.
