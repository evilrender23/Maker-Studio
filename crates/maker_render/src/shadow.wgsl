// Pasada de profundidad del mapa de sombras (luz direccional).

struct Object {
    model: mat4x4<f32>,
    normal_m: mat4x4<f32>,
    base_color: vec4<f32>,
    emissive: vec4<f32>,
    params: vec4<f32>,
};
@group(1) @binding(0) var<uniform> obj: Object;

struct VIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
};

struct ShadowGlobals {
    light_vp: mat4x4<f32>,
};
@group(0) @binding(0) var<uniform> sg: ShadowGlobals;

@vertex
fn vs_shadow(v: VIn) -> @builtin(position) vec4<f32> {
    return sg.light_vp * obj.model * vec4<f32>(v.pos, 1.0);
}

