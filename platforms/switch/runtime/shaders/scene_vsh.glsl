#version 460
// Port of crates/cmvs-wgpu/src/scene.wgsl; the layout is cmvs_core::scene_render::shader.
layout(std140, binding = 0) uniform Params { vec4 v[70]; } p;
layout(location = 0) out vec2 outUv;

void main() {
    vec2 corners[4] = vec2[4](p.v[2].xy, p.v[2].zw, p.v[3].xy, p.v[3].zw);
    vec2 xy = corners[gl_VertexID];
    vec2 t = vec2(float(gl_VertexID & 1), float(gl_VertexID >> 1));
    gl_Position = vec4(xy.x / p.v[0].x * 2.0 - 1.0, 1.0 - xy.y / p.v[0].y * 2.0, 0.0, 1.0);
    outUv = p.v[1].xy + t * p.v[1].zw;
}
