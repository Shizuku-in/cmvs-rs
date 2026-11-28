struct Params { v: array<vec4<f32>, 70> }
@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var image: texture_2d<f32>;
@group(0) @binding(2) var mask: texture_2d<f32>;
@group(0) @binding(3) var screen: texture_2d<f32>;
struct Vertex { @builtin(position) position: vec4<f32>, @location(0) uv: vec2<f32> }
@vertex fn vs(@builtin(vertex_index) index: u32) -> Vertex {
    let uv = vec2<f32>(f32(index & 1u), f32(index >> 1u));
    var corners = array<vec2<f32>,4>(p.v[2].xy,p.v[2].zw,p.v[3].xy,p.v[3].zw);
    let xy = corners[index];
    return Vertex(vec4<f32>(xy.x / p.v[0].x * 2.0 - 1.0, 1.0 - xy.y / p.v[0].y * 2.0,0.0,1.0), p.v[1].xy + uv*p.v[1].zw);
}
fn load(t: texture_2d<f32>, xy: vec2<i32>) -> vec4<f32> {
    let size=vec2<i32>(textureDimensions(t));
    var at=clamp(xy,vec2<i32>(0),size-1);
    if ((u32(p.v[4].w) & 16u)!=0u) { at=((xy % size)+size) % size; }
    return round(textureLoad(t,at,0)*255.0);
}
fn sample_linear(t: texture_2d<f32>, uv: vec2<f32>) -> vec4<f32> {
    let q=uv-0.5; let f=fract(q); let xy=vec2<i32>(floor(q));
    return round(mix(mix(load(t,xy),load(t,xy+vec2<i32>(1,0)),f.x),mix(load(t,xy+vec2<i32>(0,1)),load(t,xy+vec2<i32>(1,1)),f.x),f.y));
}
fn load_premul(t: texture_2d<f32>, xy: vec2<i32>) -> vec4<f32> {
    let c = load(t, xy);
    return vec4<f32>(c.rgb * (c.a / 255.0), c.a);
}
fn load_rect_premul(t: texture_2d<f32>, xy: vec2<i32>, lo: vec2<i32>, hi: vec2<i32>) -> vec4<f32> {
    let c = round(textureLoad(t, clamp(xy, lo, hi), 0) * 255.0);
    return vec4<f32>(c.rgb * (c.a / 255.0), c.a);
}
fn catmull_rom(d: f32) -> f32 {
    let x = abs(d);
    if (x < 1.0) {
        return 1.0 + x * x * (1.5 * x - 2.5);
    }
    if (x < 2.0) {
        let t = 2.0 - x;
        return -0.5 * (x - 1.0) * t * t;
    }
    return 0.0;
}
fn sample_bicubic_ar_premul(t: texture_2d<f32>, uv: vec2<f32>, clamp_rect: bool) -> vec4<f32> {
    let size = vec2<i32>(textureDimensions(t));
    var lo = vec2<i32>(0);
    var hi = size - 1;
    if (clamp_rect) {
        lo = clamp(vec2<i32>(floor(p.v[1].xy)), vec2<i32>(0), size - 1);
        hi = clamp(vec2<i32>(ceil(p.v[1].xy + p.v[1].zw)) - 1, lo, size - 1);
    }
    let q = uv - 0.5;
    let f = fract(q);
    let base = vec2<i32>(floor(q));
    let wx = vec4<f32>(
        catmull_rom(f.x + 1.0),
        catmull_rom(f.x),
        catmull_rom(1.0 - f.x),
        catmull_rom(2.0 - f.x),
    );
    let wy = vec4<f32>(
        catmull_rom(f.y + 1.0),
        catmull_rom(f.y),
        catmull_rom(1.0 - f.y),
        catmull_rom(2.0 - f.y),
    );
    var sum = vec4<f32>(0.0);
    var nmin = vec4<f32>(255.0);
    var nmax = vec4<f32>(0.0);
    for (var j = -1; j <= 2; j += 1) {
        var row = vec4<f32>(0.0);
        let wj = wy[j + 1];
        for (var i = -1; i <= 2; i += 1) {
            let xy = base + vec2<i32>(i, j);
            let s = select(load_premul(t, xy), load_rect_premul(t, xy, lo, hi), clamp_rect);
            if ((i == 0 || i == 1) && (j == 0 || j == 1)) {
                nmin = min(nmin, s);
                nmax = max(nmax, s);
            }
            row += s * wx[i + 1];
        }
        sum += row * wj;
    }
    return clamp(sum, nmin, nmax);
}
fn sample_linear_premul(t: texture_2d<f32>, uv: vec2<f32>) -> vec4<f32> {
    let q = uv - 0.5;
    let f = fract(q);
    let xy = vec2<i32>(floor(q));
    return mix(
        mix(load_premul(t, xy), load_premul(t, xy + vec2<i32>(1, 0)), f.x),
        mix(load_premul(t, xy + vec2<i32>(0, 1)), load_premul(t, xy + vec2<i32>(1, 1)), f.x),
        f.y
    );
}
fn sample_aa_premul(t: texture_2d<f32>, uv: vec2<f32>, du: vec2<f32>, dv: vec2<f32>) -> vec4<f32> {
    let step = max(length(du), length(dv));
    if (step < 0.95) {
        return sample_bicubic_ar_premul(t, uv, false);
    }
    if (step <= 1.05) {
        return sample_linear_premul(t, uv);
    }
    if (step <= 2.5) {
        let s0 = sample_linear_premul(t, uv + du * -0.375 + dv * -0.125);
        let s1 = sample_linear_premul(t, uv + du * 0.125 + dv * -0.375);
        let s2 = sample_linear_premul(t, uv + du * 0.375 + dv * 0.125);
        let s3 = sample_linear_premul(t, uv + du * -0.125 + dv * 0.375);
        return (s0 + s1 + s2 + s3) * 0.25;
    }
    var sum = vec4<f32>(0.0);
    for (var yi = 0; yi < 4; yi += 1) {
        let oy = (f32(yi) - 1.5) * 0.25;
        for (var xi = 0; xi < 4; xi += 1) {
            let ox = (f32(xi) - 1.5) * 0.25;
            sum += sample_linear_premul(t, uv + du * ox + dv * oy);
        }
    }
    return sum * 0.0625;
}
fn div255(x: f32) -> f32 {
    return floor(x / 255.0 + 0.001);
}
@fragment fn fs(in: Vertex) -> @location(0) vec4<f32> {
    // Compute derivatives in uniform control flow before any conditional discard.
    let du = dpdx(in.uv);
    let dv = dpdy(in.uv);
    let flags=u32(p.v[4].w);
    var texel=vec4<f32>(p.v[4].xyz,255.0);
    if ((flags & 32u)!=0u) {
        if ((flags & 64u)!=0u) {
            let at=vec2<i32>(floor(in.position.xy)-p.v[2].xy);
            let uv=vec2<i32>(p.v[6].xy)+at.x*vec2<i32>(p.v[6].zw)+at.y*vec2<i32>(p.v[7].xy);
            let xy=vec2<i32>(p.v[1].xy)+(uv >> vec2<u32>(10u));
            if (any(xy < max(vec2<i32>(p.v[1].xy),vec2<i32>(0))) ||
                any(xy >= min(vec2<i32>(p.v[1].zw),vec2<i32>(textureDimensions(image))))) {
                discard;
            }
            texel=load(image,xy);
        } else if ((flags & 1u)==0u) {
            texel=load(image,vec2<i32>(floor(in.uv)));
        }
        let weight=p.v[5].x;
        var rgb=floor((weight*texel.rgb+(256.0-weight)*p.v[4].xyz)/256.0);
        var alpha=p.v[0].z;
        if (p.v[5].y!=0.0 && texel.a<255.0) {
            alpha=floor(alpha*texel.a/256.0);
        }
        if (p.v[5].z!=0.0) {
            rgb=floor(rgb*alpha/256.0);
            alpha=256.0;
        }
        return vec4<f32>(rgb/255.0,alpha/256.0);
    }
    let aa = (flags & 1u) == 0u && p.v[0].w > 1.5;
    if ((flags & 1u)==0u) {
        if (aa) {
            let pm = select(
                sample_aa_premul(image, in.uv, du, dv),
                sample_bicubic_ar_premul(image, in.uv, true),
                p.v[0].w > 2.5,
            );
            if (pm.a > 0.0001) {
                texel = vec4<f32>(clamp(pm.rgb * (255.0 / pm.a), vec3<f32>(0.0), vec3<f32>(255.0)), pm.a);
            } else {
                texel = vec4<f32>(0.0);
            }
        } else {
            texel=load(image,vec2<i32>(floor(in.uv)));
            if (p.v[0].w>0.0) { texel=sample_linear(image,in.uv); }
        }
    }
    var alpha = select(div255(texel.a * p.v[0].z + 127.0), texel.a * p.v[0].z / 255.0, aa);
    if ((flags & 4u)!=0u) {
        let green=u32(load(mask,vec2<i32>(floor(in.uv))).g);
        let cov = p.v[6u+green/4u][green%4u];
        alpha = select(round(alpha * cov), alpha * cov, aa);
    }
    if ((flags & 8u)!=0u) {
        let target_size = select(p.v[0].xy, p.v[5].xy, p.v[5].x > 0.0 && p.v[5].y > 0.0);
        let logical_pos = in.position.xy * (p.v[0].xy / target_size);
        let uv=(logical_pos-0.5)*vec2<f32>(textureDimensions(screen))/p.v[0].xy;
        let sg = sample_linear(screen, uv).g;
        alpha = select(div255(alpha * sg + 127.0), alpha * sg / 255.0, aa);
    }
    if ((flags & 2u)!=0u) { texel=vec4<f32>(0.0,0.0,0.0,texel.a); }
    return vec4<f32>(texel.rgb/255.0,alpha/255.0);
}
