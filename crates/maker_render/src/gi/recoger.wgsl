// Recolección final (final gather) a la resolución del trazado en pantalla.
//
// Difusa: N rayos con distribución coseno por píxel y fotograma. Cada rayo
// intenta primero un trazado corto contra la profundidad de la pantalla; si
// choca, toma la radiancia del vóxel de la superficie alcanzada. Si sale de
// la pantalla o no choca, sigue como cono por el clipmap de radiancia y lo
// que escapa ve el cielo (multiplicado por la luz ambiente).
// Reflejos: rayo reflejado (perturbado según la rugosidad) con trazado en
// pantalla más largo y refinado; si falla, un cono cuya apertura depende de
// la rugosidad; después el cielo. Las superficies rugosas reutilizan la
// difusa.

@group(1) @binding(0) var rad: texture_3d<f32>;
@group(1) @binding(1) var gb_normal: texture_2d<f32>;
@group(1) @binding(2) var gb_depth: texture_depth_2d;
@group(1) @binding(3) var dif_out: texture_storage_2d<rgba16float, write>;
@group(1) @binding(4) var spec_out: texture_storage_2d<rgba16float, write>;

const RUGOSO: f32 = 0.75;

fn mundo(px: vec2<f32>, d: f32) -> vec3<f32> {
    let uv = px * gi.res.zw;
    let w = gi.inv_view_proj * vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, d, 1.0);
    return w.xyz / w.w;
}

fn a_pixel(ndc: vec2<f32>) -> vec2<i32> {
    let p = (ndc * vec2<f32>(0.5, -0.5) + 0.5) * gi.res.xy;
    return clamp(vec2<i32>(p), vec2<i32>(0), vec2<i32>(gi.res.xy) - 1);
}

struct Choque {
    pos: vec3<f32>,
    n: vec3<f32>,
    ok: bool,
};

// ¿El punto `q` del rayo quedó detrás de la superficie visible? Devuelve la
// superficie si está a menos de `grosor` (si no, el rayo pasa por detrás).
fn probar(q: vec3<f32>, dir: vec3<f32>) -> Choque {
    var r: Choque;
    r.ok = false;
    let c = gi.view_proj * vec4<f32>(q, 1.0);
    if (c.w <= 1e-4) {
        return r;
    }
    let ndc = c.xyz / c.w;
    let px = a_pixel(ndc.xy);
    let ds = textureLoad(gb_depth, px, 0);
    if (ds >= 1.0 || ndc.z <= ds) {
        return r;
    }
    let s = mundo(vec2<f32>(px) + 0.5, ds);
    let dc = length(s - gi.cam_pos.xyz);
    let grosor = max(0.12, dc * 0.03);
    if (length(q - gi.cam_pos.xyz) - dc > grosor) {
        return r;
    }
    let n = textureLoad(gb_normal, px, 0).xyz;
    // Debe mirar hacia el rayo (descarta la propia superficie de salida).
    if (dot(n, dir) > -0.05) {
        return r;
    }
    r.pos = s;
    r.n = n;
    r.ok = true;
    return r;
}

fn fuera(q: vec3<f32>) -> bool {
    let c = gi.view_proj * vec4<f32>(q, 1.0);
    if (c.w <= 1e-4) {
        return true;
    }
    return any(abs(c.xy / c.w) > vec2<f32>(1.0));
}

// Trazado en pantalla con pasos crecientes y refinado binario al cruzar.
fn traza_pantalla(p: vec3<f32>, dir: vec3<f32>, dist: f32, pasos: u32, jit: f32) -> Choque {
    var r: Choque;
    r.ok = false;
    var t_ant = 0.0;
    for (var i = 0u; i < pasos; i += 1u) {
        let x = (f32(i) + jit) / f32(pasos);
        let t = dist * x * x;
        let q = p + dir * t;
        if (fuera(q)) {
            return r;
        }
        let h = probar(q, dir);
        if (h.ok) {
            // Refinado entre el paso anterior y este.
            var lo = t_ant;
            var hi = t;
            var mejor = h;
            for (var k = 0; k < 4; k += 1) {
                let m = (lo + hi) * 0.5;
                let hm = probar(p + dir * m, dir);
                if (hm.ok) {
                    hi = m;
                    mejor = hm;
                } else {
                    lo = m;
                }
            }
            return mejor;
        }
        t_ant = t;
    }
    return r;
}

// Radiancia que llega por un rayo: pantalla → volumen → cielo.
fn radiancia_rayo(p: vec3<f32>, n: vec3<f32>, dir: vec3<f32>, tan_mitad: f32, dist_pantalla: f32, pasos: u32, jit: f32, vs: f32) -> vec3<f32> {
    if (pasos > 0u) {
        let h = traza_pantalla(p + n * vs * 0.25, dir, dist_pantalla, pasos, jit);
        if (h.ok) {
            let l = radiancia_en(h.pos - h.n * tam_voxel(0u, false) * 0.25);
            if (l.a > 0.0) {
                return l.rgb;
            }
        }
    }
    let acc = traza(p + n * vs, dir, tan_mitad, vs, 150.0, false, 0.0);
    return acc.rgb + (1.0 - acc.a) * cielo(dir) * gi.params.x;
}

@compute @workgroup_size(8, 8, 1)
fn recoger(@builtin(global_invocation_id) id: vec3<u32>) {
    let res = vec2<u32>(gi.res.xy);
    if (id.x >= res.x || id.y >= res.y) {
        return;
    }
    let px = vec2<i32>(id.xy);
    let d = textureLoad(gb_depth, px, 0);
    let gb = textureLoad(gb_normal, px, 0);
    if (d >= 1.0 || gb.a < 0.0) {
        textureStore(dif_out, px, vec4<f32>(0.0));
        textureStore(spec_out, px, vec4<f32>(0.0));
        return;
    }
    let pos = mundo(vec2<f32>(px) + 0.5, d);
    let n = normalize(gb.xyz);
    let rough = gb.a;
    // Separación de la superficie: un vóxel de la cascada que la contiene.
    let c = min(cascada_de(pos, 1.0, false), n_casc() - 1u);
    let vs = tam_voxel(c, false) * 1.5;
    var semilla = pcg(id.x + pcg(id.y + pcg(u32(gi.cam_pos.w))));
    let rayos = max(gi.vol.z, 1u);
    let dist_cam = length(pos - gi.cam_pos.xyz);
    let dist_corta = clamp(dist_cam * 0.25, 0.5, 3.0);
    var dif = vec3<f32>(0.0);
    for (var k = 0u; k < rayos; k += 1u) {
        let dir = coseno(n, vec2<f32>(azar(&semilla), azar(&semilla)));
        dif += radiancia_rayo(pos, n, dir, 0.3, dist_corta, 8u, azar(&semilla), vs);
    }
    dif /= f32(rayos);
    var spec = dif;
    if (rough < RUGOSO) {
        let v = normalize(gi.cam_pos.xyz - pos);
        let a = rough * rough;
        var r = reflect(-v, n);
        let j = vec3<f32>(azar(&semilla), azar(&semilla), azar(&semilla)) * 2.0 - 1.0;
        let rj = normalize(r + j * a * 0.35);
        if (dot(rj, n) > 0.0) {
            r = rj;
        }
        let tan_mitad = clamp(a * 0.9, 0.012, 0.6);
        let s = radiancia_rayo(pos, n, r, tan_mitad, clamp(dist_cam * 1.5, 4.0, 40.0), gi.vol.w, azar(&semilla), vs * 0.5);
        spec = mix(s, dif, smoothstep(RUGOSO * 0.6, RUGOSO, rough));
    }
    textureStore(dif_out, px, vec4<f32>(dif, 1.0));
    textureStore(spec_out, px, vec4<f32>(spec, 1.0));
}

// Vista de depuración: el volumen visto desde la cámara (radiancia de los vóxeles).
@compute @workgroup_size(8, 8, 1)
fn ver_voxeles(@builtin(global_invocation_id) id: vec3<u32>) {
    let res = vec2<u32>(gi.res.xy);
    if (id.x >= res.x || id.y >= res.y) {
        return;
    }
    let px = vec2<i32>(id.xy);
    let lejos = mundo(vec2<f32>(px) + 0.5, 1.0);
    let dir = normalize(lejos - gi.cam_pos.xyz);
    let acc = cono(gi.cam_pos.xyz, dir, 0.0005, 0.0, 300.0, false);
    let col = acc.rgb / max(acc.a, 1e-3) * step(0.05, acc.a) + (1.0 - acc.a) * cielo(dir) * 0.15;
    textureStore(dif_out, px, vec4<f32>(col, 1.0));
    textureStore(spec_out, px, vec4<f32>(col, 1.0));
}
