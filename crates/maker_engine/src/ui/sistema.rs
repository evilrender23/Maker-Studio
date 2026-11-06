//! Sistema de interfaz en ejecución: layout (anclas y contenedores), estilos
//! resueltos, entrada (ratón, teclado, foco), eventos para los scripts,
//! transiciones y la lista de dibujo.
//!
//! El layout y la resolución de estilos solo se rehacen cuando algo cambia
//! (`marcar_sucio`); la lista de dibujo, cuando cambia el layout o hay una
//! transición o un cursor en marcha. En régimen estable no reserva memoria:
//! todos los búferes se reutilizan.

use super::dibujo::*;
use super::estilo::{Estado, PropsEstilo, RecursosUi, Relleno, lineal};
use super::fuente::{AtlasGlifos, Fuentes, GlifoPos, Linea, maquetar};
use super::layout;
use super::{AlinH, AlinV, ClaseWidget, Contenedor, Direccion, Lienzo, RectUi, TipoWidget, WidgetUi};
use crate::components::{EntityId, Inactive};
use crate::input::Input;
use crate::scene::Scene;
use maker_api::{Key, UiDrawCmd, ui_draw};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Bits de eventos (1 << `maker_api::ui_event::*`).
pub mod ev {
    use maker_api::ui_event as e;
    pub const CLIC: u32 = 1 << e::CLICKED;
    pub const CAMBIO: u32 = 1 << e::CHANGED;
    pub const ENTRA: u32 = 1 << e::HOVER_ENTER;
    pub const PULSA: u32 = 1 << e::PRESSED;
    pub const SUELTA: u32 = 1 << e::RELEASED;
    pub const ENVIA: u32 = 1 << e::SUBMITTED;
    pub const SALE: u32 = 1 << e::HOVER_EXIT;
}

/// Imágenes usadas por la interfaz: ruta resuelta y tamaño real (cabecera del archivo).
#[derive(Default)]
pub struct Imagenes {
    rutas: Vec<PathBuf>,
    tams: Vec<Option<[u32; 2]>>,
    por_ruta: HashMap<String, u32>,
}

impl Imagenes {
    pub fn id(&mut self, ruta: &str, raiz: Option<&Path>) -> u32 {
        if ruta.is_empty() {
            return SIN_IMAGEN;
        }
        if let Some(&i) = self.por_ruta.get(ruta) {
            return i;
        }
        let p = crate::biblioteca::resolve(ruta, raiz);
        let tam = match image::image_dimensions(&p) {
            Ok((w, h)) => Some([w, h]),
            Err(e) => {
                crate::log::error("interfaz", format!("imagen {ruta}: {e}"));
                None
            }
        };
        self.rutas.push(p);
        self.tams.push(tam);
        let i = (self.rutas.len() - 1) as u32;
        self.por_ruta.insert(ruta.to_string(), i);
        i
    }
    /// Ruta absoluta (para cargar la textura).
    pub fn ruta(&self, id: u32) -> Option<&Path> {
        self.rutas.get(id as usize).map(PathBuf::as_path)
    }
    pub fn tam(&self, id: u32) -> Option<[u32; 2]> {
        self.tams.get(id as usize).copied().flatten()
    }
}

/// Valores animables (colores lineales).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Visual {
    fondo: [f32; 4],
    fondo2: [f32; 4],
    borde: [f32; 4],
    tinte: [f32; 4],
    texto: [f32; 4],
    escala: f32,
    desp: [f32; 2],
    opacidad: f32,
}

fn mezcla4(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t, a[3] + (b[3] - a[3]) * t]
}

impl Visual {
    fn mezcla(&self, o: &Visual, t: f32) -> Visual {
        Visual {
            fondo: mezcla4(self.fondo, o.fondo, t),
            fondo2: mezcla4(self.fondo2, o.fondo2, t),
            borde: mezcla4(self.borde, o.borde, t),
            tinte: mezcla4(self.tinte, o.tinte, t),
            texto: mezcla4(self.texto, o.texto, t),
            escala: self.escala + (o.escala - self.escala) * t,
            desp: [self.desp[0] + (o.desp[0] - self.desp[0]) * t, self.desp[1] + (o.desp[1] - self.desp[1]) * t],
            opacidad: self.opacidad + (o.opacidad - self.opacidad) * t,
        }
    }
}

/// Estilo resuelto de una parte en un estado, listo para dibujar.
#[derive(Clone, Copy, Debug, Default)]
struct Resuelto {
    v: Visual,
    degradado: Option<f32>,
    imagen: u32,
    nueve: [f32; 4],
    borde_ancho: f32,
    radio: [f32; 4],
    sombra: Option<([f32; 4], [f32; 2], f32, f32)>,
    relleno: [f32; 4],
    fuente: u32,
    tam_texto: f32,
    contorno: ([f32; 4], f32),
    sombra_texto: ([f32; 4], [f32; 2]),
    transicion: f32,
}

#[derive(Clone, Copy, Default)]
struct Anim {
    estado: Estado,
    desde: Visual,
    t: f32,
    dur: f32,
    iniciada: bool,
}

struct Nodo {
    id: EntityId,
    padre: u32,
    /// Índice tras el último descendiente.
    fin: u32,
    lienzo: u32,
    rect: RectUi,
    contenedor: Option<Contenedor>,
    widget: Option<WidgetUi>,
    tipo: Option<TipoWidget>,
    /// Imagen del widget Imagen.
    img: u32,
    min: [f32; 2],
    max: [f32; 2],
    xf: Afin,
    inv: Afin,
    recorte: Option<[f32; 4]>,
    recorte_hijos: Option<[f32; 4]>,
    visible: bool,
    opacidad: f32,
    interactuable: bool,
    /// [parte][estado]: parte 0 = principal.
    estilos: Vec<[Resuelto; 5]>,
    anim: Anim,
}

const NADA: u32 = u32::MAX;

struct LienzoActivo {
    factor: f32,
}

/// Lo que dibuja un script en un widget personalizado (retenido).
#[derive(Default)]
struct DibujoScript {
    cmds: Vec<(UiDrawCmd, u32, u32, u32)>,
    texto: String,
}

pub struct SistemaUi {
    pub recursos: RecursosUi,
    gen_recursos: u64,
    pub fuentes: Fuentes,
    pub atlas: AtlasGlifos,
    pub imagenes: Imagenes,
    raiz: Option<PathBuf>,
    nodos: Vec<Nodo>,
    indice: HashMap<EntityId, u32>,
    lienzos: Vec<LienzoActivo>,
    pantalla: [f32; 2],
    sucio: bool,
    redibujar: bool,
    pub lista: ListaDibujo,
    encima: Option<EntityId>,
    pulsado: Option<EntityId>,
    foco: Option<EntityId>,
    raton_antes: bool,
    eventos: HashMap<EntityId, u32>,
    /// Estados forzados (vista previa del editor de estilos).
    pub forzar: HashMap<EntityId, Estado>,
    dibujos: HashMap<EntityId, DibujoScript>,
    reloj: f32,
    glifos: Vec<GlifoPos>,
    lineas: Vec<Linea>,
    prefs: Vec<Vec<[f32; 2]>>,
    rects: Vec<Vec<([f32; 2], [f32; 2])>>,
    sobre_ui: bool,
    captura_raton: bool,
    captura_teclado: bool,
    /// Tema activo (sobrescribe el del proyecto; lo cambia un script).
    pub tema: Option<String>,
    /// Dibujos generados (estadística y tests).
    pub reconstrucciones: u64,
}

impl Default for SistemaUi {
    fn default() -> Self {
        Self::new(RecursosUi::default(), None)
    }
}

impl SistemaUi {
    pub fn new(recursos: RecursosUi, raiz: Option<&Path>) -> SistemaUi {
        SistemaUi {
            gen_recursos: u64::MAX,
            recursos,
            fuentes: Fuentes::new(),
            atlas: AtlasGlifos::default(),
            imagenes: Imagenes::default(),
            raiz: raiz.map(Path::to_path_buf),
            nodos: Vec::new(),
            indice: HashMap::new(),
            lienzos: Vec::new(),
            pantalla: [0.0, 0.0],
            sucio: true,
            redibujar: true,
            lista: ListaDibujo::default(),
            encima: None,
            pulsado: None,
            foco: None,
            raton_antes: false,
            eventos: HashMap::new(),
            forzar: HashMap::new(),
            dibujos: HashMap::new(),
            reloj: 0.0,
            glifos: Vec::new(),
            lineas: Vec::new(),
            prefs: Vec::new(),
            rects: Vec::new(),
            sobre_ui: false,
            captura_raton: false,
            captura_teclado: false,
            tema: None,
            reconstrucciones: 0,
        }
    }
    /// Carga estilos y temas del proyecto.
    pub fn del_proyecto(raiz: &Path, tema: &str) -> SistemaUi {
        SistemaUi::new(RecursosUi::cargar(raiz, tema), Some(raiz))
    }
    pub fn raiz(&self) -> Option<&Path> {
        self.raiz.as_deref()
    }
    pub fn marcar_sucio(&mut self) {
        self.sucio = true;
    }
    pub fn puntero_sobre_ui(&self) -> bool {
        self.sobre_ui
    }
    pub fn captura_raton(&self) -> bool {
        self.captura_raton
    }
    pub fn captura_teclado(&self) -> bool {
        self.captura_teclado
    }
    pub fn evento(&self, id: EntityId, bit: u32) -> bool {
        self.eventos.get(&id).is_some_and(|e| e & bit != 0)
    }
    pub fn foco(&self) -> Option<EntityId> {
        self.foco
    }
    pub fn hay_interfaz(&self) -> bool {
        !self.lienzos.is_empty()
    }

    // ───────────────────────────── layout ─────────────────────────────

    /// Pone al día layout y lista de dibujo para una pantalla de `pantalla`
    /// píxeles. Barato si nada cambió.
    pub fn actualizar(&mut self, scene: &Scene, pantalla: [f32; 2], dt: f32) {
        self.reloj += dt;
        if self.gen_recursos != self.recursos.generacion {
            self.gen_recursos = self.recursos.generacion;
            self.sucio = true;
        }
        if self.pantalla != pantalla {
            self.pantalla = pantalla;
            self.sucio = true;
        }
        if self.sucio {
            self.reconstruir(scene);
            self.sucio = false;
            self.redibujar = true;
        }
        if self.animar(dt) {
            self.redibujar = true;
        }
        if self.foco.is_some_and(|f| self.indice.get(&f).is_some_and(|&i| self.nodos[i as usize].tipo == Some(TipoWidget::CampoTexto))) {
            // Cursor parpadeante.
            self.redibujar = true;
        }
        if self.redibujar {
            self.redibujar = false;
            for _ in 0..3 {
                let g = self.atlas.generacion;
                self.dibujar();
                if self.atlas.generacion == g {
                    break;
                }
            }
        }
    }

    fn tema_de(&self, lienzo: &Lienzo) -> String {
        if !lienzo.tema.is_empty() {
            return lienzo.tema.clone();
        }
        self.tema.clone().unwrap_or_default()
    }

    fn reconstruir(&mut self, scene: &Scene) {
        self.reconstrucciones += 1;
        let anims: HashMap<EntityId, Anim> = self.nodos.iter().map(|n| (n.id, n.anim)).collect();
        self.nodos.clear();
        self.indice.clear();
        self.lienzos.clear();
        let mut lienzos: Vec<(i32, u64, EntityId, Lienzo)> =
            scene.world.query::<(&EntityId, &Lienzo)>().iter().filter(|(id, _)| scene.is_active_in_hierarchy(**id)).map(|(id, l)| (l.orden, id.0, *id, l.clone())).collect();
        if lienzos.is_empty() {
            self.lista.limpiar([self.pantalla[0] as u32, self.pantalla[1] as u32]);
            return;
        }
        lienzos.sort_by_key(|l| (l.0, l.1));
        let h = scene.hierarchy();
        for (_, _, id, l) in &lienzos {
            let factor = l.factor(self.pantalla[0].max(1.0), self.pantalla[1].max(1.0));
            let tam = [self.pantalla[0] / factor, self.pantalla[1] / factor];
            let li = self.lienzos.len() as u32;
            self.lienzos.push(LienzoActivo { factor });
            let tema = self.tema_de(l);
            let raiz = self.nodos.len() as u32;
            self.nodos.push(Nodo {
                id: *id,
                padre: NADA,
                fin: 0,
                lienzo: li,
                rect: RectUi::estirado(),
                contenedor: scene.get::<Contenedor>(*id),
                widget: None,
                tipo: None,
                img: SIN_IMAGEN,
                min: [0.0, 0.0],
                max: tam,
                xf: Afin::escala(factor, factor),
                inv: Afin::escala(1.0 / factor, 1.0 / factor),
                recorte: None,
                recorte_hijos: None,
                visible: true,
                opacidad: 1.0,
                interactuable: true,
                estilos: Vec::new(),
                anim: Anim::default(),
            });
            self.indice.insert(*id, raiz);
            self.recoger(scene, &h, *id, raiz, li, &tema);
            self.nodos[raiz as usize].fin = self.nodos.len() as u32;
        }
        for n in &mut self.nodos {
            if let Some(a) = anims.get(&n.id) {
                n.anim = *a;
            }
        }
        // Rectángulos, de cada lienzo hacia abajo.
        let mut i = 0;
        while i < self.nodos.len() {
            let (mn, mx) = (self.nodos[i].min, self.nodos[i].max);
            self.colocar(i as u32, mn, mx, 0);
            i = self.nodos[i].fin as usize;
        }
        self.transformar();
    }

    fn recoger(&mut self, scene: &Scene, h: &crate::scene::Hierarchy, id: EntityId, padre: u32, lienzo: u32, tema: &str) {
        for &c in h.children_of(id) {
            let Some(rect) = scene.get::<RectUi>(c) else { continue };
            if scene.has::<Inactive>(c) {
                continue;
            }
            let widget = scene.get::<WidgetUi>(c);
            let tipo = widget.as_ref().map(|w| w.clase.tipo());
            let mut estilos = Vec::new();
            if let (Some(w), Some(t)) = (&widget, tipo) {
                estilos.push(self.resolver_partes(w, tema, None));
                for p in t.partes() {
                    estilos.push(self.resolver_partes(w, tema, Some(p)));
                }
            }
            let img = match widget.as_ref().map(|w| &w.clase) {
                Some(ClaseWidget::Imagen(im)) => self.imagenes.id(&im.imagen, self.raiz.as_deref()),
                _ => SIN_IMAGEN,
            };
            let i = self.nodos.len() as u32;
            self.nodos.push(Nodo {
                id: c,
                padre,
                fin: 0,
                lienzo,
                rect,
                contenedor: scene.get::<Contenedor>(c),
                widget,
                tipo,
                img,
                min: [0.0; 2],
                max: [0.0; 2],
                xf: Afin::IDENTIDAD,
                inv: Afin::IDENTIDAD,
                recorte: None,
                recorte_hijos: None,
                visible: true,
                opacidad: 1.0,
                interactuable: true,
                estilos,
                anim: Anim::default(),
            });
            self.indice.insert(c, i);
            self.recoger(scene, h, c, i, lienzo, tema);
            self.nodos[i as usize].fin = self.nodos.len() as u32;
        }
    }

    fn resolver_partes(&mut self, w: &WidgetUi, tema: &str, parte: Option<&str>) -> [Resuelto; 5] {
        let mut out = [Resuelto::default(); 5];
        for e in Estado::TODOS {
            let p = self.recursos.resolver(w, tema, parte, e);
            out[e.indice()] = self.convertir(&p);
        }
        out
    }

    fn convertir(&mut self, p: &PropsEstilo) -> Resuelto {
        let raiz = self.raiz.clone();
        let (fondo, fondo2, degradado) = match p.fondo.unwrap_or(Relleno::Ninguno) {
            Relleno::Ninguno => ([0.0; 4], [0.0; 4], None),
            Relleno::Color(c) => (lineal(c), lineal(c), None),
            Relleno::Degradado { desde, hasta, angulo } => (lineal(desde), lineal(hasta), Some(angulo.to_radians())),
        };
        let s = p.sombra.unwrap_or_default();
        let ct = p.contorno_texto.unwrap_or_default();
        let st = p.sombra_texto.unwrap_or_default();
        Resuelto {
            v: Visual {
                fondo,
                fondo2,
                borde: lineal(p.borde_color.unwrap_or([0.0; 4])),
                tinte: lineal(p.tinte.unwrap_or([1.0; 4])),
                texto: lineal(p.color_texto.unwrap_or([1.0; 4])),
                escala: p.escala.unwrap_or(1.0),
                desp: p.desplazamiento.unwrap_or([0.0; 2]),
                opacidad: p.opacidad.unwrap_or(1.0),
            },
            degradado,
            imagen: self.imagenes.id(p.imagen.as_deref().unwrap_or(""), raiz.as_deref()),
            nueve: p.nueve.unwrap_or([0.0; 4]),
            borde_ancho: p.borde_ancho.unwrap_or(0.0).max(0.0),
            radio: p.radio.unwrap_or([0.0; 4]),
            sombra: (s.color[3] > 0.0).then(|| (lineal(s.color), s.desplazamiento, s.difuminado.max(0.0), s.expansion)),
            relleno: p.relleno.unwrap_or([0.0; 4]),
            fuente: self.fuentes.id(p.fuente.as_deref().unwrap_or(""), raiz.as_deref()),
            tam_texto: p.tam_texto.unwrap_or(24.0).max(1.0),
            contorno: (lineal(ct.color), ct.ancho.max(0.0)),
            sombra_texto: (lineal(st.color), st.desplazamiento),
            transicion: p.transicion.unwrap_or(0.0).max(0.0),
        }
    }

    /// Hijos directos de un nodo.
    fn hijos(&self, i: u32) -> impl Iterator<Item = u32> + '_ {
        let fin = self.nodos[i as usize].fin;
        let mut j = i + 1;
        std::iter::from_fn(move || {
            if j >= fin {
                return None;
            }
            let k = j;
            j = self.nodos[k as usize].fin;
            Some(k)
        })
    }

    fn en_layout(&self, i: u32) -> bool {
        let n = &self.nodos[i as usize];
        n.padre != NADA && self.nodos[n.padre as usize].contenedor.is_some() && !n.rect.ignorar_layout
    }

    /// Tamaño preferido (unidades del lienzo).
    fn preferido(&mut self, i: u32, prof: usize) -> [f32; 2] {
        let n = &self.nodos[i as usize];
        if let Some(c) = n.contenedor.clone()
            && c.ajustar
        {
            let propio = n.rect.tam();
            let t = self.contenido(i, &c, prof);
            return match ejes_ajuste(&c) {
                [true, true] => t,
                [x, _] => if x { [t[0], if propio[1] > 0.0 { propio[1] } else { t[1] }] } else { [if propio[0] > 0.0 { propio[0] } else { t[0] }, t[1]] },
            };
        }
        if let Some(w) = &n.widget
            && let ClaseWidget::Texto(t) = &w.clase
            && t.ajustar_tam
        {
            let r = n.estilos[0][0];
            let texto = t.texto.clone();
            let f = self.lienzos[n.lienzo as usize].factor;
            let px = (r.tam_texto * f).round().max(1.0);
            let tam = maquetar(&mut self.fuentes, r.fuente, px, &texto, None, &mut self.glifos, &mut self.lineas);
            return [tam[0] / f + r.relleno[0] + r.relleno[2], tam[1] / f + r.relleno[1] + r.relleno[3]];
        }
        n.rect.tam().map(|v| v.max(0.0))
    }

    fn contenido(&mut self, i: u32, c: &Contenedor, prof: usize) -> [f32; 2] {
        let hijos: Vec<u32> = self.hijos(i).filter(|&h| !self.nodos[h as usize].rect.ignorar_layout).collect();
        let prefs: Vec<[f32; 2]> = hijos.into_iter().map(|h| self.preferido(h, prof + 1)).collect();
        layout::contenido(c, &prefs)
    }

    fn colocar(&mut self, i: u32, mut mn: [f32; 2], mut mx: [f32; 2], prof: usize) {
        if prof > 64 {
            return;
        }
        let cont = self.nodos[i as usize].contenedor.clone();
        if let Some(c) = &cont
            && c.ajustar
            && self.nodos[i as usize].padre != NADA
        {
            // Toma el tamaño del contenido conservando la posición del pivote.
            let t = self.contenido(i, c, prof);
            let pv = self.nodos[i as usize].rect.pivote;
            let ejes = ejes_ajuste(c);
            for k in (0..2).filter(|&k| ejes[k]) {
                let p = mn[k] + (mx[k] - mn[k]) * pv[k];
                mn[k] = p - t[k] * pv[k];
                mx[k] = mn[k] + t[k];
            }
        }
        self.nodos[i as usize].min = mn;
        self.nodos[i as usize].max = mx;
        let hijos: Vec<u32> = self.hijos(i).collect();
        if let Some(c) = &cont {
            while self.prefs.len() <= prof {
                self.prefs.push(Vec::new());
                self.rects.push(Vec::new());
            }
            let mut prefs = std::mem::take(&mut self.prefs[prof]);
            let mut rects = std::mem::take(&mut self.rects[prof]);
            prefs.clear();
            let mut flex = Vec::new();
            let en: Vec<u32> = hijos.iter().copied().filter(|&h| self.en_layout(h)).collect();
            for &h in &en {
                let p = self.preferido(h, prof + 1);
                prefs.push(p);
                flex.push(self.nodos[h as usize].rect.flex);
            }
            layout::distribuir(c, (mn, mx), &prefs, &flex, &mut rects);
            for (k, &h) in en.iter().enumerate() {
                let (a, b) = rects[k];
                self.colocar(h, a, b, prof + 1);
            }
            self.prefs[prof] = prefs;
            self.rects[prof] = rects;
        }
        for h in hijos {
            if !self.en_layout(h) {
                let (a, b) = self.nodos[h as usize].rect.en((mn, mx));
                self.colocar(h, a, b, prof + 1);
            }
        }
    }

    fn transformar(&mut self) {
        for i in 0..self.nodos.len() {
            let p = self.nodos[i].padre;
            if p == NADA {
                continue;
            }
            let (pxf, pvis, pop, pint, prec) = {
                let pn = &self.nodos[p as usize];
                (pn.xf, pn.visible, pn.opacidad, pn.interactuable, pn.recorte_hijos)
            };
            let n = &mut self.nodos[i];
            let piv = [n.min[0] + (n.max[0] - n.min[0]) * n.rect.pivote[0], n.min[1] + (n.max[1] - n.min[1]) * n.rect.pivote[1]];
            let local = Afin::traslacion(piv[0], piv[1]).por(&Afin::rotacion(n.rect.rotacion)).por(&Afin::escala(n.rect.escala[0], n.rect.escala[1])).por(&Afin::traslacion(-piv[0], -piv[1]));
            n.xf = pxf.por(&local);
            n.inv = n.xf.inversa();
            n.visible = pvis && n.rect.visible;
            n.opacidad = pop * n.rect.opacidad.clamp(0.0, 1.0);
            n.interactuable = pint && n.rect.interactuable;
            n.recorte = prec;
            n.recorte_hijos = if n.rect.recortar {
                let c = esquinas(&n.xf, n.min, n.max);
                let mut r = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
                for q in c {
                    r = [r[0].min(q[0]), r[1].min(q[1]), r[2].max(q[0]), r[3].max(q[1])];
                }
                Some(match prec {
                    Some(p) => [r[0].max(p[0]), r[1].max(p[1]), r[2].min(p[2]), r[3].min(p[3])],
                    None => r,
                })
            } else {
                prec
            };
        }
    }

    // ───────────────────────────── estados y transiciones ─────────────────────────────

    fn estado_de(&self, n: &Nodo) -> Estado {
        if let Some(e) = self.forzar.get(&n.id) {
            return *e;
        }
        let Some(t) = n.tipo else { return Estado::Normal };
        if !n.interactuable {
            return Estado::Desactivado;
        }
        if t == TipoWidget::Texto {
            return Estado::Normal;
        }
        let id = Some(n.id);
        if self.pulsado == id && (self.encima == id || t == TipoWidget::Deslizador) {
            Estado::Pulsado
        } else if self.encima == id && (self.pulsado.is_none() || self.pulsado == id) {
            Estado::Encima
        } else if self.foco == id {
            Estado::Foco
        } else {
            Estado::Normal
        }
    }

    /// Avanza las transiciones. Devuelve si hay que redibujar.
    fn animar(&mut self, dt: f32) -> bool {
        let mut algo = false;
        for i in 0..self.nodos.len() {
            if self.nodos[i].estilos.is_empty() {
                continue;
            }
            let e = self.estado_de(&self.nodos[i]);
            let n = &mut self.nodos[i];
            let objetivo = n.estilos[0][e.indice()];
            if !n.anim.iniciada {
                n.anim = Anim { estado: e, desde: objetivo.v, t: 1.0, dur: 0.0, iniciada: true };
                continue;
            }
            if n.anim.estado != e {
                let actual = visual_actual(n);
                n.anim = Anim { estado: e, desde: actual, t: 0.0, dur: objetivo.transicion, iniciada: true };
                algo = true;
            } else if n.anim.t < n.anim.dur {
                n.anim.t += dt;
                algo = true;
            }
        }
        algo
    }

    // ───────────────────────────── entrada ─────────────────────────────

    /// Nodo con widget más arriba bajo un punto (píxeles). `solo_bloqueantes`
    /// ignora los textos (no tapan lo de debajo).
    fn tocado(&self, p: [f32; 2], solo_bloqueantes: bool) -> Option<u32> {
        for i in (0..self.nodos.len()).rev() {
            let n = &self.nodos[i];
            let Some(t) = n.tipo else { continue };
            if !n.visible || n.opacidad <= 0.001 || (solo_bloqueantes && t == TipoWidget::Texto) {
                continue;
            }
            if contiene(n, p) {
                return Some(i as u32);
            }
        }
        None
    }

    /// Procesa ratón y teclado del fotograma: estados, valores y eventos.
    /// Usa el layout del fotograma anterior (lo que el jugador ve).
    pub fn procesar_entrada(&mut self, scene: &mut Scene, input: &Input) {
        let antes = (self.encima, self.pulsado, self.foco);
        self.eventos.clear();
        let p = [input.mouse_pos.x, input.mouse_pos.y];
        let toc = self.tocado(p, true);
        self.sobre_ui = toc.is_some();
        let nuevo_encima = toc.map(|i| &self.nodos[i as usize]).filter(|n| n.interactuable).map(|n| n.id);
        if nuevo_encima != self.encima {
            if let Some(a) = self.encima {
                *self.eventos.entry(a).or_default() |= ev::SALE;
            }
            if let Some(b) = nuevo_encima {
                *self.eventos.entry(b).or_default() |= ev::ENTRA;
            }
            self.encima = nuevo_encima;
        }
        let abajo = input.mouse_buttons[0];
        let pulsa = abajo && !self.raton_antes;
        let suelta = !abajo && self.raton_antes;
        self.raton_antes = abajo;
        if pulsa {
            self.captura_raton = toc.is_some();
            self.pulsado = self.encima;
            self.foco = None;
            if let Some(id) = self.pulsado {
                *self.eventos.entry(id).or_default() |= ev::PULSA;
                if self.tipo(id) == Some(TipoWidget::CampoTexto) {
                    self.foco = Some(id);
                }
            }
        }
        // Arrastre del deslizador.
        if abajo
            && let Some(id) = self.pulsado
            && self.tipo(id) == Some(TipoWidget::Deslizador)
            && let Some(&i) = self.indice.get(&id)
        {
            let n = &self.nodos[i as usize];
            let l = n.inv.aplicar(p);
            if let Some(ClaseWidget::Deslizador(d)) = n.widget.as_ref().map(|w| &w.clase) {
                let r = &n.estilos[0][0].relleno;
                let (a, b, x) = if d.direccion.vertical() { (n.min[1] + r[1], n.max[1] - r[3], l[1]) } else { (n.min[0] + r[0], n.max[0] - r[2], l[0]) };
                let mut t = ((x - a) / (b - a).max(1e-3)).clamp(0.0, 1.0);
                if matches!(d.direccion, Direccion::DerIzq | Direccion::AbajoArriba) {
                    t = 1.0 - t;
                }
                let v = super::cuantizar(d.min + (d.max - d.min) * t, d.min, d.max, d.paso);
                if (v - d.valor).abs() > 1e-7 {
                    self.cambiar_valor(scene, id, v);
                    *self.eventos.entry(id).or_default() |= ev::CAMBIO;
                }
            }
        }
        if suelta {
            if let Some(id) = self.pulsado.take() {
                *self.eventos.entry(id).or_default() |= ev::SUELTA;
                if self.encima == Some(id) {
                    *self.eventos.entry(id).or_default() |= ev::CLIC;
                    if self.tipo(id) == Some(TipoWidget::Casilla) {
                        let v = self.valor(scene, id);
                        self.cambiar_valor(scene, id, if v > 0.5 { 0.0 } else { 1.0 });
                        *self.eventos.entry(id).or_default() |= ev::CAMBIO;
                    }
                }
            }
            self.captura_raton = false;
        }
        // Teclado: navegación con Tab y edición del campo con foco.
        if input.key_pressed(Key::Tab as u32) && self.hay_interfaz() {
            let lista: Vec<EntityId> = self.nodos.iter().filter(|n| n.visible && n.interactuable && n.tipo.is_some_and(TipoWidget::interactivo)).map(|n| n.id).collect();
            if !lista.is_empty() {
                let atras = input.key_down(Key::Shift as u32);
                let k = self.foco.and_then(|f| lista.iter().position(|x| *x == f));
                let n = lista.len();
                let j = match (k, atras) {
                    (None, false) => 0,
                    (None, true) => n - 1,
                    (Some(k), false) => (k + 1) % n,
                    (Some(k), true) => (k + n - 1) % n,
                };
                self.foco = Some(lista[j]);
            }
        }
        self.captura_teclado = false;
        if let Some(f) = self.foco {
            if !self.indice.contains_key(&f) {
                self.foco = None;
            } else if input.key_pressed(Key::Escape as u32) {
                self.foco = None;
            } else {
                self.captura_teclado = true;
                match self.tipo(f) {
                    Some(TipoWidget::CampoTexto) => {
                        let mut texto = scene.get::<WidgetUi>(f).and_then(|w| w.clase.texto().map(str::to_owned)).unwrap_or_default();
                        let max = match scene.get::<WidgetUi>(f).map(|w| w.clase) {
                            Some(ClaseWidget::CampoTexto(c)) => c.max,
                            _ => 0,
                        };
                        let antes = texto.len();
                        for c in input.texto.chars().filter(|c| !c.is_control()) {
                            if max == 0 || texto.chars().count() < max as usize {
                                texto.push(c);
                            }
                        }
                        let borrar = input.key_pressed(Key::Backspace as u32);
                        if borrar {
                            texto.pop();
                        }
                        if texto.len() != antes || borrar {
                            self.fijar_texto(scene, f, &texto);
                            *self.eventos.entry(f).or_default() |= ev::CAMBIO;
                        }
                        if input.key_pressed(Key::Enter as u32) {
                            *self.eventos.entry(f).or_default() |= ev::ENVIA;
                            self.foco = None;
                        }
                    }
                    _ => {
                        if input.key_pressed(Key::Enter as u32) || input.key_pressed(Key::Space as u32) {
                            *self.eventos.entry(f).or_default() |= ev::CLIC;
                            if self.tipo(f) == Some(TipoWidget::Casilla) {
                                let v = self.valor(scene, f);
                                self.cambiar_valor(scene, f, if v > 0.5 { 0.0 } else { 1.0 });
                                *self.eventos.entry(f).or_default() |= ev::CAMBIO;
                            }
                        }
                        let paso = (input.key_pressed(Key::Right as u32) as i32 - input.key_pressed(Key::Left as u32) as i32) as f32;
                        if paso != 0.0
                            && let Some(ClaseWidget::Deslizador(d)) = scene.get::<WidgetUi>(f).map(|w| w.clase)
                        {
                            let s = if d.paso > 0.0 { d.paso } else { (d.max - d.min) / 20.0 };
                            self.cambiar_valor(scene, f, d.valor + s * paso);
                            *self.eventos.entry(f).or_default() |= ev::CAMBIO;
                        }
                    }
                }
            }
        }
        if antes != (self.encima, self.pulsado, self.foco) || !self.eventos.is_empty() {
            self.redibujar = true;
        }
    }

    /// Copia la entrada para el juego quitando lo que consumió la interfaz.
    pub fn filtrar(&self, input: &Input, out: &mut Input) {
        out.copiar_de(input);
        if self.captura_raton || (self.sobre_ui && !input.mouse_buttons[0]) {
            out.mouse_buttons = [false; 3];
        }
        if self.captura_teclado {
            out.soltar_teclas();
        }
    }

    fn tipo(&self, id: EntityId) -> Option<TipoWidget> {
        self.indice.get(&id).and_then(|&i| self.nodos[i as usize].tipo)
    }

    // ───────────────────────────── acceso para scripts y editor ─────────────────────────────

    pub fn fijar_texto(&mut self, scene: &mut Scene, id: EntityId, t: &str) {
        if let Some(mut w) = scene.get_mut::<WidgetUi>(id)
            && let Some(s) = w.clase.texto_mut()
            && s != t
        {
            s.clear();
            s.push_str(t);
            self.sucio = true;
        }
    }
    pub fn valor(&self, scene: &Scene, id: EntityId) -> f32 {
        scene.get::<WidgetUi>(id).map(|w| w.clase.valor()).unwrap_or(0.0)
    }
    pub fn cambiar_valor(&mut self, scene: &mut Scene, id: EntityId, v: f32) {
        if let Some(mut w) = scene.get_mut::<WidgetUi>(id) {
            let antes = w.clase.valor();
            w.clase.fijar_valor(v);
            if w.clase.valor() != antes {
                self.sucio = true;
            }
        }
    }
    pub fn editar_rect(&mut self, scene: &mut Scene, id: EntityId, f: impl FnOnce(&mut RectUi)) {
        if let Some(mut r) = scene.get_mut::<RectUi>(id) {
            let antes = r.clone();
            f(&mut r);
            if *r != antes {
                self.sucio = true;
            }
        }
    }
    pub fn editar_widget(&mut self, scene: &mut Scene, id: EntityId, f: impl FnOnce(&mut WidgetUi)) {
        if let Some(mut w) = scene.get_mut::<WidgetUi>(id) {
            f(&mut w);
            self.sucio = true;
        }
    }
    /// Rectángulo en unidades de su lienzo (min, max), tras el layout.
    pub fn rect(&self, id: EntityId) -> Option<([f32; 2], [f32; 2])> {
        self.indice.get(&id).map(|&i| (self.nodos[i as usize].min, self.nodos[i as usize].max))
    }
    /// Esquinas en píxeles de pantalla (sup. izq., sup. der., inf. der., inf. izq.).
    pub fn esquinas(&self, id: EntityId) -> Option<[[f32; 2]; 4]> {
        self.indice.get(&id).map(|&i| {
            let n = &self.nodos[i as usize];
            esquinas(&n.xf, n.min, n.max)
        })
    }
    /// Píxeles → unidades locales del padre del elemento (para arrastrar en el editor).
    pub fn a_local_del_padre(&self, id: EntityId, p: [f32; 2]) -> Option<[f32; 2]> {
        let &i = self.indice.get(&id)?;
        let pd = self.nodos[i as usize].padre;
        (pd != NADA).then(|| self.nodos[pd as usize].inv.aplicar(p))
    }
    /// Tamaño del rectángulo del padre (unidades del lienzo).
    pub fn tam_padre(&self, id: EntityId) -> Option<[f32; 2]> {
        let &i = self.indice.get(&id)?;
        let pd = self.nodos[i as usize].padre;
        (pd != NADA).then(|| {
            let p = &self.nodos[pd as usize];
            [p.max[0] - p.min[0], p.max[1] - p.min[1]]
        })
    }
    /// ¿Lo coloca un contenedor?
    pub fn en_contenedor(&self, id: EntityId) -> bool {
        self.indice.get(&id).is_some_and(|&i| self.en_layout(i))
    }
    /// Elemento (con o sin widget) más arriba bajo un punto en píxeles: para
    /// seleccionar en el editor.
    pub fn elemento_en(&self, p: [f32; 2]) -> Option<EntityId> {
        for i in (0..self.nodos.len()).rev() {
            let n = &self.nodos[i];
            if n.padre == NADA || !n.visible {
                continue;
            }
            if contiene(n, p) {
                return Some(n.id);
            }
        }
        None
    }
    /// Lienzos activos: (id, tamaño en píxeles de pantalla).
    pub fn lienzos(&self) -> impl Iterator<Item = (EntityId, f32)> + '_ {
        self.nodos.iter().filter(|n| n.padre == NADA).map(|n| (n.id, self.lienzos[n.lienzo as usize].factor))
    }

    /// Sustituye el dibujo de un widget personalizado.
    pub fn fijar_dibujo(&mut self, id: EntityId, cmds: &[UiDrawCmd]) {
        let raiz = self.raiz.clone();
        let d = self.dibujos.entry(id).or_default();
        d.cmds.clear();
        d.texto.clear();
        for c in cmds {
            let s = if c.str_len > 0 && !c.str_ptr.is_null() { std::str::from_utf8(unsafe { std::slice::from_raw_parts(c.str_ptr, c.str_len) }).unwrap_or("") } else { "" };
            let a = d.texto.len() as u32;
            d.texto.push_str(s);
            let img = if c.kind == ui_draw::IMAGE { self.imagenes.id(s, raiz.as_deref()) } else { SIN_IMAGEN };
            d.cmds.push((*c, a, d.texto.len() as u32, img));
        }
        self.redibujar = true;
    }
    pub fn quitar_dibujo(&mut self, id: EntityId) {
        self.dibujos.remove(&id);
    }

    // ───────────────────────────── dibujo ─────────────────────────────

    fn dibujar(&mut self) {
        let tam = [self.pantalla[0].max(0.0) as u32, self.pantalla[1].max(0.0) as u32];
        let SistemaUi { nodos, lista, atlas, fuentes, imagenes, glifos, lineas, foco, reloj, dibujos, lienzos, .. } = self;
        lista.limpiar(tam);
        let mut d = Pintor { lista, atlas, fuentes, imagenes, glifos, lineas };
        for n in nodos.iter() {
            let Some(w) = &n.widget else { continue };
            if !n.visible || n.opacidad <= 0.001 || n.estilos.is_empty() {
                continue;
            }
            let est = &n.estilos[0][n.anim.estado.indice()];
            let v = visual_actual(n);
            let op = n.opacidad * v.opacidad;
            // Escala y desplazamiento visual del estado, alrededor del centro.
            let c = [(n.min[0] + n.max[0]) * 0.5, (n.min[1] + n.max[1]) * 0.5];
            let xf = n.xf.por(&Afin::traslacion(c[0] + v.desp[0], c[1] + v.desp[1])).por(&Afin::escala(v.escala, v.escala)).por(&Afin::traslacion(-c[0], -c[1]));
            let rc = n.recorte;
            let (mn, mx) = (n.min, n.max);
            let interior = |r: &Resuelto| ([mn[0] + r.relleno[0], mn[1] + r.relleno[1]], [mx[0] - r.relleno[2], mx[1] - r.relleno[3]]);
            let factor = lienzos[n.lienzo as usize].factor;
            let parte = |k: usize| n.estilos.get(k).map(|p| p[n.anim.estado.indice()]);
            match &w.clase {
                ClaseWidget::Casilla(cs) => {
                    // La caja es un cuadrado a la izquierda; la etiqueta, a su derecha.
                    let lado = mx[1] - mn[1];
                    d.caja(&xf, mn, [mn[0] + lado, mx[1]], est, &v, op, rc);
                    if cs.activa
                        && let Some(m) = parte(1)
                    {
                        let k = lado * 0.25;
                        d.caja(&xf, [mn[0] + k, mn[1] + k], [mn[0] + lado - k, mx[1] - k], &m, &m.v, op, rc);
                    }
                    let a = ([mn[0] + lado + 10.0, mn[1]], mx);
                    d.texto(&xf, a, &cs.texto, est, v.texto, AlinH::Izquierda, AlinV::Medio, false, op, rc, factor);
                }
                _ => d.caja(&xf, mn, mx, est, &v, op, rc),
            }
            match &w.clase {
                ClaseWidget::Panel | ClaseWidget::Casilla(_) => {}
                ClaseWidget::Imagen(im) => {
                    let (a, b) = interior(est);
                    let t = lineal(im.tinte);
                    let tinte = [t[0] * v.tinte[0], t[1] * v.tinte[1], t[2] * v.tinte[2], t[3] * v.tinte[3]];
                    d.imagen(&xf, a, b, n.img, im.nueve, im.conservar_aspecto, tinte, est.radio, op, rc);
                }
                ClaseWidget::Texto(t) => {
                    d.texto(&xf, interior(est), &t.texto, est, v.texto, t.alinear_h, t.alinear_v, t.ajuste_linea, op, rc, factor);
                }
                ClaseWidget::Boton(b) => {
                    d.texto(&xf, interior(est), &b.texto, est, v.texto, AlinH::Centro, AlinV::Medio, false, op, rc, factor);
                }
                ClaseWidget::Barra(b) => {
                    if let Some(r) = parte(1) {
                        let t = ((b.valor - b.min) / (b.max - b.min).abs().max(1e-6)).clamp(0.0, 1.0);
                        let (a, z) = interior(est);
                        let (a, z) = llenar(a, z, t, b.direccion);
                        if t > 0.0 {
                            d.caja(&xf, a, z, &r, &r.v, op, rc);
                        }
                    }
                }
                ClaseWidget::Deslizador(s) => {
                    let t = ((s.valor - s.min) / (s.max - s.min).abs().max(1e-6)).clamp(0.0, 1.0);
                    let (a, z) = interior(est);
                    let (fa, fz) = llenar(a, z, t, s.direccion);
                    if let Some(r) = parte(1)
                        && t > 0.0
                    {
                        d.caja(&xf, fa, fz, &r, &r.v, op, rc);
                    }
                    if let Some(r) = parte(2) {
                        // Tirador: cuadrado del alto (o ancho) del widget, centrado en el valor.
                        let vertical = s.direccion.vertical();
                        let lado = if vertical { mx[0] - mn[0] } else { mx[1] - mn[1] };
                        let p = match s.direccion {
                            Direccion::IzqDer => [fz[0], (mn[1] + mx[1]) * 0.5],
                            Direccion::DerIzq => [fa[0], (mn[1] + mx[1]) * 0.5],
                            Direccion::AbajoArriba => [(mn[0] + mx[0]) * 0.5, fa[1]],
                            Direccion::ArribaAbajo => [(mn[0] + mx[0]) * 0.5, fz[1]],
                        };
                        let h = lado * 0.5 * r.v.escala;
                        d.caja(&xf, [p[0] - h, p[1] - h], [p[0] + h, p[1] + h], &r, &r.v, op, rc);
                    }
                }
                ClaseWidget::CampoTexto(c) => {
                    let (a, z) = interior(est);
                    let vacio = c.texto.is_empty();
                    let rc2 = intersecar(rc, aabb(&xf, a, z));
                    if vacio {
                        if let Some(m) = parte(2) {
                            d.texto(&xf, (a, z), &c.marcador, est, m.v.texto, AlinH::Izquierda, AlinV::Medio, false, op, rc2, factor);
                        }
                    } else {
                        d.texto(&xf, (a, z), &c.texto, est, v.texto, AlinH::Izquierda, AlinV::Medio, false, op, rc2, factor);
                    }
                    if *foco == Some(n.id)
                        && (*reloj * 2.0).fract() < 0.6
                        && let Some(cur) = parte(1)
                    {
                        let ancho = if vacio { 0.0 } else { d.ancho_texto(&c.texto, est, factor) };
                        let x = (a[0] + ancho + 1.0).min(z[0]);
                        let hh = est.tam_texto * 0.6;
                        let cy = (a[1] + z[1]) * 0.5;
                        d.caja(&xf, [x, cy - hh], [x + 2.0, cy + hh], &cur, &cur.v, op, rc2);
                    }
                }
                ClaseWidget::Personalizado => {
                    if let Some(ds) = dibujos.get(&n.id) {
                        d.script(&xf, mn, ds, op, rc, factor);
                    }
                }
            }
        }
    }
}

/// Ejes en los que un contenedor con `ajustar` toma el tamaño del contenido:
/// el principal en filas y columnas (el cruzado lo decide su rectángulo), los dos en rejillas.
fn ejes_ajuste(c: &Contenedor) -> [bool; 2] {
    match c.tipo {
        super::TipoContenedor::Horizontal => [true, false],
        super::TipoContenedor::Vertical => [false, true],
        super::TipoContenedor::Rejilla => [true, true],
    }
}

fn visual_actual(n: &Nodo) -> Visual {
    let obj = n.estilos[0][n.anim.estado.indice()].v;
    if n.anim.dur <= 0.0 || n.anim.t >= n.anim.dur {
        return obj;
    }
    let t = n.anim.t / n.anim.dur;
    // Suavizado (ease-out).
    let t = 1.0 - (1.0 - t) * (1.0 - t);
    n.anim.desde.mezcla(&obj, t)
}

fn esquinas(xf: &Afin, mn: [f32; 2], mx: [f32; 2]) -> [[f32; 2]; 4] {
    [xf.aplicar(mn), xf.aplicar([mx[0], mn[1]]), xf.aplicar(mx), xf.aplicar([mn[0], mx[1]])]
}

fn aabb(xf: &Afin, mn: [f32; 2], mx: [f32; 2]) -> [f32; 4] {
    let mut r = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
    for q in esquinas(xf, mn, mx) {
        r = [r[0].min(q[0]), r[1].min(q[1]), r[2].max(q[0]), r[3].max(q[1])];
    }
    r
}

fn intersecar(a: Option<[f32; 4]>, b: [f32; 4]) -> Option<[f32; 4]> {
    Some(match a {
        Some(a) => [a[0].max(b[0]), a[1].max(b[1]), a[2].min(b[2]), a[3].min(b[3])],
        None => b,
    })
}

fn contiene(n: &Nodo, p: [f32; 2]) -> bool {
    if let Some(r) = n.recorte
        && (p[0] < r[0] || p[1] < r[1] || p[0] > r[2] || p[1] > r[3])
    {
        return false;
    }
    let l = n.inv.aplicar(p);
    l[0] >= n.min[0] && l[0] <= n.max[0] && l[1] >= n.min[1] && l[1] <= n.max[1]
}

/// Parte llena de una barra.
fn llenar(a: [f32; 2], z: [f32; 2], t: f32, d: Direccion) -> ([f32; 2], [f32; 2]) {
    match d {
        Direccion::IzqDer => (a, [a[0] + (z[0] - a[0]) * t, z[1]]),
        Direccion::DerIzq => ([z[0] - (z[0] - a[0]) * t, a[1]], z),
        Direccion::ArribaAbajo => (a, [z[0], a[1] + (z[1] - a[1]) * t]),
        Direccion::AbajoArriba => ([a[0], z[1] - (z[1] - a[1]) * t], z),
    }
}

/// Emite quads en la lista.
struct Pintor<'a> {
    lista: &'a mut ListaDibujo,
    atlas: &'a mut AtlasGlifos,
    fuentes: &'a mut Fuentes,
    imagenes: &'a mut Imagenes,
    glifos: &'a mut Vec<GlifoPos>,
    lineas: &'a mut Vec<Linea>,
}

impl Pintor<'_> {
    /// Fondo (color o degradado), imagen de fondo, borde, radio y sombra.
    #[allow(clippy::too_many_arguments)]
    fn caja(&mut self, xf: &Afin, mn: [f32; 2], mx: [f32; 2], r: &Resuelto, v: &Visual, op: f32, rc: Option<[f32; 4]>) {
        let s = xf.escala_media();
        let radios = r.radio.map(|x| x.max(0.0) * s);
        if let Some((col, desp, dif, exp)) = r.sombra {
            let a = [mn[0] - exp + desp[0], mn[1] - exp + desp[1]];
            let b = [mx[0] + exp + desp[0], mx[1] + exp + desp[1]];
            let mut q = quad_rect(xf, a, b);
            q.color = con_opacidad(col, op);
            q.color2 = q.color;
            q.radios = radios;
            q.params = [0.0, MODO_SOMBRA, dif * s, 0.0];
            self.lista.empujar(q, SIN_IMAGEN, rc);
        }
        let hay_fondo = v.fondo[3] > 0.0 || v.fondo2[3] > 0.0;
        let hay_borde = r.borde_ancho > 0.0 && v.borde[3] > 0.0;
        let con_imagen = r.imagen != SIN_IMAGEN && self.imagenes.tam(r.imagen).is_some();
        if hay_fondo || (hay_borde && !con_imagen) {
            let mut q = quad_rect(xf, mn, mx);
            q.color = con_opacidad(v.fondo, op);
            q.color2 = con_opacidad(v.fondo2, op);
            q.borde = con_opacidad(v.borde, op);
            q.radios = radios;
            q.params = [if con_imagen { 0.0 } else { r.borde_ancho * s }, MODO_CAJA, 0.0, r.degradado.unwrap_or(0.0)];
            if con_imagen {
                q.borde = [0.0; 4];
            }
            self.lista.empujar(q, SIN_IMAGEN, rc);
        }
        if con_imagen {
            self.imagen(xf, mn, mx, r.imagen, r.nueve, false, v.tinte, r.radio, op, rc);
            if hay_borde {
                let mut q = quad_rect(xf, mn, mx);
                q.color = [0.0; 4];
                q.color2 = [0.0; 4];
                q.borde = con_opacidad(v.borde, op);
                q.radios = radios;
                q.params = [r.borde_ancho * s, MODO_CAJA, 0.0, 0.0];
                self.lista.empujar(q, SIN_IMAGEN, rc);
            }
        }
    }

    /// Imagen (con 9-slice o conservando el aspecto real del archivo).
    #[allow(clippy::too_many_arguments)]
    fn imagen(&mut self, xf: &Afin, mut mn: [f32; 2], mut mx: [f32; 2], id: u32, nueve: [f32; 4], aspecto: bool, tinte: [f32; 4], radio: [f32; 4], op: f32, rc: Option<[f32; 4]>) {
        let Some([iw, ih]) = self.imagenes.tam(id) else { return };
        let (iw, ih) = (iw as f32, ih as f32);
        let color = con_opacidad(tinte, op);
        if nueve.iter().any(|&b| b > 0.0) {
            // 9 trozos: las esquinas no se estiran, los bordes en un eje, el centro en dos.
            let [l, t, r, b] = nueve;
            let xs = [mn[0], mn[0] + l, mx[0] - r, mx[0]];
            let ys = [mn[1], mn[1] + t, mx[1] - b, mx[1]];
            let us = [0.0, l / iw, 1.0 - r / iw, 1.0];
            let vs = [0.0, t / ih, 1.0 - b / ih, 1.0];
            for j in 0..3 {
                for i in 0..3 {
                    if xs[i + 1] - xs[i] <= 0.0 || ys[j + 1] - ys[j] <= 0.0 {
                        continue;
                    }
                    let mut q = quad_rect(xf, [xs[i], ys[j]], [xs[i + 1], ys[j + 1]]);
                    q.uv = [us[i], vs[j], us[i + 1], vs[j + 1]];
                    q.color = color;
                    q.params = [0.0, MODO_IMAGEN, 0.0, 0.0];
                    self.lista.empujar(q, id, rc);
                }
            }
            return;
        }
        if aspecto && iw > 0.0 && ih > 0.0 {
            let (w, h) = (mx[0] - mn[0], mx[1] - mn[1]);
            let k = (w / iw).min(h / ih);
            let (nw, nh) = (iw * k, ih * k);
            mn = [mn[0] + (w - nw) * 0.5, mn[1] + (h - nh) * 0.5];
            mx = [mn[0] + nw, mn[1] + nh];
        }
        let mut q = quad_rect(xf, mn, mx);
        q.color = color;
        q.radios = radio.map(|x| x.max(0.0) * xf.escala_media());
        q.params = [0.0, MODO_IMAGEN, 0.0, 0.0];
        self.lista.empujar(q, id, rc);
    }

    fn ancho_texto(&mut self, texto: &str, r: &Resuelto, factor: f32) -> f32 {
        let px = (r.tam_texto * factor).round().max(1.0);
        maquetar(self.fuentes, r.fuente, px, texto, None, self.glifos, self.lineas)[0] / factor
    }

    /// Texto alineado dentro de `area` (unidades locales), con sombra y contorno.
    #[allow(clippy::too_many_arguments)]
    fn texto(&mut self, xf: &Afin, area: ([f32; 2], [f32; 2]), texto: &str, r: &Resuelto, color: [f32; 4], ah: AlinH, av: AlinV, ajuste: bool, op: f32, rc: Option<[f32; 4]>, factor: f32) {
        self.texto_tam(xf, area, texto, r.fuente, r.tam_texto, color, Some(r), ah, av, ajuste, op, rc, factor);
    }

    #[allow(clippy::too_many_arguments)]
    fn texto_tam(&mut self, xf: &Afin, area: ([f32; 2], [f32; 2]), texto: &str, fuente: u32, tam: f32, color: [f32; 4], r: Option<&Resuelto>, ah: AlinH, av: AlinV, ajuste: bool, op: f32, rc: Option<[f32; 4]>, _factor: f32) {
        if texto.is_empty() {
            return;
        }
        let s = xf.escala_media().max(1e-4);
        let px = (tam * s).round().max(1.0);
        let (a, z) = area;
        let ancho = (z[0] - a[0]).max(0.0);
        let total = maquetar(self.fuentes, fuente, px, texto, if ajuste && ancho > 0.0 { Some(ancho * s) } else { None }, self.glifos, self.lineas);
        let kv = match av {
            AlinV::Arriba => 0.0,
            AlinV::Medio => 0.5,
            AlinV::Abajo => 1.0,
        };
        let kh = match ah {
            AlinH::Izquierda => 0.0,
            AlinH::Centro => 0.5,
            AlinH::Derecha => 1.0,
        };
        let y0 = a[1] + ((z[1] - a[1]) - total[1] / s) * kv;
        let pasadas: [([f32; 4], [f32; 2]); 10] = {
            let mut p = [([0.0; 4], [0.0; 2]); 10];
            if let Some(r) = r {
                p[0] = (r.sombra_texto.0, r.sombra_texto.1);
                let w = r.contorno.1;
                if w > 0.0 {
                    let d = std::f32::consts::FRAC_1_SQRT_2 * w;
                    for (k, o) in [[w, 0.0], [-w, 0.0], [0.0, w], [0.0, -w], [d, d], [-d, d], [d, -d], [-d, -d]].into_iter().enumerate() {
                        p[1 + k] = (r.contorno.0, o);
                    }
                }
            }
            p[9] = (color, [0.0, 0.0]);
            p
        };
        let alineada = xf.alineada();
        let tam_atlas = self.atlas.tam as f32;
        for (col, off) in pasadas {
            if col[3] <= 0.0 {
                continue;
            }
            let col = con_opacidad(col, op);
            for l in 0..self.lineas.len() {
                let lin = self.lineas[l];
                let x0 = a[0] + (ancho - lin.ancho / s) * kh;
                for gi in lin.inicio..lin.fin {
                    let g = self.glifos[gi];
                    let Some(e) = self.atlas.obtener(self.fuentes, g.fuente, g.gid, px as u32) else { continue };
                    let lx = x0 + off[0] + (g.x + e.izq as f32) / s;
                    let ly = y0 + off[1] + (g.y + e.arriba as f32) / s;
                    let (w, h) = (e.w as f32 / s, e.h as f32 / s);
                    let mut q = quad_rect(xf, [lx, ly], [lx + w, ly + h]);
                    if alineada {
                        // A píxel entero: el glifo se muestrea 1:1, nítido.
                        let p0 = xf.aplicar([lx, ly]);
                        let (px0, py0) = (p0[0].round(), p0[1].round());
                        q.centro = [px0 + e.w as f32 * 0.5, py0 + e.h as f32 * 0.5];
                        q.eje_x = [e.w as f32 * 0.5, 0.0];
                        q.eje_y = [0.0, e.h as f32 * 0.5];
                    }
                    q.uv = [e.x as f32 / tam_atlas, e.y as f32 / tam_atlas, (e.x + e.w) as f32 / tam_atlas, (e.y + e.h) as f32 / tam_atlas];
                    q.color = col;
                    q.params = [0.0, MODO_GLIFO, 0.0, 0.0];
                    self.lista.empujar(q, SIN_IMAGEN, rc);
                }
            }
        }
    }

    fn segmento(&mut self, xf: &Afin, p0: [f32; 2], p1: [f32; 2], ancho: f32, color: [f32; 4], rc: Option<[f32; 4]>) {
        let d = [p1[0] - p0[0], p1[1] - p0[1]];
        let l = (d[0] * d[0] + d[1] * d[1]).sqrt();
        if l < 1e-5 {
            return;
        }
        let u = [d[0] / l, d[1] / l];
        let c = xf.aplicar([(p0[0] + p1[0]) * 0.5, (p0[1] + p1[1]) * 0.5]);
        let q = QuadUi {
            centro: c,
            eje_x: xf.lineal([u[0] * l * 0.5, u[1] * l * 0.5]),
            eje_y: xf.lineal([-u[1] * ancho * 0.5, u[0] * ancho * 0.5]),
            uv: [0.0, 0.0, 1.0, 1.0],
            color,
            color2: color,
            params: [0.0, MODO_CAJA, 0.0, 0.0],
            ..Default::default()
        };
        self.lista.empujar(q, SIN_IMAGEN, rc);
    }

    /// Órdenes de un widget personalizado.
    fn script(&mut self, xf: &Afin, o: [f32; 2], d: &DibujoScript, op: f32, rc: Option<[f32; 4]>, factor: f32) {
        let s = xf.escala_media();
        for (c, a, b, img) in &d.cmds {
            let color = con_opacidad(lineal([c.color.r, c.color.g, c.color.b, c.color.a]), op);
            let [x, y, w, h] = c.rect;
            match c.kind {
                ui_draw::RECT | ui_draw::ROUNDED_RECT => {
                    let mut q = quad_rect(xf, [o[0] + x, o[1] + y], [o[0] + x + w, o[1] + y + h]);
                    q.radios = [c.radius.max(0.0) * s; 4];
                    if c.width > 0.0 {
                        q.borde = color;
                        q.params = [c.width * s, MODO_CAJA, 0.0, 0.0];
                    } else {
                        q.color = color;
                        q.color2 = color;
                        q.params = [0.0, MODO_CAJA, 0.0, 0.0];
                    }
                    self.lista.empujar(q, SIN_IMAGEN, rc);
                }
                ui_draw::CIRCLE => {
                    let r = w.max(0.0);
                    let mut q = quad_rect(xf, [o[0] + x - r, o[1] + y - r], [o[0] + x + r, o[1] + y + r]);
                    q.radios = [r * s; 4];
                    if c.width > 0.0 {
                        q.borde = color;
                        q.params = [c.width * s, MODO_CAJA, 0.0, 0.0];
                    } else {
                        q.color = color;
                        q.color2 = color;
                        q.params = [0.0, MODO_CAJA, 0.0, 0.0];
                    }
                    self.lista.empujar(q, SIN_IMAGEN, rc);
                }
                ui_draw::LINE => self.segmento(xf, [o[0] + x, o[1] + y], [o[0] + w, o[1] + h], c.width.max(0.1), color, rc),
                ui_draw::ARC => {
                    let (r, a0, a1) = (w, c.extra[0].to_radians(), c.extra[1].to_radians());
                    // Segmentos de unos 3 px de pantalla.
                    let largo = (a1 - a0).abs() * r * s;
                    let n = ((largo / 3.0).ceil() as usize).clamp(2, 256);
                    let pt = |t: f32| {
                        let ang = a0 + (a1 - a0) * t;
                        [o[0] + x + r * ang.cos(), o[1] + y + r * ang.sin()]
                    };
                    // Prolonga cada tramo medio grosor para que no queden huecos entre ellos.
                    for k in 0..n {
                        let (p0, p1) = (pt(k as f32 / n as f32), pt((k + 1) as f32 / n as f32));
                        let dd = [p1[0] - p0[0], p1[1] - p0[1]];
                        let l = (dd[0] * dd[0] + dd[1] * dd[1]).sqrt().max(1e-5);
                        let e = (c.width * 0.25).min(l);
                        let u = [dd[0] / l * e, dd[1] / l * e];
                        self.segmento(xf, [p0[0] - u[0], p0[1] - u[1]], [p1[0] + u[0], p1[1] + u[1]], c.width.max(0.1), color, rc);
                    }
                }
                ui_draw::IMAGE => {
                    let tinte = lineal([c.color.r, c.color.g, c.color.b, c.color.a]);
                    self.imagen(xf, [o[0] + x, o[1] + y], [o[0] + x + w, o[1] + y + h], *img, [0.0; 4], false, tinte, [0.0; 4], op, rc);
                }
                ui_draw::TEXT => {
                    let t = &d.texto[*a as usize..*b as usize];
                    let ah = match c.extra[0] as i32 {
                        1 => AlinH::Centro,
                        2 => AlinH::Derecha,
                        _ => AlinH::Izquierda,
                    };
                    let av = match c.extra[1] as i32 {
                        1 => AlinV::Medio,
                        2 => AlinV::Abajo,
                        _ => AlinV::Arriba,
                    };
                    let area = ([o[0] + x, o[1] + y], [o[0] + x + w.max(0.0), o[1] + y + h.max(0.0)]);
                    let col = lineal([c.color.r, c.color.g, c.color.b, c.color.a]);
                    self.texto_tam(xf, area, t, 0, c.radius.max(1.0), col, None, ah, av, w > 0.0, op, rc, factor);
                }
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::*;

    /// Lienzo de 800×600 a escala 1 con un contenedor vertical y dos botones.
    fn escena() -> (Scene, EntityId, EntityId, EntityId) {
        let mut s = Scene::new();
        let l = s.spawn("Lienzo");
        s.insert(l, Lienzo { resolucion: [800.0, 600.0], ..Default::default() });
        let menu = s.spawn("Menú");
        s.insert(menu, RectUi::centrado(300.0, 0.0));
        s.insert(menu, Contenedor { tipo: TipoContenedor::Vertical, relleno: [10.0; 4], espacio: [0.0, 10.0], ajustar: true, cruzado: Alinear::Estirar, ..Default::default() });
        s.insert(menu, WidgetUi::nuevo(TipoWidget::Panel));
        s.set_parent(menu, Some(l), false);
        let mut ids = Vec::new();
        for n in ["Jugar", "Salir"] {
            let b = s.spawn(n);
            s.insert(b, RectUi::centrado(100.0, 50.0));
            s.insert(b, WidgetUi { clase: ClaseWidget::Boton(Boton { texto: n.into() }), ..Default::default() });
            s.set_parent(b, Some(menu), false);
            ids.push(b);
        }
        (s, menu, ids[0], ids[1])
    }

    #[test]
    fn layout_con_anclas_contenedor_y_ajuste() {
        let (s, menu, a, b) = escena();
        let mut ui = SistemaUi::default();
        ui.actualizar(&s, [800.0, 600.0], 0.0);
        // Menú: ajusta su alto al contenido (10 + 50 + 10 + 50 + 10) y conserva el centro.
        let (mn, mx) = ui.rect(menu).unwrap();
        assert_eq!((mn, mx), ([250.0, 235.0], [550.0, 365.0]));
        // Botones estirados al ancho interior, apilados.
        assert_eq!(ui.rect(a).unwrap(), ([260.0, 245.0], [540.0, 295.0]));
        assert_eq!(ui.rect(b).unwrap(), ([260.0, 305.0], [540.0, 355.0]));
        // Pantalla al doble: el lienzo escala ×2 y el layout en unidades no cambia.
        ui.actualizar(&s, [1600.0, 1200.0], 0.0);
        assert_eq!(ui.rect(b).unwrap(), ([260.0, 305.0], [540.0, 355.0]));
        assert_eq!(ui.esquinas(b).unwrap()[0], [520.0, 610.0]);
    }

    #[test]
    fn clic_eventos_y_consumo_de_entrada() {
        let (mut s, _, a, b) = escena();
        let mut ui = SistemaUi::default();
        ui.actualizar(&s, [800.0, 600.0], 0.0);
        let mut i = Input::default();
        i.mouse_pos = glam::Vec2::new(400.0, 275.0);
        ui.procesar_entrada(&mut s, &i);
        assert!(ui.puntero_sobre_ui());
        assert!(ui.evento(a, ev::ENTRA));
        i.mouse_buttons[0] = true;
        ui.procesar_entrada(&mut s, &i);
        assert!(ui.evento(a, ev::PULSA) && ui.captura_raton());
        let mut juego = Input::default();
        ui.filtrar(&i, &mut juego);
        assert!(!juego.mouse_buttons[0], "el juego no ve el clic sobre la interfaz");
        i.mouse_buttons[0] = false;
        ui.procesar_entrada(&mut s, &i);
        assert!(ui.evento(a, ev::CLIC) && !ui.evento(b, ev::CLIC));
        // Pulsar en a y soltar en b no es clic.
        i.mouse_buttons[0] = true;
        ui.procesar_entrada(&mut s, &i);
        i.mouse_pos = glam::Vec2::new(400.0, 335.0);
        ui.procesar_entrada(&mut s, &i);
        i.mouse_buttons[0] = false;
        ui.procesar_entrada(&mut s, &i);
        assert!(!ui.evento(a, ev::CLIC) && !ui.evento(b, ev::CLIC));
        // Fuera de la interfaz el juego recibe el ratón.
        i.mouse_pos = glam::Vec2::new(10.0, 10.0);
        i.mouse_buttons[0] = true;
        ui.procesar_entrada(&mut s, &i);
        ui.filtrar(&i, &mut juego);
        assert!(!ui.puntero_sobre_ui() && juego.mouse_buttons[0]);
        // Desactivado: ni eventos ni estado encima.
        s.get_mut::<RectUi>(a).unwrap().interactuable = false;
        ui.marcar_sucio();
        ui.actualizar(&s, [800.0, 600.0], 0.0);
        i.mouse_buttons[0] = false;
        i.mouse_pos = glam::Vec2::new(400.0, 275.0);
        ui.procesar_entrada(&mut s, &i);
        assert!(ui.puntero_sobre_ui() && !ui.evento(a, ev::ENTRA));
    }

    #[test]
    fn deslizador_casilla_y_campo_de_texto() {
        let mut s = Scene::new();
        let l = s.spawn("Lienzo");
        s.insert(l, Lienzo { resolucion: [800.0, 600.0], ..Default::default() });
        let mk = |s: &mut Scene, t: TipoWidget, y: f32| {
            let e = s.spawn(t.nombre());
            s.insert(e, RectUi::anclado([0.0, 0.0], [0.0, 0.0], [100.0, y], [200.0, 40.0]));
            s.insert(e, WidgetUi::nuevo(t));
            s.set_parent(e, Some(l), false);
            e
        };
        let des = mk(&mut s, TipoWidget::Deslizador, 0.0);
        let cas = mk(&mut s, TipoWidget::Casilla, 100.0);
        let campo = mk(&mut s, TipoWidget::CampoTexto, 200.0);
        let mut ui = SistemaUi::default();
        ui.actualizar(&s, [800.0, 600.0], 0.0);
        let mut i = Input::default();
        // Arrastrar el deslizador al 75 %.
        i.mouse_pos = glam::Vec2::new(150.0, 20.0);
        i.mouse_buttons[0] = true;
        ui.procesar_entrada(&mut s, &i);
        i.mouse_pos = glam::Vec2::new(250.0, 20.0);
        ui.procesar_entrada(&mut s, &i);
        assert!(ui.evento(des, ev::CAMBIO));
        assert!((ui.valor(&s, des) - 0.75).abs() < 1e-4);
        i.mouse_buttons[0] = false;
        ui.procesar_entrada(&mut s, &i);
        // Casilla: clic la marca.
        i.mouse_pos = glam::Vec2::new(110.0, 120.0);
        ui.procesar_entrada(&mut s, &i);
        i.mouse_buttons[0] = true;
        ui.procesar_entrada(&mut s, &i);
        i.mouse_buttons[0] = false;
        ui.procesar_entrada(&mut s, &i);
        assert!(ui.evento(cas, ev::CAMBIO) && ui.valor(&s, cas) == 1.0);
        // Campo: clic, escribir, borrar e Intro.
        i.mouse_pos = glam::Vec2::new(150.0, 220.0);
        ui.procesar_entrada(&mut s, &i);
        i.mouse_buttons[0] = true;
        ui.procesar_entrada(&mut s, &i);
        i.mouse_buttons[0] = false;
        ui.procesar_entrada(&mut s, &i);
        assert_eq!(ui.foco(), Some(campo));
        i.texto.push_str("hola!");
        i.set_key(Key::A, true);
        ui.procesar_entrada(&mut s, &i);
        let mut juego = Input::default();
        ui.filtrar(&i, &mut juego);
        assert!(!juego.key_down(Key::A as u32), "las teclas van al campo, no al juego");
        i.end_frame();
        i.set_key(Key::Backspace, true);
        ui.procesar_entrada(&mut s, &i);
        i.end_frame();
        i.set_key(Key::Enter, true);
        ui.procesar_entrada(&mut s, &i);
        assert!(ui.evento(campo, ev::ENVIA) && ui.foco().is_none());
        assert_eq!(s.get::<WidgetUi>(campo).unwrap().clase.texto(), Some("hola"));
    }

    #[test]
    fn lista_de_dibujo_un_lote_y_sin_reservas_en_regimen() {
        let (s, _, _, _) = escena();
        let mut ui = SistemaUi::default();
        ui.actualizar(&s, [800.0, 600.0], 0.0);
        let n = ui.lista.quads.len();
        // Panel + 2 botones + glifos de «Jugar» y «Salir».
        assert!(n >= 3 + 10, "{n} quads");
        assert_eq!(ui.lista.lotes.len(), 1, "sin imágenes ni recortes: un solo dibujo");
        let glifos = ui.lista.quads.iter().filter(|q| q.params[1] == MODO_GLIFO).count();
        assert_eq!(glifos, 10);
        let r = ui.reconstrucciones;
        let cap = ui.lista.quads.capacity();
        for _ in 0..10 {
            ui.actualizar(&s, [800.0, 600.0], 1.0 / 60.0);
        }
        assert_eq!(ui.reconstrucciones, r, "sin cambios no se rehace el layout");
        assert_eq!(ui.lista.quads.capacity(), cap);
    }

    #[test]
    fn mascara_recorta_y_opacidad_de_grupo() {
        let (s, menu, a, _) = escena();
        s.get_mut::<RectUi>(menu).unwrap().recortar = true;
        s.get_mut::<RectUi>(menu).unwrap().opacidad = 0.5;
        let mut ui = SistemaUi::default();
        ui.actualizar(&s, [800.0, 600.0], 0.0);
        assert_eq!(ui.lista.lotes.len(), 2, "el panel sin recorte y sus hijos recortados");
        assert_eq!(ui.lista.lotes[1].recorte, [250, 235, 300, 130]);
        let panel = ui.lista.quads[0];
        assert!((panel.color[3] - 0.85 * 0.5).abs() < 1e-5, "opacidad de grupo");
        // Un punto fuera de la máscara no toca al hijo.
        s.get_mut::<RectUi>(a).unwrap().ignorar_layout = true;
        s.get_mut::<RectUi>(a).unwrap().offset_min = [-500.0, 0.0];
        ui.marcar_sucio();
        ui.actualizar(&s, [800.0, 600.0], 0.0);
        assert!(ui.elemento_en([100.0, 300.0]).is_none_or(|e| e != a));
    }

    #[test]
    fn transicion_de_estado_interpola() {
        let (mut s, _, a, _) = escena();
        let mut hoja = HojaEstilos::default();
        let mut e = Estilo::default();
        e.estados.normal.fondo = Some(Relleno::Color([0.0, 0.0, 0.0, 1.0]));
        e.estados.normal.transicion = Some(0.0);
        e.estados.encima.fondo = Some(Relleno::Color([1.0, 1.0, 1.0, 1.0]));
        e.estados.encima.transicion = Some(1.0);
        hoja.estilos.insert("b".into(), e);
        let mut rec = RecursosUi::default();
        rec.hojas.insert("x.mestilo".into(), hoja);
        rec.reindexar();
        s.get_mut::<WidgetUi>(a).unwrap().estilo = "b".into();
        let mut ui = SistemaUi::new(rec, None);
        ui.actualizar(&s, [800.0, 600.0], 0.0);
        let mut i = Input::default();
        i.mouse_pos = glam::Vec2::new(400.0, 275.0);
        ui.procesar_entrada(&mut s, &i);
        ui.actualizar(&s, [800.0, 600.0], 0.0);
        ui.actualizar(&s, [800.0, 600.0], 0.5);
        let q = ui.lista.quads.iter().find(|q| q.centro == [400.0, 270.0] && q.params[1] == MODO_CAJA).unwrap();
        assert!(q.color[0] > 0.3 && q.color[0] < 0.95, "a medio camino: {:?}", q.color);
        ui.actualizar(&s, [800.0, 600.0], 1.0);
        let q = ui.lista.quads.iter().find(|q| q.centro == [400.0, 270.0] && q.params[1] == MODO_CAJA).unwrap();
        assert!((q.color[0] - 1.0).abs() < 1e-5);
    }

    #[test]
    fn dibujo_de_script_en_widget_personalizado() {
        let mut s = Scene::new();
        let l = s.spawn("Lienzo");
        s.insert(l, Lienzo { resolucion: [800.0, 600.0], ..Default::default() });
        let w = s.spawn("Radar");
        s.insert(w, RectUi::anclado([0.0, 0.0], [0.0, 0.0], [10.0, 10.0], [100.0, 100.0]));
        s.insert(w, WidgetUi { clase: ClaseWidget::Personalizado, local: Estilo { estados: EstadosEstilo { normal: PropsEstilo { fondo: Some(Relleno::Ninguno), ..Default::default() }, ..Default::default() }, ..Default::default() }, ..Default::default() });
        s.set_parent(w, Some(l), false);
        let mut ui = SistemaUi::default();
        ui.actualizar(&s, [800.0, 600.0], 0.0);
        assert_eq!(ui.lista.quads.len(), 0);
        let txt = "N";
        let cmds = [
            UiDrawCmd { kind: ui_draw::CIRCLE, rect: [50.0, 50.0, 10.0, 0.0], color: maker_api::Rgba { r: 1.0, g: 0.0, b: 0.0, a: 1.0 }, ..Default::default() },
            UiDrawCmd { kind: ui_draw::ARC, rect: [50.0, 50.0, 40.0, 0.0], width: 4.0, extra: [0.0, 180.0], color: maker_api::Rgba { r: 1.0, g: 1.0, b: 1.0, a: 1.0 }, ..Default::default() },
            UiDrawCmd { kind: ui_draw::TEXT, rect: [0.0, 0.0, 100.0, 20.0], radius: 16.0, extra: [1.0, 1.0], str_ptr: txt.as_ptr(), str_len: 1, color: maker_api::Rgba { r: 1.0, g: 1.0, b: 1.0, a: 1.0 }, ..Default::default() },
        ];
        ui.fijar_dibujo(w, &cmds);
        ui.actualizar(&s, [800.0, 600.0], 0.0);
        let c = ui.lista.quads[0];
        assert_eq!(c.centro, [60.0, 60.0], "relativo a la esquina del widget");
        assert_eq!(c.radios, [10.0; 4]);
        assert!(ui.lista.quads.len() > 10, "el arco se teseló");
        assert!(ui.lista.quads.iter().any(|q| q.params[1] == MODO_GLIFO));
    }
}
