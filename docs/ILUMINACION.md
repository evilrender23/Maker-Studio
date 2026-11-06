# Iluminación global

**Inspector → Entorno de la escena → Iluminación global** (sin nada
seleccionado). La GI añade luz indirecta dinámica (la luz que rebota en las
paredes, los objetos emisivos que iluminan lo de alrededor, el cielo tapado
por el techo) y reflejos trazados. Todo se recalcula cada fotograma: mover
una luz, un objeto o la cámara no necesita ningún horneado.

| Ajuste | Qué hace |
|--------|----------|
| **Modo** | *Desactivada* (por defecto; ambiente plano y reflejo aproximado del cielo, como siempre) o *Lumen (software)*. Las escenas antiguas cargan desactivadas. |
| **Calidad** | Baja, Media, Alta o Épica (tabla abajo). |
| **Luz indirecta** | Multiplica la luz difusa que rebota. |
| **Reflejos** | Multiplica los reflejos trazados. |
| **Rebotes múltiples** | La luz rebota más de una vez (la radiancia del fotograma anterior se realimenta y converge en unos fotogramas). Apagado: un solo rebote. |

**Luz ambiente** (en la misma sección del Entorno) sigue mandando: es la
intensidad del cielo que ven los rayos que escapan.

Se guarda en la escena (`environment.iluminacion_global`) y pasa por
deshacer/rehacer. Funciona en el visor del editor, en Play, en
`maker-player` y en los juegos exportados. Las miniaturas no la usan.

En la barra del visor, **GI** la apaga solo en el visor (aparece cuando la
escena la tiene activada) y el desplegable de al lado cambia la vista:
**Imagen final**, **Vóxeles** (el volumen tal como lo ven los rayos),
**Solo luz indirecta** y **Solo reflejos**. Con consultas de marca de
tiempo disponibles, las estadísticas del visor muestran su coste en ms.

El proyecto de demostración (`--new-demo`) trae `scenes/cornell.mscene`: una
caja de Cornell con paredes roja y verde, una lámpara emisiva en el techo, un
cubo emisivo azul, una esfera metálica y un suelo pulido.

## Cómo funciona

Es un híbrido por pasos, en `crates/maker_render/src/gi/` (Rust + WGSL):

1. **Representación de la escena: un clipmap de vóxeles.** 3–4 cascadas de
   N³ vóxeles centradas en la cámara; cada una dobla el tamaño de vóxel de la
   anterior. Cada malla opaca o recortada se rasteriza tres veces (una por
   eje) a la resolución de la cascada y el fragmento suma con atómicos su
   albedo (último mip de la textura), normal, orientación sin signo y
   emisión. Un pase de cómputo lo resuelve en texturas 3D.
   - **Caché:** cada cascada guarda una huella de lo que contiene (matriz,
     material, malla y, con piel, la paleta de huesos). Solo se revoxeliza la
     cascada que cambió o cuyo origen saltó (los orígenes se ajustan a saltos
     de 4 vóxeles). Una escena quieta no revoxeliza nada.
2. **Inyección de luz** (cómputo, cada fotograma): radiancia saliente de cada
   vóxel = emisión + albedo/π · (sol con el mapa de sombras, o un cono hacia
   el sol fuera de él; luces puntuales con un cono de oclusión) + albedo ·
   luz indirecta leída con 6 conos del volumen del fotograma anterior
   (rebotes múltiples). Los vóxeles de una cascada gruesa que también cubre
   la fina son la media de sus 8 hijos, como un mip.
3. **Prepase:** normal, rugosidad y profundidad a la resolución del trazado.
4. **Recolección final** por píxel: N rayos difusos con distribución coseno,
   distintos cada fotograma.
   - Primero un trazado corto contra la profundidad de la pantalla; si choca
     con algo que mira hacia el rayo, toma la radiancia del vóxel alcanzado
     (detalle de contacto que el volumen no ve).
   - Si no, un cono por el clipmap. Al llegar a una superficie en una
     cascada gruesa, afina el choque en la cascada fina y toma la radiancia
     desde el lado del rayo: así una pared más fina que los vóxeles gruesos
     no deja pasar la luz de fuera.
   - Lo que escapa ve el cielo, tapado por lo que haya delante.
   - **Reflejos:** un rayo reflejado (perturbado según la rugosidad) con un
     trazado en pantalla más largo y refinado, después un cono cuya apertura
     crece con la rugosidad, después el cielo. Por encima de rugosidad 0,75
     se reutiliza la difusa.
5. **Acumulación temporal** con reproyección; cada muestra del historial se
   valida con el plano y la normal, así lo desocluido empieza de cero en vez
   de arrastrar luz de otro objeto. Los reflejos pulidos acumulan menos
   fotogramas. Después, **dos pasadas à-trous** 5×5 con paradas por
   geometría y luminancia.
6. **Sombreado:** el pase de color lee la GI con un bilineal guiado por la
   profundidad (sube la resolución sin sangrar entre objetos).
   - **PBR:** difusa indirecta × albedo y reflejos × Fresnel (sustituye al
     ambiente hemisférico y al reflejo del cielo).
   - **Toon:** la luz indirecta, posterizada en escalones y con su tono,
     hace de luz de relleno de las bandas.
   - **Plumilla:** su luminancia decide la densidad del rayado.
   - **Sin luz:** la ignora.
   - Los transparentes y los píxeles sin GI válida usan el ambiente de
     siempre.

Orden del fotograma: sombras → voxelizar (si algo cambió) → inyectar →
prepase → recoger → temporal + filtro → color → posprocesado → interfaz.

### Por qué vóxeles y no SDF de mallas

Lumen usa SDF por malla y un *surface cache*. Aquí se eligió un clipmap de
vóxeles porque:

- sirve para cualquier malla (glTF importados no cerrados, láminas de
  follaje, mallas con piel animadas) sin generar ni guardar un SDF por
  asset;
- se rehace en GPU en fracciones de milisegundo cuando algo se mueve (la
  voxelización completa de todas las cascadas cuesta 0,1–1,5 ms);
- da a la vez oclusión, radiancia filtrada (conos) y multirrebote con una sola
  estructura.

El precio es la resolución (0,09–0,25 m en la cascada fina) y las fugas por
paredes finas, que se atacan con el afinado en cascada fina, la media
orientada de los vóxeles gruesos y una opacidad reforzada en las cascadas
gruesas.

## Calidad

| Nivel | Cascadas × vóxeles | Vóxel fino | Alcance de la fina | Trazado en pantalla | Rayos difusos | Pasos de reflejo |
|-------|-------------------|-----------:|------------------:|---------------------|--------------:|-----------------:|
| Baja  | 3 × 48³  | 0,25 m | 12 m   | ¼ de resolución | 2 | 12 |
| Media | 4 × 64³  | 0,16 m | 10,2 m | ½ | 2 | 16 |
| Alta  | 4 × 96³  | 0,12 m | 11,5 m | ½ | 4 | 24 |
| Épica | 4 × 128³ | 0,09 m | 11,5 m | ½ | 6 | 32 |

La cascada gruesa llega a 8× el alcance de la fina (≈ 90 m en Media).
La calidad se ajusta a los límites reales del dispositivo (tamaño máximo de
búfer de almacenamiento y de textura 3D): si no cabe, baja la resolución del
volumen manteniendo su alcance. Sin compute o sin texturas de almacenamiento
la sección avisa y la escena se ve como sin GI.

### Coste medido

RTX 4090, Vulkan, 1920×1080, consultas de marca de tiempo de GPU
(`cargo run -p maker_render --example gi_bench`), media de 60 fotogramas
tras 30 de calentamiento, escena quieta:

| Nivel | Demo (`--new-demo`) | Caja de Cornell | Voxelizar todo (1.er fotograma) | Memoria de GPU |
|-------|------:|------:|------:|------:|
| Baja  | 0,42 ms | 0,22 ms | 0,06–0,07 ms | 28 MB |
| Media | 1,11 ms | 0,55 ms | 0,09–0,11 ms | 96 MB |
| Alta  | 2,96 ms | 1,27 ms | 0,17 ms | 203 MB |
| Épica | 5,24 ms | 3,18 ms | 0,74–1,50 ms | 411 MB |

Casi todo el coste es la recolección (rayos por píxel). La memoria solo se
reserva con la GI activa y se libera al apagarla; los recursos de pantalla
se rehacen al cambiar de tamaño.

## Limitaciones (comparación honesta con Lumen)

- **Resolución:** Lumen traza contra SDF por malla y un *surface cache* con
  la iluminación de cada superficie a varios texeles por metro. Aquí el
  detalle fuera de la pantalla está limitado al vóxel (0,09–0,25 m cerca,
  mucho más lejos): los objetos pequeños que no están en pantalla casi no
  ocluyen ni rebotan.
- **Fugas de luz:** con paredes más finas que dos vóxeles finos puede
  colarse algo de luz (en la prueba de la habitación cerrada, ≈ ¼ de la de
  la abierta, en vez de ~0). Paredes de 0,3 m o más gruesas funcionan bien
  en Media.
- **Vóxeles isótropos:** una sola radiancia por vóxel. Las caras opuestas
  de una lámina fina se resuelven hacia la cámara.
- **Superficies metálicas** rebotan en el volumen como si fueran difusas
  (con su color).
- **Reflejos:** fuera de la pantalla salen del volumen, así que son borrosos
  (del tamaño del vóxel); dentro de la pantalla toman la radiancia del
  vóxel alcanzado, no el color final del píxel. No hay reflejos de los
  transparentes ni sobre ellos.
- **Luces puntuales** se ocluyen en el volumen con un cono (sombra blanda
  y aproximada); en el sombreado directo siguen sin sombra, como antes.
- **Retardo temporal:** la luz indirecta tarda unos fotogramas en
  converger (hasta 24 de historial en la difusa) y los rebotes múltiples
  unos pocos más. Un cambio brusco (encender una luz) se ve en ≈ 0,1–0,4 s.
- **Fuera del alcance** del volumen solo hay cielo, sin oclusión.
- **Sin trazado de rayos por hardware.** wgpu 30 tiene
  `Features::EXPERIMENTAL_RAY_QUERY`, pero exige activar funciones
  experimentales del dispositivo (que hoy crean eframe y el reproductor) y
  construir y mantener estructuras de aceleración por malla y por
  fotograma. No se incluyó porque no era estable en el tiempo de esta
  versión; el modo por software funciona en cualquier GPU con compute.
- Sin GI en escenas de interfaz ni en miniaturas.

Lo verificado por pruebas sin ventana (`crates/maker_render/src/gi/tests.rs`):
pared roja que tiñe una caja blanca, cubo emisivo que ilumina el suelo,
habitación cerrada más oscura que abierta, suelo pulido que refleja un
objeto, caché estática (una escena quieta no revoxeliza; mover un objeto sí),
cambio de tamaño, todas las calidades, vistas de depuración y modelos de
sombreado, y que sin GI la imagen y los recursos son los de siempre. El
aspecto en el editor no se ha revisado en pantalla.
