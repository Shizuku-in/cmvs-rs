struct Params {
    rect: vec4<i32>,
    source: vec4<i32>, // width, height, filter, checkerboard
    intermediate: vec4<i32>, // visible origin, vertical-first, anti-ringing
}
@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var image: texture_2d<f32>;
@group(0) @binding(2) var original: texture_2d<f32>;

// Filter selectors packed by Scaling::draw.
const BILINEAR = 1;
const KEYS = 2;
const LANCZOS4 = 3;
const MITCHELL = 4;
const LANCZOS3 = 5;

@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let xy = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(xy * 2.0 - 1.0, 0.0, 1.0);
}

fn texel(texture: texture_2d<f32>, xy: vec2<i32>) -> vec4<f32> {
    return textureLoad(texture, clamp(xy, vec2<i32>(0), vec2<i32>(textureDimensions(texture)) - 1), 0);
}

fn premultiplied(color: vec4<f32>) -> vec4<f32> {
    return vec4<f32>(color.rgb * color.a, color.a);
}

fn source_sample(xy: vec2<i32>, straight_alpha: bool) -> vec4<f32> {
    let color = texel(image, xy);
    return select(color, premultiplied(color), straight_alpha);
}

fn keys(distance: f32) -> f32 {
    let x = abs(distance);
    if x < 1.0 {
        return 1.0 + x * x * (x - 2.0);
    }
    return (1.0 - x) * (x - 2.0) * (x - 2.0);
}

fn mitchell(distance: f32) -> f32 {
    let x = abs(distance);
    if x < 1.0 {
        return (16.0 + x * x * (21.0 * x - 36.0)) / 18.0;
    }
    return (8.0 - 7.0 * x) * (2.0 - x) * (2.0 - x) / 18.0;
}

fn lanczos(distance: f32, offset: i32, radius: i32) -> f32 {
    // sin(pi * distance) differs from sin(pi * fraction) only in sign.
    // The common magnitude cancels when the axis is normalized.
    let sign = select(1.0, -1.0, (offset & 1) != 0);
    let window = (3.141592653589793 / f32(radius)) * distance;
    return sign * sin(window) / (distance * distance);
}

struct SamplePosition {
    low: vec2<i32>,
    fraction: vec2<f32>,
}

fn sample_position(pixel: vec2<i32>) -> SamplePosition {
    let size = p.source.xy;
    let span = p.rect.zw;
    // Express the coordinate relative to the view center. In a cropped view
    // this bounds the product by the visible size times the source size.
    let centered = 2 * (pixel - p.rect.xy) + 1 - span;
    let numerator = centered * size + (1 - (size & vec2<i32>(1))) * span;
    let divisor = 2 * span;
    // Euclidean division keeps exact texel centers and the clamp neighborhood
    // in sync with the CPU even when the scale ratio is inexact in f32.
    let quotient = (numerator - select(vec2<i32>(0), divisor - 1, numerator < vec2<i32>(0))) / divisor;
    let remainder = numerator - quotient * divisor;
    return SamplePosition((size - 1) / 2 + quotient, vec2<f32>(remainder) / vec2<f32>(divisor));
}

fn filter_axis(base: vec2<i32>, axis: vec2<i32>, position: SamplePosition, straight_alpha: bool) -> vec4<f32> {
    let vertical = axis.y != 0;
    let center = select(position.low.x, position.low.y, vertical);
    let fraction = select(position.fraction.x, position.fraction.y, vertical);
    let same_size = select(p.source.x == p.rect.z, p.source.y == p.rect.w, vertical);
    let kernel = p.source.z;
    // Mitchell blends its neighbors at integral coordinates on a resized axis.
    if same_size || (kernel != MITCHELL && (fraction == 0.0 || fraction == 1.0)) {
        return source_sample(base + axis * (center + select(0, 1, fraction == 1.0)), straight_alpha);
    }
    var radius = 2;
    switch kernel {
        case LANCZOS3: { radius = 3; }
        case LANCZOS4: { radius = 4; }
        default: {}
    }
    var color = vec4<f32>(0.0);
    var total = 0.0;
    for (var offset = 1 - radius; offset <= radius; offset += 1) {
        let distance = fraction - f32(offset);
        var weight: f32;
        switch kernel {
            case KEYS: { weight = keys(distance); }
            case MITCHELL: { weight = mitchell(distance); }
            default: { weight = lanczos(distance, offset, radius); }
        }
        let sample = source_sample(base + axis * (center + offset), straight_alpha);
        color += sample * weight;
        total += weight;
    }
    return color / total;
}

fn clamp_ringing(color: vec4<f32>, position: SamplePosition) -> vec4<f32> {
    // Match the CPU's adjacent premultiplied texel range, after both passes.
    let low = position.low;
    let high = low + select(vec2<i32>(0), vec2<i32>(1), position.fraction != vec2<f32>(0.0));
    let a = premultiplied(texel(original, low));
    let b = premultiplied(texel(original, vec2<i32>(high.x, low.y)));
    let c = premultiplied(texel(original, vec2<i32>(low.x, high.y)));
    let d = premultiplied(texel(original, high));
    return clamp(color, min(min(a, b), min(c, d)), max(max(a, b), max(c, d)));
}

fn first_axis() -> vec2<i32> {
    return select(vec2<i32>(1, 0), vec2<i32>(0, 1), p.intermediate.z != 0);
}

@fragment fn fs_intermediate(@builtin(position) at: vec4<f32>) -> @location(0) vec4<f32> {
    let axis = first_axis();
    let pixel = vec2<i32>(at.xy);
    let base = pixel * (vec2<i32>(1) - axis);
    return filter_axis(base, axis, sample_position(pixel + p.intermediate.xy), true);
}

@fragment fn fs(@builtin(position) at: vec4<f32>) -> @location(0) vec4<f32> {
    let pixel = vec2<i32>(at.xy);
    var background = vec3<f32>(0.0);
    if p.source.w != 0 {
        let checker = ((pixel.x / 32 + pixel.y / 32) & 1) != 0;
        background = select(vec3<f32>(38.0, 52.0, 71.0), vec3<f32>(44.0, 60.0, 80.0), checker) / 255.0;
    }
    let q = pixel - p.rect.xy;
    if any(q < vec2<i32>(0)) || any(q >= p.rect.zw) {
        return vec4<f32>(background, 1.0);
    }
    var c: vec4<f32>;
    // Integer-aligned 1:1 presentation always samples exact texels.
    if all(p.source.xy == p.rect.zw) {
        c = source_sample(q, true);
    } else if p.source.z >= KEYS {
        let first = first_axis();
        let axis = vec2<i32>(1) - first;
        let base = (pixel - p.intermediate.xy) * first;
        let position = sample_position(pixel);
        var color = filter_axis(base, axis, position, false);
        if p.intermediate.w != 0 {
            color = clamp_ringing(color, position);
        }
        let alpha = clamp(color.a, 0.0, 1.0);
        c = vec4<f32>(clamp(color.rgb, vec3<f32>(0.0), vec3<f32>(alpha)), alpha);
    } else if p.source.z == BILINEAR {
        let size = vec2<f32>(p.source.xy);
        let coord = floor((vec2<f32>(q) + 0.5) * size / vec2<f32>(p.rect.zw) * 256.0) - 128.0;
        let uv = clamp(coord, vec2<f32>(0.0), (size - 1.0) * 256.0) / 256.0;
        let low = vec2<i32>(floor(uv));
        let f = fract(uv);
        let a = source_sample(low, true);
        let b = source_sample(low + vec2<i32>(1, 0), true);
        let bottom_left = source_sample(low + vec2<i32>(0, 1), true);
        let bottom_right = source_sample(low + vec2<i32>(1, 1), true);
        c = mix(mix(a, b, f.x), mix(bottom_left, bottom_right, f.x), f.y);
    } else {
        let coord = vec2<f32>(q) * vec2<f32>(p.source.xy) / vec2<f32>(p.rect.zw);
        c = source_sample(vec2<i32>(floor(coord)), true);
    }
    return vec4<f32>(c.rgb + background * (1.0 - c.a), 1.0);
}
