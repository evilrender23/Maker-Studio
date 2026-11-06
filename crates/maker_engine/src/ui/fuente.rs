//! Fuentes TTF/OTF y atlas de glifos.
//!
//! Las fuentes se leen con `skrifa` (el lector de fuentes que ya usa egui):
//! métricas reales del archivo (unidades por em, ascendente, descendente,
//! interlineado y avance de cada glifo) y contornos. Los glifos se rasterizan
//! aquí con un acumulador de área exacta (el algoritmo de `font-rs`) y se
//! guardan en un atlas de cobertura de 8 bits que el renderizador sube a la GPU
//! solo en las filas que cambian.
//!
//! Sin *shaping*: texto de izquierda a derecha, un glifo por carácter, sin
//! ligaduras ni *kerning*.

use skrifa::instance::{LocationRef, Size};
use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::{FontRef, GlyphId, MetadataProvider};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

/// Fuente del motor (id 0): Ubuntu Light, la que trae egui.
pub const FUENTE_MOTOR: &[u8] = epaint_default_fonts::UBUNTU_LIGHT;

pub struct Fuente {
    datos: Arc<[u8]>,
    pub upem: f32,
    pub ascendente: f32,
    pub descendente: f32,
    pub interlineado: f32,
    glifos: HashMap<char, (u32, f32)>,
}

impl Fuente {
    pub fn desde(datos: Arc<[u8]>) -> Result<Fuente, String> {
        let f = FontRef::new(&datos).map_err(|e| format!("fuente no válida: {e}"))?;
        let m = f.metrics(Size::unscaled(), LocationRef::default());
        if m.units_per_em == 0 {
            return Err("fuente sin unidades por em".into());
        }
        Ok(Fuente { upem: m.units_per_em as f32, ascendente: m.ascent, descendente: m.descent, interlineado: m.leading, glifos: HashMap::new(), datos: datos.clone() })
    }
    fn fuente(&self) -> FontRef<'_> {
        FontRef::new(&self.datos).expect("validada al cargar")
    }
    /// Glifo y avance (en unidades de la fuente) de un carácter; glifo 0 si no lo tiene.
    pub fn glifo(&mut self, c: char) -> (u32, f32) {
        if let Some(g) = self.glifos.get(&c) {
            return *g;
        }
        let f = FontRef::new(&self.datos).expect("validada al cargar");
        let gid = f.charmap().map(c).unwrap_or(GlyphId::NOTDEF);
        let adv = f.glyph_metrics(Size::unscaled(), LocationRef::default()).advance_width(gid).unwrap_or(0.0);
        let g = (gid.to_u32(), adv);
        self.glifos.insert(c, g);
        g
    }
    /// Alto de línea en píxeles para un tamaño (`px` = tamaño del em).
    pub fn alto_linea(&self, px: f32) -> f32 {
        (self.ascendente - self.descendente + self.interlineado) * px / self.upem
    }
    pub fn ascendente_px(&self, px: f32) -> f32 {
        self.ascendente * px / self.upem
    }
    /// Cobertura del glifo a `px` píxeles por em: (ancho, alto, píxeles,
    /// izquierda, arriba) con la esquina relativa al origen del glifo (y hacia abajo).
    pub fn rasterizar(&self, gid: u32, px: f32) -> Option<(u32, u32, Vec<u8>, i32, i32)> {
        let f = self.fuente();
        let g = f.outline_glyphs().get(GlyphId::new(gid))?;
        let mut pen = Trazos { s: Vec::new(), ini: (0.0, 0.0), cur: (0.0, 0.0), tol: 0.2 };
        g.draw(DrawSettings::unhinted(Size::new(px), LocationRef::default()), &mut pen).ok()?;
        if pen.s.is_empty() {
            return None;
        }
        // y de la fuente hacia arriba → píxeles hacia abajo.
        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for &(a, b) in &pen.s {
            for p in [a, b] {
                x0 = x0.min(p.0);
                x1 = x1.max(p.0);
                y0 = y0.min(-p.1);
                y1 = y1.max(-p.1);
            }
        }
        let (ix, iy) = (x0.floor() as i32 - 1, y0.floor() as i32 - 1);
        let w = (x1.ceil() as i32 - ix + 1).max(1) as u32;
        let h = (y1.ceil() as i32 - iy + 1).max(1) as u32;
        if w > 1024 || h > 1024 {
            return None;
        }
        let mut r = Raster::new(w as usize, h as usize);
        for &(a, b) in &pen.s {
            r.linea((a.0 - ix as f32, -a.1 - iy as f32), (b.0 - ix as f32, -b.1 - iy as f32));
        }
        Some((w, h, r.cobertura(), ix, iy))
    }
}

/// Contorno aplanado en segmentos.
struct Trazos {
    s: Vec<((f32, f32), (f32, f32))>,
    ini: (f32, f32),
    cur: (f32, f32),
    tol: f32,
}

impl Trazos {
    fn seg(&mut self, p: (f32, f32)) {
        if p != self.cur {
            self.s.push((self.cur, p));
        }
        self.cur = p;
    }
    fn pasos(&self, largo: f32) -> usize {
        ((largo / self.tol).sqrt().ceil() as usize).clamp(1, 64)
    }
}

fn dist(a: (f32, f32), b: (f32, f32)) -> f32 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

impl OutlinePen for Trazos {
    fn move_to(&mut self, x: f32, y: f32) {
        self.ini = (x, y);
        self.cur = (x, y);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.seg((x, y));
    }
    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        let p0 = self.cur;
        let n = self.pasos(dist(p0, (cx, cy)) + dist((cx, cy), (x, y)));
        for i in 1..=n {
            let t = i as f32 / n as f32;
            let u = 1.0 - t;
            self.seg((u * u * p0.0 + 2.0 * u * t * cx + t * t * x, u * u * p0.1 + 2.0 * u * t * cy + t * t * y));
        }
    }
    fn curve_to(&mut self, c0x: f32, c0y: f32, c1x: f32, c1y: f32, x: f32, y: f32) {
        let p0 = self.cur;
        let n = self.pasos(dist(p0, (c0x, c0y)) + dist((c0x, c0y), (c1x, c1y)) + dist((c1x, c1y), (x, y)));
        for i in 1..=n {
            let t = i as f32 / n as f32;
            let u = 1.0 - t;
            let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
            self.seg((a * p0.0 + b * c0x + c * c1x + d * x, a * p0.1 + b * c0y + c * c1y + d * y));
        }
    }
    fn close(&mut self) {
        let i = self.ini;
        self.seg(i);
    }
}

/// Rasterizador de área exacta (Raph Levien, `font-rs`): cada segmento suma
/// su área firmada a un acumulador; la suma por filas da la cobertura.
struct Raster {
    w: usize,
    h: usize,
    a: Vec<f32>,
}

impl Raster {
    fn new(w: usize, h: usize) -> Raster {
        Raster { w, h, a: vec![0.0; w * h + 4] }
    }
    fn linea(&mut self, p0: (f32, f32), p1: (f32, f32)) {
        if (p0.1 - p1.1).abs() <= f32::EPSILON {
            return;
        }
        let (dir, p0, p1) = if p0.1 < p1.1 { (1.0, p0, p1) } else { (-1.0, p1, p0) };
        let dxdy = (p1.0 - p0.0) / (p1.1 - p0.1);
        let mut x = p0.0;
        let y0 = p0.1.max(0.0) as usize;
        if p0.1 < 0.0 {
            x -= p0.1 * dxdy;
        }
        let ymax = (p1.1.ceil() as usize).min(self.h);
        for y in y0..ymax {
            let ls = y * self.w;
            let dy = ((y + 1) as f32).min(p1.1) - (y as f32).max(p0.1);
            let xn = x + dxdy * dy;
            let d = dy * dir;
            let (xa, xb) = if x < xn { (x, xn) } else { (xn, x) };
            let xa = xa.max(0.0);
            let xb = xb.min(self.w as f32 - 0.001).max(xa);
            let xaf = xa.floor();
            let xai = xaf as usize;
            let xbc = xb.ceil();
            let xbi = xbc as usize;
            if xbi <= xai + 1 {
                let xmf = (0.5 * (x + xn)).clamp(xa, xb) - xaf;
                self.a[ls + xai] += d - d * xmf;
                self.a[ls + xai + 1] += d * xmf;
            } else {
                let s = (xb - xa).recip();
                let x0f = xa - xaf;
                let a0 = 0.5 * s * (1.0 - x0f) * (1.0 - x0f);
                let x1f = xb - xbc + 1.0;
                let am = 0.5 * s * x1f * x1f;
                self.a[ls + xai] += d * a0;
                if xbi == xai + 2 {
                    self.a[ls + xai + 1] += d * (1.0 - a0 - am);
                } else {
                    let a1 = s * (1.5 - x0f);
                    self.a[ls + xai + 1] += d * (a1 - a0);
                    for xi in xai + 2..xbi - 1 {
                        self.a[ls + xi] += d * s;
                    }
                    let a2 = a1 + (xbi - xai - 3) as f32 * s;
                    self.a[ls + xbi - 1] += d * (1.0 - a2 - am);
                }
                self.a[ls + xbi] += d * am;
            }
            x = xn;
        }
    }
    fn cobertura(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.w * self.h);
        // Suma corrida sobre todo el búfer, como `font-rs`: lo que un segmento
        // deja pasado el borde derecho se compensa al empezar la fila siguiente.
        let mut acc = 0.0f32;
        for i in 0..self.w * self.h {
            acc += self.a[i];
            out.push((acc.abs().min(1.0) * 255.0 + 0.5) as u8);
        }
        out
    }
}

/// Fuentes cargadas. La 0 es la del motor.
pub struct Fuentes {
    lista: Vec<Fuente>,
    por_ruta: HashMap<String, u32>,
}

impl Default for Fuentes {
    fn default() -> Self {
        Self::new()
    }
}

impl Fuentes {
    pub fn new() -> Fuentes {
        let motor = Fuente::desde(Arc::from(FUENTE_MOTOR)).expect("fuente del motor válida");
        Fuentes { lista: vec![motor], por_ruta: HashMap::new() }
    }
    /// Id de una fuente por ruta del proyecto (`biblioteca://` vale); "" o
    /// fallo = la del motor. Se carga una sola vez.
    pub fn id(&mut self, ruta: &str, raiz: Option<&Path>) -> u32 {
        if ruta.is_empty() {
            return 0;
        }
        if let Some(&i) = self.por_ruta.get(ruta) {
            return i;
        }
        let p = crate::biblioteca::resolve(ruta, raiz);
        let id = match std::fs::read(&p).map_err(|e| e.to_string()).and_then(|b| Fuente::desde(Arc::from(b))) {
            Ok(f) => {
                self.lista.push(f);
                (self.lista.len() - 1) as u32
            }
            Err(e) => {
                crate::log::error("interfaz", format!("fuente {ruta}: {e}"));
                0
            }
        };
        self.por_ruta.insert(ruta.to_string(), id);
        id
    }
    pub fn get(&self, id: u32) -> &Fuente {
        self.lista.get(id as usize).unwrap_or(&self.lista[0])
    }
    /// Glifo de un carácter: si la fuente no lo tiene, el de la fuente del motor.
    /// Devuelve (fuente, glifo, avance en píxeles a `px`).
    pub fn glifo(&mut self, id: u32, c: char, px: f32) -> (u32, u32, f32) {
        let id = if (id as usize) < self.lista.len() { id } else { 0 };
        let f = &mut self.lista[id as usize];
        let (g, adv) = f.glifo(c);
        if g == 0 && id != 0 && !c.is_whitespace() {
            let m = &mut self.lista[0];
            let (g0, adv0) = m.glifo(c);
            if g0 != 0 {
                return (0, g0, adv0 * px / m.upem);
            }
        }
        let f = &self.lista[id as usize];
        (id, g, adv * px / f.upem)
    }
}

/// Un glifo colocado (píxeles; `y` = línea base).
#[derive(Clone, Copy, Debug)]
pub struct GlifoPos {
    pub fuente: u32,
    pub gid: u32,
    pub x: f32,
    pub y: f32,
    pub linea: u32,
}

/// Una línea maquetada: rango de glifos y ancho.
#[derive(Clone, Copy, Debug)]
pub struct Linea {
    pub inicio: usize,
    pub fin: usize,
    pub ancho: f32,
}

/// Maqueta `texto` a `px` píxeles por em con la fuente `id`, partiendo líneas
/// en espacios si pasan de `ancho_max`. Deja los glifos en `out` y las
/// líneas en `lineas` (sin alinear) y devuelve el tamaño total.
pub fn maquetar(fs: &mut Fuentes, id: u32, px: f32, texto: &str, ancho_max: Option<f32>, out: &mut Vec<GlifoPos>, lineas: &mut Vec<Linea>) -> [f32; 2] {
    out.clear();
    lineas.clear();
    let (lh, asc) = {
        let f = fs.get(id);
        (f.alto_linea(px), f.ascendente_px(px))
    };
    let mut x = 0.0f32;
    // x tras el último glifo con tinta de la línea (los espacios finales no cuentan).
    let mut tinta = 0.0f32;
    let mut inicio = 0usize;
    // Último espacio de la línea: (índice del glifo siguiente, x tras el espacio, tinta antes).
    let mut corte: Option<(usize, f32, f32)> = None;
    let mut n_linea = 0u32;
    for c in texto.chars() {
        if c == '\n' {
            lineas.push(Linea { inicio, fin: out.len(), ancho: tinta });
            n_linea += 1;
            inicio = out.len();
            x = 0.0;
            tinta = 0.0;
            corte = None;
            continue;
        }
        let (f, g, adv) = fs.glifo(id, c, px);
        if c.is_whitespace() {
            out.push(GlifoPos { fuente: f, gid: g, x, y: 0.0, linea: n_linea });
            corte = Some((out.len(), x + adv, tinta));
            x += adv;
            continue;
        }
        if let Some(maxw) = ancho_max
            && x + adv > maxw + 0.01
            && out.len() > inicio
        {
            match corte {
                Some((i, xs, t)) if i > inicio => {
                    // Lo que hay tras el último espacio pasa a la línea nueva.
                    lineas.push(Linea { inicio, fin: i, ancho: t });
                    n_linea += 1;
                    for g in &mut out[i..] {
                        g.x -= xs;
                        g.linea = n_linea;
                    }
                    inicio = i;
                    x -= xs;
                }
                _ => {
                    lineas.push(Linea { inicio, fin: out.len(), ancho: tinta });
                    n_linea += 1;
                    inicio = out.len();
                    x = 0.0;
                }
            }
            corte = None;
        }
        out.push(GlifoPos { fuente: f, gid: g, x, y: 0.0, linea: n_linea });
        x += adv;
        tinta = x;
    }
    lineas.push(Linea { inicio, fin: out.len(), ancho: tinta });
    for g in out.iter_mut() {
        g.y = asc + g.linea as f32 * lh;
    }
    let ancho = lineas.iter().map(|l| l.ancho).fold(0.0, f32::max);
    [ancho, lineas.len() as f32 * lh]
}

/// Glifo en el atlas (píxeles del atlas y desplazamiento desde el origen del glifo).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EntradaAtlas {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    pub izq: i32,
    pub arriba: i32,
}

/// Atlas de cobertura (R8) con empaquetado por estantes.
pub struct AtlasGlifos {
    pub tam: u32,
    pub pixeles: Vec<u8>,
    /// Filas [y0, y1) cambiadas desde la última subida.
    pub sucio: Option<(u32, u32)>,
    /// Cambia cuando el atlas crece o se vacía (hay que recrear la textura).
    pub generacion: u64,
    x: u32,
    y: u32,
    fila: u32,
    mapa: HashMap<(u32, u32, u32), Option<EntradaAtlas>>,
}

pub const ATLAS_INICIAL: u32 = 512;
pub const ATLAS_MAX: u32 = 4096;

impl Default for AtlasGlifos {
    fn default() -> Self {
        Self::new(ATLAS_INICIAL)
    }
}

impl AtlasGlifos {
    pub fn new(tam: u32) -> AtlasGlifos {
        AtlasGlifos { tam, pixeles: vec![0; (tam * tam) as usize], sucio: Some((0, tam)), generacion: 1, x: 1, y: 1, fila: 0, mapa: HashMap::new() }
    }
    pub fn glifos(&self) -> usize {
        self.mapa.len()
    }
    /// El glifo `gid` de la fuente `f` a `px` píxeles por em (entero). `None`
    /// si no tiene contorno (espacio) o si el atlas se vació para hacer sitio
    /// (`generacion` cambia: hay que repetir el fotograma).
    pub fn obtener(&mut self, fs: &Fuentes, f: u32, gid: u32, px: u32) -> Option<EntradaAtlas> {
        let k = (f, gid, px);
        if let Some(e) = self.mapa.get(&k) {
            return *e;
        }
        let Some((w, h, cov, izq, arriba)) = fs.get(f).rasterizar(gid, px as f32) else {
            self.mapa.insert(k, None);
            return None;
        };
        let Some((x, y)) = self.sitio(w, h) else {
            self.vaciar();
            return None;
        };
        for r in 0..h {
            let d = ((y + r) * self.tam + x) as usize;
            self.pixeles[d..d + w as usize].copy_from_slice(&cov[(r * w) as usize..((r + 1) * w) as usize]);
        }
        self.marcar(y, y + h);
        let e = EntradaAtlas { x, y, w, h, izq, arriba };
        self.mapa.insert(k, Some(e));
        Some(e)
    }
    fn marcar(&mut self, y0: u32, y1: u32) {
        self.sucio = Some(match self.sucio {
            Some((a, b)) => (a.min(y0), b.max(y1)),
            None => (y0, y1),
        });
    }
    fn sitio(&mut self, w: u32, h: u32) -> Option<(u32, u32)> {
        loop {
            if self.x + w + 1 > self.tam {
                self.y += self.fila + 1;
                self.x = 1;
                self.fila = 0;
            }
            if self.y + h + 1 <= self.tam && w + 2 <= self.tam {
                let p = (self.x, self.y);
                self.x += w + 1;
                self.fila = self.fila.max(h);
                return Some(p);
            }
            if self.tam >= ATLAS_MAX {
                return None;
            }
            self.crecer();
        }
    }
    /// Dobla el lado conservando lo que hay (las coordenadas en píxeles no cambian).
    fn crecer(&mut self) {
        let nuevo = self.tam * 2;
        let mut p = vec![0u8; (nuevo * nuevo) as usize];
        for r in 0..self.tam {
            let s = (r * self.tam) as usize;
            let d = (r * nuevo) as usize;
            p[d..d + self.tam as usize].copy_from_slice(&self.pixeles[s..s + self.tam as usize]);
        }
        // Lo nuevo empieza en una franja bajo lo ocupado, a todo el ancho.
        self.y += self.fila + 1;
        self.x = 1;
        self.fila = 0;
        self.tam = nuevo;
        self.pixeles = p;
        self.sucio = Some((0, nuevo));
        self.generacion += 1;
    }
    fn vaciar(&mut self) {
        self.pixeles.fill(0);
        self.mapa.clear();
        self.x = 1;
        self.y = 1;
        self.fila = 0;
        self.sucio = Some((0, self.tam));
        self.generacion += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metricas_reales_de_la_fuente() {
        let f = Fuente::desde(Arc::from(FUENTE_MOTOR)).unwrap();
        // Ubuntu: 1000 unidades por em (dato del propio archivo, no supuesto).
        use skrifa::raw::TableProvider;
        let r = FontRef::new(FUENTE_MOTOR).unwrap();
        assert_eq!(f.upem, r.head().unwrap().units_per_em() as f32);
        assert!(f.ascendente > 0.0 && f.descendente < 0.0);
        assert!(f.alto_linea(20.0) > 20.0);
    }

    #[test]
    fn rasteriza_un_glifo_con_cobertura() {
        let mut fs = Fuentes::new();
        let (f, g, adv) = fs.glifo(0, 'H', 32.0);
        assert!(g != 0 && adv > 10.0);
        let (w, h, px, izq, arriba) = fs.get(f).rasterizar(g, 32.0).unwrap();
        assert!(w > 10 && h > 18, "{w}x{h}");
        assert!(arriba < 0 && izq >= -2, "la H está sobre la línea base: {izq} {arriba}");
        let llenos = px.iter().filter(|&&v| v > 200).count();
        let vacios = px.iter().filter(|&&v| v == 0).count();
        assert!(llenos > 40 && vacios > 40, "trazos sólidos y huecos: {llenos} {vacios}");
        // La barra horizontal de la H: la fila central tiene tinta de lado a lado.
        let fila = (h / 2) as usize;
        let tinta = (0..w as usize).filter(|&x| px[fila * w as usize + x] > 128).count();
        assert!(tinta as u32 > w / 2, "barra de la H: {tinta}/{w}");
        // La 'O' tiene un hueco en el centro.
        let (_, go, _) = fs.glifo(0, 'O', 48.0);
        let (w, h, px, _, _) = fs.get(0).rasterizar(go, 48.0).unwrap();
        assert!(px[(h / 2 * w + w / 2) as usize] < 30, "centro de la O vacío");
    }

    #[test]
    fn maqueta_y_parte_lineas() {
        let mut fs = Fuentes::new();
        let (mut g, mut l) = (Vec::new(), Vec::new());
        let t = maquetar(&mut fs, 0, 20.0, "hola mundo", None, &mut g, &mut l);
        assert_eq!(l.len(), 1);
        assert_eq!(g.len(), 10);
        let ancho_hola = maquetar(&mut fs, 0, 20.0, "hola", None, &mut Vec::new(), &mut Vec::new())[0];
        let ancho_mundo = maquetar(&mut fs, 0, 20.0, "mundo", None, &mut Vec::new(), &mut Vec::new())[0];
        let t2 = maquetar(&mut fs, 0, 20.0, "hola mundo", Some(ancho_hola.max(ancho_mundo) + 5.0), &mut g, &mut l);
        assert_eq!(l.len(), 2, "parte en el espacio");
        assert!((l[0].ancho - ancho_hola).abs() < 0.01, "el espacio final no cuenta: {} {}", l[0].ancho, ancho_hola);
        assert_eq!(g[5].x, 0.0, "«mundo» empieza la línea 2");
        assert!(t2[1] > t[1] * 1.9);
        let n = maquetar(&mut fs, 0, 20.0, "a\nb", None, &mut g, &mut l);
        assert_eq!(l.len(), 2);
        assert!(n[1] > 2.0 * 20.0);
        // Palabra más larga que el ancho: se corta por caracteres.
        maquetar(&mut fs, 0, 20.0, "aaaaaaaaaa", Some(30.0), &mut g, &mut l);
        assert!(l.len() > 2);
    }

    #[test]
    fn atlas_cachea_crece_y_marca_filas() {
        let fs = Fuentes::new();
        let mut a = AtlasGlifos::new(64);
        a.sucio = None;
        let mut f = Fuentes::new();
        let (_, g, _) = f.glifo(0, 'M', 30.0);
        let e = a.obtener(&fs, 0, g, 30).unwrap();
        assert!(a.sucio.is_some());
        assert_eq!(a.obtener(&fs, 0, g, 30), Some(e), "cacheado");
        let gen0 = a.generacion;
        for c in "ABCDEFGHIJKLMNOPQRSTUVWXYZ".chars() {
            let (_, g, _) = f.glifo(0, c, 30.0);
            a.obtener(&fs, 0, g, 30);
        }
        assert!(a.tam > 64 && a.generacion > gen0, "creció");
        assert_eq!(a.obtener(&fs, 0, g, 30), Some(e), "lo anterior sigue en su sitio");
        let (_, esp, _) = f.glifo(0, ' ', 30.0);
        assert_eq!(a.obtener(&fs, 0, esp, 30), None, "el espacio no tiene contorno");
    }
}
