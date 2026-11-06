# Esculpir

El visor tiene dos modos, como Blender: **Objeto** (mover, rotar y escalar entidades) y
**Esculpir** (modelar la malla de una entidad con pinceles, como en un programa de escultura
digital). Esta es la fase 1: pinceles, simetría, máscara y guardado. Cómo se diseñó y qué
vendrá después está en [investigacion/SCULPTFORGE.md](investigacion/SCULPTFORGE.md).

## Entrar y salir

- Selecciona una entidad con malla y pulsa **Tab**, o **Esculpir** al principio de la barra
  del visor. Tab (u **Objeto**) vuelve al modo Objeto.
- **Crear → Nueva escultura** crea una esfera y entra directamente.
- Al entrar:
  - una **primitiva** se convierte en una base esculpible de unos 60 000 vértices: la esfera,
    el cubo y el plano con celdas regulares; el cilindro, el cono, la cápsula y el toro
    subdividiendo su malla;
  - un **modelo** glTF/GLB se usa tal cual; los vértices repetidos en las costuras (UV,
    aristas duras) se sueldan para que la superficie no se abra, y al guardar vuelven con sus
    UV, colores y materiales;
  - los modelos con esqueleto no se pueden esculpir todavía.
- La barra del visor muestra solo las herramientas de esculpir, abajo aparece el estante de
  pinceles y los gizmos se ocultan.
- Seleccionar otra entidad, darle a Play o abrir otra escena sale del modo (guardando).

## Ratón

| Acción | Qué hace |
|---|---|
| Arrastrar con el botón izquierdo **sobre el modelo** | Esculpe |
| Arrastrar con el botón izquierdo **fuera del modelo** | Orbita |
| Botón derecho + WASDQE | Vuela, como en el modo Objeto |
| Botón central | Desplaza |
| Rueda | Acerca y aleja |

Un trazo que empieza en el modelo sigue aunque el cursor se salga por el borde de la silueta,
mientras el pincel aún toque la malla.

El cursor dibuja dos anillos sobre la superficie: el exterior es el tamaño del pincel y el
interior marca dónde trabaja a media fuerza. Con simetría, un punto señala dónde cae cada
copia. El anillo es azul al añadir, rojo al restar y oscuro al pintar máscara.

## Pinceles

Se eligen en el estante de abajo (cada casilla lleva su tecla) o con las teclas 1–0.

| Pincel | Tecla | Qué hace | Con Alt |
|---|---|---|---|
| Estándar | 1 | Levanta la superficie a lo largo del trazo | Hunde |
| Arcilla | 2 | Añade material que llena antes los huecos y deja el tope algo plano | Quita |
| Suavizar | 3 | Alisa (el mismo que Mayús) | — |
| Mover | 4 | Agarra lo que hay bajo el pincel y lo arrastra paralelo a la pantalla | Lo mueve a lo largo de la normal |
| Inflar | 5 | Empuja la superficie hacia fuera por sus normales | Hacia dentro |
| Pliegue | 6 | Hunde una línea y junta sus lados | Levanta una cresta |
| Aplanar | 7 | Lleva la zona hacia su plano medio | La aleja del plano |
| Pellizcar | 8 | Junta la superficie hacia el centro del trazo | La separa |
| Pliegue fino | 9 | Pliegue estrecho y más profundo, en V | Cresta afilada |
| Bloque | 0 | Como Arcilla, con punta cuadrada y tope plano | Talla plano |
| Recortar | | Rasura lo que sobresale de un plano | Rellena hasta el plano |
| Capa | | Sube hasta una altura fija por trazo: repasar no acumula | Baja |
| Empujar | | Desliza la superficie de lado, en la dirección del trazo | — |

Mover no deja toques: agarra al pulsar y sigue al cursor hasta soltar. Con Alt al empezar, la
superficie sale o entra por la normal media de la zona: arrastrar hacia donde apunta la normal
en pantalla tira hacia fuera.

### Modificadores durante el trazo

| Tecla | Efecto |
|---|---|
| Alt | Invierte el pincel (o Mover a lo largo de la normal) |
| Mayús al empezar | Suaviza, con la intensidad propia de Suavizar |
| Mayús a mitad del trazo | Línea recta desde ese punto, en pasos de 45°; al soltarla sigue a mano alzada |
| Ctrl | Pinta máscara |
| Ctrl+Alt | Borra máscara |

## Ajustes del pincel

Están en la barra superior. Cada pincel recuerda los suyos.

| Ajuste | Rango | Por defecto | Atajo |
|---|---|---|---|
| Tamaño | 2–800 px de radio en pantalla | 60 | **S** (deslizador en el cursor), **[** y **]** (±10 %) |
| Intensidad | 1–100 | 50 | **U** |
| Caída | −100 a 100 | 0 | **O** |

El tamaño se mide en pantalla: al acercarse, el pincel abarca menos modelo, que es lo que se
quiere para el detalle. En **Trazo → Tamaño en el mundo** se fija en metros.

La **caída** decide cómo se reparte la fuerza: en 0 es la curva suave clásica; por debajo de 0
el centro a plena fuerza se ensancha y el borde se endurece; por encima, queda una punta
estrecha y blanda.

Los deslizadores de S, U y O aparecen junto al cursor: se arrastran y al soltar se cierran, o
se escribe un número y Enter. Esc los cierra sin cambiar nada.

Suavizar tiene su propia intensidad: con Suavizar elegido (o en el deslizador de U con él) se
cambia la que usa Mayús, sin tocar la del pincel activo.

### Opciones

| Botón | Qué hace |
|---|---|
| Añadir / Restar | Lo que hacen los pinceles por defecto (Alt lo da la vuelta mientras se pulsa) |
| Frontal | Solo caras que miran a la cámara: no traspasa a la cara de atrás de partes finas (activo por defecto) |
| Conectado | Solo lo unido al punto bajo el cursor: no salta a la pieza de al lado aunque esté cerca |
| Taubin | El suavizado conserva el volumen (un paso hacia dentro y otro algo mayor hacia fuera) en vez de encoger la forma |
| Trazo → Caída de vista | El pincel es un cilindro a lo largo de la vista que atraviesa el modelo |
| Trazo → Cuerda | El pincel va detrás del cursor atado a una cuerda de esa longitud: trazos firmes sin temblor |
| Trazo → Espaciado | Distancia entre toques en % del radio (10 por defecto) |
| Trazo → Presión → intensidad | La presión del lápiz escala la intensidad |

El punto de la superficie bajo el cursor se filtra un poco: en mallas muy densas el trazo no
tiembla con cada bulto, pero sigue al instante un escalón real.

## Simetría

- **X**, **Y** y **Z** reflejan cada toque en ese eje y se combinan entre sí. X está activa por
  defecto y la tecla **X** la cambia.
- **R** repite cada toque alrededor de un eje: el número de copias (2–64) y el eje aparecen al
  activarla. Con un espejo además, cada copia radial se refleja también (8 copias con X dan 16).
- **Local** pone los planos y el eje en el centro de la caja de la pieza en vez del origen.

La simetría vale para todos los pinceles, Mover y la máscara.

## Máscara

Lo enmascarado se protege de todos los pinceles y operaciones, y se ve más oscuro. Ctrl pinta
máscara y Ctrl+Alt la borra. El menú **Máscara** tiene **Invertir**, **Borrar**,
**Difuminar** (suaviza el borde) y **Afilar** (lo endurece). La máscara vive mientras se
esculpe: no se guarda en el `.glb`.

## Malla

El menú **Malla** actúa sobre toda la pieza, respetando la máscara, con la **Cantidad** que se
elija:

- **Suavizar**: alisa todo (con Taubin activo, sin encoger).
- **Relajar**: reparte los vértices por la superficie sin cambiar la forma.
- **Hinchar**: hincha la pieza como un globo.
- **Subdividir**: parte cada triángulo en cuatro para tener más detalle. Cambia la topología,
  así que borra el historial de trazos y la máscara.

**Alambre** (o **Mayús+F**) dibuja las aristas sobre la superficie.

## Deshacer

Dentro del modo, **Ctrl+Z** deshace el último trazo y **Ctrl+Mayús+Z** (o Ctrl+Y) lo rehace;
también desde **Editar**. Cada trazo es un paso. Solo se guardan los vértices que el trazo
tocó, y el historial se limita a 1 GB por escultura (lo más antiguo se olvida antes).

El historial de la escena no se toca mientras se esculpe. Al guardar la escultura se añade un
paso «Esculpir *nombre*» al historial de la escena.

## Guardar

Salir del modo o **Ctrl+S** hornea la escultura a `assets/esculturas/<nombre>.glb` en el
proyecto (con el nombre de la entidad; si ya existe otro, `<nombre>_2.glb`…) y la entidad pasa a
usar ese archivo. Ctrl+S además guarda la escena. Si la entidad ya usaba una escultura, se
reescribe la misma.

- El archivo aparece en **Contenido** con su miniatura y se puede arrastrar al visor como
  cualquier modelo.
- Volver a esculpir la entidad continúa desde el archivo. Si es la última que se esculpió y el
  archivo no ha cambiado, se conserva también su historial de trazos.
- El material de la entidad (color, sombreado, normal map) no cambia: se aplica igual a la
  escultura. Los materiales y texturas propios de un modelo se guardan dentro del `.glb`.

## Atajos del modo

Solo funcionan con el modo activo y sin escribir en un campo.

| Tecla | Acción |
|---|---|
| 1 … 9, 0 | Los diez primeros pinceles |
| [ / ] | Tamaño −10 % / +10 % |
| S / U / O | Deslizador de tamaño / intensidad / caída en el cursor |
| X | Simetría en X |
| Mayús+F | Alambre |
| Ctrl+Z / Ctrl+Mayús+Z | Deshacer / rehacer trazo |
| Ctrl+S | Guardar la escultura y la escena |
| Tab | Volver al modo Objeto |

## Rendimiento

Cada toque trabaja por grupos de 128 a 256 triángulos en paralelo (un hilo por grupo, sin
reservar memoria). Solo se recalculan las normales y las cajas de lo tocado, y a la GPU solo
suben los rangos de vértices cambiados. En una esfera de un millón de vértices (prueba
`bench_dab_on_million_vertex_sphere`, perfil de pruebas):

| Toque | Tiempo (con su copia en espejo) |
|---|---|
| Estándar, radio pequeño (~12 000 vértices) | ~0,3 ms |
| Estándar, radio medio (~67 000 vértices) | ~0,5 ms |
| Suavizar Taubin, radio medio | ~3 ms |
| Estándar sobre ~400 000 vértices | ~4 ms |

## Límites de esta fase

- La malla no cambia de topología mientras se esculpe: no hay topología dinámica (más
  triángulos donde hace falta) ni niveles de subdivisión. Para más detalle, **Malla →
  Subdividir**.
- Al soldar las costuras, la escultura se ve y se guarda con normales suaves: las aristas
  duras de un modelo se redondean.
- La presión del lápiz solo llega si el sistema la entrega como toque con fuerza; con la
  mayoría de tabletas en Linux la intensidad queda a 1.
- Con una escala no uniforme en la entidad, el radio del pincel es la escala media.
- Dos entidades que usen el mismo `.glb` de escultura cambian juntas.
- Deshacer en la escena el paso «Esculpir» devuelve la malla que la entidad usaba antes, pero
  si el `.glb` ya existía y se reescribió, su contenido anterior no vuelve.
- La máscara no se guarda.

## Próximas fases

1. **Topología**: topología dinámica (dividir, colapsar y girar aristas bajo el pincel),
   niveles de subdivisión, remallado por vóxeles y diezmado.
2. **Detalle**: alfas (generadas y desde imagen), sellos, mapas de desplazamiento vectorial,
   pinceles elásticos (Kelvinlets), capas de esculpido y grupos de caras con visibilidad.
3. **Pintura**: color de vértice, texturas y PBR, UV y horneado de normal maps de la malla
   alta a la baja.
4. **Automatización**: órdenes con nombre (trazos, operaciones de malla) para los scripts y el
   modo agente.
