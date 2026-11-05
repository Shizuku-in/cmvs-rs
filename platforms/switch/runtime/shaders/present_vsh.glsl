#version 460
// One triangle covering the target.
void main() {
    vec2 xy = vec2(float((gl_VertexID << 1) & 2), float(gl_VertexID & 2));
    gl_Position = vec4(xy * 2.0 - 1.0, 0.0, 1.0);
}
