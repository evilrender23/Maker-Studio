//! Estilos y temas de interfaz: assets del proyecto en JSON.
//!
//! - **Hoja de estilos** (`ui/estilos/*.mestilo`): estilos con nombre. Cada
//!   estilo tiene propiedades por estado (normal, encima, pulsado,
//!   desactivado, foco), partes con estilo propio (el relleno de una barra, el
//!   tirador de un deslizador…) y puede heredar de otro (`hereda`).
//! - **Tema** (`ui/temas/*.mtema`): qué estilo usa cada tipo de widget por
//!   defecto; también puede heredar de otro tema.
//!
//! Orden de resolución de cada propiedad (gana el primero que la define):
//! retoque local del elemento → estilo del elemento (y los que hereda) →
//! estilo del tema para ese tipo de widget (y los que hereda) → valor neutro
//! del motor. Dentro de cada capa, el estado actual gana a `normal`
//! (`pulsado` cae en `encima` antes que en `normal`).

use super::{TipoWidget, WidgetUi};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

/// Relleno de una caja.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Relleno {
    Ninguno,
    /// sRGB + alfa.
    Color([f32; 4]),
    /// Degradado lineal; `angulo` en grados (0 = de izquierda a derecha, 90 = de arriba abajo).
    Degradado { desde: [f32; 4], hasta: [f32; 4], angulo: f32 },
}

impl Relleno {
    /// Color principal (para editar y para `set_color`).
    pub fn color(&self) -> [f32; 4] {
        match *self {
            Relleno::Ninguno => [0.0; 4],
            Relleno::Color(c) => c,
            Relleno::Degradado { desde, .. } => desde,
        }
    }
}

/// Sombra paralela de una caja.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Sombra {
    pub color: [f32; 4],
    pub desplazamiento: [f32; 2],
    pub difuminado: f32,
    pub expansion: f32,
}

impl Default for Sombra {
    fn default() -> Self {
        Self { color: [0.0, 0.0, 0.0, 0.0], desplazamiento: [0.0, 4.0], difuminado: 8.0, expansion: 0.0 }
    }
}

/// Contorno o sombra del texto (`desplazamiento` [0,0] = contorno de `ancho`).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EfectoTexto {
    pub color: [f32; 4],
    pub ancho: f32,
    pub desplazamiento: [f32; 2],
}

impl Default for EfectoTexto {
    fn default() -> Self {
        Self { color: [0.0, 0.0, 0.0, 0.0], ancho: 0.0, desplazamiento: [0.0, 0.0] }
    }
}

macro_rules! props {
    ($($(#[$m:meta])* $f:ident : $t:ty),* $(,)?) => {
        /// Propiedades de un estado de un estilo. `None` = no la define esta capa.
        #[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
        #[serde(default)]
        pub struct PropsEstilo {
            $($(#[$m])* #[serde(skip_serializing_if = "Option::is_none")] pub $f: Option<$t>,)*
        }
        impl PropsEstilo {
            /// Rellena las propiedades que faltan con las de `o`.
            pub fn completar(&mut self, o: &PropsEstilo) {
                $(if self.$f.is_none() { if let Some(v) = &o.$f { self.$f = Some(v.clone()); } })*
            }
            pub fn vacio(&self) -> bool {
                true $(&& self.$f.is_none())*
            }
            /// Nombres de las propiedades definidas.
            pub fn definidas(&self) -> Vec<&'static str> {
                let mut v = Vec::new();
                $(if self.$f.is_some() { v.push(stringify!($f)); })*
                v
            }
            /// Quita una propiedad por nombre.
            pub fn quitar(&mut self, nombre: &str) {
                match nombre { $(stringify!($f) => self.$f = None,)* _ => {} }
            }
            /// Copia una propiedad por nombre desde `o`.
            pub fn copiar(&mut self, nombre: &str, o: &PropsEstilo) {
                match nombre { $(stringify!($f) => self.$f = o.$f.clone(),)* _ => {} }
            }
            pub const NOMBRES: &'static [&'static str] = &[$(stringify!($f)),*];
        }
    };
}

props! {
    fondo: Relleno,
    /// Imagen de fondo (ruta del proyecto o `biblioteca://`); "" = ninguna.
    imagen: String,
    /// Bordes 9-slice de la imagen de fondo, en píxeles de la imagen.
    nueve: [f32; 4],
    /// Tinte de la imagen, sRGB.
    tinte: [f32; 4],
    borde_color: [f32; 4],
    borde_ancho: f32,
    /// Radio de cada esquina: sup. izq., sup. der., inf. der., inf. izq.
    radio: [f32; 4],
    sombra: Sombra,
    /// Relleno interior del contenido: izq., arriba, der., abajo.
    relleno: [f32; 4],
    /// Fuente TTF/OTF; "" = la del motor.
    fuente: String,
    tam_texto: f32,
    color_texto: [f32; 4],
    contorno_texto: EfectoTexto,
    sombra_texto: EfectoTexto,
    /// Escala visual (p. ej. crecer al pasar el ratón).
    escala: f32,
    /// Desplazamiento visual (p. ej. hundirse al pulsar).
    desplazamiento: [f32; 2],
    opacidad: f32,
    /// Segundos del fundido al entrar en este estado.
    transicion: f32,
}

/// Estado de interacción de un widget.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Estado {
    #[default]
    Normal,
    Encima,
    Pulsado,
    Desactivado,
    Foco,
}

impl Estado {
    pub const TODOS: [Estado; 5] = [Estado::Normal, Estado::Encima, Estado::Pulsado, Estado::Desactivado, Estado::Foco];
    pub fn nombre(self) -> &'static str {
        match self {
            Estado::Normal => "Normal",
            Estado::Encima => "Encima",
            Estado::Pulsado => "Pulsado",
            Estado::Desactivado => "Desactivado",
            Estado::Foco => "Foco",
        }
    }
    pub fn indice(self) -> usize {
        self as usize
    }
}

/// Propiedades por estado.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EstadosEstilo {
    #[serde(skip_serializing_if = "PropsEstilo::vacio")]
    pub normal: PropsEstilo,
    #[serde(skip_serializing_if = "PropsEstilo::vacio")]
    pub encima: PropsEstilo,
    #[serde(skip_serializing_if = "PropsEstilo::vacio")]
    pub pulsado: PropsEstilo,
    #[serde(skip_serializing_if = "PropsEstilo::vacio")]
    pub desactivado: PropsEstilo,
    #[serde(skip_serializing_if = "PropsEstilo::vacio")]
    pub foco: PropsEstilo,
}

impl EstadosEstilo {
    pub fn de(&self, e: Estado) -> &PropsEstilo {
        match e {
            Estado::Normal => &self.normal,
            Estado::Encima => &self.encima,
            Estado::Pulsado => &self.pulsado,
            Estado::Desactivado => &self.desactivado,
            Estado::Foco => &self.foco,
        }
    }
    pub fn de_mut(&mut self, e: Estado) -> &mut PropsEstilo {
        match e {
            Estado::Normal => &mut self.normal,
            Estado::Encima => &mut self.encima,
            Estado::Pulsado => &mut self.pulsado,
            Estado::Desactivado => &mut self.desactivado,
            Estado::Foco => &mut self.foco,
        }
    }
    pub fn vacio(&self) -> bool {
        Estado::TODOS.iter().all(|e| self.de(*e).vacio())
    }
    /// Aplica esta capa: el estado (con sus respaldos) y luego `normal`.
    fn aplicar(&self, acc: &mut PropsEstilo, e: Estado) {
        match e {
            Estado::Normal => {}
            Estado::Pulsado => {
                acc.completar(&self.pulsado);
                acc.completar(&self.encima);
            }
            _ => acc.completar(self.de(e)),
        }
        acc.completar(&self.normal);
    }
}

/// Un estilo con nombre (o los retoques locales de un elemento).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Estilo {
    /// Estilo del que hereda lo que este no define.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub hereda: String,
    #[serde(flatten)]
    pub estados: EstadosEstilo,
    /// Partes del widget con estilo propio (`relleno`, `tirador`, `marca`…).
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub partes: BTreeMap<String, EstadosEstilo>,
}

impl Estilo {
    pub fn capa(&self, parte: Option<&str>) -> Option<&EstadosEstilo> {
        match parte {
            None => Some(&self.estados),
            Some(p) => self.partes.get(p),
        }
    }
    pub fn capa_mut(&mut self, parte: Option<&str>) -> &mut EstadosEstilo {
        match parte {
            None => &mut self.estados,
            Some(p) => self.partes.entry(p.to_string()).or_default(),
        }
    }
    pub fn vacio(&self) -> bool {
        self.hereda.is_empty() && self.estados.vacio() && self.partes.values().all(EstadosEstilo::vacio)
    }
    /// Quita partes sin nada (tras borrar retoques).
    pub fn limpiar(&mut self) {
        self.partes.retain(|_, p| !p.vacio());
    }
}

/// Archivo `.mestilo`: estilos con nombre.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HojaEstilos {
    pub estilos: BTreeMap<String, Estilo>,
}

/// Archivo `.mtema`: estilo por defecto de cada tipo de widget.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Tema {
    #[serde(skip_serializing_if = "String::is_empty")]
    pub hereda: String,
    /// Clave del tipo (`Boton`, `Texto`…) → nombre del estilo.
    pub tipos: BTreeMap<String, String>,
}

/// Estilos y temas cargados de un proyecto.
#[derive(Clone, Debug, Default)]
pub struct RecursosUi {
    /// Hojas por ruta relativa al proyecto.
    pub hojas: BTreeMap<String, HojaEstilos>,
    /// Temas por nombre (nombre del archivo sin extensión).
    pub temas: BTreeMap<String, Tema>,
    /// Ruta de cada tema.
    pub rutas_temas: BTreeMap<String, String>,
    /// Tema activo del proyecto ("" = ninguno: aspecto neutro del motor).
    pub tema: String,
    /// Estilo → hoja donde está definido (la última por orden de ruta gana).
    indice: HashMap<String, String>,
    /// Sube con cada cambio (los sistemas de interfaz se reconstruyen).
    pub generacion: u64,
}

const MAX_HERENCIA: usize = 16;

impl RecursosUi {
    /// Carga todos los `.mestilo` y `.mtema` del proyecto.
    pub fn cargar(raiz: &Path, tema: &str) -> RecursosUi {
        let mut r = RecursosUi { tema: tema.to_string(), ..Default::default() };
        crate::project::walk(raiz, &mut |p| {
            let rel = p.strip_prefix(raiz).unwrap_or(p).to_string_lossy().replace('\\', "/");
            match p.extension().and_then(|e| e.to_str()) {
                Some(super::EXT_ESTILO) => match std::fs::read_to_string(p).map_err(|e| e.to_string()).and_then(|t| serde_json::from_str::<HojaEstilos>(&t).map_err(|e| e.to_string())) {
                    Ok(h) => {
                        r.hojas.insert(rel, h);
                    }
                    Err(e) => crate::log::error("interfaz", format!("{rel}: {e}")),
                },
                Some(super::EXT_TEMA) => match std::fs::read_to_string(p).map_err(|e| e.to_string()).and_then(|t| serde_json::from_str::<Tema>(&t).map_err(|e| e.to_string())) {
                    Ok(t) => {
                        let nombre = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                        r.rutas_temas.insert(nombre.clone(), rel);
                        r.temas.insert(nombre, t);
                    }
                    Err(e) => crate::log::error("interfaz", format!("{rel}: {e}")),
                },
                _ => {}
            }
        });
        r.reindexar();
        r
    }

    /// Recalcula el índice de estilos tras editar hojas en memoria.
    pub fn reindexar(&mut self) {
        self.indice.clear();
        for (ruta, h) in &self.hojas {
            for n in h.estilos.keys() {
                if let Some(prev) = self.indice.insert(n.clone(), ruta.clone()) {
                    crate::log::warn("interfaz", format!("el estilo '{n}' está en {prev} y en {ruta}: se usa el de {ruta}"));
                }
            }
        }
        self.generacion += 1;
    }

    pub fn estilo(&self, nombre: &str) -> Option<&Estilo> {
        let hoja = self.indice.get(nombre)?;
        self.hojas.get(hoja)?.estilos.get(nombre)
    }
    /// Hoja que define un estilo.
    pub fn hoja_de(&self, nombre: &str) -> Option<&str> {
        self.indice.get(nombre).map(String::as_str)
    }
    pub fn nombres_estilos(&self) -> Vec<String> {
        let mut v: Vec<String> = self.indice.keys().cloned().collect();
        v.sort();
        v
    }

    /// Estilo del tema para un tipo de widget (siguiendo la herencia de temas).
    pub fn estilo_de_tipo(&self, tema: &str, tipo: TipoWidget) -> Option<&str> {
        let mut t = if tema.is_empty() { self.tema.as_str() } else { tema };
        for _ in 0..MAX_HERENCIA {
            let tm = self.temas.get(t)?;
            if let Some(s) = tm.tipos.get(tipo.clave()) {
                return Some(s);
            }
            if tm.hereda.is_empty() {
                return None;
            }
            t = &tm.hereda;
        }
        None
    }

    /// Valores finales (todas las propiedades definidas) de una parte de un
    /// widget en un estado, con el tema `tema` ("" = el del proyecto).
    pub fn resolver(&self, w: &WidgetUi, tema: &str, parte: Option<&str>, estado: Estado) -> PropsEstilo {
        let mut acc = PropsEstilo::default();
        if let Some(c) = w.local.capa(parte) {
            c.aplicar(&mut acc, estado);
        }
        if !w.estilo.is_empty() {
            self.aplicar_cadena(&w.estilo, parte, estado, &mut acc);
        }
        if let Some(s) = self.estilo_de_tipo(tema, w.clase.tipo()) {
            self.aplicar_cadena(s, parte, estado, &mut acc);
        }
        acc.completar(&PropsEstilo::motor(w.clase.tipo(), parte));
        acc
    }

    /// Como `resolver`, pero solo las capas del estilo y del tema (lo que un
    /// retoque local sustituye): para mostrar el valor heredado en el Inspector.
    pub fn resolver_sin_local(&self, w: &WidgetUi, tema: &str, parte: Option<&str>, estado: Estado) -> PropsEstilo {
        let sin = WidgetUi { local: Estilo::default(), clase: w.clase.clone(), estilo: w.estilo.clone() };
        self.resolver(&sin, tema, parte, estado)
    }

    fn aplicar_cadena(&self, nombre: &str, parte: Option<&str>, estado: Estado, acc: &mut PropsEstilo) {
        let mut n = nombre;
        for _ in 0..MAX_HERENCIA {
            let Some(e) = self.estilo(n) else { return };
            if let Some(c) = e.capa(parte) {
                c.aplicar(acc, estado);
            }
            if e.hereda.is_empty() || e.hereda == n {
                return;
            }
            n = &e.hereda;
        }
    }
}

impl PropsEstilo {
    /// Valores neutros del motor: lo mínimo para que cada widget se vea y se
    /// use. El aspecto real lo dan los estilos y temas del proyecto.
    pub fn motor(tipo: TipoWidget, parte: Option<&str>) -> PropsEstilo {
        let gris = |v: f32, a: f32| [v, v, v, a];
        let mut p = PropsEstilo {
            fondo: Some(Relleno::Ninguno),
            imagen: Some(String::new()),
            nueve: Some([0.0; 4]),
            tinte: Some([1.0; 4]),
            borde_color: Some(gris(0.0, 0.0)),
            borde_ancho: Some(0.0),
            radio: Some([0.0; 4]),
            sombra: Some(Sombra::default()),
            relleno: Some([0.0; 4]),
            fuente: Some(String::new()),
            tam_texto: Some(24.0),
            color_texto: Some(gris(1.0, 1.0)),
            contorno_texto: Some(EfectoTexto::default()),
            sombra_texto: Some(EfectoTexto::default()),
            escala: Some(1.0),
            desplazamiento: Some([0.0, 0.0]),
            opacidad: Some(1.0),
            transicion: Some(0.0),
        };
        match (tipo, parte) {
            (TipoWidget::Panel, None) => p.fondo = Some(Relleno::Color(gris(0.15, 0.85))),
            (TipoWidget::Boton | TipoWidget::CampoTexto | TipoWidget::Casilla, None) => {
                p.fondo = Some(Relleno::Color(gris(0.3, 1.0)));
                p.relleno = Some([8.0, 4.0, 8.0, 4.0]);
            }
            (TipoWidget::Barra | TipoWidget::Deslizador, None) => p.fondo = Some(Relleno::Color(gris(0.2, 1.0))),
            (_, Some("relleno")) => p.fondo = Some(Relleno::Color(gris(0.85, 1.0))),
            (_, Some("tirador")) => p.fondo = Some(Relleno::Color(gris(1.0, 1.0))),
            (_, Some("marca")) => p.fondo = Some(Relleno::Color(gris(1.0, 1.0))),
            (_, Some("cursor")) => p.fondo = Some(Relleno::Color(gris(1.0, 1.0))),
            (_, Some("marcador")) => p.color_texto = Some(gris(1.0, 0.45)),
            _ => {}
        }
        p
    }
}

/// Tema inicial que el proyecto puede copiar y editar (Contenido → Interfaz →
/// Crear tema inicial). No lo usa el motor por su cuenta.
pub fn tema_inicial() -> (HojaEstilos, Tema) {
    let txt = include_str!("tema_inicial.mestilo");
    let hoja: HojaEstilos = serde_json::from_str(txt).expect("tema inicial válido");
    let mut tema = Tema::default();
    for (k, v) in [("Panel", "panel"), ("Texto", "texto"), ("Boton", "boton"), ("Barra", "barra"), ("Deslizador", "deslizador"), ("Casilla", "casilla"), ("CampoTexto", "campo")] {
        tema.tipos.insert(k.into(), v.into());
    }
    (hoja, tema)
}

/// sRGB → lineal (los estilos guardan colores como los ve el usuario).
pub fn lineal(c: [f32; 4]) -> [f32; 4] {
    let f = |v: f32| if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
    [f(c[0]), f(c[1]), f(c[2]), c[3]]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::{ClaseWidget, TipoWidget, WidgetUi};

    fn recursos() -> RecursosUi {
        let hoja: HojaEstilos = serde_json::from_str(
            r#"{"estilos":{
                "base":{"normal":{"fondo":{"Color":[0.1,0.1,0.1,1]},"radio":[4,4,4,4],"tam_texto":20},
                        "encima":{"fondo":{"Color":[0.2,0.2,0.2,1]}}},
                "primario":{"hereda":"base","normal":{"fondo":{"Color":[0,0,1,1]}},
                            "pulsado":{"escala":0.95},
                            "partes":{"relleno":{"normal":{"fondo":{"Color":[0,1,0,1]}}}}},
                "rojo":{"normal":{"fondo":{"Color":[1,0,0,1]}}}
            }}"#,
        )
        .unwrap();
        let tema: Tema = serde_json::from_str(r#"{"tipos":{"Boton":"base"}}"#).unwrap();
        let tema2: Tema = serde_json::from_str(r#"{"hereda":"claro","tipos":{"Texto":"rojo"}}"#).unwrap();
        let mut r = RecursosUi { tema: "claro".into(), ..Default::default() };
        r.hojas.insert("ui/estilos/a.mestilo".into(), hoja);
        r.temas.insert("claro".into(), tema);
        r.temas.insert("hijo".into(), tema2);
        r.reindexar();
        r
    }

    #[test]
    fn orden_local_estilo_tema_motor() {
        let r = recursos();
        let mut w = WidgetUi::nuevo(TipoWidget::Boton);
        // Solo el tema: estilo «base».
        let p = r.resolver(&w, "", None, Estado::Normal);
        assert_eq!(p.fondo, Some(Relleno::Color([0.1, 0.1, 0.1, 1.0])));
        assert_eq!(p.tam_texto, Some(20.0));
        assert_eq!(p.opacidad, Some(1.0), "lo que nadie define viene del motor");
        // El estado gana a normal en la misma capa.
        assert_eq!(r.resolver(&w, "", None, Estado::Encima).fondo, Some(Relleno::Color([0.2, 0.2, 0.2, 1.0])));
        // Estilo del elemento (hereda de base): su fondo gana al tema, el radio se hereda.
        w.estilo = "primario".into();
        let p = r.resolver(&w, "", None, Estado::Normal);
        assert_eq!(p.fondo, Some(Relleno::Color([0.0, 0.0, 1.0, 1.0])));
        assert_eq!(p.radio, Some([4.0; 4]));
        // Pulsado cae en encima de la misma capa antes que en normal.
        let p = r.resolver(&w, "", None, Estado::Pulsado);
        assert_eq!(p.escala, Some(0.95));
        // Retoque local: gana a todo, solo esa propiedad.
        w.local.estados.normal.radio = Some([0.0; 4]);
        let p = r.resolver(&w, "", None, Estado::Normal);
        assert_eq!(p.radio, Some([0.0; 4]));
        assert_eq!(p.fondo, Some(Relleno::Color([0.0, 0.0, 1.0, 1.0])));
        // Partes.
        assert_eq!(r.resolver(&w, "", Some("relleno"), Estado::Normal).fondo, Some(Relleno::Color([0.0, 1.0, 0.0, 1.0])));
        let sin = r.resolver_sin_local(&w, "", None, Estado::Normal);
        assert_eq!(sin.radio, Some([4.0; 4]));
    }

    #[test]
    fn temas_heredan_y_cambiar_tema_reestiliza() {
        let r = recursos();
        let t = WidgetUi { clase: ClaseWidget::Texto(Default::default()), ..Default::default() };
        let b = WidgetUi::nuevo(TipoWidget::Boton);
        assert_eq!(r.estilo_de_tipo("hijo", TipoWidget::Texto), Some("rojo"));
        assert_eq!(r.estilo_de_tipo("hijo", TipoWidget::Boton), Some("base"), "heredado del tema padre");
        assert_eq!(r.resolver(&t, "hijo", None, Estado::Normal).fondo, Some(Relleno::Color([1.0, 0.0, 0.0, 1.0])));
        assert_eq!(r.resolver(&t, "", None, Estado::Normal).fondo, Some(Relleno::Ninguno), "el tema del proyecto no estiliza textos");
        assert_eq!(r.resolver(&b, "inexistente", None, Estado::Normal).fondo, PropsEstilo::motor(TipoWidget::Boton, None).fondo);
    }

    #[test]
    fn herencia_circular_no_cuelga() {
        let mut r = recursos();
        let h = r.hojas.get_mut("ui/estilos/a.mestilo").unwrap();
        h.estilos.insert("x".into(), Estilo { hereda: "y".into(), ..Default::default() });
        h.estilos.insert("y".into(), Estilo { hereda: "x".into(), ..Default::default() });
        r.reindexar();
        let w = WidgetUi { estilo: "x".into(), ..WidgetUi::nuevo(TipoWidget::Panel) };
        assert!(r.resolver(&w, "", None, Estado::Normal).fondo.is_some());
    }

    #[test]
    fn json_compacto_e_ida_y_vuelta() {
        let (hoja, tema) = tema_inicial();
        assert!(hoja.estilos.contains_key("boton"));
        let txt = serde_json::to_string(&hoja).unwrap();
        assert!(!txt.contains("null"), "las propiedades sin definir no se escriben");
        assert_eq!(serde_json::from_str::<HojaEstilos>(&txt).unwrap(), hoja);
        assert_eq!(serde_json::from_str::<Tema>(&serde_json::to_string(&tema).unwrap()).unwrap(), tema);
        let mut p = PropsEstilo { tam_texto: Some(3.0), ..Default::default() };
        assert_eq!(p.definidas(), vec!["tam_texto"]);
        p.quitar("tam_texto");
        assert!(p.vacio());
    }
}
