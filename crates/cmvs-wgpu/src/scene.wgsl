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
            let pm = sample_aa_premul(image, in.uv, du, dv);
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
        let uv=(in.position.xy-0.5)*vec2<f32>(textureDimensions(screen))/p.v[0].xy;
        let sg = sample_linear(screen, uv).g;
        alpha = select(div255(alpha * sg + 127.0), alpha * sg / 255.0, aa);
    }
    if ((flags & 2u)!=0u) { texel=vec4<f32>(0.0,0.0,0.0,texel.a); }
    return vec4<f32>(texel.rgb/255.0,alpha/255.0);
}
