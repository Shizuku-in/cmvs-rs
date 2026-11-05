#version 460
// Port of crates/cmvs-wgpu/src/scene.wgsl; the layout is cmvs_core::scene_render::shader.
layout(std140, binding = 0) uniform Params { vec4 v[70]; } p;
layout(binding = 0) uniform sampler2D image;
layout(binding = 1) uniform sampler2D mask;
layout(binding = 2) uniform sampler2D screen;
layout(location = 0) in vec2 inUv;
layout(location = 0) out vec4 outColor;

// GPU division is reciprocal-based; exact byte quotients must not round down.
float div255(float x) { return floor(x / 255.0 + 0.001); }

vec4 load(sampler2D t, ivec2 xy) {
    ivec2 size = textureSize(t, 0);
    ivec2 at = clamp(xy, ivec2(0), size - 1);
    if ((uint(p.v[4].w) & 16u) != 0u) {
        // Maxwell has no integer division. The half-texel offset keeps the float
        // quotient away from integers, so floor() is exact for any texture size.
        vec2 s = vec2(size);
        at = clamp(ivec2(vec2(xy) - s * floor((vec2(xy) + 0.5) / s)), ivec2(0), size - 1);
    }
    return roundEven(texelFetch(t, at, 0) * 255.0);
}

vec4 sampleLinear(sampler2D t, vec2 uv) {
    vec2 q = uv - 0.5;
    vec2 f = fract(q);
    ivec2 xy = ivec2(floor(q));
    return roundEven(mix(mix(load(t, xy), load(t, xy + ivec2(1, 0)), f.x),
                         mix(load(t, xy + ivec2(0, 1)), load(t, xy + ivec2(1, 1)), f.x), f.y));
}

void main() {
    uint flags = uint(p.v[4].w);
    vec4 texel = vec4(p.v[4].xyz, 255.0);
    if ((flags & 1u) == 0u) {
        texel = p.v[0].w > 0.0 ? sampleLinear(image, inUv) : load(image, ivec2(floor(inUv)));
    }
    float alpha = div255(texel.a * p.v[0].z + 127.0);
    if ((flags & 4u) != 0u) {
        uint green = uint(load(mask, ivec2(floor(inUv))).g);
        alpha = roundEven(alpha * p.v[6u + green / 4u][green % 4u]);
    }
    if ((flags & 8u) != 0u) {
        vec2 uv = (gl_FragCoord.xy - 0.5) * vec2(textureSize(screen, 0)) / p.v[0].xy;
        alpha = div255(alpha * sampleLinear(screen, uv).g + 127.0);
    }
    if ((flags & 2u) != 0u) {
        texel.rgb = vec3(0.0);
    }
    outColor = vec4(texel.rgb / 255.0, alpha / 255.0);
}
