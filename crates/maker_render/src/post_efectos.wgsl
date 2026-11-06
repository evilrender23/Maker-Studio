// ───────────── Efectos de posprocesado integrados ─────────────
// Se compilan junto con `post_preludio.wgsl`. Cada efecto es un entry point
// de fragmento; sus parámetros llegan en `post.p[i]` (ver postproceso.rs).

// Curvas de tonemap: las mismas que `shaders.wgsl` hace en la escena cuando
// el tonemap se fusiona con ella.
fn aces(x: vec3<f32>) -> vec3<f32> {
    let a = 2.51;
    let b = 0.03;
    let c = 2.43;
    let d = 0.59;
    let e = 0.14;
    return clamp((x * (a * x + b)) / (x * (c * x + d) + e), vec3<f32>(0.0), vec3<f32>(1.0));
}

fn agx_contraste(x: vec3<f32>) -> vec3<f32> {
    let x2 = x * x;
    let x4 = x2 * x2;
    return 15.5 * x4 * x2 - 40.14 * x4 * x + 31.96 * x4 - 6.868 * x2 * x + 0.4298 * x2 + 0.1191 * x - 0.00232;
}

// AgX (aproximación polinómica de Benjamin Wrensch), sRGB lineal → lineal.
fn agx(c: vec3<f32>) -> vec3<f32> {
    let entra = mat3x3<f32>(
        vec3<f32>(0.842479062253094, 0.0423282422610123, 0.0423756549057051),
        vec3<f32>(0.0784335999999992, 0.878468636469772, 0.0784336),
        vec3<f32>(0.0792237451477643, 0.0791661274605434, 0.879142973793104),
    );
    let sale = mat3x3<f32>(
        vec3<f32>(1.19687900512017, -0.0528968517574562, -0.0529716355144438),
        vec3<f32>(-0.0980208811401368, 1.15190312990417, -0.0980434501171241),
        vec3<f32>(-0.0990297440797205, -0.0989611768448433, 1.15107367264116),
    );
    let min_ev = -12.47393;
    let max_ev = 4.026069;
    var v = entra * max(c, vec3<f32>(1e-10));
    v = clamp(log2(v), vec3<f32>(min_ev), vec3<f32>(max_ev));
    v = (v - min_ev) / (max_ev - min_ev);
    v = sale * agx_contraste(v);
    return clamp(pow(max(v, vec3<f32>(0.0)), vec3<f32>(2.2)), vec3<f32>(0.0), vec3<f32>(1.0));
}

fn tonemap_op(c: vec3<f32>, op: u32) -> vec3<f32> {
    switch op {
        case 0u: { return aces(c); }
        case 1u: { return agx(c); }
        case 2u: { return clamp(c, vec3<f32>(0.0), vec3<f32>(1.0)); }
        default: { return c; }
    }
}

// p0: operador, multiplicador de exposición.
@fragment
fn fs_tonemap(in: PostIn) -> @location(0) vec4<f32> {
    let c = muestra(in.uv).rgb * post.p[0].y;
    return vec4<f32>(tonemap_op(c, u32(post.p[0].x)), 1.0);
}

// LUT en tira N²×N (sRGB) en `extra`.
fn aplicar_lut(c: vec3<f32>) -> vec3<f32> {
    let dim = vec2<f32>(textureDimensions(extra));
    let n = dim.y;
    let b = clamp(c.b, 0.0, 1.0) * (n - 1.0);
    let b0 = floor(b);
    let b1 = min(b0 + 1.0, n - 1.0);
    let xy = clamp(c.rg, vec2<f32>(0.0), vec2<f32>(1.0)) * (n - 1.0) + 0.5;
    let uv0 = vec2<f32>((b0 * n + xy.x) / dim.x, xy.y / dim.y);
    let uv1 = vec2<f32>((b1 * n + xy.x) / dim.x, xy.y / dim.y);
    return mix(textureSampleLevel(extra, muestreo, uv0, 0.0).rgb, textureSampleLevel(extra, muestreo, uv1, 0.0).rgb, b - b0);
}

// p0: contraste, saturación, temperatura, matiz; p1: lift (w: hay LUT); p2: gamma; p3: gain.
@fragment
fn fs_grading(in: PostIn) -> @location(0) vec4<f32> {
    let src = muestra(in.uv).rgb;
    let t = post.p[0].z;
    let m = post.p[0].w;
    var c = src * vec3<f32>(1.0 + 0.1 * t, 1.0 - 0.1 * m, 1.0 - 0.1 * t);
    var s = a_srgb(clamp(c, vec3<f32>(0.0), vec3<f32>(1.0)));
    s = (s - 0.5) * post.p[0].x + 0.5;
    s = s * post.p[3].rgb + post.p[1].rgb * (1.0 - s);
    s = pow(max(s, vec3<f32>(0.0)), 1.0 / max(post.p[2].rgb, vec3<f32>(0.01)));
    s = mix(vec3<f32>(luma(s)), s, post.p[0].y);
    s = clamp(s, vec3<f32>(0.0), vec3<f32>(1.0));
    if (post.p[1].w > 0.5) {
        s = aplicar_lut(s);
    }
    c = a_lineal(s);
    return vec4<f32>(mix(src, c, post.intensidad), 1.0);
}

// ─── Bloom: umbral + cadena de reducción y ampliación (aditiva) ───

fn caja4(uv: vec2<f32>) -> vec3<f32> {
    let t = post.texel.xy;
    return (muestra(uv + vec2<f32>(-t.x, -t.y)).rgb + muestra(uv + vec2<f32>(t.x, -t.y)).rgb
          + muestra(uv + vec2<f32>(-t.x, t.y)).rgb + muestra(uv + vec2<f32>(t.x, t.y)).rgb) * 0.25;
}

// p0: umbral.
@fragment
fn fs_bloom_umbral(in: PostIn) -> @location(0) vec4<f32> {
    let c = min(caja4(in.uv), vec3<f32>(256.0));
    let th = post.p[0].x;
    let br = max(c.r, max(c.g, c.b));
    let rodilla = max(th * 0.5, 1e-4);
    var suave = clamp(br - th + rodilla, 0.0, 2.0 * rodilla);
    suave = suave * suave / (4.0 * rodilla + 1e-4);
    let k = max(suave, br - th) / max(br, 1e-4);
    return vec4<f32>(c * k, 1.0);
}

@fragment
fn fs_bloom_reduce(in: PostIn) -> @location(0) vec4<f32> {
    return vec4<f32>(caja4(in.uv), 1.0);
}

// Tienda 3×3; se suma (mezcla aditiva) al nivel de destino.
@fragment
fn fs_bloom_amplia(in: PostIn) -> @location(0) vec4<f32> {
    let t = post.texel.xy;
    let uv = in.uv;
    var s = muestra(uv).rgb * 4.0;
    s += (muestra(uv + vec2<f32>(t.x, 0.0)).rgb + muestra(uv - vec2<f32>(t.x, 0.0)).rgb + muestra(uv + vec2<f32>(0.0, t.y)).rgb + muestra(uv - vec2<f32>(0.0, t.y)).rgb) * 2.0;
    s += muestra(uv + t).rgb + muestra(uv - t).rgb + muestra(uv + vec2<f32>(t.x, -t.y)).rgb + muestra(uv + vec2<f32>(-t.x, t.y)).rgb;
    return vec4<f32>(s / 16.0, 1.0);
}

// `extra` = nivel 0 del bloom. p0.x: 1 / niveles.
@fragment
fn fs_bloom_mezcla(in: PostIn) -> @location(0) vec4<f32> {
    let b = textureSampleLevel(extra, muestreo, in.uv, 0.0).rgb * post.p[0].x;
    return vec4<f32>(muestra(in.uv).rgb + b * post.intensidad, 1.0);
}

// p0: radio, suavidad; p1: color.
@fragment
fn fs_vineta(in: PostIn) -> @location(0) vec4<f32> {
    let c = muestra(in.uv).rgb;
    let a = post.resolucion.x / post.resolucion.y;
    let d = length((in.uv - 0.5) * vec2<f32>(a, 1.0)) / length(vec2<f32>(a, 1.0) * 0.5);
    let r = post.p[0].x;
    let s = max(post.p[0].y, 1e-3);
    let f = smoothstep(r - s * 0.5, r + s * 0.5, d);
    return vec4<f32>(mix(c, post.p[1].rgb, f * post.intensidad), 1.0);
}

// p0.x: desplazamiento (px a 1080 p) en el borde.
@fragment
fn fs_aberracion(in: PostIn) -> @location(0) vec4<f32> {
    let dir = (in.uv - 0.5) * 2.0;
    let off = dir * post.p[0].x * post.escala * post.intensidad * post.resolucion.zw;
    let r = muestra(in.uv + off).r;
    let g = muestra(in.uv).g;
    let b = muestra(in.uv - off).b;
    return vec4<f32>(r, g, b, 1.0);
}

// p0.x: tamaño del grano (px).
@fragment
fn fs_grano(in: PostIn) -> @location(0) vec4<f32> {
    let c = muestra(in.uv).rgb;
    let celda = floor(in.pos.xy / max(post.p[0].x, 1.0));
    let n = ruido(celda + fract(post.tiempo * 7.31) * 517.0) - 0.5;
    let s = a_srgb(c) + n * 0.2 * post.intensidad * (1.0 - 0.5 * luma(a_srgb(c)));
    return vec4<f32>(a_lineal(s), 1.0);
}

// Umbral Bayer 4×4 en -0.5..0.5.
fn bayer4(p: vec2<u32>) -> f32 {
    let m = array<f32, 16>(0.0, 8.0, 2.0, 10.0, 12.0, 4.0, 14.0, 6.0, 3.0, 11.0, 1.0, 9.0, 15.0, 7.0, 13.0, 5.0);
    return (m[(p.y % 4u) * 4u + (p.x % 4u)] + 0.5) / 16.0 - 0.5;
}

// p0: tamaño del píxel, niveles (0 = sin posterizar), tramado.
@fragment
fn fs_retro(in: PostIn) -> @location(0) vec4<f32> {
    let src = muestra(in.uv).rgb;
    let tam = max(post.p[0].x, 1.0);
    let celda = floor(in.pos.xy / tam);
    let centro = (celda + 0.5) * tam * post.resolucion.zw;
    let tx = clamp(vec2<i32>(centro * post.texel.zw), vec2<i32>(0), vec2<i32>(post.texel.zw) - 1);
    var c = textureLoad(entrada, tx, 0).rgb;
    let niveles = post.p[0].y;
    if (niveles >= 2.0) {
        let n = niveles - 1.0;
        let t = bayer4(vec2<u32>(celda)) * post.p[0].z;
        c = a_lineal(clamp(floor(a_srgb(c) * n + 0.5 + t) / n, vec3<f32>(0.0), vec3<f32>(1.0)));
    }
    return vec4<f32>(mix(src, c, post.intensidad), 1.0);
}

// p0.x: radio (px).
@fragment
fn fs_enfoque(in: PostIn) -> @location(0) vec4<f32> {
    let t = post.texel.xy * max(post.p[0].x, 0.5);
    let c = muestra(in.uv).rgb;
    let vecinos = muestra(in.uv + vec2<f32>(t.x, 0.0)).rgb + muestra(in.uv - vec2<f32>(t.x, 0.0)).rgb
                + muestra(in.uv + vec2<f32>(0.0, t.y)).rgb + muestra(in.uv - vec2<f32>(0.0, t.y)).rgb;
    let k = post.intensidad * 0.5;
    return vec4<f32>(max(c * (1.0 + 4.0 * k) - vecinos * k, vec3<f32>(0.0)), 1.0);
}

fn luma_p(c: vec3<f32>) -> f32 {
    return luma(sqrt(max(c, vec3<f32>(0.0))));
}

// FXAA (variante rápida de Timothy Lottes).
@fragment
fn fs_fxaa(in: PostIn) -> @location(0) vec4<f32> {
    let t = post.texel.xy;
    let uv = in.uv;
    let m = muestra(uv).rgb;
    let nw = luma_p(muestra(uv + vec2<f32>(-t.x, -t.y)).rgb);
    let ne = luma_p(muestra(uv + vec2<f32>(t.x, -t.y)).rgb);
    let sw = luma_p(muestra(uv + vec2<f32>(-t.x, t.y)).rgb);
    let se = luma_p(muestra(uv + vec2<f32>(t.x, t.y)).rgb);
    let lm = luma_p(m);
    let lmin = min(lm, min(min(nw, ne), min(sw, se)));
    let lmax = max(lm, max(max(nw, ne), max(sw, se)));
    var dir = vec2<f32>(-((nw + ne) - (sw + se)), (nw + sw) - (ne + se));
    let reduce = max((nw + ne + sw + se) * 0.25 * (1.0 / 8.0), 1.0 / 128.0);
    let inv = 1.0 / (min(abs(dir.x), abs(dir.y)) + reduce);
    dir = clamp(dir * inv, vec2<f32>(-8.0), vec2<f32>(8.0)) * t;
    let a = 0.5 * (muestra(uv + dir * (1.0 / 3.0 - 0.5)).rgb + muestra(uv + dir * (2.0 / 3.0 - 0.5)).rgb);
    let b = a * 0.5 + 0.25 * (muestra(uv - dir * 0.5).rgb + muestra(uv + dir * 0.5).rgb);
    let lb = luma_p(b);
    var r = b;
    if (lb < lmin || lb > lmax) {
        r = a;
    }
    return vec4<f32>(mix(m, r, post.intensidad), 1.0);
}

fn dist_borde(uv: vec2<f32>) -> f32 {
    if (es_cielo(uv)) {
        return 1e6;
    }
    return distancia(uv);
}

// p0: grosor (px), umbral de distancia, umbral de normal; p1: color.
@fragment
fn fs_contorno(in: PostIn) -> @location(0) vec4<f32> {
    let c = muestra(in.uv).rgb;
    let px = max(post.p[0].x, 0.5) / vec2<f32>(textureDimensions(profundidad_tex));
    var o = array<vec2<f32>, 4>(vec2<f32>(px.x, 0.0), vec2<f32>(-px.x, 0.0), vec2<f32>(0.0, px.y), vec2<f32>(0.0, -px.y));
    let dc = dist_borde(in.uv);
    let cielo = dc > 1e5;
    let nc = normal_mundo(in.uv);
    var borde = 0.0;
    for (var i = 0; i < 4; i += 1) {
        let uv = in.uv + o[i];
        let d = dist_borde(uv);
        let cerca = min(d, dc);
        if (abs(d - dc) / max(cerca, 1e-3) > post.p[0].y) {
            // El borde se dibuja en el lado cercano (el objeto, no el fondo).
            borde = max(borde, select(0.0, 1.0, dc <= d));
        } else if (!cielo && post.p[0].z > 0.0) {
            let n = normal_mundo(uv);
            borde = max(borde, step(post.p[0].z, 1.0 - dot(nc, n)));
        }
    }
    return vec4<f32>(mix(c, post.p[1].rgb, borde * post.intensidad), 1.0);
}

// p0: densidad, altura, caída; p1: color (lineal).
@fragment
fn fs_niebla_altura(in: PostIn) -> @location(0) vec4<f32> {
    let c = muestra(in.uv).rgb;
    let cam = post.camara.xyz;
    let v = posicion_mundo(in.uv) - cam;
    let dist = length(v);
    let k = max(post.p[0].z, 1e-4);
    let base = post.p[0].x * exp(-k * (cam.y - post.p[0].y));
    let ky = k * v.y;
    var integral = 1.0;
    if (abs(ky) > 1e-4) {
        integral = (1.0 - exp(-ky)) / ky;
    }
    let f = 1.0 - exp(-max(base * dist * integral, 0.0));
    return vec4<f32>(mix(c, post.p[1].rgb, clamp(f, 0.0, 1.0) * post.intensidad), 1.0);
}

// Radio de desenfoque (px) de un punto. p0: foco, rango, máximo.
fn coc(uv: vec2<f32>) -> f32 {
    let d = distancia(uv);
    let fuera = (abs(d - post.p[0].x) - post.p[0].y * 0.5) / max(post.p[0].y, 1e-3);
    return clamp(fuera, 0.0, 1.0) * post.p[0].z * post.escala * post.intensidad;
}

@fragment
fn fs_profundidad_campo(in: PostIn) -> @location(0) vec4<f32> {
    let maxr = post.p[0].z * post.escala * post.intensidad;
    let cc = coc(in.uv);
    var suma = muestra(in.uv).rgb;
    var peso = 1.0;
    if (maxr < 0.5) {
        return vec4<f32>(suma, 1.0);
    }
    let n = 24;
    for (var i = 0; i < n; i += 1) {
        let f = f32(i) + 0.5;
        let r = sqrt(f / f32(n)) * maxr;
        let a = f * 2.39996323;
        let uv = in.uv + vec2<f32>(cos(a), sin(a)) * r * post.resolucion.zw;
        let cs = coc(uv);
        // La muestra cuenta si su propio desenfoque (o el de este píxel) llega hasta aquí.
        let w = clamp(max(cs, cc) - r + 1.0, 0.0, 1.0);
        suma += muestra(uv).rgb * w;
        peso += w;
    }
    return vec4<f32>(suma / peso, 1.0);
}
