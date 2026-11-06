//! Interfaces de juego (HUD, menús): el equivalente a UGUI de Unity, los
//! `Control` de Godot o UMG de Unreal.
//!
//! Los elementos son entidades de la escena con componentes de interfaz, así
//! que aparecen en la jerarquía, se anidan por padre/hijo y se guardan en el
//! `.mscene`:
//!
//! - [`Lienzo`]: raíz en espacio de pantalla con resolución de referencia y
//!   modo de escala.
//! - [`RectUi`]: anclas, pivote, márgenes, rotación y escala (como el
//!   `RectTransform` de Unity), visibilidad, interacción, opacidad de grupo y
//!   recorte de hijos.
//! - [`Contenedor`]: caja horizontal, vertical o rejilla (como los
//!   contenedores de Godot).
//! - [`WidgetUi`]: qué se dibuja (panel, imagen, texto, botón, barra,
//!   deslizador, casilla, campo de texto o widget dibujado por script), con un
//!   estilo por nombre y retoques locales.
//!
//! El aspecto sale de estilos y temas del proyecto ([`estilo`]); el motor solo
//! trae un aspecto neutro de último recurso. El sistema de ejecución
//! ([`sistema::SistemaUi`]) hace el layout, la entrada, los eventos y la lista
//! de dibujo que pinta `maker_render`.

pub mod dibujo;
pub mod estilo;
pub mod fuente;
pub mod layout;
pub mod plantilla;
pub mod sistema;

use serde::{Deserialize, Serialize};

pub use estilo::{Estado, Estilo, EstadosEstilo, HojaEstilos, PropsEstilo, RecursosUi, Tema};
pub use sistema::SistemaUi;

/// Cómo se adapta un lienzo al tamaño de la pantalla.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModoEscala {
    /// Una unidad = `escala` píxeles, sea cual sea la pantalla.
    PixelConstante,
    /// Escala respecto a la resolución de referencia (como el *Scale With
    /// Screen Size* de Unity): `ajuste` 0 iguala el ancho, 1 el alto.
    #[default]
    EscalarConPantalla,
}

/// Raíz de una interfaz en espacio de pantalla (overlay sobre el 3D).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Lienzo {
    pub resolucion: [f32; 2],
    pub modo: ModoEscala,
    /// 0 = iguala el ancho, 1 = el alto (en escala logarítmica, como Unity).
    pub ajuste: f32,
    /// Píxeles por unidad en `PixelConstante`.
    pub escala: f32,
    /// Orden de dibujo entre lienzos (mayor = encima).
    pub orden: i32,
    /// Tema propio; vacío = el del proyecto.
    pub tema: String,
}

impl Default for Lienzo {
    fn default() -> Self {
        Self { resolucion: [1920.0, 1080.0], modo: ModoEscala::EscalarConPantalla, ajuste: 0.5, escala: 1.0, orden: 0, tema: String::new() }
    }
}

impl Lienzo {
    /// Píxeles por unidad del lienzo para una pantalla de `w`×`h` píxeles.
    pub fn factor(&self, w: f32, h: f32) -> f32 {
        match self.modo {
            ModoEscala::PixelConstante => self.escala.max(0.01),
            ModoEscala::EscalarConPantalla => {
                let rw = (w / self.resolucion[0].max(1.0)).max(1e-4).log2();
                let rh = (h / self.resolucion[1].max(1.0)).max(1e-4).log2();
                let t = self.ajuste.clamp(0.0, 1.0);
                (rw + (rh - rw) * t).exp2()
            }
        }
    }
}

/// Rectángulo de un elemento: anclas (0..1 dentro del padre, y hacia abajo),
/// márgenes de las esquinas respecto a sus anclas (como los *offsets* de
/// Godot), pivote, rotación y escala.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RectUi {
    pub ancla_min: [f32; 2],
    pub ancla_max: [f32; 2],
    /// Esquina superior izquierda respecto a `ancla_min` (unidades del lienzo).
    pub offset_min: [f32; 2],
    /// Esquina inferior derecha respecto a `ancla_max`.
    pub offset_max: [f32; 2],
    pub pivote: [f32; 2],
    /// Grados, en sentido horario (pantalla con y hacia abajo).
    pub rotacion: f32,
    pub escala: [f32; 2],
    pub visible: bool,
    /// Recibe ratón y teclado (y los pasa a sus hijos).
    pub interactuable: bool,
    /// Opacidad del elemento y sus hijos (como un CanvasGroup).
    pub opacidad: f32,
    /// Recorta a los hijos a este rectángulo (máscara).
    pub recortar: bool,
    /// Dentro de un contenedor: no lo coloca el contenedor (usa sus anclas).
    pub ignorar_layout: bool,
    /// Peso para repartir el espacio libre de un contenedor que expande.
    pub flex: f32,
}

impl Default for RectUi {
    fn default() -> Self {
        Self::centrado(160.0, 40.0)
    }
}

impl RectUi {
    /// Centrado en el padre con un tamaño fijo.
    pub fn centrado(w: f32, h: f32) -> Self {
        Self {
            ancla_min: [0.5, 0.5],
            ancla_max: [0.5, 0.5],
            offset_min: [-w * 0.5, -h * 0.5],
            offset_max: [w * 0.5, h * 0.5],
            pivote: [0.5, 0.5],
            rotacion: 0.0,
            escala: [1.0, 1.0],
            visible: true,
            interactuable: true,
            opacidad: 1.0,
            recortar: false,
            ignorar_layout: false,
            flex: 1.0,
        }
    }
    /// Ocupa todo el padre.
    pub fn estirado() -> Self {
        Self { ancla_min: [0.0, 0.0], ancla_max: [1.0, 1.0], offset_min: [0.0, 0.0], offset_max: [0.0, 0.0], ..Self::centrado(0.0, 0.0) }
    }
    /// Con anclas en un punto: esquina superior izquierda `pos` respecto al
    /// ancla y tamaño `tam`.
    pub fn anclado(ancla: [f32; 2], pivote: [f32; 2], pos: [f32; 2], tam: [f32; 2]) -> Self {
        let mut r = Self { ancla_min: ancla, ancla_max: ancla, pivote, ..Self::centrado(0.0, 0.0) };
        r.fijar_posicion(pos);
        r.fijar_tam(tam);
        r.fijar_posicion(pos);
        r
    }
    /// ¿Las anclas coinciden en el eje `i` (tamaño fijo)?
    pub fn fijo(&self, i: usize) -> bool {
        (self.ancla_max[i] - self.ancla_min[i]).abs() < 1e-6
    }
    /// Tamaño propio (en ejes estirados, lo que añade al del padre).
    pub fn tam(&self) -> [f32; 2] {
        [self.offset_max[0] - self.offset_min[0], self.offset_max[1] - self.offset_min[1]]
    }
    /// Posición del pivote respecto al punto de ancla (como `anchoredPosition`).
    pub fn posicion(&self) -> [f32; 2] {
        let t = self.tam();
        [self.offset_min[0] + t[0] * self.pivote[0], self.offset_min[1] + t[1] * self.pivote[1]]
    }
    pub fn fijar_posicion(&mut self, p: [f32; 2]) {
        let t = self.tam();
        for i in 0..2 {
            self.offset_min[i] = p[i] - t[i] * self.pivote[i];
            self.offset_max[i] = self.offset_min[i] + t[i];
        }
    }
    /// Cambia el tamaño conservando la posición del pivote.
    pub fn fijar_tam(&mut self, t: [f32; 2]) {
        let p = self.posicion();
        for i in 0..2 {
            self.offset_min[i] = p[i] - t[i] * self.pivote[i];
            self.offset_max[i] = self.offset_min[i] + t[i];
        }
    }
    /// Rectángulo (min, max) dentro de un padre `p` (min, max).
    pub fn en(&self, p: ([f32; 2], [f32; 2])) -> ([f32; 2], [f32; 2]) {
        let s = [p.1[0] - p.0[0], p.1[1] - p.0[1]];
        let mn = [p.0[0] + s[0] * self.ancla_min[0] + self.offset_min[0], p.0[1] + s[1] * self.ancla_min[1] + self.offset_min[1]];
        let mx = [p.0[0] + s[0] * self.ancla_max[0] + self.offset_max[0], p.0[1] + s[1] * self.ancla_max[1] + self.offset_max[1]];
        (mn, mx)
    }
    /// Cambia las anclas conservando el rectángulo en un padre de tamaño `padre`
    /// (como los preajustes de Unity sin Alt).
    pub fn cambiar_anclas(&mut self, amin: [f32; 2], amax: [f32; 2], padre: [f32; 2]) {
        let (mn, mx) = self.en(([0.0, 0.0], padre));
        self.ancla_min = amin;
        self.ancla_max = amax;
        for i in 0..2 {
            self.offset_min[i] = mn[i] - padre[i] * amin[i];
            self.offset_max[i] = mx[i] - padre[i] * amax[i];
        }
    }
}

/// Tipo de contenedor.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TipoContenedor {
    #[default]
    Horizontal,
    Vertical,
    Rejilla,
}

/// Alineación de los hijos de un contenedor.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Alinear {
    #[default]
    Inicio,
    Centro,
    Fin,
    /// Solo en el eje cruzado: el hijo ocupa todo el alto (o ancho).
    Estirar,
}

impl Alinear {
    pub const TODOS: [Alinear; 4] = [Alinear::Inicio, Alinear::Centro, Alinear::Fin, Alinear::Estirar];
    pub fn nombre(self) -> &'static str {
        match self {
            Alinear::Inicio => "Inicio",
            Alinear::Centro => "Centro",
            Alinear::Fin => "Fin",
            Alinear::Estirar => "Estirar",
        }
    }
}

/// Coloca a sus hijos en fila, columna o rejilla.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Contenedor {
    pub tipo: TipoContenedor,
    /// Relleno interior: izquierda, arriba, derecha, abajo.
    pub relleno: [f32; 4],
    /// Separación entre hijos (x, y).
    pub espacio: [f32; 2],
    /// Alineación en el eje principal.
    pub alinear: Alinear,
    /// Alineación en el eje cruzado.
    pub cruzado: Alinear,
    /// Reparte el espacio libre entre los hijos según su `flex` (*child force expand*).
    pub expandir_hijos: bool,
    /// El contenedor toma el tamaño de su contenido (*fit content*).
    pub ajustar: bool,
    /// Rejilla: columnas y tamaño de celda.
    pub columnas: u32,
    pub celda: [f32; 2],
}

impl Default for Contenedor {
    fn default() -> Self {
        Self {
            tipo: TipoContenedor::Vertical,
            relleno: [8.0; 4],
            espacio: [8.0, 8.0],
            alinear: Alinear::Inicio,
            cruzado: Alinear::Estirar,
            expandir_hijos: false,
            ajustar: false,
            columnas: 3,
            celda: [96.0, 96.0],
        }
    }
}

/// Alineación horizontal de un texto.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum AlinH {
    Izquierda,
    #[default]
    Centro,
    Derecha,
}

/// Alineación vertical de un texto.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum AlinV {
    Arriba,
    #[default]
    Medio,
    Abajo,
}

/// Dirección de llenado de una barra o deslizador.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Direccion {
    #[default]
    IzqDer,
    DerIzq,
    AbajoArriba,
    ArribaAbajo,
}

impl Direccion {
    pub const TODAS: [Direccion; 4] = [Direccion::IzqDer, Direccion::DerIzq, Direccion::AbajoArriba, Direccion::ArribaAbajo];
    pub fn nombre(self) -> &'static str {
        match self {
            Direccion::IzqDer => "Izquierda → derecha",
            Direccion::DerIzq => "Derecha → izquierda",
            Direccion::AbajoArriba => "Abajo → arriba",
            Direccion::ArribaAbajo => "Arriba → abajo",
        }
    }
    pub fn vertical(self) -> bool {
        matches!(self, Direccion::AbajoArriba | Direccion::ArribaAbajo)
    }
}

/// Imagen del proyecto o de la biblioteca.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Imagen {
    pub imagen: String,
    /// sRGB + alfa.
    pub tinte: [f32; 4],
    pub conservar_aspecto: bool,
    /// Bordes 9-slice en píxeles de la imagen (izq, arr, der, aba); 0 = sin.
    pub nueve: [f32; 4],
}

impl Default for Imagen {
    fn default() -> Self {
        Self { imagen: String::new(), tinte: [1.0; 4], conservar_aspecto: true, nueve: [0.0; 4] }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Texto {
    pub texto: String,
    pub alinear_h: AlinH,
    pub alinear_v: AlinV,
    /// Parte las líneas al ancho del rectángulo.
    pub ajuste_linea: bool,
    /// Dentro de un contenedor, su tamaño preferido es el del texto.
    pub ajustar_tam: bool,
}

impl Default for Texto {
    fn default() -> Self {
        Self { texto: "Texto".into(), alinear_h: AlinH::Centro, alinear_v: AlinV::Medio, ajuste_linea: true, ajustar_tam: false }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Boton {
    pub texto: String,
}

impl Default for Boton {
    fn default() -> Self {
        Self { texto: "Botón".into() }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Barra {
    pub valor: f32,
    pub min: f32,
    pub max: f32,
    pub direccion: Direccion,
}

impl Default for Barra {
    fn default() -> Self {
        Self { valor: 0.75, min: 0.0, max: 1.0, direccion: Direccion::IzqDer }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Deslizador {
    pub valor: f32,
    pub min: f32,
    pub max: f32,
    /// 0 = continuo.
    pub paso: f32,
    pub direccion: Direccion,
}

impl Default for Deslizador {
    fn default() -> Self {
        Self { valor: 0.5, min: 0.0, max: 1.0, paso: 0.0, direccion: Direccion::IzqDer }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Casilla {
    pub activa: bool,
    pub texto: String,
}

impl Default for Casilla {
    fn default() -> Self {
        Self { activa: false, texto: "Opción".into() }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CampoTexto {
    pub texto: String,
    /// Texto gris cuando está vacío.
    pub marcador: String,
    /// Caracteres máximos (0 = sin límite).
    pub max: u32,
}

/// Lo que dibuja un elemento.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ClaseWidget {
    /// Caja con el fondo, borde, radio y sombra del estilo.
    Panel,
    Imagen(Imagen),
    Texto(Texto),
    Boton(Boton),
    Barra(Barra),
    Deslizador(Deslizador),
    Casilla(Casilla),
    CampoTexto(CampoTexto),
    /// Lo dibuja un script con `ctx.ui_draw` (minimapa, indicadores…).
    Personalizado,
}

/// Tipo de widget sin datos (claves de los temas).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum TipoWidget {
    Panel,
    Imagen,
    Texto,
    Boton,
    Barra,
    Deslizador,
    Casilla,
    CampoTexto,
    Personalizado,
}

impl TipoWidget {
    pub const TODOS: [TipoWidget; 9] = [
        TipoWidget::Panel,
        TipoWidget::Imagen,
        TipoWidget::Texto,
        TipoWidget::Boton,
        TipoWidget::Barra,
        TipoWidget::Deslizador,
        TipoWidget::Casilla,
        TipoWidget::CampoTexto,
        TipoWidget::Personalizado,
    ];
    /// Clave en los temas (`.mtema`).
    pub fn clave(self) -> &'static str {
        match self {
            TipoWidget::Panel => "Panel",
            TipoWidget::Imagen => "Imagen",
            TipoWidget::Texto => "Texto",
            TipoWidget::Boton => "Boton",
            TipoWidget::Barra => "Barra",
            TipoWidget::Deslizador => "Deslizador",
            TipoWidget::Casilla => "Casilla",
            TipoWidget::CampoTexto => "CampoTexto",
            TipoWidget::Personalizado => "Personalizado",
        }
    }
    pub fn nombre(self) -> &'static str {
        match self {
            TipoWidget::Panel => "Panel",
            TipoWidget::Imagen => "Imagen",
            TipoWidget::Texto => "Texto",
            TipoWidget::Boton => "Botón",
            TipoWidget::Barra => "Barra de progreso",
            TipoWidget::Deslizador => "Deslizador",
            TipoWidget::Casilla => "Casilla",
            TipoWidget::CampoTexto => "Campo de texto",
            TipoWidget::Personalizado => "Widget personalizado",
        }
    }
    /// Partes con estilo propio además de la principal.
    pub fn partes(self) -> &'static [&'static str] {
        match self {
            TipoWidget::Barra => &["relleno"],
            TipoWidget::Deslizador => &["relleno", "tirador"],
            TipoWidget::Casilla => &["marca"],
            TipoWidget::CampoTexto => &["cursor", "marcador"],
            _ => &[],
        }
    }
    /// ¿Reacciona al ratón por sí mismo (cambia de estado al pasar o pulsar)?
    pub fn interactivo(self) -> bool {
        matches!(self, TipoWidget::Boton | TipoWidget::Deslizador | TipoWidget::Casilla | TipoWidget::CampoTexto)
    }
    pub fn nuevo(self) -> ClaseWidget {
        match self {
            TipoWidget::Panel => ClaseWidget::Panel,
            TipoWidget::Imagen => ClaseWidget::Imagen(Imagen::default()),
            TipoWidget::Texto => ClaseWidget::Texto(Texto::default()),
            TipoWidget::Boton => ClaseWidget::Boton(Boton::default()),
            TipoWidget::Barra => ClaseWidget::Barra(Barra::default()),
            TipoWidget::Deslizador => ClaseWidget::Deslizador(Deslizador::default()),
            TipoWidget::Casilla => ClaseWidget::Casilla(Casilla::default()),
            TipoWidget::CampoTexto => ClaseWidget::CampoTexto(CampoTexto { marcador: "Escribe…".into(), ..Default::default() }),
            TipoWidget::Personalizado => ClaseWidget::Personalizado,
        }
    }
    /// Tamaño al crearlo desde el editor (unidades del lienzo).
    pub fn tam_inicial(self) -> [f32; 2] {
        match self {
            TipoWidget::Panel => [400.0, 260.0],
            TipoWidget::Imagen => [128.0, 128.0],
            TipoWidget::Texto => [320.0, 60.0],
            TipoWidget::Boton => [240.0, 64.0],
            TipoWidget::Barra | TipoWidget::Deslizador => [320.0, 28.0],
            TipoWidget::Casilla => [240.0, 40.0],
            TipoWidget::CampoTexto => [320.0, 52.0],
            TipoWidget::Personalizado => [200.0, 200.0],
        }
    }
}

impl ClaseWidget {
    pub fn tipo(&self) -> TipoWidget {
        match self {
            ClaseWidget::Panel => TipoWidget::Panel,
            ClaseWidget::Imagen(_) => TipoWidget::Imagen,
            ClaseWidget::Texto(_) => TipoWidget::Texto,
            ClaseWidget::Boton(_) => TipoWidget::Boton,
            ClaseWidget::Barra(_) => TipoWidget::Barra,
            ClaseWidget::Deslizador(_) => TipoWidget::Deslizador,
            ClaseWidget::Casilla(_) => TipoWidget::Casilla,
            ClaseWidget::CampoTexto(_) => TipoWidget::CampoTexto,
            ClaseWidget::Personalizado => TipoWidget::Personalizado,
        }
    }
    /// Texto principal (etiqueta, contenido…), si lo tiene.
    pub fn texto(&self) -> Option<&str> {
        match self {
            ClaseWidget::Texto(t) => Some(&t.texto),
            ClaseWidget::Boton(b) => Some(&b.texto),
            ClaseWidget::Casilla(c) => Some(&c.texto),
            ClaseWidget::CampoTexto(c) => Some(&c.texto),
            _ => None,
        }
    }
    pub fn texto_mut(&mut self) -> Option<&mut String> {
        match self {
            ClaseWidget::Texto(t) => Some(&mut t.texto),
            ClaseWidget::Boton(b) => Some(&mut b.texto),
            ClaseWidget::Casilla(c) => Some(&mut c.texto),
            ClaseWidget::CampoTexto(c) => Some(&mut c.texto),
            _ => None,
        }
    }
    /// Valor numérico: barra y deslizador; la casilla da 0 o 1.
    pub fn valor(&self) -> f32 {
        match self {
            ClaseWidget::Barra(b) => b.valor,
            ClaseWidget::Deslizador(d) => d.valor,
            ClaseWidget::Casilla(c) => c.activa as i32 as f32,
            _ => 0.0,
        }
    }
    pub fn fijar_valor(&mut self, v: f32) -> bool {
        match self {
            ClaseWidget::Barra(b) => b.valor = v.clamp(b.min.min(b.max), b.max.max(b.min)),
            ClaseWidget::Deslizador(d) => d.valor = cuantizar(v, d.min, d.max, d.paso),
            ClaseWidget::Casilla(c) => c.activa = v > 0.5,
            _ => return false,
        }
        true
    }
}

/// Ajusta un valor al rango y al paso.
pub fn cuantizar(v: f32, min: f32, max: f32, paso: f32) -> f32 {
    let (lo, hi) = (min.min(max), max.max(min));
    let mut v = v.clamp(lo, hi);
    if paso > 0.0 {
        v = (lo + ((v - lo) / paso).round() * paso).clamp(lo, hi);
    }
    v
}

/// Widget de un elemento con su estilo.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WidgetUi {
    pub clase: ClaseWidget,
    /// Nombre del estilo; vacío = el del tema para este tipo.
    pub estilo: String,
    /// Retoques locales (ganan al estilo y al tema).
    pub local: Estilo,
}

impl Default for WidgetUi {
    fn default() -> Self {
        Self { clase: ClaseWidget::Panel, estilo: String::new(), local: Estilo::default() }
    }
}

impl WidgetUi {
    pub fn nuevo(t: TipoWidget) -> Self {
        Self { clase: t.nuevo(), ..Default::default() }
    }
}

/// Raíz de una instancia de plantilla (`.mui`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct InstanciaUi {
    /// Ruta de la plantilla en el proyecto.
    pub plantilla: String,
    /// Propiedades retocadas en esta instancia (`<id local>/<campo>`); no se
    /// pisan al sincronizar con la plantilla.
    pub overrides: std::collections::BTreeSet<String>,
}

/// Entidad creada desde una plantilla: su id dentro de ella.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct OrigenPlantilla(pub u64);

/// Ruta de las carpetas de interfaz dentro del proyecto.
pub const DIR_UI: &str = "ui";
pub const EXT_ESTILO: &str = "mestilo";
pub const EXT_TEMA: &str = "mtema";
pub const EXT_PLANTILLA: &str = "mui";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escala_del_lienzo_como_unity() {
        let l = Lienzo::default();
        assert!((l.factor(1920.0, 1080.0) - 1.0).abs() < 1e-5);
        assert!((l.factor(3840.0, 2160.0) - 2.0).abs() < 1e-5);
        let ancho = Lienzo { ajuste: 0.0, ..Default::default() };
        assert!((ancho.factor(960.0, 2000.0) - 0.5).abs() < 1e-5, "iguala el ancho");
        let alto = Lienzo { ajuste: 1.0, ..Default::default() };
        assert!((alto.factor(4000.0, 540.0) - 0.5).abs() < 1e-5, "iguala el alto");
        let fijo = Lienzo { modo: ModoEscala::PixelConstante, escala: 2.0, ..Default::default() };
        assert_eq!(fijo.factor(123.0, 456.0), 2.0);
    }

    #[test]
    fn rect_anclas_posicion_y_tam() {
        let mut r = RectUi::centrado(100.0, 50.0);
        let (mn, mx) = r.en(([0.0, 0.0], [800.0, 600.0]));
        assert_eq!((mn, mx), ([350.0, 275.0], [450.0, 325.0]));
        r.fijar_tam([200.0, 50.0]);
        assert_eq!(r.posicion(), [0.0, 0.0], "el pivote no se mueve");
        assert_eq!(r.en(([0.0, 0.0], [800.0, 600.0])).0, [300.0, 275.0]);
        // Cambiar a anclas estiradas conserva el rectángulo.
        r.cambiar_anclas([0.0, 0.0], [1.0, 1.0], [800.0, 600.0]);
        assert_eq!(r.en(([0.0, 0.0], [800.0, 600.0])), ([300.0, 275.0], [500.0, 325.0]));
        // Y estirado sigue al padre.
        let (mn, mx) = r.en(([0.0, 0.0], [1000.0, 600.0]));
        assert_eq!((mn[0], mx[0]), (300.0, 700.0));
        let a = RectUi::anclado([1.0, 0.0], [1.0, 0.0], [-20.0, 20.0], [100.0, 40.0]);
        assert_eq!(a.en(([0.0, 0.0], [800.0, 600.0])), ([680.0, 20.0], [780.0, 60.0]));
    }

    #[test]
    fn widgets_serializan_y_valores() {
        let mut w = WidgetUi::nuevo(TipoWidget::Deslizador);
        if let ClaseWidget::Deslizador(d) = &mut w.clase {
            d.paso = 0.25;
        }
        assert!(w.clase.fijar_valor(0.6));
        assert_eq!(w.clase.valor(), 0.5);
        let json = serde_json::to_string(&w).unwrap();
        let back: WidgetUi = serde_json::from_str(&json).unwrap();
        assert_eq!(back, w);
        // Campos ausentes por defecto (escenas antiguas o escritas a mano).
        let t: WidgetUi = serde_json::from_str(r#"{"clase":{"Texto":{"texto":"Hola"}}}"#).unwrap();
        assert_eq!(t.clase.texto(), Some("Hola"));
        assert!(t.estilo.is_empty());
        let r: RectUi = serde_json::from_str(r#"{"rotacion":10}"#).unwrap();
        assert_eq!((r.rotacion, r.opacidad, r.visible), (10.0, 1.0, true));
    }
}
