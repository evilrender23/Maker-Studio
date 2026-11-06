// ───────────── Preludio de los efectos de posprocesado de Maker Studio ─────────────
// Se añade DETRÁS del código de cada efecto (las declaraciones de WGSL no
// dependen del orden), así los números de línea de los errores coinciden con
// los del archivo del usuario.

struct Post {
    resolucion: vec4<f32>,   // ancho, alto, 1/ancho, 1/alto de la salida (px)
    texel: vec4<f32>,        // 1/ancho, 1/alto, ancho, alto de la entrada (px)
    tiempo: f32,             // segundos desde que arrancó el renderizador
    intensidad: f32,         // fuerza del efecto (Inspector)
    escala: f32,             // alto de la salida / 1080
    hdr: f32,                // 1 si la entrada aún no pasó por el tonemap
    inv_vista_proy: mat4x4<f32>,
    camara: vec4<f32>,       // posición de la cámara (mundo)
    p: array<vec4<f32>, 16>, // parámetros (uno por vec4)
};

@group(0) @binding(0) var entrada: texture_2d<f32>;
@group(0) @binding(1) var muestreo: sampler;
@group(0) @binding(2) var profundidad_tex: texture_depth_multisampled_2d;
@group(0) @binding(3) var<uniform> post: Post;
@group(0) @binding(4) var extra: texture_2d<f32>;

struct PostIn {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

// Triángulo a pantalla completa; uv (0,0) arriba a la izquierda.
@vertex
fn vs_post(@builtin(vertex_index) i: u32) -> PostIn {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var o: PostIn;
    o.pos = vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
    o.uv = vec2<f32>(p.x, 1.0 - p.y);
    return o;
}

fn muestra(uv: vec2<f32>) -> vec4<f32> {
    return textureSampleLevel(entrada, muestreo, uv, 0.0);
}

// Profundidad del búfer (0 = cerca, 1 = cielo), primera muestra del MSAA.
fn profundidad(uv: vec2<f32>) -> f32 {
    let d = vec2<i32>(textureDimensions(profundidad_tex));
    let c = clamp(vec2<i32>(uv * vec2<f32>(d)), vec2<i32>(0), d - 1);
    return textureLoad(profundidad_tex, c, 0);
}

fn es_cielo(uv: vec2<f32>) -> bool {
    return profundidad(uv) >= 1.0;
}

fn posicion_mundo(uv: vec2<f32>) -> vec3<f32> {
    let ndc = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, profundidad(uv), 1.0);
    let w = post.inv_vista_proy * ndc;
    return w.xyz / w.w;
}

// Distancia a la cámara en metros.
fn distancia(uv: vec2<f32>) -> f32 {
    return length(posicion_mundo(uv) - post.camara.xyz);
}

// Normal de mundo reconstruida de la profundidad (vecino más cercano en cada eje).
fn normal_mundo(uv: vec2<f32>) -> vec3<f32> {
    let px = 1.0 / vec2<f32>(textureDimensions(profundidad_tex));
    let c = posicion_mundo(uv);
    let r = posicion_mundo(uv + vec2<f32>(px.x, 0.0));
    let l = posicion_mundo(uv - vec2<f32>(px.x, 0.0));
    let d = posicion_mundo(uv + vec2<f32>(0.0, px.y));
    let u = posicion_mundo(uv - vec2<f32>(0.0, px.y));
    let dx = select(c - l, r - c, length(r - c) < length(c - l));
    let dy = select(c - u, d - c, length(d - c) < length(c - u));
    var n = normalize(cross(dx, dy));
    if (dot(n, post.camara.xyz - c) < 0.0) {
        n = -n;
    }
    return n;
}

fn luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

fn a_srgb(c: vec3<f32>) -> vec3<f32> {
    let x = max(c, vec3<f32>(0.0));
    return select(1.055 * pow(x, vec3<f32>(1.0 / 2.4)) - 0.055, x * 12.92, x <= vec3<f32>(0.0031308));
}

fn a_lineal(c: vec3<f32>) -> vec3<f32> {
    let x = max(c, vec3<f32>(0.0));
    return select(pow((x + 0.055) / 1.055, vec3<f32>(2.4)), x / 12.92, x <= vec3<f32>(0.04045));
}

// Ruido blanco 0..1.
fn ruido(p: vec2<f32>) -> f32 {
    let q = fract(p * vec2<f32>(0.1031, 0.1030));
    let r = q + dot(q, q.yx + 33.33);
    return fract((r.x + r.y) * r.x);
}
