// Acumulación temporal: reproyecta el historial con la cámara anterior y lo
// mezcla con la recolección de este fotograma. Cada muestra del bilineal se
// valida con la geometría (plano y normal): lo que se desocluye empieza de
// cero en vez de arrastrar luz de otro objeto. alfa = nº de fotogramas
// acumulados.

@group(1) @binding(0) var dif_nuevo: texture_2d<f32>;
@group(1) @binding(1) var spec_nuevo: texture_2d<f32>;
@group(1) @binding(2) var dif_hist: texture_2d<f32>;
@group(1) @binding(3) var spec_hist: texture_2d<f32>;
@group(1) @binding(4) var gb_normal: texture_2d<f32>;
@group(1) @binding(5) var gb_depth: texture_depth_2d;
@group(1) @binding(6) var gb_normal_prev: texture_2d<f32>;
@group(1) @binding(7) var gb_depth_prev: texture_depth_2d;
@group(1) @binding(8) var dif_out: texture_storage_2d<rgba16float, write>;
@group(1) @binding(9) var spec_out: texture_storage_2d<rgba16float, write>;

const MAX_DIF: f32 = 24.0;

fn mundo_con(m: mat4x4<f32>, px: vec2<f32>, d: f32) -> vec3<f32> {
    let uv = px * gi.res.zw;
    let w = m * vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, d, 1.0);
    return w.xyz / w.w;
}

@compute @workgroup_size(8, 8, 1)
fn temporal(@builtin(global_invocation_id) id: vec3<u32>) {
    let res = vec2<i32>(gi.res.xy);
    let px = vec2<i32>(id.xy);
    if (px.x >= res.x || px.y >= res.y) {
        return;
    }
    let d = textureLoad(gb_depth, px, 0);
    let nuevo_d = textureLoad(dif_nuevo, px, 0);
    let nuevo_s = textureLoad(spec_nuevo, px, 0);
    if (d >= 1.0) {
        textureStore(dif_out, px, vec4<f32>(0.0));
        textureStore(spec_out, px, vec4<f32>(0.0));
        return;
    }
    let gb = textureLoad(gb_normal, px, 0);
    let n = gb.xyz;
    let pos = mundo_con(gi.inv_view_proj, vec2<f32>(px) + 0.5, d);
    let dist = length(pos - gi.cam_pos.xyz);
    var h_d = vec4<f32>(0.0);
    var h_s = vec3<f32>(0.0);
    var suma = 0.0;
    if (gi.params.z > 0.5) {
        let c = gi.prev_view_proj * vec4<f32>(pos, 1.0);
        if (c.w > 1e-4) {
            let ndc = c.xy / c.w;
            let pp = (ndc * vec2<f32>(0.5, -0.5) + 0.5) * gi.res.xy - 0.5;
            let base = vec2<i32>(floor(pp));
            let f = pp - floor(pp);
            let tol = 0.04 + dist * 0.015;
            for (var k = 0; k < 4; k += 1) {
                let o = vec2<i32>(k & 1, k >> 1);
                let q = base + o;
                if (any(q < vec2<i32>(0)) || any(q >= res)) {
                    continue;
                }
                let dp = textureLoad(gb_depth_prev, q, 0);
                if (dp >= 1.0) {
                    continue;
                }
                let wp = mundo_con(gi.prev_inv_view_proj, vec2<f32>(q) + 0.5, dp);
                let np = textureLoad(gb_normal_prev, q, 0).xyz;
                if (abs(dot(wp - pos, n)) > tol || dot(np, n) < 0.8) {
                    continue;
                }
                let wb = select(1.0 - f.x, f.x, o.x == 1) * select(1.0 - f.y, f.y, o.y == 1);
                h_d += textureLoad(dif_hist, q, 0) * wb;
                h_s += textureLoad(spec_hist, q, 0).rgb * wb;
                suma += wb;
            }
        }
    }
    var len = 0.0;
    if (suma > 0.05) {
        h_d /= suma;
        h_s /= suma;
        len = h_d.a;
    }
    let len_d = min(len + 1.0, MAX_DIF);
    // Los reflejos dependen de la vista: menos historial cuanto más pulida.
    let max_s = mix(4.0, MAX_DIF, clamp(gb.a / 0.75, 0.0, 1.0));
    let len_s = min(len + 1.0, max_s);
    let dif = mix(h_d.rgb, nuevo_d.rgb, 1.0 / len_d);
    let spec = mix(h_s, nuevo_s.rgb, 1.0 / len_s);
    textureStore(dif_out, px, vec4<f32>(dif, len_d));
    textureStore(spec_out, px, vec4<f32>(spec, 1.0));
}
