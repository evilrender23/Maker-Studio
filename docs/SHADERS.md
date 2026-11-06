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

## Limitaciones

- La plumilla está en espacio de pantalla: los trazos no se pegan a la
  superficie al mover la cámara.
- En caras planas vistas de canto, la luz de borde ilumina la cara entera.
- El contorno sigue la silueta de la malla; no dibuja aristas interiores
  (pliegues) ni contornos en planos de una sola cara.
