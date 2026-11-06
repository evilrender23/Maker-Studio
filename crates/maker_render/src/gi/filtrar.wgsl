// Filtro espacial à-trous (5×5, B3-spline) con paradas por geometría: solo
// mezcla vecinos en el mismo plano y con normal parecida. Dos pasadas con
// separación 1 y 2. Menos filtrado donde el historial ya es largo; los
// reflejos pulidos casi no se filtran.

@group(1) @binding(0) var dif_in: texture_2d<f32>;
@group(1) @binding(1) var spec_in: texture_2d<f32>;
@group(1) @binding(2) var gb_normal: texture_2d<f32>;
@group(1) @binding(3) var gb_depth: texture_depth_2d;
@group(1) @binding(4) var dif_out: texture_storage_2d<rgba16float, write>;
@group(1) @binding(5) var spec_out: texture_storage_2d<rgba16float, write>;

override PASO: i32 = 1;

fn mundo(px: vec2<f32>, d: f32) -> vec3<f32> {
    let uv = px * gi.res.zw;
    let w = gi.inv_view_proj * vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, d, 1.0);
    return w.xyz / w.w;
}

@compute @workgroup_size(8, 8, 1)
fn filtrar(@builtin(global_invocation_id) id: vec3<u32>) {
    let res = vec2<i32>(gi.res.xy);
    let px = vec2<i32>(id.xy);
    if (px.x >= res.x || px.y >= res.y) {
        return;
    }
    let d = textureLoad(gb_depth, px, 0);
    let c_d = textureLoad(dif_in, px, 0);
    let c_s = textureLoad(spec_in, px, 0);
    if (d >= 1.0) {
        textureStore(dif_out, px, c_d);
        textureStore(spec_out, px, c_s);
        return;
    }
    let gb = textureLoad(gb_normal, px, 0);
    let n = gb.xyz;
    let pos = mundo(vec2<f32>(px) + 0.5, d);
    let tol = 0.03 + length(pos - gi.cam_pos.xyz) * 0.01;
    // Con historial largo el ruido ya bajó: núcleo más estrecho en valor.
    let hist = clamp(c_d.a / 24.0, 0.0, 1.0);
    let k = array<f32, 3>(0.375, 0.25, 0.0625);
    var sd = vec3<f32>(0.0);
    var ss = vec3<f32>(0.0);
    var wd = 0.0;
    var ws = 0.0;
    let filtra_spec = gb.a > 0.25;
    for (var y = -2; y <= 2; y += 1) {
        for (var x = -2; x <= 2; x += 1) {
            let q = px + vec2<i32>(x, y) * PASO;
            if (any(q < vec2<i32>(0)) || any(q >= res)) {
                continue;
            }
            let dq = textureLoad(gb_depth, q, 0);
            if (dq >= 1.0) {
                continue;
            }
            let wq = mundo(vec2<f32>(q) + 0.5, dq);
            let nq = textureLoad(gb_normal, q, 0).xyz;
            let plano = exp(-abs(dot(wq - pos, n)) / tol);
            let w = k[abs(x)] * k[abs(y)] * plano * pow(max(dot(nq, n), 0.0), 32.0);
            let vd = textureLoad(dif_in, q, 0).rgb;
            // Parada por luminancia, más estricta con historial largo.
            let wl = exp(-abs(luma(vd) - luma(c_d.rgb)) / (0.15 + 4.0 * (1.0 - hist)) );
            sd += vd * w * wl;
            wd += w * wl;
            if (filtra_spec) {
                ss += textureLoad(spec_in, q, 0).rgb * w;
                ws += w;
            }
        }
    }
    let dif = sd / max(wd, 1e-5);
    let spec = select(c_s.rgb, ss / max(ws, 1e-5), filtra_spec && ws > 1e-5);
    textureStore(dif_out, px, vec4<f32>(dif, c_d.a));
    textureStore(spec_out, px, vec4<f32>(spec, 1.0));
}
