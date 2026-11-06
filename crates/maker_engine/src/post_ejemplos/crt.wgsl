// CRT: pantalla curva, líneas de barrido y máscara de fósforo.
//
// @param curvatura float 0.15 0 0.5
// @param lineas float 0.35 0 1
// @param mascara float 0.25 0 1
// @param brillo float 1.2 0.5 2

@fragment
fn efecto(in: PostIn) -> @location(0) vec4<f32> {
    let original = muestra(in.uv).rgb;
    // Curvatura del tubo (barril).
    var p = in.uv * 2.0 - 1.0;
    p = p * (1.0 + curvatura() * dot(p, p) * 0.5);
    let uv = p * 0.5 + 0.5;
    if (any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0))) {
        return vec4<f32>(mix(original, vec3<f32>(0.0), post.intensidad), 1.0);
    }
    var c = muestra(uv).rgb;
    // Una línea de barrido cada 2 px de 1080 p (se escala con la resolución).
    let y = uv.y * post.resolucion.y / max(post.escala, 1.0);
    c *= 1.0 - lineas() * (0.5 + 0.5 * cos(y * 3.14159265));
    // Tríada RGB de 3 px.
    let k = u32(in.pos.x / max(post.escala, 1.0)) % 3u;
    var tri = vec3<f32>(0.0);
    tri[k] = 1.0;
    c *= mix(vec3<f32>(1.0), tri * 2.0 + 0.3, mascara());
    // Esquinas oscurecidas.
    let v = smoothstep(1.0, 0.7, max(abs(p.x), abs(p.y)));
    c *= brillo() * mix(0.6, 1.0, v);
    return vec4<f32>(mix(original, c, post.intensidad), 1.0);
}
