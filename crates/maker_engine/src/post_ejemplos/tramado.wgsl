// Tramado ordenado (Bayer 4×4) con paleta de dos colores, estilo Game Boy.
//
// @param niveles float 4 2 16
// @param pixel float 3 1 8
// @param oscuro color 0.06 0.22 0.06
// @param claro color 0.61 0.74 0.06
// @param paleta float 1 0 1

fn bayer(p: vec2<u32>) -> f32 {
    var m = array<f32, 16>(0.0, 8.0, 2.0, 10.0, 12.0, 4.0, 14.0, 6.0, 3.0, 11.0, 1.0, 9.0, 15.0, 7.0, 13.0, 5.0);
    return (m[(p.y % 4u) * 4u + p.x % 4u] + 0.5) / 16.0;
}

@fragment
fn efecto(in: PostIn) -> @location(0) vec4<f32> {
    let original = muestra(in.uv).rgb;
    let tam = max(pixel() * post.escala, 1.0);
    let celda = floor(in.pos.xy / tam);
    let c = a_srgb(muestra((celda + 0.5) * tam * post.resolucion.zw).rgb);
    let n = max(niveles() - 1.0, 1.0);
    let umbral = bayer(vec2<u32>(celda)) - 0.5;
    // Color posterizado con tramado…
    let q = floor(c * n + 0.5 + umbral) / n;
    // …o luminancia tramada sobre la paleta de dos colores.
    let l = clamp(floor(luma(c) * n + 0.5 + umbral) / n, 0.0, 1.0);
    let pal = mix(a_srgb(oscuro()), a_srgb(claro()), l);
    let r = a_lineal(clamp(mix(q, pal, paleta()), vec3<f32>(0.0), vec3<f32>(1.0)));
    return vec4<f32>(mix(original, r, post.intensidad), 1.0);
}
