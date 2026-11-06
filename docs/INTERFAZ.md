# Interfaz de juego

Menús, HUD y pantallas del juego, en la línea de UGUI (Unity), los `Control`
de Godot y UMG (Unreal). El aspecto lo pone cada proyecto con estilos, temas,
imágenes, fuentes y widgets propios. El motor solo trae un aspecto neutro de
reserva, y un **tema inicial** que se copia al proyecto para editarlo.

## Conceptos

Los elementos de interfaz son **entidades de la escena** con componentes de
interfaz. Por eso salen en la jerarquía, se anidan por padre/hijo, se guardan
en el `.mscene` y pasan por deshacer/rehacer como todo lo demás.

| Componente   | Qué es |
|--------------|--------|
| `Lienzo`     | Raíz de una interfaz en pantalla. Tiene resolución de referencia, modo de escala, orden y tema propio opcional. |
| `RectUi`     | Rectángulo del elemento: anclas, desplazamientos, pivote, rotación, escala, visibilidad, interacción, opacidad y recorte de hijos. Funciona como el `RectTransform` de Unity. |
| `Contenedor` | Coloca a los hijos en caja horizontal, caja vertical o rejilla. Funciona como los `Container` de Godot. |
| `WidgetUi`   | Qué se dibuja y cómo reacciona: Panel, Imagen, Texto, Botón, Barra, Deslizador, Casilla, Campo de texto o Personalizado. Lleva el estilo y los retoques locales. |

Un elemento sin `WidgetUi` no dibuja nada. Sirve para agrupar o como contenedor.

Crear: **jerarquía → Crear → Interfaz**. Ahí están Lienzo, Panel, Imagen,
Texto, Botón, Barra, Deslizador, Casilla, Campo de texto, Máscara, Caja
horizontal, Caja vertical, Rejilla y Personalizado. Si no hay ningún lienzo,
se crea uno. Los elementos nuevos van dentro del elemento de interfaz
seleccionado o del primer lienzo.

## Lienzo y escala

El lienzo ocupa la vista del juego. Sus unidades salen del modo de escala:

- **Con la pantalla**: el lienzo mide la resolución de referencia (por defecto
  1920×1080) y se escala para llenar la pantalla. **Igualar** decide qué
  dimensión manda: 0 = el ancho, 1 = el alto. Los valores intermedios mezclan
  en escala logarítmica, como `CanvasScaler` de Unity.
- **Píxel constante**: una unidad equivale a `escala` píxeles, sea cual sea la
  pantalla.

Los lienzos se dibujan por **orden** (de menor a mayor). Si el lienzo tiene un
**tema**, ese tema sustituye al del proyecto dentro del lienzo.

## Anclas, pivote y desplazamientos

Cada `RectUi` se coloca respecto al rectángulo de su padre:

- **Anclas** `ancla_min`/`ancla_max` (0..1, con *y* hacia abajo). Son puntos
  del padre a los que se pegan las esquinas del elemento. Si coinciden en un
  eje, el elemento tiene tamaño fijo en ese eje. Si difieren, el elemento se
  estira con el padre.
- **Desplazamientos** `offset_min`/`offset_max`: distancia de cada esquina a su
  ancla, en unidades del lienzo. Con anclas que coinciden, el Inspector los
  muestra como *X · Ancho* e *Y · Alto*. Si no, como márgenes.
- **Pivote**: punto del elemento alrededor del cual gira y escala. Es también
  el punto que se coloca en el ancla.
- **Rotación** (grados) y **escala**: se aplican al elemento y a sus hijos. El
  ratón también acierta en los elementos girados.

El menú **Anclas ▾** del Inspector tiene la rejilla de preajustes de Unity
(4×4: izquierda/centro/derecha/estirar por arriba/medio/abajo/estirar).
Cambiar de ancla conserva el rectángulo en pantalla. Con **Alt**, el elemento
se lleva también al ancla. Con **Mayús**, el pivote pasa al ancla.

**Visible**, **Interactuable** y **Opacidad** se heredan, como un
`CanvasGroup`: un padre oculto oculta a sus hijos, y la opacidad se multiplica
hacia abajo. **Recortar hijos** convierte el elemento en máscara: los hijos
solo se ven dentro de su rectángulo.

## Contenedores

El `Contenedor` coloca a los hijos directos con `RectUi` y pasa por alto sus
anclas y desplazamientos. Un hijo con **Ignorar contenedor** sale del reparto.

- **Horizontal / Vertical**: uno tras otro, con **relleno** interior y
  **espacio** entre hijos.
  - **Alinear** sitúa el grupo en el eje principal.
  - **Alinear cruzado** sitúa a cada hijo en el otro eje. **Estirar** le da
    toda la altura (o anchura).
  - **Expandir hijos** reparte el espacio libre según el **flex** de cada hijo.
- **Rejilla**: celdas de tamaño fijo en `columnas` columnas, con alineación en
  los dos ejes.
- **Ajustar al contenido**: el contenedor toma el tamaño que suman sus hijos,
  solo en los ejes en que su tamaño es fijo, y conserva el pivote. Un Texto
  con **Tamaño del texto** activado mide lo que ocupa su texto.

Los contenedores se anidan: un menú puede ser una caja vertical de cajas
horizontales.

## Widgets

| Widget          | Datos | Partes con estilo propio |
|-----------------|-------|--------------------------|
| Panel           | — | — |
| Imagen          | imagen (del proyecto o `biblioteca://`), tinte, conservar aspecto, 9-slice | — |
| Texto           | texto, alineación horizontal y vertical, partir líneas, ajustar tamaño | — |
| Botón           | texto | — |
| Barra           | valor, mín., máx., dirección | `relleno` |
| Deslizador      | valor, mín., máx., paso, dirección | `relleno`, `tirador` |
| Casilla         | activa, texto | `marca` |
| Campo de texto  | texto, marcador, máx. caracteres | `cursor`, `marcador` |
| Personalizado   | lo dibuja un script (ver más abajo) | — |

Los botones, deslizadores, casillas y campos cambian de **estado**: `normal`,
`encima`, `pulsado`, `desactivado` (si no es interactuable) y `foco`.

## Estilos

Una **hoja de estilos** es un JSON en `ui/estilos/*.mestilo` con estilos con
nombre. Cada estilo define propiedades por estado y, si las tiene, por parte.
También puede **heredar** de otro estilo:

```json
{
  "estilos": {
    "boton_peligro": {
      "hereda": "boton",
      "normal":  { "fondo": { "Degradado": { "desde": [0.9, 0.2, 0.2, 1], "hasta": [0.6, 0.05, 0.1, 1], "angulo": 90 } },
                   "radio": [14, 0, 14, 0], "transicion": 0.15 },
      "encima":  { "escala": 1.05, "sombra": { "color": [1, 0.2, 0.2, 0.5], "desplazamiento": [0, 4], "difuminado": 16, "expansion": 0 } },
      "pulsado": { "escala": 0.96 }
    },
    "barra_vida": {
      "normal": { "fondo": { "Color": [0.1, 0.1, 0.1, 0.8] }, "radio": [6, 6, 6, 6] },
      "partes": { "relleno": { "normal": { "fondo": { "Color": [0.2, 0.9, 0.4, 1] } } } }
    }
  }
}
```

| Propiedad | Valor |
|-----------|-------|
| `fondo` | `"Ninguno"`, `{"Color": rgba}` o `{"Degradado": {desde, hasta, angulo}}` |
| `imagen`, `nueve`, `tinte` | imagen de fondo, bordes 9-slice (izq., arriba, der., abajo, en píxeles de la imagen) y tinte |
| `borde_color`, `borde_ancho` | borde interior |
| `radio` | radio de cada esquina (sup. izq., sup. der., inf. der., inf. izq.) |
| `sombra` | `color`, `desplazamiento`, `difuminado`, `expansion` |
| `relleno` | margen interior (texto del botón, del campo…) |
| `fuente`, `tam_texto`, `color_texto` | fuente TTF/OTF del proyecto, tamaño y color |
| `contorno_texto`, `sombra_texto` | `color`, `ancho`, `desplazamiento` |
| `escala`, `desplazamiento`, `opacidad` | transformación visual del estado; no afecta al layout |
| `transicion` | segundos para fundir colores, escala, desplazamiento y opacidad al entrar en el estado |

Los colores se escriben en sRGB, como se ven en el selector. El motor los pasa
a lineal y mezcla en lineal, con alfa premultiplicado.

## Temas

Un **tema** (`ui/temas/<nombre>.mtema`) asigna a cada tipo de widget su
estilo por defecto. Puede heredar de otro tema:

```json
{ "hereda": "base", "tipos": { "Boton": "boton_peligro", "Texto": "texto_grande" } }
```

El tema del proyecto se elige en la pestaña **Estilos**, y se guarda como
`"tema_ui"` en `project.maker`. Cambiarlo cambia el aspecto de toda la
interfaz. Un lienzo puede usar otro tema, y un script puede cambiarlo en
marcha con `ui_set_theme`.

### Orden de resolución

Para cada propiedad gana la primera capa que la define:

1. el **retoque local** del elemento;
2. el **estilo del elemento** y los estilos que hereda;
3. el **estilo del tema** para ese tipo de widget y su herencia, incluida la
   de los temas;
4. el **valor neutro del motor**.

Dentro de cada capa, el estado actual gana a `normal`. `pulsado` cae en
`encima` antes que en `normal`. Las partes siguen el mismo orden con su propio
bloque `partes.<parte>`.

## Retoques locales

En el Inspector, **Retoques locales** muestra las propiedades del widget por
estado y parte, con el valor ya resuelto. Al editar una propiedad, esa
propiedad pasa a ser un retoque del elemento: sale en negrita y color de
acento, con un **↺** para quitarla. Las demás siguen viniendo del estilo o del
tema. Los retoques se guardan en la escena dentro del widget, en `local`.

## Editor de estilos

Es la pestaña **Estilos** del panel inferior. Se abre también con un doble
clic en un `.mestilo` o un `.mtema` en Contenido, o con **Editar** junto al
estilo en el Inspector. Tiene:

- la lista de hojas, estilos y temas, con opciones para crear, duplicar,
  renombrar y eliminar (renombrar actualiza las referencias de la escena);
- el editor de estilo: hereda de, widget, estado y parte, y la rejilla de
  propiedades. Lo que define el estilo sale marcado, con ↺;
- el editor de tema: qué estilo usa cada tipo de widget;
- una **vista previa en vivo** con el widget en sus cinco estados, dibujada
  con el mismo pase que el juego.

Sin estilos en el proyecto, **Crear tema inicial** copia `ui/estilos/base.mestilo`
y `ui/temas/base.mtema` para tener un punto de partida.

Las ediciones de estilos y temas entran en el historial (Ctrl+Z) junto con la
escena, y se escriben a disco al confirmar cada cambio.

## Fuentes

Cualquier `.ttf` u `.otf` del proyecto sirve para la propiedad `fuente` (ruta
relativa, p. ej. `ui/fuentes/Mia.ttf`). Las métricas (ascendente,
descendente, interlineado y avances) se leen del archivo. Los glifos se
rasterizan en CPU con cobertura exacta y se guardan en un atlas que solo sube
las filas nuevas. Sin `fuente`, se usa la del motor (Ubuntu Light).

## Plantillas

Una **plantilla** (`ui/plantillas/*.mui`) es un subárbol de interfaz guardado
como asset, como un *prefab* de Unity o una escena instanciada de Godot.

- **Guardar como plantilla**: clic derecho en un elemento de la jerarquía. El
  subárbol pasa a ser una instancia de la plantilla nueva.
- **Instanciar**: arrastra el `.mui` desde Contenido a la jerarquía o a la
  vista, o haz doble clic en él.
- **Retoques por instancia**: lo que cambies en una instancia queda anotado
  en su raíz. Al abrir la escena, o al aplicar cambios a la plantilla, todo lo
  demás se pone al día con la plantilla. La sección **Plantilla** del
  Inspector lista los retoques, con un ↺ en cada uno. Tiene también los
  botones **Aplicar a la plantilla**, **Revertir todo** y **Desvincular**.

Las instancias se guardan enteras en la escena: el juego no necesita leer la
plantilla para cargarla.

## Edición en la vista

- **Interfaz** (barra de la vista) dibuja la interfaz sobre el 3D, en edición
  y en Play.
- **2D** activa la edición de la interfaz:
  - clic para seleccionar el elemento más alto bajo el ratón;
  - arrastrar para moverlo;
  - las asas de esquinas y lados lo redimensionan;
  - **Ctrl** ajusta a múltiplos de 8 unidades.

  Los elementos que coloca un contenedor no tienen asas. Cada arrastre es un
  único paso de deshacer.

## Entrada

En Play y en el juego exportado, la interfaz ve la entrada antes que los
scripts.

- **Ratón**: el clic que empieza sobre un elemento visible de la interfaz no
  llega al juego mientras se mantiene. `ui_pointer_over()` dice si el puntero
  está sobre la interfaz.
- **Deslizadores**: se arrastran con el ratón.
- **Campos de texto**: toman el foco al hacer clic. Con el foco se quedan
  todo el teclado, aceptan texto y Retroceso, e Intro envía.
- **Teclado**:
  - **Tab** y **Mayús+Tab** pasan el foco entre los controles visibles.
  - Con un control enfocado, Intro o Espacio lo pulsan y las flechas mueven un
    deslizador. Esas teclas no llegan al juego; las demás sí.
  - **Esc** quita el foco.

## Scripting

Los elementos se buscan por nombre. Los eventos se **consultan** en el
fotograma en que ocurren, igual que `key_pressed`:

```rust
use maker_api::prelude::*;

#[derive(Default)]
pub struct Menu { puntos: u32 }

impl Script for Menu {
    fn update(&mut self, ctx: &mut Ctx) {
        if let Some(b) = ctx.ui("Jugar") && ctx.ui_clicked(b) {
            self.puntos += 10;
        }
        if let Some(t) = ctx.ui("Puntos") {
            ctx.ui_set_text(t, &format!("{}", self.puntos));
        }
        if let Some(v) = ctx.ui("Volumen") && ctx.ui_changed(v) {
            ctx.log(format!("volumen {}", ctx.ui_value(v)));
        }
        if ctx.key_pressed(Key::Escape) && let Some(p) = ctx.ui("Pausa") {
            let ver = !ctx.ui_visible(p);
            ctx.ui_set_visible(p, ver);
            ctx.set_time_scale(if ver { 0.0 } else { 1.0 });
        }
    }
}
```

| Función | Qué hace |
|---------|----------|
| `ui(nombre)` | elemento por nombre |
| `ui_set_text` / `ui_text` | texto de Texto, Botón, Casilla o Campo |
| `ui_set_value` / `ui_value` | valor de Barra o Deslizador; la Casilla vale 0 o 1 |
| `ui_set_visible` / `ui_visible`, `ui_set_interactable`, `ui_set_opacity` | visibilidad, interacción y opacidad de grupo |
| `ui_set_style(e, "nombre")` | cambia el estilo (`""` = el del tema) |
| `ui_set_theme("nombre")` | cambia el tema de todo el juego |
| `ui_set_image(e, ruta)` | imagen de una Imagen (o de fondo en otros widgets) |
| `ui_set_color(e, rgba)` | color del texto, tinte de imagen o fondo, como retoque local |
| `ui_clicked`, `ui_changed`, `ui_event(e, ui_event::HOVER_ENTER…)` | eventos: `CLICKED`, `CHANGED`, `HOVER_ENTER`, `HOVER_EXIT`, `PRESSED`, `RELEASED`, `SUBMITTED` |
| `ui_rect(e)` | rectángulo tras el layout (unidades del lienzo) |
| `ui_set_position`, `ui_set_size` | mueve o redimensiona respecto a las anclas |
| `ui_instantiate(ruta_mui, padre)` | instancia una plantilla (`Entity::NONE` = primer lienzo) |
| `destroy(e)` | borra un elemento y sus hijos |
| `ui_pointer_over()` | ¿el puntero está sobre la interfaz? |
| `set_time_scale(s)` | escala el tiempo del juego (0 = pausa); la interfaz sigue respondiendo |
| `ui_draw(e, \|p\| …)` | dibujo propio de un widget Personalizado |

### Widgets personalizados

Un widget **Personalizado** lo dibuja un script con una API inmediata, en
unidades del lienzo y desde su esquina superior izquierda. Lo dibujado se
queda hasta la siguiente llamada:

```rust
let Some(g) = ctx.ui("Indicador") else { return };
let r = ctx.ui_rect(g);
let (cx, cy, rad) = (r.w / 2.0, r.h / 2.0, r.w.min(r.h) / 2.0 - 4.0);
let v = self.velocidad / 200.0;
ctx.ui_draw(g, |p| {
    p.ring(cx, cy, rad, 3.0, [1.0, 1.0, 1.0, 0.3]);
    p.arc(cx, cy, rad - 6.0, 135.0, 135.0 + 270.0 * v, 10.0, [1.0, 0.6, 0.1, 1.0]);
    p.text_centered(0.0, cy - 16.0, r.w, 32.0, 28.0, &format!("{:.0}", self.velocidad), [1.0; 4]);
});
```

Primitivas disponibles:

- `rect`;
- `rounded_rect` y `rounded_rect_outline`;
- `circle` y `ring`;
- `arc` (grados, 0 = derecha, sentido horario);
- `line`;
- `image`;
- `text` y `text_centered`.

Todas usan el mismo pase que los demás widgets (SDF), así que salen suaves a
cualquier escala. La demo dibuja así su radar.

## Render

La interfaz se dibuja **después** de la escena y del posprocesado, sobre la
imagen final, y no se posprocesa. El pase está en `maker_render::ui`:

- un solo pipeline de quads instanciados;
- el sombreador evalúa por píxel una SDF de caja con radio por esquina para
  el relleno, el degradado, el borde y la sombra difuminada;
- imágenes y 9-slice;
- glifos desde el atlas, con contorno y sombra;
- el destino es sRGB: el resultado se mezcla en lineal con alfa
  premultiplicado.

La lista de dibujo se agrupa en lotes por imagen y recorte. Un lienzo sin
imágenes ni máscaras se dibuja con **una sola llamada**. Los recortes usan
*scissor*.

El layout y los estilos resueltos solo se recalculan cuando algo cambia. La
lista de dibujo se rehace cuando cambia el layout o hay una transición en
marcha. En régimen estable no se reserva memoria. El búfer de instancias de la
GPU crece por potencias de dos. Las imágenes se cargan una vez, con mipmaps,
y su tamaño real se lee del archivo.

## Exportar

Exportar copia la carpeta `ui/` entera (estilos, temas, plantillas y
fuentes). Copia también las imágenes y fuentes que usan los widgets y los
estilos aunque estén fuera de `ui/`, y las de la biblioteca integrada que
hagan falta. El reproductor carga el tema del proyecto igual que el editor.

## La demo

`maker-studio --new-demo DIR` crea un proyecto con:

- el tema propio **neon** (`ui/estilos/neon.mestilo`, `ui/temas/neon.mtema`):
  degradados, esquinas asimétricas, brillos con sombra y botones que se
  desplazan y crecen al pasar el ratón;
- la fuente **Hack** del proyecto, en `ui/fuentes/`;
- el HUD del script `Hud`:
  - puntos por cada caja derribada;
  - barra de vida, que baja al caer del mapa;
  - un **radar** dibujado por el script;
  - un menú de pausa con **Esc**: pausa el tiempo y tiene botones *Continuar*
    y *Reiniciar puntos*, y una casilla *Radar*.

## Limitaciones

- **Sin lienzo en el mundo 3D**: solo hay lienzos en pantalla.
- **Sin vistas con desplazamiento**: no hay *scroll views*, y la rueda no se
  usa. Para listas largas, usa máscaras y mueve el contenido desde un script.
- **Sin mando**: no hay navegación con mando, y la de teclado es por orden de
  la jerarquía (Tab), no espacial.
- **Texto sin *shaping***: no hay kerning, ligaduras ni escrituras complejas
  (árabe, índicas). Las líneas se parten por espacios.
- **Campo de texto básico**: añade al final y borra con Retroceso, sin mover
  el cursor, sin selección y sin portapapeles.
- **Máscaras con rotación**: con un padre girado, la máscara recorta por el
  rectángulo que lo envuelve, alineado con la pantalla.
- **Fuentes del sistema**: no se leen; la fuente tiene que estar en el
  proyecto.
