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
@fragment fn fs(in: Vertex) -> @location(0) vec4<f32> {
    let flags=u32(p.v[4].w);
    var texel=vec4<f32>(p.v[4].xyz,255.0);
    if ((flags & 1u)==0u) {
        texel=load(image,vec2<i32>(floor(in.uv)));
        if (p.v[0].w>0.0) { texel=sample_linear(image,in.uv); }
    }
    var alpha=floor((texel.a*p.v[0].z+127.0)/255.0);
    if ((flags & 4u)!=0u) {
        let green=u32(load(mask,vec2<i32>(floor(in.uv))).g);
        alpha=round(alpha*p.v[6u+green/4u][green%4u]);
    }
    if ((flags & 8u)!=0u) {
        let uv=(in.position.xy-0.5)*vec2<f32>(textureDimensions(screen))/p.v[0].xy;
        alpha=floor((alpha*sample_linear(screen,uv).g+127.0)/255.0);
    }
    if ((flags & 2u)!=0u) { texel=vec4<f32>(0.0,0.0,0.0,texel.a); }
    return vec4<f32>(texel.rgb/255.0,alpha/255.0);
}
