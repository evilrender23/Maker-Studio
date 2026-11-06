//! Cálculo de los contenedores (filas, columnas y rejillas), sin árbol: el
//! sistema le pasa los tamaños preferidos de los hijos y recibe sus
//! rectángulos.

use super::{Alinear, Contenedor, TipoContenedor};

/// Tamaño del contenido de un contenedor (relleno incluido).
pub fn contenido(c: &Contenedor, prefs: &[[f32; 2]]) -> [f32; 2] {
    let [l, t, r, b] = c.relleno;
    let n = prefs.len();
    let hueco = |k: usize, s: f32| if k > 1 { (k - 1) as f32 * s } else { 0.0 };
    match c.tipo {
        TipoContenedor::Horizontal => {
            let w: f32 = prefs.iter().map(|p| p[0]).sum::<f32>() + hueco(n, c.espacio[0]);
            let h = prefs.iter().map(|p| p[1]).fold(0.0, f32::max);
            [l + r + w, t + b + h]
        }
        TipoContenedor::Vertical => {
            let h: f32 = prefs.iter().map(|p| p[1]).sum::<f32>() + hueco(n, c.espacio[1]);
            let w = prefs.iter().map(|p| p[0]).fold(0.0, f32::max);
            [l + r + w, t + b + h]
        }
        TipoContenedor::Rejilla => {
            let cols = (c.columnas.max(1) as usize).min(n.max(1));
            let filas = n.div_ceil(c.columnas.max(1) as usize);
            [l + r + cols as f32 * c.celda[0] + hueco(cols, c.espacio[0]), t + b + filas as f32 * c.celda[1] + hueco(filas, c.espacio[1])]
        }
    }
}

fn desplazar(al: Alinear, libre: f32) -> f32 {
    match al {
        Alinear::Inicio | Alinear::Estirar => 0.0,
        Alinear::Centro => libre * 0.5,
        Alinear::Fin => libre,
    }
}

/// Coloca los hijos dentro de `rect` (min, max). `flex` reparte el espacio
/// libre si el contenedor expande a sus hijos.
pub fn distribuir(c: &Contenedor, rect: ([f32; 2], [f32; 2]), prefs: &[[f32; 2]], flex: &[f32], out: &mut Vec<([f32; 2], [f32; 2])>) {
    out.clear();
    let [l, t, r, b] = c.relleno;
    let x0 = rect.0[0] + l;
    let y0 = rect.0[1] + t;
    let iw = (rect.1[0] - rect.0[0] - l - r).max(0.0);
    let ih = (rect.1[1] - rect.0[1] - t - b).max(0.0);
    let n = prefs.len();
    if n == 0 {
        return;
    }
    match c.tipo {
        TipoContenedor::Horizontal | TipoContenedor::Vertical => {
            let h = c.tipo == TipoContenedor::Horizontal;
            let (eje, cruz) = if h { (0, 1) } else { (1, 0) };
            let disponible = if h { iw } else { ih };
            let cruz_disp = if h { ih } else { iw };
            let esp = c.espacio[eje];
            let total: f32 = prefs.iter().map(|p| p[eje]).sum::<f32>() + esp * (n - 1) as f32;
            let libre = disponible - total;
            let suma_flex: f32 = flex.iter().map(|f| f.max(0.0)).sum();
            let expandir = c.expandir_hijos && libre > 0.0 && suma_flex > 0.0;
            let mut pos = if expandir { 0.0 } else { desplazar(c.alinear, libre) };
            for (i, p) in prefs.iter().enumerate() {
                let mut largo = p[eje];
                if expandir {
                    largo += libre * flex.get(i).copied().unwrap_or(1.0).max(0.0) / suma_flex;
                }
                let (ancho_cruz, off) = if c.cruzado == Alinear::Estirar { (cruz_disp, 0.0) } else { (p[cruz], desplazar(c.cruzado, cruz_disp - p[cruz])) };
                let mut mn = [0.0; 2];
                let mut mx = [0.0; 2];
                mn[eje] = pos;
                mx[eje] = pos + largo;
                mn[cruz] = off;
                mx[cruz] = off + ancho_cruz;
                out.push(([x0 + mn[0], y0 + mn[1]], [x0 + mx[0], y0 + mx[1]]));
                pos += largo + esp;
            }
        }
        TipoContenedor::Rejilla => {
            let cols = c.columnas.max(1) as usize;
            let tam = contenido(&Contenedor { relleno: [0.0; 4], ..c.clone() }, prefs);
            let ox = desplazar(c.alinear, iw - tam[0]);
            let oy = desplazar(c.cruzado, ih - tam[1]);
            for i in 0..n {
                let (col, fila) = ((i % cols) as f32, (i / cols) as f32);
                let x = x0 + ox + col * (c.celda[0] + c.espacio[0]);
                let y = y0 + oy + fila * (c.celda[1] + c.espacio[1]);
                out.push(([x, y], [x + c.celda[0], y + c.celda[1]]));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caja(tipo: TipoContenedor) -> Contenedor {
        Contenedor { tipo, relleno: [10.0, 5.0, 10.0, 5.0], espacio: [4.0, 4.0], alinear: Alinear::Inicio, cruzado: Alinear::Inicio, ..Default::default() }
    }

    #[test]
    fn fila_alinea_y_estira() {
        let mut c = caja(TipoContenedor::Horizontal);
        let prefs = [[50.0, 20.0], [30.0, 40.0]];
        assert_eq!(contenido(&c, &prefs), [10.0 + 50.0 + 4.0 + 30.0 + 10.0, 5.0 + 40.0 + 5.0]);
        let mut out = Vec::new();
        distribuir(&c, ([0.0, 0.0], [200.0, 100.0]), &prefs, &[1.0, 1.0], &mut out);
        assert_eq!(out[0], ([10.0, 5.0], [60.0, 25.0]));
        assert_eq!(out[1], ([64.0, 5.0], [94.0, 45.0]));
        c.alinear = Alinear::Fin;
        c.cruzado = Alinear::Estirar;
        distribuir(&c, ([0.0, 0.0], [200.0, 100.0]), &prefs, &[1.0, 1.0], &mut out);
        assert_eq!(out[1].1, [190.0, 95.0], "al final, estirado en alto");
        c.alinear = Alinear::Centro;
        c.cruzado = Alinear::Centro;
        distribuir(&c, ([0.0, 0.0], [200.0, 100.0]), &prefs, &[1.0, 1.0], &mut out);
        let libre = 180.0 - 84.0;
        assert_eq!(out[0].0, [10.0 + libre / 2.0, 5.0 + 35.0]);
    }

    #[test]
    fn columna_expande_por_flex() {
        let mut c = caja(TipoContenedor::Vertical);
        c.expandir_hijos = true;
        let mut out = Vec::new();
        // Interior 90 de alto, 2 hijos de 10 y un hueco de 4 → 66 libres a repartir 1:2.
        distribuir(&c, ([0.0, 0.0], [100.0, 100.0]), &[[20.0, 10.0], [20.0, 10.0]], &[1.0, 2.0], &mut out);
        assert!((out[0].1[1] - out[0].0[1] - 32.0).abs() < 1e-4);
        assert!((out[1].1[1] - out[1].0[1] - 54.0).abs() < 1e-4);
        assert!((out[1].1[1] - 95.0).abs() < 1e-4, "llena hasta el relleno inferior");
    }

    #[test]
    fn rejilla_por_celdas() {
        let mut c = caja(TipoContenedor::Rejilla);
        c.columnas = 2;
        c.celda = [20.0, 10.0];
        let prefs = [[0.0; 2]; 3];
        assert_eq!(contenido(&c, &prefs), [10.0 + 44.0 + 10.0, 5.0 + 24.0 + 5.0]);
        let mut out = Vec::new();
        distribuir(&c, ([0.0, 0.0], [100.0, 100.0]), &prefs, &[1.0; 3], &mut out);
        assert_eq!(out[2], ([10.0, 19.0], [30.0, 29.0]));
    }
}
