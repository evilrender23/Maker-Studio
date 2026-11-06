// Volumen de vóxeles: inyectar la luz.
//
// inyectar: radiancia saliente de cada vóxel = emisión + albedo/π · luz
//   directa (sol con el mapa de sombras o, fuera de él, un cono hacia el sol;
//   luces puntuales con oclusión por vóxeles) + albedo · luz indirecta del
//   fotograma anterior (rebotes múltiples por realimentación).

struct PointLight {
    pos_range: vec4<f32>,
    color: vec4<f32>,
};

// Igual que `Globals` en shaders.wgsl (solo se leen estos campos).
struct Globals {
    view_proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    light_vp: mat4x4<f32>,
    cam_pos: vec4<f32>,
    sun_dir: vec4<f32>,
    sun_color: vec4<f32>,
    sky_top: vec4<f32>,
    sky_horizon: vec4<f32>,
    ground: vec4<f32>,
    fog: vec4<f32>,
    params: vec4<f32>,
    points: array<PointLight, 16>,
};

@group(1) @binding(0) var albedo_tex: texture_3d<f32>;
@group(1) @binding(1) var normal_tex: texture_3d<f32>;
@group(1) @binding(2) var emision_tex: texture_3d<f32>;
@group(1) @binding(3) var rad: texture_3d<f32>;
@group(1) @binding(4) var rad_out: texture_storage_3d<rgba16float, write>;
@group(1) @binding(5) var<uniform> g: Globals;
@group(1) @binding(6) var shadow_map: texture_depth_2d;
@group(1) @binding(7) var shadow_sampler: sampler_comparison;


fn sombra_sol(p: vec3<f32>, n: vec3<f32>, vs: f32, l: vec3<f32>) -> f32 {
    if (g.sun_color.w > 0.5) {
        let lp = g.light_vp * vec4<f32>(p + n * vs, 1.0);
        let ndc = lp.xyz / lp.w;
        let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
        if (all(uv > vec2<f32>(0.0)) && all(uv < vec2<f32>(1.0)) && ndc.z < 1.0) {
            return textureSampleCompareLevel(shadow_map, shadow_sampler, uv, ndc.z - 0.002);
        }
    }
    // Fuera del mapa de sombras (o sin sombras): un cono estrecho hacia el sol.
    let occ = cono(p + n * vs, l, 0.04, vs, 200.0, true).a;
    return 1.0 - occ;
}

@compute @workgroup_size(4, 4, 4)
fn inyectar(@builtin(global_invocation_id) id: vec3<u32>) {
    let ni = gi.vol.x;
    let nc = n_casc();
    if (id.x >= ni || id.y >= ni || id.z >= ni * nc) {
        return;
    }
    let tid = vec3<i32>(id);
    let alb = textureLoad(albedo_tex, tid, 0);
    if (alb.a <= 0.0) {
        textureStore(rad_out, tid, vec4<f32>(0.0));
        return;
    }
    let c = id.z / ni;
    let vs = gi.cascadas[c].w;
    let local = vec3<f32>(f32(id.x), f32(id.y), f32(id.z - c * ni)) + 0.5;
    let p = gi.cascadas[c].xyz + local * vs;
    let nt = textureLoad(normal_tex, tid, 0);
    var n = nt.xyz;
    if (nt.w > 0.5) {
        // Lámina de dos caras: la que mira a la cámara.
        n = normalize(n) * select(-1.0, 1.0, dot(n, gi.cam_pos.xyz - p) >= 0.0);
    }
    let dos_caras = dot(n, n) < 0.1;
    // Donde la cascada más fina también llega, este vóxel es la media de sus
    // 8 hijos (un muestreo trilineal en su centro, que es una esquina de la
    // fina): como un nivel de mip, sin conos propios que empiecen al otro
    // lado de una pared fina.
    // Las caras que no miran a la cámara pesan poco: así el vóxel grueso de
    // una pared fina no mezcla la luz de fuera con la de dentro.
    if (c > 0u && dentro(p, c - 1u, 2.0, true) && dentro(p, c - 1u, 2.0, false)) {
        let f = c - 1u;
        let vf = gi.cascadas[f].w;
        var suma = vec3<f32>(0.0);
        var peso = 0.0;
        for (var k = 0u; k < 8u; k += 1u) {
            let o = vec3<f32>(f32(k & 1u), f32((k >> 1u) & 1u), f32(k >> 2u)) - 0.5;
            let q = p + o * vf;
            let ip = vec3<i32>(floor((q - gi.prev_cascadas[f].xyz) / gi.prev_cascadas[f].w)) + vec3<i32>(0, 0, i32(f * ni));
            let ic = vec3<i32>(floor((q - gi.cascadas[f].xyz) / vf)) + vec3<i32>(0, 0, i32(f * ni));
            let s = textureLoad(rad, ip, 0);
            if (s.a <= 0.0) {
                continue;
            }
            let nh = textureLoad(normal_tex, ic, 0);
            let mira = nh.w > 0.5 || dot(nh.xyz, gi.cam_pos.xyz - q) >= 0.0;
            suma += s.rgb * select(0.1, 1.0, mira);
            peso += s.a;
        }
        let hijo = suma / max(peso, 1e-4);
        textureStore(rad_out, tid, vec4<f32>(hijo * alb.a, alb.a));
        return;
    }
    let emision = textureLoad(emision_tex, tid, 0).rgb;
    var e = vec3<f32>(0.0);
    if (g.sun_dir.w > 0.5) {
        let l = -normalize(g.sun_dir.xyz);
        let ndl = select(max(dot(n, l), 0.0), 1.0, dos_caras);
        if (ndl > 0.0) {
            e += g.sun_color.rgb * ndl * sombra_sol(p, select(n, l, dos_caras), vs, l);
        }
    }
    let cuenta = min(u32(g.params.x), 16u);
    for (var i = 0u; i < cuenta; i += 1u) {
        let pl = g.points[i];
        let d = pl.pos_range.xyz - p;
        let dist = length(d);
        let r = dist / max(pl.pos_range.w, 0.01);
        let falloff = clamp(1.0 - r * r * r * r, 0.0, 1.0);
        if (falloff <= 0.0) {
            continue;
        }
        let l = d / max(dist, 1e-4);
        let ndl = select(max(dot(n, l), 0.0), 1.0, dos_caras);
        if (ndl <= 0.0) {
            continue;
        }
        let atten = falloff * falloff / (dist * dist + 1.0);
        // Oclusión de la luz puntual con un cono fino por el volumen.
        let off = select(n, l, dos_caras) * vs;
        let occ = cono(p + off, l, 0.05, vs, max(dist - 2.0 * vs, vs), true).a;
        e += pl.color.rgb * atten * ndl * (1.0 - occ);
    }
    var rad_sal = emision + alb.rgb / PI * e;
    // Rebotes múltiples: luz indirecta del volumen del fotograma anterior.
    if (gi.params.y > 0.5) {
        var ind = vec3<f32>(0.0);
        var peso = 0.0;
        let ejes = array<vec3<f32>, 6>(
            vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(-1.0, 0.0, 0.0), vec3<f32>(0.0, 1.0, 0.0),
            vec3<f32>(0.0, -1.0, 0.0), vec3<f32>(0.0, 0.0, 1.0), vec3<f32>(0.0, 0.0, -1.0),
        );
        for (var k = 0u; k < 6u; k += 1u) {
            let d = ejes[k];
            let w = select(max(dot(n, d), 0.0), 1.0, dos_caras);
            if (w <= 0.0) {
                continue;
            }
            let acc = traza(p + d * vs, d, 0.577, vs, 100.0, true, 0.0);
            ind += w * (acc.rgb + (1.0 - acc.a) * cielo(d) * gi.params.x);
            peso += w;
        }
        rad_sal += alb.rgb * ind / max(peso, 1e-4);
    }
    textureStore(rad_out, tid, vec4<f32>(rad_sal * alb.a, alb.a));
}
