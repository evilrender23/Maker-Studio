#!/bin/bash
# Instala o actualiza Maker Studio en Mac o en Linux compilándolo desde una rama del repositorio
# (main por defecto). Es el equivalente de instalar.ps1.
#
#   git clone https://github.com/evilrender23/Maker-Studio
#   bash Maker-Studio/instalar.sh [--repo URL|RUTA] [--rama main] [--destino CARPETA] [--bin CARPETA]
#
# O en una línea:
#   curl -fsSL https://raw.githubusercontent.com/evilrender23/Maker-Studio/main/instalar.sh | bash
#
# Desde un clon, sin --repo, instala desde el origin de ese clon.
#
# En <destino> (por defecto ~/Library/Application Support/MakerStudio en Mac y
# ~/.local/share/MakerStudio en Linux) deja:
#   codigo/    el clon, con la caché de cargo (target/) para que la siguiente compilación sea rápida.
#              No lo borres: los scripts de tus proyectos se compilan contra codigo/crates/maker_api;
#   app/       maker-studio, maker-player y version.json (commit, fecha, rama, repo);
#   registro/  compilacion.log, con la salida de git y cargo.
# Y enlaza `maker-studio` y `maker-player` en --bin (por defecto ~/.local/bin). En Linux crea un
# lanzador en el menú de aplicaciones; en Mac, ~/Applications/Maker Studio.app.
#
# Pregunta antes de instalar lo que falte: Mac, las herramientas de línea de comandos de Xcode; Linux,
# un compilador de C, git y curl (con el gestor de paquetes y sudo); los dos, Rust. Rust se queda
# instalado: el editor lo usa para compilar los scripts de cada proyecto.
#
# Para actualizar, vuelve a ejecutarlo: trae lo último de la rama y recompila aprovechando la caché.

set -uo pipefail

case "$(uname -s)" in
    Darwin) LINUX=false ;;
    Linux) LINUX=true ;;
    *) echo "Este instalador es para Mac y Linux; en Windows, usa instalar.ps1." >&2; exit 2 ;;
esac

REPO='https://github.com/evilrender23/Maker-Studio'
RAMA='main'
if $LINUX; then DESTINO="${XDG_DATA_HOME:-$HOME/.local/share}/MakerStudio"; else DESTINO="$HOME/Library/Application Support/MakerStudio"; fi
BIN="$HOME/.local/bin"
REPO_INDICADO=false
# eframe 0.37, wgpu 30 y la edición 2024 piden Rust 1.95 o más nuevo.
RUST_MINIMO='1.95'

while [ $# -gt 0 ]; do
    case "$1" in
        --repo) REPO="$2"; REPO_INDICADO=true; shift 2 ;;
        --rama) RAMA="$2"; shift 2 ;;
        --destino) DESTINO="$2"; shift 2 ;;
        --bin) BIN="$2"; shift 2 ;;
        -h|--help) sed -n '2,25p' "${BASH_SOURCE[0]:-/dev/null}" 2>/dev/null | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "Opción desconocida: $1" >&2; exit 2 ;;
    esac
done

export PATH="$HOME/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:$PATH"

# Ejecutado desde un clon de Maker Studio y sin --repo: el mismo repositorio (un fork, una copia local).
if [ -n "${BASH_SOURCE[0]:-}" ] && [ -f "${BASH_SOURCE[0]}" ]; then
    PROPIO=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
    if ! $REPO_INDICADO && [ -f "$PROPIO/crates/maker_editor/Cargo.toml" ] && command -v git >/dev/null 2>&1; then
        ORIGEN=$(git -C "$PROPIO" remote get-url origin 2>/dev/null) && [ -n "$ORIGEN" ] && REPO="$ORIGEN"
    fi
fi

CODIGO="$DESTINO/codigo"
APP="$DESTINO/app"
NUEVO="$DESTINO/app.nuevo"
REGISTRO="$DESTINO/registro"
LOG="$REGISTRO/compilacion.log"
ULTIMA="$REGISTRO/ultima-salida.log"
CERROJO="$REGISTRO/instalar.lock"
TOTAL=6

paso() { printf '\033[36m[%s/%s] %s\033[0m\n' "$1" "$TOTAL" "$2"; }
fallar() {
    printf '\033[31mERROR: %s\033[0m\n' "$(printf '%s' "$1" | tr '\n' ' ')" >&2
    exit 1
}
tiene() { command -v "$1" >/dev/null 2>&1; }

# ejecutar CARPETA PROGRAMA ARGUMENTOS…: la salida va a compilacion.log (y a la pantalla, atenuada).
ejecutar() {
    local carpeta=$1 resultado
    shift
    printf '> %s\n' "$*" >> "$LOG"
    (cd "$carpeta" && "$@") 2>&1 | tee "$ULTIMA" | tee -a "$LOG" | sed $'s/^/\033[2m/; s/$/\033[0m/'
    resultado=${PIPESTATUS[0]}
    return "$resultado"
}

# ---------------------------------------------------------------- requisitos

hay_xcode() { xcode-select -p >/dev/null 2>&1 && tiene git; }
hay_sistema() { tiene cc && tiene git && tiene curl; }
version_rust() { rustc --version 2>/dev/null | sed -n 's/^rustc \([0-9]*\.[0-9]*\).*/\1/p'; }
hay_rust() {
    local v
    v=$(version_rust)
    [ -n "$v" ] && tiene cargo && [ "$(printf '%s\n%s\n' "$RUST_MINIMO" "$v" | sort -t. -k1,1n -k2,2n | head -1)" = "$RUST_MINIMO" ]
}

instalar_xcode() {
    xcode-select --install >/dev/null 2>&1
    echo 'Acepta la ventana de macOS que pide instalar las herramientas de línea de comandos; se espera a que acabe…'
    until hay_xcode; do sleep 5; done
}

instalar_sistema() {
    if tiene apt-get; then
        sudo apt-get update && sudo apt-get install -y build-essential curl git
    elif tiene dnf; then
        sudo dnf install -y gcc gcc-c++ make curl git
    elif tiene pacman; then
        sudo pacman -S --needed --noconfirm base-devel curl git
    elif tiene zypper; then
        sudo zypper install -y gcc gcc-c++ make curl git
    else
        fallar "No se reconoce el gestor de paquetes: instala a mano un compilador de C, git y curl."
    fi
}

instalar_rust() {
    if tiene rustup; then
        ejecutar "$DESTINO" rustup update stable
        ejecutar "$DESTINO" rustup default stable
    else
        ejecutar "$DESTINO" bash -c "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal"
    fi
}

if $LINUX; then REQUISITOS='sistema rust'; else REQUISITOS='xcode rust'; fi
nombre_de() {
    case "$1" in
        xcode) echo 'Herramientas de línea de comandos de Xcode (con Git)' ;;
        sistema) echo 'Compilador de C, Git y curl (con sudo)' ;;
        rust) echo "Rust $RUST_MINIMO o más nuevo (rustup)" ;;
    esac
}
hay() { "hay_$1"; }

comprobar_requisitos() {
    local faltan='' nombres='' r respuesta si
    for r in $REQUISITOS; do
        hay "$r" || { faltan="$faltan $r"; nombres="$nombres${nombres:+, }$(nombre_de "$r")"; }
    done
    [ -z "$faltan" ] && return 0
    printf '\033[33mFalta: %s\033[0m\n' "$nombres"
    # Con `curl | bash` la entrada es el propio script: se pregunta en la terminal.
    printf '¿Instalarlo? [T]odo / [u]no a uno / [n]ada '
    read -r respuesta < /dev/tty || respuesta=n
    respuesta=$(printf '%s' "$respuesta" | tr '[:upper:]' '[:lower:]' | tr -d ' ')
    for r in $faltan; do
        hay "$r" && continue
        case "$respuesta" in
            ''|t|todo|s|si|sí) si=true ;;
            u*)
                printf '¿Instalar %s? [S/n] ' "$(nombre_de "$r")"
                read -r si < /dev/tty || si=n
                case "$si" in [nN]*) si=false ;; *) si=true ;; esac ;;
            *) si=false ;;
        esac
        $si || fallar "Sin $(nombre_de "$r") no se puede compilar Maker Studio."
        echo "Instalando $(nombre_de "$r")…"
        "instalar_$r"
        hay "$r" || fallar "Se instaló $(nombre_de "$r"), pero no aparece: abre una terminal nueva y vuelve a ejecutar el instalador."
    done
}

# ---------------------------------------------------------------- código

explicar_error_git() {
    if grep -qiE 'Authentication failed|Repository not found|could not read Username|(^|[^0-9])40[13]([^0-9]|$)' "$ULTIMA"; then
        fallar "Sin acceso a $1: hace falta permiso para ver el repositorio y haber iniciado sesión en GitHub con Git."
    fi
    if grep -qiE 'Could not resolve host|unable to access' "$ULTIMA"; then
        fallar "Sin conexión con $1."
    fi
    fallar "git falló con $1 (detalles en $LOG)"
}

descargar_codigo() {
    if [ -d "$CODIGO/.git" ]; then
        $REPO_INDICADO && ejecutar "$CODIGO" git remote set-url origin "$REPO"
        local origen
        origen=$(git -C "$CODIGO" remote get-url origin)
        ejecutar "$CODIGO" git fetch --prune origin "$RAMA" || explicar_error_git "$origen"
        ejecutar "$CODIGO" git reset --hard "origin/$RAMA" || fallar "git reset falló (detalles en $LOG)"
        # Sin -x: lo ignorado (target/) es la caché.
        ejecutar "$CODIGO" git clean -fd || fallar "git clean falló (detalles en $LOG)"
    else
        ejecutar "$DESTINO" git clone --branch "$RAMA" "$REPO" "$CODIGO" || explicar_error_git "$REPO"
    fi
}

json() { printf '"%s"' "$(printf '%s' "$1" | sed 's/\\/\\\\/g; s/"/\\"/g')"; }

# ---------------------------------------------------------------- accesos

crear_lanzador_linux() {
    local carpeta="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
    mkdir -p "$carpeta" || return 1
    cat > "$carpeta/maker-studio.desktop" <<LANZADOR
[Desktop Entry]
Type=Application
Name=Maker Studio
Comment=Motor 3D en Rust con editor y scripting en Rust
Exec="$APP/maker-studio" %f
Terminal=false
Categories=Development;Game;Graphics;3DGraphics;
StartupWMClass=maker-studio
LANZADOR
    tiene update-desktop-database && update-desktop-database "$carpeta" >/dev/null 2>&1
    return 0
}

# Un .app mínimo que abre el editor (sin firma de desarrollador: macOS lo deja abrir porque se creó en local).
crear_app_mac() {
    local app="$HOME/Applications/Maker Studio.app"
    rm -rf "$app"
    mkdir -p "$app/Contents/MacOS" || return 1
    cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleName</key><string>Maker Studio</string>
  <key>CFBundleIdentifier</key><string>com.evilrender23.makerstudio</string>
  <key>CFBundleExecutable</key><string>maker-studio</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>NSHighResolutionCapable</key><true/>
</dict></plist>
PLIST
    # Desde Finder el PATH no incluye ~/.cargo/bin, y el editor necesita cargo para los scripts.
    printf '#!/bin/bash\nexport PATH="$HOME/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:$PATH"\nexec "%s/maker-studio" "$@"\n' "$APP" > "$app/Contents/MacOS/maker-studio"
    chmod +x "$app/Contents/MacOS/maker-studio"
}

# ---------------------------------------------------------------- cambio de versión

# El editor y los juegos que se lanzaron desde esta instalación.
procesos_de_la_app() { pgrep -f "^$APP/maker-(studio|player)( |$)" >/dev/null 2>&1; }

esperar_cierre() {
    local avisado=false
    while procesos_de_la_app; do
        if ! $avisado; then
            printf '\033[33mCierra Maker Studio y los juegos abiertos desde él para terminar la instalación…\033[0m\n'
            avisado=true
        fi
        sleep 1
    done
}

limpiar() { rm -rf "$CERROJO"; }

tomar_cerrojo() {
    local dueno
    mkdir "$CERROJO" 2>/dev/null && return 0
    dueno=$(cat "$CERROJO/pid" 2>/dev/null)
    if [ -n "$dueno" ] && kill -0 "$dueno" 2>/dev/null; then
        fallar 'Ya hay otra instalación de Maker Studio en marcha.'
    fi
    # Cerrojo de una instalación que ya no existe.
    rm -rf "$CERROJO"
    mkdir "$CERROJO" 2>/dev/null || fallar 'Ya hay otra instalación de Maker Studio en marcha.'
}

# ---------------------------------------------------------------- principal

principal() {
    mkdir -p "$REGISTRO" || { echo "No se puede crear $REGISTRO" >&2; exit 1; }
    tomar_cerrojo
    echo $$ > "$CERROJO/pid"
    trap limpiar EXIT
    : > "$LOG"

    if $LINUX; then paso 1 'Comprobando el compilador de C, Git y Rust'
    else paso 1 'Comprobando las herramientas de Xcode, Git y Rust'; fi
    comprobar_requisitos

    paso 2 "Descargando el código ($RAMA)"
    descargar_codigo
    local commit fecha origen
    commit=$(git -C "$CODIGO" rev-parse HEAD)
    fecha=$(git -C "$CODIGO" log -1 --format=%cI)
    origen=$(git -C "$CODIGO" remote get-url origin)

    paso 3 'Compilando el motor, el editor y el reproductor (la primera vez tarda unos minutos)'
    # Sin un CARGO_TARGET_DIR heredado: la caché y los binarios deben quedar en codigo/target.
    ejecutar "$CODIGO" env -u CARGO_TARGET_DIR cargo build --release --locked -p maker_editor -p maker_player \
        || fallar "La compilación falló (detalles en $LOG)"
    local compilado="$CODIGO/target/release"
    for b in maker-studio maker-player; do
        [ -x "$compilado/$b" ] || fallar "La compilación no dejó $compilado/$b"
    done

    paso 4 'Preparando la versión nueva'
    rm -rf "$NUEVO"
    mkdir -p "$NUEVO"
    # El editor lanza y exporta los juegos con el maker-player que tenga al lado: van juntos.
    cp "$compilado/maker-studio" "$compilado/maker-player" "$NUEVO/" || fallar 'No se pudo copiar lo compilado.'
    printf '{\n  "commit": %s,\n  "fecha": %s,\n  "rama": %s,\n  "repo": %s\n}\n' \
        "$(json "$commit")" "$(json "$fecha")" "$(json "$RAMA")" "$(json "$origen")" > "$NUEVO/version.json"
    if ! $LINUX; then
        xattr -dr com.apple.quarantine "$NUEVO" 2>/dev/null
    fi

    if [ -d "$APP" ]; then
        paso 5 'Cambiando a la versión nueva'
        esperar_cierre
    else
        paso 5 "Instalando en $APP"
    fi
    rm -rf "$APP.anterior"
    [ -d "$APP" ] && mv "$APP" "$APP.anterior"
    mv "$NUEVO" "$APP" || { [ -d "$APP.anterior" ] && mv "$APP.anterior" "$APP"; fallar "No se pudo instalar en $APP."; }
    rm -rf "$APP.anterior"

    paso 6 "Enlazando los comandos en $BIN"
    mkdir -p "$BIN" || fallar "No se puede crear $BIN"
    ln -sf "$APP/maker-studio" "$BIN/maker-studio"
    ln -sf "$APP/maker-player" "$BIN/maker-player"
    if $LINUX; then
        crear_lanzador_linux || echo 'Aviso: no se pudo crear el lanzador del menú de aplicaciones.'
    else
        crear_app_mac || echo 'Aviso: no se pudo crear Maker Studio.app.'
    fi

    printf '\033[32mMaker Studio %s instalado en %s.\033[0m\n' "${commit:0:7}" "$APP"
    case ":$PATH:" in
        *":$BIN:"*) ;;
        *) printf '\033[33m%s no está en el PATH: añade  export PATH="%s:$PATH"  a tu ~/.bashrc o ~/.zshrc.\033[0m\n' "$BIN" "$BIN" ;;
    esac
    echo 'Pruébalo:  maker-studio    (Project Hub → Crear proyecto → Demo jugable, F7 y ▶ Play)'
    echo '      o:   maker-studio --new-demo ~/MakerProjects/Demo --build-scripts'
}

# En la misma línea: bash lee el script según avanza, y git puede haberlo cambiado mientras tanto.
principal; exit $?
