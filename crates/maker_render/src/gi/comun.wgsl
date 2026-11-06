// Maker Studio — iluminación global: declaraciones comunes.
// Se antepone a cada módulo de `gi/` (voxelizar, volumen, pantalla).
//
// El volumen es un clipmap de `C` cascadas de N³ vóxeles centradas en la
// cámara; cada cascada dobla el tamaño de vóxel de la anterior. Todas viven
// apiladas en Z dentro de una textura 3D (N × N × N·C). La radiancia se
// guarda premultiplicada por la opacidad (rgb·a, a) para que el filtrado
// trilineal de una superficie con huecos no la oscurezca al despremultiplicar.

const PI: f32 = 3.14159265;
const MAX_CASCADAS: u32 = 4u;

struct Gi {
    view_proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    prev_view_proj: mat4x4<f32>,
    prev_inv_view_proj: mat4x4<f32>,
    cam_pos: vec4<f32>,          // xyz; w: nº de fotograma
    res: vec4<f32>,              // ancho y alto del trazado en pantalla, 1/ancho, 1/alto
    cascadas: array<vec4<f32>, 4>,      // esquina mínima xyz, tamaño de vóxel
    prev_cascadas: array<vec4<f32>, 4>, // las del fotograma anterior (realimentación)
    vol: vec4<u32>,              // N, nº de cascadas, rayos difusos por píxel, pasos de reflejo
    params: vec4<f32>,           // luz ambiente (cielo), rebotes múltiples, historial válido, modo de depuración
    sky_top: vec4<f32>,
    sky_horizon: vec4<f32>,
    ground: vec4<f32>,
};

@group(0) @binding(0) var<uniform> gi: Gi;
@group(0) @binding(1) var samp_lin: sampler;

fn n_vox() -> f32 {
    return f32(gi.vol.x);
}

fn n_casc() -> u32 {
    return min(gi.vol.y, MAX_CASCADAS);
}

fn tam_voxel(c: u32, prev: bool) -> f32 {
    if (prev) {
        return gi.prev_cascadas[c].w;
    }
    return gi.cascadas[c].w;
}

fn origen(c: u32, prev: bool) -> vec3<f32> {
    if (prev) {
        return gi.prev_cascadas[c].xyz;
    }
    return gi.cascadas[c].xyz;
}

// ¿Está `p` dentro de la cascada `c` con `margen` vóxeles de holgura?
fn dentro(p: vec3<f32>, c: u32, margen: f32, prev: bool) -> bool {
    let vs = tam_voxel(c, prev);
    let l = (p - origen(c, prev)) / vs;
    let n = n_vox();
    return all(l >= vec3<f32>(margen)) && all(l <= vec3<f32>(n - margen));
}

// Cascada más fina que contiene `p` (n_casc() si ninguna).
fn cascada_de(p: vec3<f32>, margen: f32, prev: bool) -> u32 {
    let nc = n_casc();
    for (var c = 0u; c < nc; c += 1u) {
        if (dentro(p, c, margen, prev)) {
            return c;
        }
    }
    return nc;
}

// Coordenada de textura de `p` en la cascada `c` (sin salirse de su bloque en Z).
fn uvw(p: vec3<f32>, c: u32, prev: bool) -> vec3<f32> {
    let n = n_vox();
    let l = clamp((p - origen(c, prev)) / (tam_voxel(c, prev) * n), vec3<f32>(0.5 / n), vec3<f32>(1.0 - 0.5 / n));
    return vec3<f32>(l.xy, (l.z + f32(c)) / f32(n_casc()));
}

fn cielo(dir: vec3<f32>) -> vec3<f32> {
    let t = dir.y;
    if (t >= 0.0) {
        return mix(gi.sky_horizon.rgb, gi.sky_top.rgb, pow(t, 0.6));
    }
    return mix(gi.sky_horizon.rgb, gi.ground.rgb, pow(min(-t * 3.0, 1.0), 0.5));
}

fn luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

// Hash entero (PCG) → [0, 1).
fn pcg(v: u32) -> u32 {
    let s = v * 747796405u + 2891336453u;
    let w = ((s >> ((s >> 28u) + 4u)) ^ s) * 277803737u;
    return (w >> 22u) ^ w;
}

fn azar(semilla: ptr<function, u32>) -> f32 {
    *semilla = pcg(*semilla);
    return f32(*semilla >> 8u) / 16777216.0;
}

// Base ortonormal alrededor de `n` (Duff et al. 2017).
fn base_de(n: vec3<f32>) -> mat3x3<f32> {
    let s = select(-1.0, 1.0, n.z >= 0.0);
    let a = -1.0 / (s + n.z);
    let b = n.x * n.y * a;
    let t = vec3<f32>(1.0 + s * n.x * n.x * a, s * b, -s * n.x);
    let bt = vec3<f32>(b, s + n.y * n.y * a, -n.y);
    return mat3x3<f32>(t, bt, n);
}

// Dirección con distribución coseno alrededor de `n`.
fn coseno(n: vec3<f32>, u: vec2<f32>) -> vec3<f32> {
    let r = sqrt(u.x);
    let phi = 2.0 * PI * u.y;
    let l = vec3<f32>(r * cos(phi), r * sin(phi), sqrt(max(1.0 - u.x, 0.0)));
    return normalize(base_de(n) * l);
}
