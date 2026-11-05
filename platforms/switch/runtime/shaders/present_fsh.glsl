#version 460
// Port of crates/cmvs-wgpu/src/present.wgsl without the desktop preview background:
// rect is the viewport (left, top, width, height), source is (width, height, linear, 0).
layout(std140, binding = 0) uniform Params { vec4 rect; vec4 source; } p;
layout(binding = 0) uniform sampler2D image;
layout(location = 0) out vec4 outColor;

vec4 texel(ivec2 xy) {
    return texelFetch(image, clamp(xy, ivec2(0), textureSize(image, 0) - 1), 0);
}

void main() {
    vec2 q = gl_FragCoord.xy - p.rect.xy;
    if (any(lessThan(q, vec2(0.0))) || any(greaterThanEqual(q, p.rect.zw))) {
        outColor = vec4(0.0, 0.0, 0.0, 1.0);
        return;
    }
    vec4 c;
    // Integer-aligned 1:1 presentation has zero bilinear weights, even in linear mode.
    if (all(equal(p.source.xy, p.rect.zw)) && all(equal(p.rect.xy, floor(p.rect.xy)))) {
        c = texel(ivec2(floor(q)));
        c.rgb *= c.a;
    } else if (p.source.z > 0.0) {
        vec2 coord = floor(q * p.source.xy / p.rect.zw * 256.0) - 128.0;
        vec2 uv = clamp(coord, vec2(0.0), (p.source.xy - 1.0) * 256.0) / 256.0;
        ivec2 lo = ivec2(floor(uv));
        vec2 f = fract(uv);
        vec4 taps[4] = vec4[4](texel(lo), texel(lo + ivec2(1, 0)), texel(lo + ivec2(0, 1)), texel(lo + ivec2(1, 1)));
        for (int i = 0; i < 4; i++) {
            taps[i] = vec4(taps[i].rgb * taps[i].a, taps[i].a);
        }
        c = mix(mix(taps[0], taps[1], f.x), mix(taps[2], taps[3], f.x), f.y);
    } else {
        c = texel(ivec2(floor(floor(q) * p.source.xy / p.rect.zw)));
        c.rgb *= c.a;
    }
    outColor = vec4(c.rgb, 1.0);
}
