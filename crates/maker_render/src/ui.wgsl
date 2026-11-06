// Interfaz: un único quad instanciado para cajas (esquinas redondeadas y
// borde por SDF, color o degradado), sombras difuminadas, imágenes y glifos.
// Colores lineales; la salida va premultiplicada (mezcla One, OneMinusSrcAlpha)
// a un destino sRGB, así que la mezcla se hace en lineal.

struct Globales {
    tam: vec2<f32>,
    _r: vec2<f32>,
};

@group(0) @binding(0) var<uniform> g: Globales;
@group(0) @binding(1) var atlas: texture_2d<f32>;
@group(0) @binding(2) var muestreo: sampler;
@group(1) @binding(0) var imagen: texture_2d<f32>;

struct Instancia {
    @location(0) centro: vec2<f32>,
    @location(1) eje_x: vec2<f32>,
    @location(2) eje_y: vec2<f32>,
    @location(3) uv: vec4<f32>,
    @location(4) color: vec4<f32>,
    @location(5) color2: vec4<f32>,
    @location(6) borde: vec4<f32>,
    @location(7) radios: vec4<f32>,
    @location(8) params: vec4<f32>,
};

struct Salida {
    @builtin(position) pos: vec4<f32>,
    // Punto en el espacio local del rectángulo, en píxeles desde el centro.
    @location(0) local: vec2<f32>,
    @location(1) @interpolate(flat) mitad: vec2<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) @interpolate(flat) color: vec4<f32>,
    @location(4) @interpolate(flat) color2: vec4<f32>,
    @location(5) @interpolate(flat) borde: vec4<f32>,
    @location(6) @interpolate(flat) radios: vec4<f32>,
    @location(7) @interpolate(flat) params: vec4<f32>,
};

@vertex
fn vs_ui(@builtin(vertex_index) vi: u32, i: Instancia) -> Salida {
    let esquina = vec2<f32>(f32(vi & 1u) * 2.0 - 1.0, f32((vi >> 1u) & 1u) * 2.0 - 1.0);
    let lx = length(i.eje_x);
    let ly = length(i.eje_y);
    let ux = i.eje_x / max(lx, 1e-6);
    let uy = i.eje_y / max(ly, 1e-6);
    let modo = i.params.y;
    // Las cajas y sombras crecen un poco para el antialias y el difuminado.
    var margen = 0.0;
    if (modo < 0.5 || modo > 2.5 || (modo < 1.5 && max(i.radios.x, max(i.radios.y, max(i.radios.z, i.radios.w))) > 0.0)) {
        margen = 1.0 + i.params.z * 2.0;
    }
    let local = esquina * (vec2<f32>(lx, ly) + vec2<f32>(margen));
    let p = i.centro + ux * local.x + uy * local.y;
    var o: Salida;
    o.pos = vec4<f32>(p.x / g.tam.x * 2.0 - 1.0, 1.0 - p.y / g.tam.y * 2.0, 0.0, 1.0);
    o.local = local;
    o.mitad = vec2<f32>(lx, ly);
    let t = local / max(vec2<f32>(lx, ly), vec2<f32>(1e-6)) * 0.5 + 0.5;
    o.uv = mix(i.uv.xy, i.uv.zw, t);
    o.color = i.color;
    o.color2 = i.color2;
    o.borde = i.borde;
    o.radios = i.radios;
    o.params = i.params;
    return o;
}

// Distancia con signo a un rectángulo redondeado (radios: sup. izq., sup. der., inf. der., inf. izq.).
fn sd_caja(p: vec2<f32>, mitad: vec2<f32>, r4: vec4<f32>) -> f32 {
    var r = r4.x;
    if (p.x > 0.0 && p.y < 0.0) { r = r4.y; }
    if (p.x > 0.0 && p.y > 0.0) { r = r4.z; }
    if (p.x < 0.0 && p.y > 0.0) { r = r4.w; }
    r = min(r, min(mitad.x, mitad.y));
    let q = abs(p) - mitad + vec2<f32>(r);
    return min(max(q.x, q.y), 0.0) + length(max(q, vec2<f32>(0.0))) - r;
}

fn premul(c: vec4<f32>) -> vec4<f32> {
    return vec4<f32>(c.rgb * c.a, c.a);
}

@fragment
fn fs_ui(i: Salida) -> @location(0) vec4<f32> {
    // Muestreo en control de flujo uniforme.
    let cob_glifo = textureSample(atlas, muestreo, i.uv).r;
    let tex = textureSample(imagen, muestreo, i.uv);
    let modo = i.params.y;
    let d = sd_caja(i.local, i.mitad, i.radios);
    let cob = clamp(0.5 - d, 0.0, 1.0);
    if (modo < 0.5) {
        // Caja: relleno (color o degradado) y borde hacia dentro.
        let ang = i.params.w;
        let dir = vec2<f32>(cos(ang), sin(ang));
        let t = clamp(dot(i.local / max(i.mitad, vec2<f32>(1e-6)), dir) * 0.5 + 0.5, 0.0, 1.0);
        let relleno = premul(mix(i.color, i.color2, t));
        let ancho = i.params.x;
        var out = relleno * cob;
        if (ancho > 0.0) {
            let dentro = clamp(0.5 - (d + ancho), 0.0, 1.0);
            out = relleno * dentro + premul(i.borde) * (cob - dentro);
        }
        return out;
    }
    if (modo < 1.5) {
        // Imagen (alfa recto) × tinte, recortada por las esquinas redondeadas.
        var m = 1.0;
        if (max(i.radios.x, max(i.radios.y, max(i.radios.z, i.radios.w))) > 0.0) {
            m = cob;
        }
        return premul(tex * i.color) * m;
    }
    if (modo < 2.5) {
        return premul(i.color) * cob_glifo;
    }
    // Sombra: caja redondeada difuminada.
    let dif = max(i.params.z, 0.5);
    let a = 1.0 - smoothstep(-dif, dif, d);
    return premul(i.color) * a;
}
