// Efecto de posprocesado de Maker Studio.
//
// Parámetros (el Inspector los muestra como deslizadores y selectores de color):
//   // @param nombre float defecto [mín máx]
//   // @param nombre color r g b
// Se leen con una función del mismo nombre: fuerza() -> f32, tinte() -> vec3<f32> (lineal).
//
// @param fuerza float 0.5 0 1
// @param tinte color 1 0.8 0.6
//
// Disponibles: muestra(uv), entrada, extra (la escena sin posprocesar), profundidad(uv),
// distancia(uv), posicion_mundo(uv), normal_mundo(uv), es_cielo(uv), luma(c), a_srgb(c),
// a_lineal(c), ruido(p) y el uniforme `post` (tiempo, intensidad, resolucion, texel, escala).
// Guía completa: docs/SHADERS.md, «Efectos propios».

@fragment
fn efecto(in: PostIn) -> @location(0) vec4<f32> {
    let c = muestra(in.uv).rgb;
    let r = mix(c, c * tinte(), fuerza());
    return vec4<f32>(mix(c, r, post.intensidad), 1.0);
}
