struct Params {
    rect: vec4<f32>,
    source: vec4<f32>, // width, height, filter, checkerboard
    intermediate: vec4<f32>, // visible origin, vertical-first, unused
}
@group(0) @binding(0) var<uniform> p:Params;
@group(0) @binding(1) var image:texture_2d<f32>;
@vertex fn vs(@builtin(vertex_index) i:u32)->@builtin(position) vec4<f32> {
    let xy=vec2<f32>(f32((i<<1u)&2u),f32(i&2u));return vec4<f32>(xy*2.0-1.0,0.0,1.0);
}
fn texel(xy:vec2<i32>)->vec4<f32> {return textureLoad(image,clamp(xy,vec2<i32>(0),vec2<i32>(textureDimensions(image))-1),0);}

fn source_sample(xy: vec2<i32>, premultiply: bool) -> vec4<f32> {
    let c = texel(xy);
    return select(c, vec4<f32>(c.rgb * c.a, c.a), premultiply);
}

fn keys(distance: f32) -> f32 {
    let x = abs(distance);
    if x < 1.0 {
        return 1.0 + x * x * (x - 2.0);
    }
    return (1.0 - x) * (x - 2.0) * (x - 2.0);
}

fn lanczos4(distance: f32, offset: i32) -> f32 {
    // sin(pi * distance) differs from sin(pi * fraction) only in sign.
    // The common magnitude cancels when the axis is normalized.
    let sign = select(1.0, -1.0, (offset & 1) != 0);
    return sign * sin(0.7853981633974483 * distance) / (distance * distance);
}

fn filter_axis(base: vec2<i32>, axis: vec2<i32>, coordinate: f32, premultiply: bool) -> vec4<f32> {
    let fraction = fract(coordinate);
    if fraction == 0.0 || fraction == 1.0 {
        return source_sample(base + axis * i32(round(coordinate)), premultiply);
    }
    let center = i32(floor(coordinate));
    let cubic = p.source.z == 2.0;
    let half_width = select(4, 2, cubic);
    var color = vec4<f32>(0.0);
    var total = 0.0;
    for (var offset = 1 - half_width; offset <= half_width; offset += 1) {
        let distance = fraction - f32(offset);
        var weight: f32;
        if cubic {
            weight = keys(distance);
        } else {
            weight = lanczos4(distance, offset);
        }
        let sample = source_sample(base + axis * (center + offset), premultiply);
        color += sample * weight;
        total += weight;
    }
    return color / total;
}

fn first_axis() -> vec2<i32> {
    return select(vec2<i32>(1, 0), vec2<i32>(0, 1), p.intermediate.z != 0.0);
}

@fragment fn fs_intermediate(@builtin(position) at: vec4<f32>) -> @location(0) vec4<f32> {
    let axis = first_axis();
    let base = vec2<i32>(at.xy) * (vec2<i32>(1) - axis);
    let q = at.xy + p.intermediate.xy - p.rect.xy;
    let coordinate = q * (p.source.xy / p.rect.zw) - 0.5;
    return filter_axis(base, axis, dot(coordinate, vec2<f32>(axis)), true);
}

@fragment fn fs(@builtin(position) at:vec4<f32>)->@location(0) vec4<f32> {
    var background=vec3<f32>(0.0);
    if (p.source.w>0.0) {background=vec3<f32>(38.0,52.0,71.0)/255.0;if (((u32(at.x)/32u+u32(at.y)/32u)&1u)!=0u){background=vec3<f32>(44.0,60.0,80.0)/255.0;}}
    let q=at.xy-p.rect.xy;
    if (any(q<vec2<f32>(0.0)) || any(q>=p.rect.zw)) {return vec4<f32>(background,1.0);}
    var c:vec4<f32>;
    // Integer-aligned 1:1 presentation always samples exact texels.
    if (all(p.source.xy==p.rect.zw) && all(p.rect.xy==floor(p.rect.xy))) {
        c=texel(vec2<i32>(floor(q)));c=vec4<f32>(c.rgb*c.a,c.a);
    } else if (p.source.z >= 2.0) {
        let first = first_axis();
        let axis = vec2<i32>(1) - first;
        let base = vec2<i32>(at.xy - p.intermediate.xy) * first;
        let coordinate = q * (p.source.xy / p.rect.zw) - 0.5;
        let color = filter_axis(base, axis, dot(coordinate, vec2<f32>(axis)), false);
        let alpha = clamp(color.a, 0.0, 1.0);
        c = vec4<f32>(clamp(color.rgb, vec3<f32>(0.0), vec3<f32>(alpha)), alpha);
    } else if (p.source.z>0.0) {
        let coord=floor(q*p.source.xy/p.rect.zw*256.0)-128.0;
        let uv=clamp(coord,vec2<f32>(0.0),(p.source.xy-1.0)*256.0)/256.0;
        let lo=vec2<i32>(floor(uv));let f=fract(uv);
        var taps=array<vec4<f32>,4>(texel(lo),texel(lo+vec2<i32>(1,0)),texel(lo+vec2<i32>(0,1)),texel(lo+vec2<i32>(1,1)));
        for(var i=0u;i<4u;i++){taps[i]=vec4<f32>(taps[i].rgb*taps[i].a,taps[i].a);}
        c=mix(mix(taps[0],taps[1],f.x),mix(taps[2],taps[3],f.x),f.y);
    } else {c=texel(vec2<i32>(floor(floor(q)*p.source.xy/p.rect.zw)));c=vec4<f32>(c.rgb*c.a,c.a);}
    return vec4<f32>(c.rgb+background*(1.0-c.a),1.0);
}
