struct Params { rect:vec4<f32>, source:vec4<f32> }
@group(0) @binding(0) var<uniform> p:Params;
@group(0) @binding(1) var image:texture_2d<f32>;
@vertex fn vs(@builtin(vertex_index) i:u32)->@builtin(position) vec4<f32> {
    let xy=vec2<f32>(f32((i<<1u)&2u),f32(i&2u));return vec4<f32>(xy*2.0-1.0,0.0,1.0);
}
fn texel(xy:vec2<i32>)->vec4<f32> {return textureLoad(image,clamp(xy,vec2<i32>(0),vec2<i32>(textureDimensions(image))-1),0);}
@fragment fn fs(@builtin(position) at:vec4<f32>)->@location(0) vec4<f32> {
    var background=vec3<f32>(0.0);
    if (p.source.w>0.0) {background=vec3<f32>(38.0,52.0,71.0)/255.0;if (((u32(at.x)/32u+u32(at.y)/32u)&1u)!=0u){background=vec3<f32>(44.0,60.0,80.0)/255.0;}}
    let q=at.xy-p.rect.xy;
    if (any(q<vec2<f32>(0.0)) || any(q>=p.rect.zw)) {return vec4<f32>(background,1.0);}
    var c:vec4<f32>;
    if (p.source.z>0.0) {
        let coord=floor(q*p.source.xy/p.rect.zw*256.0)-128.0;
        let uv=clamp(coord,vec2<f32>(0.0),(p.source.xy-1.0)*256.0)/256.0;
        let lo=vec2<i32>(floor(uv));let f=fract(uv);
        var taps=array<vec4<f32>,4>(texel(lo),texel(lo+vec2<i32>(1,0)),texel(lo+vec2<i32>(0,1)),texel(lo+vec2<i32>(1,1)));
        for(var i=0u;i<4u;i++){taps[i]=vec4<f32>(taps[i].rgb*taps[i].a,taps[i].a);}
        c=mix(mix(taps[0],taps[1],f.x),mix(taps[2],taps[3],f.x),f.y);
    } else {c=texel(vec2<i32>(floor(floor(q)*p.source.xy/p.rect.zw)));c=vec4<f32>(c.rgb*c.a,c.a);}
    return vec4<f32>(c.rgb+background*(1.0-c.a),1.0);
}
