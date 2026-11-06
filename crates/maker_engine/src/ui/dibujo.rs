//! Lista de dibujo de la interfaz: instancias de un único quad (cajas con
//! esquinas redondeadas y borde por SDF, imágenes, glifos y sombras) agrupadas
//! en lotes por textura de imagen y recorte. `maker_render` la sube tal cual a
//! un búfer de instancias.

use bytemuck::{Pod, Zeroable};

pub const MODO_CAJA: f32 = 0.0;
pub const MODO_IMAGEN: f32 = 1.0;
pub const MODO_GLIFO: f32 = 2.0;
pub const MODO_SOMBRA: f32 = 3.0;

/// Sin textura de imagen (el lote puede usar cualquiera).
pub const SIN_IMAGEN: u32 = u32::MAX;

/// Una instancia (píxeles de pantalla, y hacia abajo; colores lineales sin premultiplicar).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct QuadUi {
    pub centro: [f32; 2],
    /// Semiejes del rectángulo (incluyen rotación y escala).
    pub eje_x: [f32; 2],
    pub eje_y: [f32; 2],
    pub _r: [f32; 2],
    /// u0, v0, u1, v1.
    pub uv: [f32; 4],
    pub color: [f32; 4],
    /// Final del degradado (igual a `color` si no hay).
    pub color2: [f32; 4],
    pub borde: [f32; 4],
    /// Radios en píxeles: sup. izq., sup. der., inf. der., inf. izq.
    pub radios: [f32; 4],
    /// Ancho de borde, modo, difuminado (sombras), ángulo del degradado (rad).
    pub params: [f32; 4],
}

/// Tramo de instancias con la misma imagen y el mismo recorte.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lote {
    pub imagen: u32,
    /// x, y, ancho, alto en píxeles.
    pub recorte: [u32; 4],
    pub inicio: u32,
    pub fin: u32,
}

#[derive(Default)]
pub struct ListaDibujo {
    pub quads: Vec<QuadUi>,
    pub lotes: Vec<Lote>,
    /// Tamaño de la pantalla en píxeles.
    pub tam: [u32; 2],
}

impl ListaDibujo {
    pub fn limpiar(&mut self, tam: [u32; 2]) {
        self.quads.clear();
        self.lotes.clear();
        self.tam = tam;
    }
    /// Añade un quad; los que no usan imagen se unen a cualquier lote con el
    /// mismo recorte, así un HUD entero suele ser un solo dibujo.
    pub fn empujar(&mut self, q: QuadUi, imagen: u32, recorte: Option<[f32; 4]>) {
        let r = match recorte {
            Some(r) => {
                let x0 = r[0].max(0.0).floor() as u32;
                let y0 = r[1].max(0.0).floor() as u32;
                let x1 = (r[2].min(self.tam[0] as f32).ceil() as u32).max(x0);
                let y1 = (r[3].min(self.tam[1] as f32).ceil() as u32).max(y0);
                [x0, y0, x1 - x0, y1 - y0]
            }
            None => [0, 0, self.tam[0], self.tam[1]],
        };
        if r[2] == 0 || r[3] == 0 {
            return;
        }
        let n = self.quads.len() as u32;
        self.quads.push(q);
        if let Some(l) = self.lotes.last_mut()
            && l.recorte == r
            && (imagen == SIN_IMAGEN || l.imagen == SIN_IMAGEN || l.imagen == imagen)
        {
            if l.imagen == SIN_IMAGEN {
                l.imagen = imagen;
            }
            l.fin = n + 1;
            return;
        }
        self.lotes.push(Lote { imagen, recorte: r, inicio: n, fin: n + 1 });
    }
}

/// Transformación afín 2D: x' = a·x + c·y + e, y' = b·x + d·y + f.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Afin(pub [f32; 6]);

impl Default for Afin {
    fn default() -> Self {
        Afin::IDENTIDAD
    }
}

impl Afin {
    pub const IDENTIDAD: Afin = Afin([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
    pub fn escala(sx: f32, sy: f32) -> Afin {
        Afin([sx, 0.0, 0.0, sy, 0.0, 0.0])
    }
    pub fn traslacion(x: f32, y: f32) -> Afin {
        Afin([1.0, 0.0, 0.0, 1.0, x, y])
    }
    /// Rotación en grados (horaria en pantalla, con y hacia abajo).
    pub fn rotacion(grados: f32) -> Afin {
        let (s, c) = grados.to_radians().sin_cos();
        Afin([c, s, -s, c, 0.0, 0.0])
    }
    /// `self` después de `o` (primero `o`).
    pub fn por(&self, o: &Afin) -> Afin {
        let [a, b, c, d, e, f] = self.0;
        let [a2, b2, c2, d2, e2, f2] = o.0;
        Afin([a * a2 + c * b2, b * a2 + d * b2, a * c2 + c * d2, b * c2 + d * d2, a * e2 + c * f2 + e, b * e2 + d * f2 + f])
    }
    pub fn aplicar(&self, p: [f32; 2]) -> [f32; 2] {
        let [a, b, c, d, e, f] = self.0;
        [a * p[0] + c * p[1] + e, b * p[0] + d * p[1] + f]
    }
    pub fn lineal(&self, v: [f32; 2]) -> [f32; 2] {
        let [a, b, c, d, _, _] = self.0;
        [a * v[0] + c * v[1], b * v[0] + d * v[1]]
    }
    pub fn inversa(&self) -> Afin {
        let [a, b, c, d, e, f] = self.0;
        let det = a * d - b * c;
        if det.abs() < 1e-12 {
            return Afin([0.0; 6]);
        }
        let i = 1.0 / det;
        let (na, nb, nc, nd) = (d * i, -b * i, -c * i, a * i);
        Afin([na, nb, nc, nd, -(na * e + nc * f), -(nb * e + nd * f)])
    }
    /// Escala media (para tamaños de texto, radios y bordes).
    pub fn escala_media(&self) -> f32 {
        let [a, b, c, d, _, _] = self.0;
        (a * d - b * c).abs().sqrt()
    }
    /// Sin rotación ni sesgo.
    pub fn alineada(&self) -> bool {
        self.0[1].abs() < 1e-6 && self.0[2].abs() < 1e-6
    }
}

/// Quad de un rectángulo local transformado.
pub fn quad_rect(xf: &Afin, min: [f32; 2], max: [f32; 2]) -> QuadUi {
    let c = xf.aplicar([(min[0] + max[0]) * 0.5, (min[1] + max[1]) * 0.5]);
    QuadUi { centro: c, eje_x: xf.lineal([(max[0] - min[0]) * 0.5, 0.0]), eje_y: xf.lineal([0.0, (max[1] - min[1]) * 0.5]), uv: [0.0, 0.0, 1.0, 1.0], ..Default::default() }
}

/// Premultiplica nada: los colores van lineales y sin premultiplicar; el
/// sombreador premultiplica. Multiplica el alfa por la opacidad.
pub fn con_opacidad(c: [f32; 4], o: f32) -> [f32; 4] {
    [c[0], c[1], c[2], c[3] * o]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn afin_compone_e_invierte() {
        let m = Afin::traslacion(10.0, 5.0).por(&Afin::rotacion(90.0)).por(&Afin::escala(2.0, 2.0));
        let p = m.aplicar([1.0, 0.0]);
        assert!((p[0] - 10.0).abs() < 1e-4 && (p[1] - 7.0).abs() < 1e-4, "{p:?}");
        let q = m.inversa().aplicar(p);
        assert!((q[0] - 1.0).abs() < 1e-4 && q[1].abs() < 1e-4);
        assert!((m.escala_media() - 2.0).abs() < 1e-5);
        assert!(!m.alineada() && Afin::escala(3.0, 1.0).alineada());
    }

    #[test]
    fn lotes_se_agrupan_por_imagen_y_recorte() {
        let mut l = ListaDibujo::default();
        l.limpiar([100, 100]);
        let q = QuadUi::default();
        l.empujar(q, SIN_IMAGEN, None);
        l.empujar(q, 3, None);
        l.empujar(q, SIN_IMAGEN, None);
        assert_eq!(l.lotes.len(), 1, "las cajas y glifos no rompen el lote");
        assert_eq!(l.lotes[0].imagen, 3);
        l.empujar(q, 4, None);
        assert_eq!(l.lotes.len(), 2, "otra imagen sí");
        l.empujar(q, 4, Some([10.0, 10.0, 50.0, 50.0]));
        assert_eq!(l.lotes.len(), 3, "otro recorte también");
        assert_eq!(l.lotes[2].recorte, [10, 10, 40, 40]);
        l.empujar(q, 4, Some([200.0, 0.0, 300.0, 50.0]));
        assert_eq!(l.quads.len(), 5, "recorte fuera de pantalla: no se dibuja");
    }
}
