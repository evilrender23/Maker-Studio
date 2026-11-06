// Maker Studio — shaders del renderizador de escena.
// PBR metal-rugosidad (GGX / Smith / Schlick), sol con sombras PCF,
// hasta 16 luces puntuales, ambiente hemisférico, niebla y tonemap ACES.
// Modelos estilizados: toon (cel con bandas, borde y contorno de casco
// invertido) y plumilla (rayado en pantalla). Cada modelo es un pipeline.
// Todos leen el color base de la textura del material (× factor × color de
// vértice × tinte de la entidad), con recorte alfa opcional, y tienen una
// variante con piel (paleta de huesos en un búfer de almacenamiento).

const PI: f32 = 3.14159265;
const MAX_POINTS: u32 = 16u;

// Constantes de especialización por pipeline.
override ALPHA_TEST: bool = false;
override BLEND: bool = false;

struct PointLight {
    pos_range: vec4<f32>,
    color: vec4<f32>,
};

struct Globals {
    view_proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    light_vp: mat4x4<f32>,
    cam_pos: vec4<f32>,
    sun_dir: vec4<f32>,      // xyz: dirección en la que viaja la luz; w: hay sol
    sun_color: vec4<f32>,    // rgb * intensidad; w: sombras activas
    sky_top: vec4<f32>,
    sky_horizon: vec4<f32>,
    ground: vec4<f32>,       // w: intensidad ambiente
    fog: vec4<f32>,          // rgb, densidad
    params: vec4<f32>,       // x: nº luces puntuales, y: exposición, z: tiempo, w: tamaño del mapa de sombras
    points: array<PointLight, 16>,
    viewport: vec4<f32>,     // ancho, alto (px)
};

struct Object {
    model: mat4x4<f32>,
    normal_m: mat4x4<f32>,
    base_color: vec4<f32>,
    emissive: vec4<f32>,     // rgb, fuerza
    params: vec4<f32>,       // metallic, roughness, unlit, separación de plumilla (px)
    shadow_tint: vec4<f32>,  // rgb lineal (tinte de sombra), nº de bandas
    rim: vec4<f32>,          // rgb lineal, fuerza
    style: vec4<f32>,        // suavidad, especular, tamaño especular, ancho del borde
    outline: vec4<f32>,      // rgb lineal, ancho (px a 1080 p)
    skin: vec4<u32>,         // x: primer hueso en `joints`
};

struct Material {
    factor: vec4<f32>,       // color base lineal + alfa del glTF
    params: vec4<f32>,       // x: umbral de recorte alfa
};

@group(0) @binding(0) var<uniform> g: Globals;
@group(0) @binding(1) var shadow_map: texture_depth_2d;
@group(0) @binding(2) var shadow_sampler: sampler_comparison;
@group(1) @binding(0) var<uniform> obj: Object;
@group(1) @binding(1) var<storage, read> joints: array<mat4x4<f32>>;
@group(2) @binding(0) var base_tex: texture_2d<f32>;
@group(2) @binding(1) var base_samp: sampler;
@group(2) @binding(2) var<uniform> mat: Material;

struct VIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) color: vec4<f32>,
};

struct SkinIn {
    @location(4) joints: vec4<u32>,
    @location(5) weights: vec4<f32>,
};

struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) color: vec4<f32>,
};

const IDENTITY: mat4x4<f32> = mat4x4<f32>(
    vec4<f32>(1.0, 0.0, 0.0, 0.0), vec4<f32>(0.0, 1.0, 0.0, 0.0),
    vec4<f32>(0.0, 0.0, 1.0, 0.0), vec4<f32>(0.0, 0.0, 0.0, 1.0),
);

// Mezcla lineal de hasta 4 huesos (pesos renormalizados).
fn skin_matrix(s: SkinIn) -> mat4x4<f32> {
    let total = s.weights.x + s.weights.y + s.weights.z + s.weights.w;
    if (total < 1e-4) {
        return IDENTITY;
    }
    let b = obj.skin.x;
    let m = joints[b + s.joints.x] * s.weights.x + joints[b + s.joints.y] * s.weights.y
          + joints[b + s.joints.z] * s.weights.z + joints[b + s.joints.w] * s.weights.w;
    return m * (1.0 / total);
}

fn shade_vertex(v: VIn, skin: mat4x4<f32>) -> VOut {
    let w = obj.model * (skin * vec4<f32>(v.pos, 1.0));
    var o: VOut;
    o.clip = g.view_proj * w;
    o.world = w.xyz;
    o.normal = normalize((obj.normal_m * (skin * vec4<f32>(v.normal, 0.0))).xyz);
    o.uv = v.uv;
    o.color = v.color;
    return o;
}

@vertex
fn vs_main(v: VIn) -> VOut {
    return shade_vertex(v, IDENTITY);
}

@vertex
fn vs_main_skinned(v: VIn, s: SkinIn) -> VOut {
    return shade_vertex(v, skin_matrix(s));
}

// Color base: textura × factor del material × color de vértice × tinte de la entidad.
fn base_albedo(in: VOut) -> vec4<f32> {
    let t = textureSample(base_tex, base_samp, in.uv);
    return obj.base_color * mat.factor * t * in.color;
}

// Recorte alfa (hojas) al final del sombreado; alfa de salida solo al mezclar.
fn finish(color: vec3<f32>, a: f32) -> vec4<f32> {
    if (ALPHA_TEST && a < mat.params.x) {
        discard;
    }
    return vec4<f32>(color, select(1.0, a, BLEND));
}

fn aces(x: vec3<f32>) -> vec3<f32> {
    let a = 2.51;
    let b = 0.03;
    let c = 2.43;
    let d = 0.59;
    let e = 0.14;
    return clamp((x * (a * x + b)) / (x * (c * x + d) + e), vec3<f32>(0.0), vec3<f32>(1.0));
}

fn shadow_factor(world: vec3<f32>, n: vec3<f32>) -> f32 {
    if (g.sun_color.w < 0.5) {
        return 1.0;
    }
    let texel = 1.0 / g.params.w;
    let lp = g.light_vp * vec4<f32>(world + n * 0.03, 1.0);
    let ndc = lp.xyz / lp.w;
    let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    if (uv.x <= 0.0 || uv.x >= 1.0 || uv.y <= 0.0 || uv.y >= 1.0 || ndc.z >= 1.0) {
        return 1.0;
    }
    var sum = 0.0;
    for (var x: i32 = -1; x <= 1; x += 1) {
        for (var y: i32 = -1; y <= 1; y += 1) {
            sum += textureSampleCompareLevel(shadow_map, shadow_sampler, uv + vec2<f32>(f32(x), f32(y)) * texel, ndc.z - 0.0008);
        }
    }
    return sum / 9.0;
}

fn d_ggx(nh: f32, a: f32) -> f32 {
    let a2 = a * a;
    let d = nh * nh * (a2 - 1.0) + 1.0;
    return a2 / (PI * d * d + 1e-6);
}

fn g_smith(nv: f32, nl: f32, rough: f32) -> f32 {
    let k = (rough + 1.0) * (rough + 1.0) / 8.0;
    return (nv / (nv * (1.0 - k) + k)) * (nl / (nl * (1.0 - k) + k));
}

fn fresnel(c: f32, f0: vec3<f32>) -> vec3<f32> {
    return f0 + (vec3<f32>(1.0) - f0) * pow(1.0 - c, 5.0);
}

fn brdf(n: vec3<f32>, v: vec3<f32>, l: vec3<f32>, albedo: vec3<f32>, metallic: f32, rough: f32) -> vec3<f32> {
    let nl = max(dot(n, l), 0.0);
    if (nl <= 0.0) {
        return vec3<f32>(0.0);
    }
    let h = normalize(v + l);
    let nv = max(dot(n, v), 1e-4);
    let nh = max(dot(n, h), 0.0);
    let vh = max(dot(v, h), 0.0);
    let f0 = mix(vec3<f32>(0.04), albedo, metallic);
    let f = fresnel(vh, f0);
    let a = max(rough * rough, 0.002);
    let spec = d_ggx(nh, a) * g_smith(nv, nl, rough) * f / (4.0 * nv * nl + 1e-4);
    let kd = (vec3<f32>(1.0) - f) * (1.0 - metallic);
    return (kd * albedo / PI + spec) * nl;
}

fn sky_color(dir: vec3<f32>) -> vec3<f32> {
    let t = dir.y;
    if (t >= 0.0) {
        return mix(g.sky_horizon.rgb, g.sky_top.rgb, pow(t, 0.6));
    }
    return mix(g.sky_horizon.rgb, g.ground.rgb, pow(min(-t * 3.0, 1.0), 0.5));
}

@fragment
fn fs_main(in: VOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let base = base_albedo(in);
    let albedo = base.rgb;
    var n = normalize(in.normal);
    if (!front) {
        n = -n;
    }
    var color: vec3<f32>;
    if (obj.params.z > 0.5) {
        color = albedo + obj.emissive.rgb * obj.emissive.w;
    } else {
        let metallic = clamp(obj.params.x, 0.0, 1.0);
        let rough = clamp(obj.params.y, 0.04, 1.0);
        let v = normalize(g.cam_pos.xyz - in.world);
        color = vec3<f32>(0.0);
        if (g.sun_dir.w > 0.5) {
            let l = -normalize(g.sun_dir.xyz);
            color += brdf(n, v, l, albedo, metallic, rough) * g.sun_color.rgb * shadow_factor(in.world, n);
        }
        let count = min(u32(g.params.x), MAX_POINTS);
        for (var i: u32 = 0u; i < count; i += 1u) {
            let p = g.points[i];
            let d = p.pos_range.xyz - in.world;
            let dist = length(d);
            let range = max(p.pos_range.w, 0.01);
            let r = dist / range;
            let falloff = clamp(1.0 - r * r * r * r, 0.0, 1.0);
            let atten = falloff * falloff / (dist * dist + 1.0);
            color += brdf(n, v, d / max(dist, 1e-4), albedo, metallic, rough) * p.color.rgb * atten;
        }
        // Ambiente hemisférico + reflejo aproximado del cielo.
        let amb = g.ground.w;
        let hemi = mix(g.ground.rgb, g.sky_top.rgb, n.y * 0.5 + 0.5);
        let f0 = mix(vec3<f32>(0.04), albedo, metallic);
        let fr = f0 + (max(vec3<f32>(1.0 - rough), f0) - f0) * pow(1.0 - max(dot(n, v), 0.0), 5.0);
        let env = sky_color(reflect(-v, n));
        color += hemi * albedo * (1.0 - metallic) * amb + env * fr * amb * (1.0 - rough * 0.7);
        color += obj.emissive.rgb * obj.emissive.w;
        color = apply_fog(color, in.world);
    }
    return finish(aces(color * g.params.y), base.a);
}

// Niebla exponencial.
fn apply_fog(color: vec3<f32>, world: vec3<f32>) -> vec3<f32> {
    let fd = length(g.cam_pos.xyz - world);
    let fog = 1.0 - exp(-g.fog.w * fd);
    return mix(color, g.fog.rgb, clamp(fog, 0.0, 1.0));
}

fn point_atten(p: PointLight, world: vec3<f32>) -> f32 {
    let dist = length(p.pos_range.xyz - world);
    let r = dist / max(p.pos_range.w, 0.01);
    let falloff = clamp(1.0 - r * r * r * r, 0.0, 1.0);
    return falloff * falloff / (dist * dist + 1.0);
}

fn luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

// ───────────────────────────── Toon / cel ─────────────────────────────
// Bandas de luz duras con borde suave configurable, sombra tintada (no negra),
// sombras proyectadas convertidas en banda, brillo especular en banda y luz de
// borde (fresnel). El contorno se dibuja aparte (vs_ink / fs_ink).

// Escalón centrado en `t` con medio ancho `s`.
fn band_step(x: f32, t: f32, s: f32) -> f32 {
    return smoothstep(t - s - 1e-4, t + s + 1e-4, x);
}

// Nivel de luz cuantizado en `n` bandas (0 = sombra, 1 = luz plena) a partir
// de N·L en -1..1. El terminador queda en N·L = 0.
fn toon_level(ndl: f32, n: f32, s: f32) -> f32 {
    let x = ndl * 0.5 + 0.5;
    let steps = max(n - 1.0, 1.0);
    var sum = 0.0;
    for (var k: i32 = 0; k < i32(steps); k += 1) {
        sum += band_step(x, 0.5 + f32(k) * 0.4 / steps, s);
    }
    return sum / steps;
}

// Escalera de paso `step` con bordes suavizados (los saltos caen en los medios pasos).
fn posterize(x: f32, step: f32, s: f32) -> f32 {
    let t = x / step;
    return (floor(t) + band_step(fract(t), 0.5, s)) * step;
}

@fragment
fn fs_toon(in: VOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let base = base_albedo(in);
    let albedo = base.rgb;
    var n = normalize(in.normal);
    if (!front) {
        n = -n;
    }
    let v = normalize(g.cam_pos.xyz - in.world);
    let s = clamp(obj.style.x, 0.0, 0.5) * 0.5;
    let bands = clamp(obj.shadow_tint.w, 2.0, 4.0);
    let tint = obj.shadow_tint.rgb;
    // Luz de relleno plana: el ambiente sin gradiente para no romper las bandas.
    let fill = mix(g.ground.rgb, g.sky_top.rgb, 0.6) * g.ground.w * 0.35;
    var color = albedo * (tint + fill);
    var lit = 0.0;
    if (g.sun_dir.w > 0.5) {
        let l = -normalize(g.sun_dir.xyz);
        let ndl = dot(n, l);
        let shadow = band_step(shadow_factor(in.world, n), 0.5, s);
        lit = toon_level(ndl, bands, s) * shadow;
        let sun = g.sun_color.rgb / PI;
        color = mix(color, albedo * (sun + fill), lit);
        // Brillo especular en banda.
        if (obj.style.y > 0.0) {
            let h = normalize(l + v);
            let shin = mix(512.0, 16.0, clamp(obj.style.z, 0.0, 1.0));
            let sp = pow(max(dot(n, h), 0.0), shin);
            color += g.sun_color.rgb * obj.style.y * band_step(sp, 0.5, s) * shadow * step(0.0, ndl);
        }
    }
    let count = min(u32(g.params.x), MAX_POINTS);
    for (var i: u32 = 0u; i < count; i += 1u) {
        let p = g.points[i];
        let l = normalize(p.pos_range.xyz - in.world);
        // Energía posterizada: charcos de luz con borde en vez de degradado.
        let e = max(dot(n, l), 0.0) * point_atten(p, in.world) * luma(p.color.rgb) / PI;
        let q = posterize(e, 0.3, s);
        color += albedo * p.color.rgb / max(luma(p.color.rgb), 1e-4) * q;
    }
    // Luz de borde, solo del lado iluminado (como en Breath of the Wild).
    if (obj.rim.w > 0.0) {
        let fres = 1.0 - max(dot(n, v), 0.0);
        let rim = band_step(fres, 1.0 - clamp(obj.style.w, 0.0, 1.0), s);
        color += obj.rim.rgb * obj.rim.w * rim * lit;
    }
    color += obj.emissive.rgb * obj.emissive.w;
    color = apply_fog(color, in.world);
    return finish(aces(color * g.params.y), base.a);
}

// ───────────────────────────── Plumilla (hatching) ─────────────────────────────
// Trazos en espacio de pantalla sobre el color base: cuanto menos luz, más capas
// de trazos cruzados y más gruesos. La tinta usa el color del contorno.

// Cobertura 0..1 de una familia de líneas paralelas.
fn hatch_lines(p: vec2<f32>, dir: vec2<f32>, spacing: f32, width: f32) -> f32 {
    let d = dot(p, dir) / spacing;
    let dist = abs(fract(d) - 0.5) * spacing;
    let aa = max(fwidth(d) * spacing, 0.5);
    let half_w = width * spacing * 0.5;
    return (1.0 - smoothstep(half_w - aa, half_w + aa, dist)) * step(0.001, width);
}

@fragment
fn fs_hatch(in: VOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let base = base_albedo(in);
    let albedo = base.rgb;
    var n = normalize(in.normal);
    if (!front) {
        n = -n;
    }
    var tone = g.ground.w * 0.3;
    if (g.sun_dir.w > 0.5) {
        let l = -normalize(g.sun_dir.xyz);
        tone += max(dot(n, l), 0.0) * shadow_factor(in.world, n) * luma(g.sun_color.rgb / PI);
    }
    let count = min(u32(g.params.x), MAX_POINTS);
    for (var i: u32 = 0u; i < count; i += 1u) {
        let p = g.points[i];
        let l = normalize(p.pos_range.xyz - in.world);
        tone += max(dot(n, l), 0.0) * point_atten(p, in.world) * luma(p.color.rgb) / PI;
    }
    tone = clamp(tone, 0.0, 1.0);
    // Píxeles normalizados a 1080 p: el trazo no cambia con la resolución.
    let px = in.clip.xy * (1080.0 / max(g.viewport.y, 1.0));
    let spacing = max(obj.params.w, 2.0);
    let ramp = 0.12 + clamp(obj.style.x, 0.0, 0.5);
    let cover = vec4<f32>(
        clamp((0.85 - tone) / ramp, 0.0, 1.0),
        clamp((0.6 - tone) / ramp, 0.0, 1.0),
        clamp((0.38 - tone) / ramp, 0.0, 1.0),
        clamp((0.18 - tone) / ramp, 0.0, 1.0),
    ) * 0.5;
    var ink = hatch_lines(px, vec2<f32>(0.7071, 0.7071), spacing, cover.x);
    ink = max(ink, hatch_lines(px, vec2<f32>(-0.7071, 0.7071), spacing, cover.y));
    ink = max(ink, hatch_lines(px, vec2<f32>(0.0, 1.0), spacing, cover.z));
    ink = max(ink, hatch_lines(px + vec2<f32>(spacing * 0.5), vec2<f32>(0.7071, 0.7071), spacing, cover.w));
    var color = mix(albedo, obj.outline.rgb, ink);
    color += obj.emissive.rgb * obj.emissive.w;
    color = apply_fog(color, in.world);
    return finish(aces(color * g.params.y), base.a);
}

// ───────────────────────────── Contorno de casco invertido ─────────────────────────────
// Se dibuja con cull de caras frontales: las traseras, desplazadas en pantalla
// a lo largo de la normal suavizada, asoman alrededor de la silueta. El ancho es
// en píxeles (a 1080 p) y adelgaza con la distancia.

struct InkIn {
    @location(0) pos: vec3<f32>,
    @location(1) smooth_normal: vec3<f32>,
};

@vertex
fn vs_ink(v: InkIn) -> @builtin(position) vec4<f32> {
    return ink_vertex(v, IDENTITY);
}

@vertex
fn vs_ink_skinned(v: InkIn, s: SkinIn) -> @builtin(position) vec4<f32> {
    return ink_vertex(v, skin_matrix(s));
}

fn ink_vertex(v: InkIn, skin: mat4x4<f32>) -> vec4<f32> {
    let w = obj.model * (skin * vec4<f32>(v.pos, 1.0));
    let n = normalize((obj.normal_m * (skin * vec4<f32>(v.smooth_normal, 0.0))).xyz);
    var clip = g.view_proj * w;
    let dir = (g.view_proj * vec4<f32>(n, 0.0)).xy * g.viewport.xy;
    let len = length(dir);
    if (len > 1e-6 && clip.w > 0.0) {
        let px = obj.outline.w * (g.viewport.y / 1080.0) * clamp(10.0 / clip.w, 0.4, 1.0);
        clip = vec4<f32>(clip.xy + dir / len * px * 2.0 / g.viewport.xy * clip.w, clip.zw);
    }
    return clip;
}

@fragment
fn fs_ink() -> @location(0) vec4<f32> {
    return vec4<f32>(obj.outline.rgb, 1.0);
}

// ───────────────────────────── Contorno de selección ─────────────────────────────

@vertex
fn vs_outline(v: VIn) -> @builtin(position) vec4<f32> {
    return outline_vertex(v, IDENTITY);
}

@vertex
fn vs_outline_skinned(v: VIn, s: SkinIn) -> @builtin(position) vec4<f32> {
    return outline_vertex(v, skin_matrix(s));
}

fn outline_vertex(v: VIn, skin: mat4x4<f32>) -> vec4<f32> {
    let w = obj.model * (skin * vec4<f32>(v.pos, 1.0));
    let n = normalize((obj.normal_m * (skin * vec4<f32>(v.normal, 0.0))).xyz);
    let dist = length(g.cam_pos.xyz - w.xyz);
    return g.view_proj * vec4<f32>(w.xyz + n * dist * 0.006, 1.0);
}

// ───────────────────────────── Mapa de sombras ─────────────────────────────
// Se dibuja con un Globals propio cuyo `view_proj` es la matriz de la luz.

struct ShadowOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) alpha: f32,
};

fn shadow_vertex(v: VIn, skin: mat4x4<f32>) -> ShadowOut {
    var o: ShadowOut;
    o.clip = g.view_proj * (obj.model * (skin * vec4<f32>(v.pos, 1.0)));
    o.uv = v.uv;
    o.alpha = v.color.a * obj.base_color.a;
    return o;
}

@vertex
fn vs_shadow(v: VIn) -> ShadowOut {
    return shadow_vertex(v, IDENTITY);
}

@vertex
fn vs_shadow_skinned(v: VIn, s: SkinIn) -> ShadowOut {
    return shadow_vertex(v, skin_matrix(s));
}

// Solo para materiales con recorte: las hojas proyectan su silueta.
@fragment
fn fs_shadow_mask(in: ShadowOut) {
    let a = textureSample(base_tex, base_samp, in.uv).a * mat.factor.a * in.alpha;
    if (a < mat.params.x) {
        discard;
    }
}

@fragment
fn fs_outline() -> @location(0) vec4<f32> {
    // #0053fd (primario del tema) en lineal.
    return vec4<f32>(0.0, 0.0887, 0.9823, 1.0);
}

// ───────────────────────────── Cielo ─────────────────────────────

struct SkyOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) ndc: vec2<f32>,
};

@vertex
fn vs_sky(@builtin(vertex_index) i: u32) -> SkyOut {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u)) * 2.0 - 1.0;
    var o: SkyOut;
    o.clip = vec4<f32>(p, 0.0, 1.0);
    o.ndc = p;
    return o;
}

@fragment
fn fs_sky(in: SkyOut) -> @location(0) vec4<f32> {
    let far = g.inv_view_proj * vec4<f32>(in.ndc, 1.0, 1.0);
    let dir = normalize(far.xyz / far.w - g.cam_pos.xyz);
    var c = sky_color(dir);
    if (g.sun_dir.w > 0.5) {
        let s = max(dot(dir, -normalize(g.sun_dir.xyz)), 0.0);
        c += g.sun_color.rgb * (pow(s, 900.0) * 4.0 + pow(s, 12.0) * 0.06);
    }
    return vec4<f32>(aces(c * g.params.y), 1.0);
}

// ───────────────────────────── Rejilla del editor ─────────────────────────────

struct GridOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
};

@vertex
fn vs_grid(@builtin(vertex_index) i: u32) -> GridOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0),
    );
    let c = corners[i];
    let size = 300.0;
    let w = vec3<f32>(floor(g.cam_pos.x) + c.x * size, 0.0, floor(g.cam_pos.z) + c.y * size);
    var o: GridOut;
    o.clip = g.view_proj * vec4<f32>(w, 1.0);
    o.world = w;
    return o;
}

fn grid_line(coord: vec2<f32>, scale: f32) -> f32 {
    let c = coord / scale;
    let d = fwidth(c);
    let l = abs(fract(c - 0.5) - 0.5) / max(d, vec2<f32>(1e-5));
    return 1.0 - min(min(l.x, l.y), 1.0);
}

@fragment
fn fs_grid(in: GridOut) -> @location(0) vec4<f32> {
    let coord = in.world.xz;
    let minor = grid_line(coord, 1.0);
    let major = grid_line(coord, 10.0);
    var color = vec3<f32>(0.18, 0.19, 0.22);
    var alpha = max(minor * 0.22, major * 0.45);
    let fw = fwidth(coord);
    let ax = 1.0 - min(abs(coord.y) / max(fw.y, 1e-5), 1.0);   // eje X (z = 0)
    let az = 1.0 - min(abs(coord.x) / max(fw.x, 1e-5), 1.0);   // eje Z (x = 0)
    if (ax > 0.01) {
        color = mix(color, vec3<f32>(0.62, 0.03, 0.09), ax);
        alpha = max(alpha, ax * 0.9);
    }
    if (az > 0.01) {
        color = mix(color, vec3<f32>(0.0, 0.09, 0.98), az);
        alpha = max(alpha, az * 0.9);
    }
    let dist = length(g.cam_pos.xz - coord);
    let height = max(abs(g.cam_pos.y), 1.0);
    let fade = 1.0 - smoothstep(height * 6.0, height * 25.0 + 40.0, dist);
    return vec4<f32>(color, alpha * fade);
}
