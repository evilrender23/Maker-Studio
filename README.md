# Maker Studio

Motor 3D con editor, hecho en **Rust** y con **scripting en Rust**. Toma las
buenas prácticas de Unity, Unreal y Godot (ECS por arquetipos, árbol de escena,
Play-en-editor con restauración, física a paso fijo, PBR, recarga en caliente
de código) en un motor pensado para rendir: todo es código nativo, también los
scripts de cada juego.

![Editor](docs/img/editor.png)

| | |
|---|---|
| ![Play en el editor](docs/img/play.png) | ![Juego exportado](docs/img/player.png) |
| **Play en el editor**: cámara del juego, scripts y física en vivo | **`maker-player`**: el juego exportado, sin editor |

## Qué tiene

- **Editor** (`maker-studio`): Project Hub, jerarquía con arrastrar y soltar,
  inspector de componentes, viewport 3D con gizmos de mover/rotar/escalar
  (local/global, ajuste a rejilla), selección por clic, cámara de vuelo estilo
  Unity/Unreal, explorador de contenido, consola, editor de scripts con
  resaltado, deshacer/rehacer, Play / Pausa / Paso con la escena restaurada al
  parar, ejecutar en ventana aparte y exportar el juego.
- **Motor** (`maker_engine`): ECS `hecs`, escenas `.mscene` en JSON legible,
  jerarquía padre/hijo, primitivas (cubo, esfera, plano, cilindro, cono,
  cápsula, toro) e importación **glTF/GLB**, física **Rapier** (cuerpos
  dinámicos, estáticos y cinemáticos; cajas, esferas, cápsulas, cilindros;
  raycast), entrada, bucle de juego con `fixed_update` a 60 Hz.
- **Render** (`maker_render`, wgpu → Vulkan/Metal/DX12/GL): PBR metal-rugosidad,
  sol con sombras PCF, 16 luces puntuales, cielo procedural, niebla, ACES,
  MSAA 4×, frustum culling, contorno de selección, rejilla infinita.
- **Scripting en Rust** (`maker_api`): cada proyecto tiene un crate `scripts/`
  que el editor compila (F7) y recarga en caliente; propiedades editables desde
  el Inspector; los pánicos se atrapan y salen en la consola. Ver
  [docs/SCRIPTING.md](docs/SCRIPTING.md).
- **Tema claro** de Hermes Agent Desktop (colores, radios y paleta de código
  extraídos de su CSS; tabla en `crates/maker_editor/src/theme.rs`).

## Compilar y ejecutar

Requisitos: Rust estable reciente (edición 2024) y una GPU con Vulkan, Metal o
DirectX 12 (OpenGL como último recurso). Los scripts de los proyectos se
compilan con el mismo `cargo`.

```bash
cargo build --release
./target/release/maker-studio                 # Project Hub
./target/release/maker-studio ruta/al/proyecto
./target/release/maker-studio --new-demo ~/MakerProjects/Demo --build-scripts
```

En el Hub, **Crear proyecto** con la plantilla **Demo jugable** genera una
escena con un personaje físico (WASD, Espacio para saltar, F para lanzar
bolas), una pila de cajas, una luz que flota, un anillo que gira y una cámara
que sigue al jugador. Pulsa **F7** para compilar sus scripts y **▶ Play**.

### Controles del viewport

| Entrada | Acción |
|---|---|
| Clic derecho + WASDQE | volar (Shift rápido, rueda = velocidad) |
| Alt + clic izquierdo | orbitar |
| Botón central | desplazar |
| Rueda | acercar / alejar |
| W · E · R | mover · rotar · escalar |
| Ctrl al arrastrar un gizmo | ajuste (0,5 m · 15° · 0,1) |
| F | encuadrar la selección |
| F5 / Ctrl+P | Play / Stop |
| F7 | compilar y recargar scripts |
| Ctrl+S · Ctrl+Z · Ctrl+Y · Ctrl+D · Supr · F2 | guardar · deshacer · rehacer · duplicar · eliminar · renombrar |

## Un proyecto

```
MiJuego/
├── project.maker          nombre y escena principal
├── scenes/main.mscene     escenas (JSON)
├── assets/models/         .gltf / .glb
└── scripts/               crate de Rust → biblioteca dinámica
    ├── Cargo.toml
    └── src/lib.rs
```

**Archivo → Exportar juego** crea `export/<nombre>/` con el ejecutable, las
escenas, los assets y la biblioteca de scripts; ese ejecutable abre el juego
sin argumentos.

## Pruebas

```bash
cargo test --workspace --release
cargo test --release -p maker_engine --test hot_reload -- --ignored   # compila y recarga scripts reales
```

Más detalle en [docs/ARQUITECTURA.md](docs/ARQUITECTURA.md) (qué se toma de
cada motor y por qué) y [docs/ROADMAP.md](docs/ROADMAP.md).

## Licencia

[MIT](LICENSE) © 2026 EvilRender23
