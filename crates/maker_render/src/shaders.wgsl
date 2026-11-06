// Maker Studio — shaders del renderizador de escena.
// PBR metal-rugosidad (GGX / Smith / Schlick), sol con sombras PCF,
// hasta 16 luces puntuales, ambiente hemisférico, niebla y tonemap ACES.

const PI: f32 = 3.14159265;
const MAX_POINTS: u32 = 16u;

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
};

struct Object {
    model: mat4x4<f32>,
    normal_m: mat4x4<f32>,
    base_color: vec4<f32>,
    emissive: vec4<f32>,     // rgb, fuerza
    params: vec4<f32>,       // metallic, roughness, unlit, —
};

@group(0) @binding(0) var<uniform> g: Globals;
@group(0) @binding(1) var shadow_map: texture_depth_2d;
@group(0) @binding(2) var shadow_sampler: sampler_comparison;
@group(1) @binding(0) var<uniform> obj: Object;

struct VIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
};

struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
};

@vertex
fn vs_main(v: VIn) -> VOut {
    let w = obj.model * vec4<f32>(v.pos, 1.0);
    var o: VOut;
    o.clip = g.view_proj * w;
    o.world = w.xyz;
    o.normal = normalize((obj.normal_m * vec4<f32>(v.normal, 0.0)).xyz);
    o.uv = v.uv;
    return o;
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
    let albedo = obj.base_color.rgb;
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
        // Niebla exponencial.
        let fd = length(g.cam_pos.xyz - in.world);
        let fog = 1.0 - exp(-g.fog.w * fd);
        color = mix(color, g.fog.rgb, clamp(fog, 0.0, 1.0));
    }
    return vec4<f32>(aces(color * g.params.y), obj.base_color.a);
}

// ───────────────────────────── Contorno de selección ─────────────────────────────

@vertex
fn vs_outline(v: VIn) -> @builtin(position) vec4<f32> {
    let w = obj.model * vec4<f32>(v.pos, 1.0);
    let n = normalize((obj.normal_m * vec4<f32>(v.normal, 0.0)).xyz);
    let dist = length(g.cam_pos.xyz - w.xyz);
    return g.view_proj * vec4<f32>(w.xyz + n * dist * 0.006, 1.0);
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
