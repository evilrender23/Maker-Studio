# Sombreado

Cada material elige un modelo de sombreado en el Inspector (componente
**Malla → Sombreado**). El Inspector solo muestra los parámetros del modelo
elegido. Desde un script: `ctx.set_shading(e, Shading::Toon)`.

| Modelo | Uso |
|--------|-----|
| **PBR** | Metal-rugosidad (GGX). Por defecto; las escenas antiguas cargan así. |
| **Toon (cel)** | Estilo *The Legend of Zelda: Breath of the Wild*. |
| **Sin luz** | Color plano + emisión. Sustituye a la casilla «Sin luz» anterior (`unlit: true` se convierte al cargar). |
| **Plumilla** | Rayado a tinta sobre el color base («papel»). |

Todos los modelos usan como color base textura × factor del material glTF ×
color de vértice × color del Inspector; el toon aplica sus bandas sobre ese
color texturizado. Los materiales con recorte alfa descartan píxeles (también
en el mapa de sombras) y los de mezcla se dibujan después de los opacos,
ordenados de atrás adelante.

## Toon

- **Bandas** (2–4): niveles de luz del sol, sombra incluida. El terminador
  cae en N·L = 0; las bandas extra reparten la zona iluminada.
- **Suavidad**: ancho del borde entre bandas (0 = corte duro). También
  suaviza el borde de las sombras proyectadas, del brillo y de la luz de borde.
- **Sombra**: tinte de la zona oscura (multiplica el color base). Por defecto
  un gris azulado, nunca negro.
- Las **sombras proyectadas** del mapa de sombras se umbralizan, así que se
  funden con la banda oscura.
- **Brillo**: fuerza y tamaño de un brillo especular en banda (0 = sin brillo).
- **Borde**: color, fuerza y ancho de la luz de borde (fresnel), solo en el
  lado iluminado.
- **Contorno**: color y ancho en píxeles (referidos a 1080 p; adelgaza con la
  distancia). 0 lo desactiva.
- Las luces puntuales se posterizan en escalones en lugar de degradado.

## Plumilla

Trazos en espacio de pantalla: cuanto menos luz recibe un punto, más familias
de líneas (diagonal, diagonal cruzada, horizontal, diagonal densa) y más
gruesas. **Tinta** colorea los trazos y el contorno; **Separación** es la
distancia entre trazos en píxeles a 1080 p; **Suavidad** alarga la transición
entre capas.

## Contorno de casco invertido

Pase aparte con *cull* de caras frontales: las caras traseras se desplazan en
pantalla a lo largo de la normal suavizada (promedio de las normales de los
vértices que comparten posición) y asoman alrededor de la silueta. Se dibuja
tras todos los opacos y en un solo cambio de pipeline.

## Sombreado global

**Inspector → Entorno de la escena → Sombreado global** (sin nada
seleccionado). **Modelo** fuerza PBR, Toon, Sin luz o Plumilla en todos los
materiales de la escena con los parámetros que se muestran debajo (los mismos
controles que en el material). Cada material conserva su color base, textura,
alfa y emisión. **Ninguno** devuelve a cada material su propio modelo.

- Excepción por objeto: **Malla → Ignorar sombreado global** (aparece cuando
  la escena tiene un modelo global) mantiene el modelo y los parámetros del
  material.
- Se guarda en la escena (`environment.global_shading`) y pasa por
  deshacer/rehacer.
- Desde un script: `ctx.set_global_shading(Some(Shading::Toon))`;
  `ctx.set_global_shading(None)` lo quita.
- No cuesta nada: el renderizador sustituye el material al rellenar el
  uniforme de cada objeto, así que la agrupación por pipeline es la misma.

## Posprocesado

**Inspector → Entorno de la escena → Posprocesado**: una pila ordenada de
efectos. Cada uno tiene casilla de activo, **Intensidad** (0 = como si
estuviera apagado), sus parámetros, ⏶/⏷ para reordenar y ✕ para quitarlo.
**Añadir efecto** ofrece los integrados, los `.wgsl` de `shaders/post/` y
**Nuevo efecto…**. La pila se guarda en la escena (`environment.post`) y se
aplica en el visor del editor (botón **Posprocesado** de la barra del visor),
en Play, en `maker-player` y en los juegos exportados. Las miniaturas de
modelos no se posprocesan. La interfaz del juego se dibuja encima y tampoco.

| Efecto | Parámetros | Trabaja en |
|--------|-----------|------------|
| **Tonemapping** | curva (ACES, AgX, Ninguno = recorte), exposición en EV | — |
| **Corrección de color** | contraste, saturación, temperatura, matiz, lift/gamma/gain, LUT en tira N²×N | 0..1 |
| **Bloom** | umbral (HDR), radio (niveles de la cadena) | HDR |
| **Viñeta** | radio, suavidad, color | 0..1 |
| **Aberración cromática** | desplazamiento en px (a 1080 p) | 0..1 |
| **Grano** | tamaño en px | 0..1 |
| **Retro** | tamaño de píxel, niveles por canal (0 = sin posterizar) | 0..1 |
| **Enfoque** | radio | 0..1 |
| **FXAA** | — | 0..1 |
| **Contorno en pantalla** | color, grosor, umbral de distancia y de normal | 0..1 |
| **Niebla de altura** | color, densidad, altura, caída | HDR |
| **Profundidad de campo** | foco, rango nítido, desenfoque máximo | HDR |

La **Exposición** de la escena está en la misma sección: multiplica la luz
antes del tonemap.

### Cómo se ejecuta

1. Sin efectos activos la escena se dibuja como siempre: MSAA 4× resuelto a
   `Rgba8UnormSrgb` con el tonemap ACES en el propio shader. Cero pases extra,
   mismos píxeles (lo comprueba un test).
2. Con efectos, la escena se dibuja en `Rgba16Float` (MSAA resuelto) y la
   cadena alterna entre dos texturas HDR; el último pase escribe la salida
   sRGB. Los objetivos se crean solo al cambiar de tamaño y los efectos
   apagados no generan pases.
3. Si la pila no tiene un tonemap activo, se añade un ACES antes del primer
   efecto que trabaja en 0..1 (los de la columna «HDR» van antes). Si el
   tonemap queda el primero, se fusiona con el shader de la escena y no cuesta
   un pase. Solo cuenta el primer tonemap activo.
4. El bloom reduce la imagen a la mitad en cada nivel (hasta 8 o hasta 4 px)
   y la vuelve a ampliar sumando (filtro tienda 3×3).

### Efectos propios

Un efecto propio es un archivo `shaders/post/*.wgsl` del proyecto.
**Contenido → Nuevo efecto** o **Añadir efecto → Nuevo efecto…** lo crean
desde la plantilla y lo abren en el editor de código. Al guardarlo el
renderizador lo recompila solo (mira el archivo cada medio segundo). Si no
compila, el error (archivo:línea:columna) va a la Consola y al Inspector y el
efecto se omite: nunca se rompe el editor ni sale una imagen negra.

El archivo define el *entry point* de fragmento `efecto`:

```wgsl
// @param fuerza float 0.5 0 1
// @param tinte color 1 0.8 0.6

@fragment
fn efecto(in: PostIn) -> @location(0) vec4<f32> {
    let c = muestra(in.uv).rgb;
    let r = mix(c, c * tinte(), fuerza());
    return vec4<f32>(mix(c, r, post.intensidad), 1.0);
}
```

**Parámetros.** Cada línea `// @param nombre float defecto [mín máx]` o
`// @param nombre color r g b` (sRGB) es un deslizador o un selector de color
en el Inspector (como máximo 16). En el shader se leen con una función del
mismo nombre: `fuerza()` devuelve `f32` y `tinte()` `vec3<f32>` ya en lineal.

**Enlaces fijos** (los declara el preludio que se añade al compilar; no hay
que escribirlos):

| Grupo 0 | Nombre | Contenido |
|---------|--------|-----------|
| 0 | `entrada` | `texture_2d<f32>`: el resultado del efecto anterior (lineal, 0..1 tras el tonemap) |
| 1 | `muestreo` | `sampler` lineal con *clamp* |
| 2 | `profundidad_tex` | `texture_depth_multisampled_2d`: profundidad de la escena |
| 3 | `post` | uniforme `Post` |
| 4 | `extra` | `texture_2d<f32>`: la escena sin posprocesar (HDR lineal) |

`Post` tiene `resolucion` (ancho, alto, 1/ancho, 1/alto), `texel` (de la
entrada), `tiempo` (s), `intensidad` (la del Inspector), `escala`
(alto / 1080, para medir en píxeles de 1080 p), `hdr`, `inv_vista_proy`,
`camara` y `p` (los parámetros en bruto).

**Funciones del preludio:** `muestra(uv)`, `profundidad(uv)` (0..1, 1 =
cielo), `es_cielo(uv)`, `posicion_mundo(uv)`, `distancia(uv)` (m),
`normal_mundo(uv)` (reconstruida de la profundidad; no hay búfer de
normales), `luma(c)`, `a_srgb(c)`, `a_lineal(c)` y `ruido(p)`. El vértice es
`vs_post` (triángulo a pantalla completa, `in.uv` con (0,0) arriba a la
izquierda; `in.pos.xy` son píxeles).

El código del usuario va primero y el preludio detrás (en WGSL el orden de las
declaraciones no importa), así los números de línea de los errores son los del
archivo. Los efectos propios trabajan en 0..1 (después del tonemap).

El proyecto de demostración trae dos ejemplos apagados en la pila: `crt.wgsl`
(curvatura, líneas de barrido y máscara de fósforo) y `tramado.wgsl`
(Bayer 4×4 con paleta de dos colores). Exportar el juego copia los efectos
propios y las LUT que usan las escenas.

## Limitaciones

- La plumilla está en espacio de pantalla: los trazos no se pegan a la
  superficie al mover la cámara.
- En caras planas vistas de canto, la luz de borde ilumina la cara entera.
- El contorno sigue la silueta de la malla; no dibuja aristas interiores
  (pliegues) ni contornos en planos de una sola cara.
- El MSAA se resuelve antes del tonemap cuando hay posprocesado: en bordes
  muy contrastados (una luz intensa contra el cielo) el antialiasing es algo
  peor que sin efectos; FXAA lo suaviza.
- Los efectos que usan profundidad leen la primera muestra del MSAA y
  reconstruyen la normal de la profundidad (no hay búfer de normales).
- La rejilla y el contorno de selección del editor pasan por la cadena de
  posprocesado (el tonemap y los efectos los alteran un poco).
