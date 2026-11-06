// Volumen de vóxeles: resolver lo voxelizado de una cascada en albedo+opacidad,
// normal y emisión (texturas 3D persistentes: solo se rehacen cuando algo
// cambia en esa cascada).

struct Vox {
    cascada: vec4<u32>,
};

@group(0) @binding(2) var<uniform> vox: Vox;
@group(0) @binding(3) var<storage, read> acum: array<u32>;

@group(1) @binding(0) var albedo_out: texture_storage_3d<rgba8unorm, write>;
@group(1) @binding(1) var normal_out: texture_storage_3d<rgba8snorm, write>;
@group(1) @binding(2) var emision_out: texture_storage_3d<rgba16float, write>;


@compute @workgroup_size(4, 4, 4)
fn resolver(@builtin(global_invocation_id) id: vec3<u32>) {
    let ni = gi.vol.x;
    if (any(id >= vec3<u32>(ni))) {
        return;
    }
    let idx = ((id.z * ni + id.y) * ni + id.x) * 13u;
    let dst = vec3<i32>(id) + vec3<i32>(0, 0, i32(vox.cascada.x * ni));
    let cuenta = acum[idx + 3u];
    if (cuenta == 0u) {
        textureStore(albedo_out, dst, vec4<f32>(0.0));
        textureStore(normal_out, dst, vec4<f32>(0.0));
        textureStore(emision_out, dst, vec4<f32>(0.0));
        return;
    }
    let k = 1.0 / f32(cuenta);
    let albedo = vec3<f32>(f32(acum[idx]), f32(acum[idx + 1u]), f32(acum[idx + 2u])) * k / 255.0;
    let ns = vec3<f32>(f32(bitcast<i32>(acum[idx + 4u])), f32(bitcast<i32>(acum[idx + 5u])), f32(bitcast<i32>(acum[idx + 6u]))) * k / 127.0;
    // Caras opuestas en el mismo vóxel (pared más fina que el vóxel): la
    // normal media se anula. Se guarda el eje sin signo (w = 1) y la
    // inyección lo orienta hacia la cámara cada fotograma, para que el lado
    // de dentro de una habitación no reciba la luz del de fuera.
    let l = length(ns);
    let eje = vec3<f32>(f32(acum[idx + 10u]), f32(acum[idx + 11u]), f32(acum[idx + 12u]));
    var n = vec4<f32>(ns / max(l, 1e-4), 0.0);
    if (l <= 0.35) {
        n = vec4<f32>(eje / max(length(eje), 1e-4), 1.0);
    }
    let e = vec3<f32>(f32(acum[idx + 7u]), f32(acum[idx + 8u]), f32(acum[idx + 9u])) * k / 64.0;
    textureStore(albedo_out, dst, vec4<f32>(albedo, 1.0));
    textureStore(normal_out, dst, n);
    textureStore(emision_out, dst, vec4<f32>(e, 0.0));
}

