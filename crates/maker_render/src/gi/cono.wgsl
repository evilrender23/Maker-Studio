// Trazado de conos en el clipmap de radiancia (`rad`, declarado por el módulo).
// Cada paso elige la cascada cuyo vóxel se parece más al diámetro del cono:
// las cascadas hacen de niveles de mip, como en un clipmap de VXGI.

const PASOS_CONO: u32 = 96u;

// Integra de delante atrás a lo largo del cono. Devuelve la radiancia
// acumulada (rgb) y la oclusión (a). `prev` usa las cascadas del fotograma
// anterior (realimentación de rebotes).
fn cono(origen_cono: vec3<f32>, dir: vec3<f32>, tan_mitad: f32, t0: f32, t_max: f32, prev: bool) -> vec4<f32> {
    var acc = vec4<f32>(0.0);
    let vs0 = tam_voxel(0u, prev);
    let nc = n_casc();
    var t = t0;
    for (var i = 0u; i < PASOS_CONO; i += 1u) {
        let p = origen_cono + dir * t;
        let diam = max(vs0, 2.0 * tan_mitad * t);
        var c = u32(clamp(floor(log2(diam / vs0)), 0.0, f32(nc - 1u)));
        loop {
            if (c >= nc || dentro(p, c, 1.0, prev)) {
                break;
            }
            c += 1u;
        }
        if (c >= nc) {
            break;
        }
        let vs = tam_voxel(c, prev);
        let s = textureSampleLevel(rad, samp_lin, uvw(p, c, prev), 0.0);
        let paso = max(diam, vs) * 0.5;
        let grueso = c > 0u;
        if (s.a > 1e-4) {
            // Corrección de opacidad por longitud de paso.
            // En las cascadas gruesas una pared fina ocupa un solo vóxel y el
            // trilineal la deja a medias: se refuerza para que sea estanca.
            let a = 1.0 - pow(1.0 - min(s.a * select(1.0, 2.0, grueso), 0.999), paso / vs);
            acc = vec4<f32>(acc.rgb + (1.0 - acc.a) * s.rgb * (a / s.a), acc.a + (1.0 - acc.a) * a);
            if (acc.a > 0.97) {
                break;
            }
        }
        t += paso;
        if (t > t_max) {
            break;
        }
    }
    return acc;
}

// Afinado: recorre [ta, tb] con pasos de medio vóxel de la cascada más fina
// que contiene cada punto hasta encontrar la superficie, y toma su radiancia
// un poco antes de ella (del lado del rayo). Así una pared más fina que los
// vóxeles gruesos no deja pasar la luz del otro lado. w = 0 si no la encuentra.
fn afinar(o: vec3<f32>, dir: vec3<f32>, ta: f32, tb: f32, prev: bool) -> vec4<f32> {
    let nc = n_casc();
    var t = ta;
    for (var i = 0u; i < 48u; i += 1u) {
        let p = o + dir * t;
        let c = cascada_de(p, 1.0, prev);
        if (c >= nc) {
            break;
        }
        let vs = tam_voxel(c, prev);
        let s = textureSampleLevel(rad, samp_lin, uvw(p, c, prev), 0.0);
        if (s.a > 0.3) {
            let q = p - dir * vs * 0.5;
            let cq = min(cascada_de(q, 1.0, prev), nc - 1u);
            let sq = textureSampleLevel(rad, samp_lin, uvw(q, cq, prev), 0.0);
            let m = select(s, sq, sq.a > 0.05);
            return vec4<f32>(m.rgb / max(m.a, 1e-3), 1.0);
        }
        t += vs * 0.5;
        if (t > tb) {
            break;
        }
    }
    return vec4<f32>(0.0);
}

// `sesgo`: cuántas cascadas más finas que el diámetro del cono se muestrean
// (menos ocultación de más en conos anchos, más fugas por paredes finas).
// Como `cono`, pero al llegar a una superficie (opacidad ≥ ½) afina el
// choque en la cascada fina y lo trata como opaco. Para luz indirecta.
fn traza(origen_cono: vec3<f32>, dir: vec3<f32>, tan_mitad: f32, t0: f32, t_max: f32, prev: bool, sesgo: f32) -> vec4<f32> {
    var acc = vec4<f32>(0.0);
    let vs0 = tam_voxel(0u, prev);
    let nc = n_casc();
    var t = t0;
    var t_ant = t0;
    for (var i = 0u; i < PASOS_CONO; i += 1u) {
        let p = origen_cono + dir * t;
        let diam = max(vs0, 2.0 * tan_mitad * t);
        var c = u32(clamp(floor(log2(diam / vs0)) - sesgo, 0.0, f32(nc - 1u)));
        loop {
            if (c >= nc || dentro(p, c, 1.0, prev)) {
                break;
            }
            c += 1u;
        }
        if (c >= nc) {
            break;
        }
        let vs = tam_voxel(c, prev);
        let s = textureSampleLevel(rad, samp_lin, uvw(p, c, prev), 0.0);
        // Como mucho un vóxel de la cascada muestreada: no saltarse una pared fina.
        let paso = min(max(diam, vs) * 0.5, vs);
        // Muestra gruesa: el vóxel mezcla las dos caras de lo que toca.
        let grueso = c > 0u || paso > 0.6 * vs;
        if (s.a > 1e-4) {
            // En las cascadas gruesas una pared fina ocupa un solo vóxel y el
            // trilineal la deja a medias: se refuerza para que sea estanca.
            let a = 1.0 - pow(1.0 - min(s.a * select(1.0, 2.0, grueso), 0.999), paso / vs);
            var color = s.rgb * (a / s.a);
            if (grueso) {
                if (acc.a + (1.0 - acc.a) * a >= 0.5) {
                    // Desde un paso más atrás: el filtrado grueso puede cruzar ½ ya dentro de la pared.
                    let h = afinar(origen_cono, dir, max(t0, 2.0 * t_ant - t), t + paso, prev);
                    if (h.w > 0.0) {
                        return vec4<f32>(acc.rgb + (1.0 - acc.a) * h.rgb, 1.0);
                    }
                } else {
                    // Un vóxel grueso mezcla las dos caras de una pared: de él
                    // solo se toma la oclusión; la radiancia sale del afinado.
                    color = vec3<f32>(0.0);
                }
            }
            acc = vec4<f32>(acc.rgb + (1.0 - acc.a) * color, acc.a + (1.0 - acc.a) * a);
            if (acc.a > 0.97) {
                break;
            }
        }
        t_ant = t;
        t += paso;
        if (t > t_max) {
            break;
        }
    }
    return acc;
}

// Radiancia de la superficie en `p` (despremultiplicada), buscando de la
// cascada más fina a la más gruesa. a = 0 si no hay nada.
fn radiancia_en(p: vec3<f32>) -> vec4<f32> {
    let nc = n_casc();
    for (var c = 0u; c < nc; c += 1u) {
        if (dentro(p, c, 1.0, false)) {
            let s = textureSampleLevel(rad, samp_lin, uvw(p, c, false), 0.0);
            if (s.a > 0.02) {
                return vec4<f32>(s.rgb / s.a, 1.0);
            }
        }
    }
    return vec4<f32>(0.0);
}
