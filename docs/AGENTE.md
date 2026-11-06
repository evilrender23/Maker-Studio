# Modo agente

El modo agente añade al editor un panel para trabajar sobre el proyecto abierto con un
agente de programación por línea de comandos: **Claude Code** (`claude`), **Codex**
(`codex`) o **Antigravity** (`agy`). El editor lanza la CLI en modo sin cabeza en la carpeta
del proyecto, muestra lo que hace (texto, órdenes, archivos leídos y editados) y, al terminar
cada turno, recarga lo que haya cambiado.

Viene **apagado**. Apagado no hay panel, ni menú, ni se busca ninguna CLI.

## Activarlo

1. **Archivo → Preferencias…** (`Ctrl+,`).
2. Marca **Modo agente**.
3. El panel se abre a la derecha. Se muestra y se oculta con el botón **✦ Agente** de la barra
   de herramientas, con **Agente → Panel del agente** o con `Ctrl+Mayús+A`.

## Requisitos

Al menos una de las CLIs instalada y con la sesión iniciada (cada una usa su propia cuenta):

| CLI | Ejecutable | Instalación |
|---|---|---|
| Claude Code | `claude` | <https://docs.claude.com/claude-code> |
| Codex | `codex` | <https://github.com/openai/codex> |
| Antigravity | `agy` | <https://antigravity.google> |

El editor las busca en el `PATH` y en las carpetas habituales (`~/.local/bin`, `%APPDATA%\npm`…).
En Windows resuelve los `.exe` y los `.cmd` que instala npm (estos se ejecutan con `cmd /C`) y
no abre ventanas de consola. Si una CLI está en otro sitio, indica su ruta en Preferencias.

## Preferencias

Se guardan en `preferencias.json`, en la carpeta de configuración del editor
(`MAKER_STUDIO_CONFIG`, si no `$XDG_CONFIG_HOME/maker-studio`, `~/.config/maker-studio` o
`%APPDATA%\maker-studio`).

| Opción | Qué hace |
|---|---|
| Actualizar automáticamente | La actualización periódica desde GitHub (antes en el menú Ayuda). |
| Modo agente | Activa o desactiva todo lo de esta página. |
| CLI predeterminada | La que usan las sesiones nuevas. |
| Permisos | Los permisos de las sesiones nuevas (ver tabla). |
| Compactar al llegar a | Compacta sola la conversación cuando el contexto pasa de ese porcentaje. Desmarcado, nunca. |
| Recompilar scripts tras cambios del agente | Si el agente cambia algo en `scripts/src`, compila y recarga los scripts en caliente, como F7. |
| Ejecutable | Ruta propia de cada CLI. Vacío: la que se detecta. El punto verde indica que se encontró (con su versión); el rojo, que no. |
| Modelo / Esfuerzo | Los predeterminados de cada CLI. **Predeterminado** deja elegir a la CLI. |

**Volver a detectar** busca otra vez las CLIs y sus modelos.

### Permisos

| Nivel | Claude Code | Codex | Antigravity |
|---|---|---|---|
| Solo lectura | `--permission-mode plan` | sandbox `read-only` | `--mode plan` |
| Editar archivos | `--permission-mode acceptEdits` | sandbox `workspace-write` | `--mode accept-edits` |
| Acceso total | `--permission-mode bypassPermissions` | sandbox `danger-full-access` | `--dangerously-skip-permissions` |

Las CLIs nunca preguntan nada durante un turno: lo que el nivel no permite, se deniega.

### Modelos y esfuerzo

- Claude Code: `opus`, `sonnet`, `haiku` u otro nombre de modelo; esfuerzo `low` a `max`.
- Codex: la lista de `codex debug models`, con los esfuerzos que admite cada modelo.
- Antigravity: la lista de `agy models`; esfuerzo `low` a `max`.

**Otro…** permite escribir cualquier id de modelo.

## El panel

- **Sesión**: desplegable con el historial del proyecto; **+** crea una sesión, **✎** la
  renombra y **✕** la borra.
- **CLI, Modelo, Esfuerzo, Permisos**: valen para el siguiente mensaje. La CLI solo se puede
  cambiar antes del primer mensaje de la sesión; las CLIs que no se encuentran salen
  desactivadas.
- **Medidor de contexto**: tokens ocupados frente a la ventana real del modelo, medida por la
  propia CLI (Claude: `/context`, sin coste; Codex: su registro de sesión). Antigravity no la
  informa: solo se muestran los tokens. **Compactar** la reduce:
  - Claude Code: su `/compact` nativo sobre la misma conversación.
  - Codex y Antigravity: se pide un resumen y el siguiente mensaje abre una conversación
    nueva que empieza con ese resumen.
- **Conversación**: tus mensajes, las respuestas (los bloques de código en monoespaciada),
  cada herramienta en una fila (orden, archivo leído, archivo editado) que se despliega para
  ver su salida, y los errores en rojo.
- **Contexto**: los botones sobre el cuadro de texto añaden información del editor al mensaje:
  - **Escena**: la escena abierta y su lista de entidades con sus componentes.
  - **Selección**: la entidad seleccionada con todos sus componentes (JSON).
  - **Script**: el script abierto en el editor de código (también si no está guardado).
  - **Errores**: los últimos errores y avisos de la consola.

  Además, la primera vez de cada conversación se envía una guía de Maker Studio: estructura
  del proyecto, formato `.mscene`, `maker_api` y cómo se compilan los scripts. En Claude Code
  va como instrucciones de sistema (`--append-system-prompt`); en las demás, delante del
  primer mensaje.
- **Intro** envía, **Mayús+Intro** salta de línea. Mientras trabaja se ve el tiempo
  transcurrido y **Detener** mata la CLI con todos sus procesos hijos.
- Debajo: tokens de entrada y salida y coste acumulados de la sesión (cuando la CLI los da).

## Después de cada turno

- Si el agente cambió la escena abierta y en el editor no hay cambios sin guardar, se recarga
  (conservando la selección). Si los hay, avisa y no la toca.
- Si cambió el script abierto en el editor de código y no tiene cambios sin guardar, se recarga.
- Si cambió algo en `scripts/src` y está activado **Recompilar scripts**, se compilan y se
  recargan en caliente.
- La consola muestra un resumen: duración, herramientas usadas y archivos cambiados.

## Dónde se guarda el historial

Cada sesión es un JSON en `<proyecto>/.maker/agente/sesiones/`, con la CLI, el modelo, el
esfuerzo, los permisos, el id de la conversación de la CLI (para reanudarla), los mensajes,
las herramientas, el uso de tokens y las fechas. `<proyecto>/.maker/agente/.gitignore` deja
esa carpeta fuera de git. Borrar una sesión en el panel borra su archivo; la conversación
sigue existiendo en la propia CLI.

## Automatización

Para pruebas sin ventana interactiva:

```bash
maker-studio PROYECTO --agent-cli claude --agent-model haiku --agent-effort low \
  --agent-perm edit --agent-prompt "…" --screenshot captura.png
```

`--agent-continue` sigue la sesión más reciente, `--agent-compact` compacta tras el turno,
`--agent-chips escena,seleccion,script,errores` elige el contexto y `--prefs` abre
Preferencias. La captura se toma cuando termina el turno (y la compilación, si la hay), y la
sesión se imprime en la salida estándar.
