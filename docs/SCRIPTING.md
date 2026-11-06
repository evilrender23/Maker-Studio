# Scripting en Rust

Cada proyecto tiene una carpeta `scripts/` que es un crate de Rust normal
(`crate-type = ["cdylib"]`) que depende de `maker_api`. El editor lo compila con
`cargo build --release` (**F7**) y carga la biblioteca resultante **en caliente**:
no hace falta reiniciar el editor. Si compilas desde fuera (VS Code,
`cargo watch`…), el editor detecta el `.so`/`.dll` nuevo en un segundo y lo recarga.

## Un script

```rust
use maker_api::prelude::*;

#[derive(Default)]
pub struct Rotator {
    speed: f32,
}

impl Script for Rotator {
    // Propiedades editables en el Inspector (como [SerializeField] / @export).
    fn props(&mut self, p: &mut Props) {
        p.float("speed", &mut self.speed);
    }
    fn start(&mut self, ctx: &mut Ctx) {
        ctx.log("hola");
    }
    fn update(&mut self, ctx: &mut Ctx) {
        ctx.rotate_y(self.speed.to_radians() * ctx.dt);
    }
}

export_scripts!(Rotator);
```

Todos los tipos de `export_scripts!(...)` aparecen en **Inspector → Añadir
componente → Script**. Una entidad puede tener varios scripts.
**Contenido → Nuevo script** añade la plantilla y la registra en la lista.

## Ciclo de vida

| Método           | Cuándo                                        | Equivalente                          |
|------------------|-----------------------------------------------|--------------------------------------|
| `start`          | una vez, antes del primer `update`            | `Start` / `_ready` / `BeginPlay`     |
| `fixed_update`   | a 60 Hz, antes de cada paso de física         | `FixedUpdate` / `_physics_process`   |
| `update`         | cada fotograma                                | `Update` / `_process` / `Tick`       |
| `props`          | al leer/escribir propiedades desde el editor | `[SerializeField]` / `@export` / `UPROPERTY` |

Orden de un fotograma: `start` pendientes → N × (`fixed_update` → física) →
`update` → destrucciones diferidas → propagación de transforms.

## Propiedades

`p.float`, `p.int`, `p.bool`, `p.vec3`, `p.color`. Los valores que cambies en
el Inspector se guardan en la escena por nombre y se aplican al crear la
instancia; el botón ↺ vuelve al valor por defecto del código.

## API de `Ctx`

| Grupo        | Funciones |
|--------------|-----------|
| Tiempo       | `ctx.dt`, `ctx.time`, `ctx.frame`, `ctx.entity` |
| Consola      | `log`, `warn`, `error` |
| Transform    | `position`, `set_position`, `translate`, `rotation`, `set_rotation`, `rotate_y`, `rotate_local`, `scale`, `set_scale` (posición y rotación en espacio mundo) |
| Otras        | `find(nombre)`, `exists`, `position_of`, `set_position_of`, `rotation_of`, `set_rotation_of`, `set_color`, `set_shading(e, Shading::Toon)`, `spawn(Primitive, pos, flags)`, `destroy` |
| Entrada      | `key_down`, `key_pressed`, `axis(neg, pos)`, `mouse_down`, `mouse_delta` |
| Física       | `velocity`, `set_velocity`, `apply_impulse`, `apply_impulse_to`, `raycast(origen, dir, max)` |

`spawn_flags::DYNAMIC_BODY` / `STATIC_BODY` añaden cuerpo rígido y
colisionador al crear una primitiva.

## Por qué es rápido y seguro

- Los scripts son **código nativo optimizado** (`opt-level = 3`, LTO): no hay
  intérprete ni recolector de basura entre tu código y el motor.
- La frontera motor ↔ biblioteca es **C-ABI** (`#[repr(C)]`, `extern "C"`), así
  que no depende de la ABI inestable de Rust: puedes recompilar los scripts sin
  recompilar el motor. La versión de la ABI (`maker_api::ABI_VERSION`) se
  comprueba al cargar.
- Cada llamada a un script se ejecuta dentro de `catch_unwind`: un `panic!`
  aparece en la Consola, ese script se desactiva y el editor sigue vivo.
- La biblioteca se copia a un nombre temporal único antes de abrirla, así
  `cargo` puede reescribir el original con el editor abierto.
- `Destroy` es diferido (al final del fotograma), como en Unity, para no
  invalidar entidades mientras otros scripts aún corren.

## Prueba de punta a punta

`crates/maker_engine/tests/hot_reload.rs` crea un proyecto, compila sus
scripts, los ejecuta, cambia el código, recompila y comprueba el nuevo
comportamiento:

```bash
cargo test --release -p maker_engine --test hot_reload -- --ignored
```
