// Voxelización por rasterizado y prepase de normales/profundidad.
//
// Voxelizar: cada malla se dibuja tres veces (una por eje, `instance_index`)
// con una proyección ortográfica que cubre la cascada; el fragmento suma
// albedo, normal y emisión en el vóxel con atómicos. Sin geometry shaders:
// la unión de las tres proyecciones cubre también los triángulos de canto.
// Prepase: normal y rugosidad a la resolución del trazado en pantalla.

struct Object {
    model: mat4x4<f32>,
    normal_m: mat3x3<f32>,
    base_color: vec4<f32>,
    emissive: vec4<f32>,
    params: vec4<f32>,       // metallic, roughness, unlit, —
    shadow_tint: vec4<f32>,
    rim: vec4<f32>,
    style: vec4<f32>,
    outline: vec4<f32>,
    nrm: vec4<f32>,
    skin: vec4<u32>,
};

struct Material {
    factor: vec4<f32>,
    params: vec4<f32>,       // x: umbral de recorte alfa
};

struct Vox {
    cascada: vec4<u32>,
};

override ALPHA_TEST: bool = false;

@group(0) @binding(2) var<uniform> vox: Vox;
@group(0) @binding(3) var<storage, read_write> acum: array<atomic<u32>>;
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

const IDENTITY: mat4x4<f32> = mat4x4<f32>(
    vec4<f32>(1.0, 0.0, 0.0, 0.0), vec4<f32>(0.0, 1.0, 0.0, 0.0),
    vec4<f32>(0.0, 0.0, 1.0, 0.0), vec4<f32>(0.0, 0.0, 0.0, 1.0),
);

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

struct VoxOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
    @location(3) @interpolate(flat) eje: u32,
};

// Ejes permutados para que el de proyección quede en Z.
fn permutar(l: vec3<f32>, eje: u32) -> vec3<f32> {
    switch eje {
        case 0u: { return l.yzx; }
        case 1u: { return l.zxy; }
        default: { return l; }
    }
}

fn desp(l: vec3<f32>, eje: u32) -> vec3<f32> {
    switch eje {
        case 0u: { return l.zxy; }
        case 1u: { return l.yzx; }
        default: { return l; }
    }
}

fn voxel_vertex(v: VIn, skin: mat4x4<f32>, inst: u32) -> VoxOut {
    let c = vox.cascada.x;
    let w = obj.model * (skin * vec4<f32>(v.pos, 1.0));
    let l = (w.xyz - gi.cascadas[c].xyz) / (gi.cascadas[c].w * n_vox());
    let eje = inst % 3u;
    let q = permutar(l, eje);
    var o: VoxOut;
    o.clip = vec4<f32>(q.x * 2.0 - 1.0, q.y * 2.0 - 1.0, q.z, 1.0);
    o.normal = normalize(obj.normal_m * (skin * vec4<f32>(v.normal, 0.0)).xyz);
    o.uv = v.uv;
    o.color = v.color;
    o.eje = eje;
    return o;
}

@vertex
fn vs_voxel(v: VIn, @builtin(instance_index) inst: u32) -> VoxOut {
    return voxel_vertex(v, IDENTITY, inst);
}

@vertex
fn vs_voxel_skinned(v: VIn, s: SkinIn, @builtin(instance_index) inst: u32) -> VoxOut {
    return voxel_vertex(v, skin_matrix(s), inst);
}

@fragment
fn fs_voxel(in: VoxOut, @builtin(front_facing) front: bool) {
    // Color medio de la textura (último mip): un vóxel no ve detalle fino.
    let t = textureSampleLevel(base_tex, base_samp, in.uv, 16.0);
    let base = obj.base_color * mat.factor * t * in.color;
    if (ALPHA_TEST && base.a < mat.params.x) {
        discard;
    }
    let n = n_vox();
    // y del framebuffer hacia abajo.
    let q = vec3<f32>(in.clip.x / n, 1.0 - in.clip.y / n, in.clip.z);
    let l = desp(q, in.eje);
    let ni = u32(n);
    let i = min(vec3<u32>(l * n), vec3<u32>(ni - 1u));
    let idx = ((i.z * ni + i.y) * ni + i.x) * 13u;
    let a = clamp(base.rgb, vec3<f32>(0.0), vec3<f32>(1.0)) * 255.0;
    atomicAdd(&acum[idx + 0u], u32(a.r + 0.5));
    atomicAdd(&acum[idx + 1u], u32(a.g + 0.5));
    atomicAdd(&acum[idx + 2u], u32(a.b + 0.5));
    atomicAdd(&acum[idx + 3u], 1u);
    var nn = normalize(in.normal);
    if (!front) {
        nn = -nn;
    }
    let ns = vec3<i32>(round(nn * 127.0));
    atomicAdd(&acum[idx + 4u], bitcast<u32>(ns.x));
    atomicAdd(&acum[idx + 5u], bitcast<u32>(ns.y));
    atomicAdd(&acum[idx + 6u], bitcast<u32>(ns.z));
    // Orientación sin signo: sobrevive cuando caras opuestas se anulan.
    let na = vec3<u32>(abs(ns));
    atomicAdd(&acum[idx + 10u], na.x);
    atomicAdd(&acum[idx + 11u], na.y);
    atomicAdd(&acum[idx + 12u], na.z);
    if (obj.emissive.w > 0.0) {
        let e = clamp(obj.emissive.rgb * obj.emissive.w * 64.0, vec3<f32>(0.0), vec3<f32>(65535.0));
        atomicAdd(&acum[idx + 7u], u32(e.r));
        atomicAdd(&acum[idx + 8u], u32(e.g));
        atomicAdd(&acum[idx + 9u], u32(e.b));
    }
}

// ───────────────────────────── Prepase ─────────────────────────────

struct PreOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) alpha: f32,
};

fn pre_vertex(v: VIn, skin: mat4x4<f32>) -> PreOut {
    let w = obj.model * (skin * vec4<f32>(v.pos, 1.0));
    var o: PreOut;
    o.clip = gi.view_proj * w;
    o.normal = normalize(obj.normal_m * (skin * vec4<f32>(v.normal, 0.0)).xyz);
    o.uv = v.uv;
    o.alpha = v.color.a * obj.base_color.a;
    return o;
}

@vertex
fn vs_pre(v: VIn) -> PreOut {
    return pre_vertex(v, IDENTITY);
}

@vertex
fn vs_pre_skinned(v: VIn, s: SkinIn) -> PreOut {
    return pre_vertex(v, skin_matrix(s));
}

// Normal de mundo + rugosidad (negativa = sin luz: no necesita GI).
@fragment
fn fs_pre(in: PreOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    if (ALPHA_TEST) {
        let a = textureSample(base_tex, base_samp, in.uv).a * mat.factor.a * in.alpha;
        if (a < mat.params.x) {
            discard;
        }
    }
    var n = normalize(in.normal);
    if (!front) {
        n = -n;
    }
    let rough = select(clamp(obj.params.y, 0.04, 1.0), -1.0, obj.params.z > 0.5);
    return vec4<f32>(n, rough);
}
