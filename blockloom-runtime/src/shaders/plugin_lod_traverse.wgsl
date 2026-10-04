struct Node { lo: vec4<f32>, hi: vec4<f32>, links: vec4<u32> }
struct View { clip: mat4x4<f32>, counts: vec4<u32> }
@group(0) @binding(0) var<storage, read> nodes: array<Node>;
@group(0) @binding(1) var<uniform> view: View;
@group(0) @binding(2) var pyramid: texture_2d<f32>;
@group(0) @binding(3) var<storage, read_write> output: array<u32>;

fn visible(node: Node) -> bool {
    var lo = vec2<f32>(1.0); var hi = vec2<f32>(0.0); var nearest = 0.0;
    var left = true; var right = true; var top = true; var bottom = true; var far = true;
    for (var c = 0u; c < 8u; c++) {
        let p = vec3<f32>(select(node.lo.x,node.hi.x,(c & 1u) != 0u), select(node.lo.y,node.hi.y,(c & 2u) != 0u), select(node.lo.z,node.hi.z,(c & 4u) != 0u));
        let q = view.clip * vec4<f32>(p,1.0);
        // Eye/near-plane intersections cannot safely use projected-box occlusion.
        if q.w <= 0.0 || q.z >= q.w { return true; }
        let ndc = q.xyz / q.w;
        left = left && ndc.x < -1.0; right = right && ndc.x > 1.0;
        bottom = bottom && ndc.y < -1.0; top = top && ndc.y > 1.0; far = far && ndc.z < 0.0;
        let uv = ndc.xy * vec2<f32>(0.5,-0.5) + vec2<f32>(0.5);
        lo = min(lo,uv); hi = max(hi,uv); nearest = max(nearest,ndc.z);
    }
    if left || right || top || bottom || far { return false; }
    lo = clamp(lo,vec2<f32>(0.0),vec2<f32>(1.0)); hi = clamp(hi,vec2<f32>(0.0),vec2<f32>(1.0));
    let pixels = (hi-lo)*vec2<f32>(textureDimensions(pyramid,0));
    let mip = min(i32(view.counts.y)-1, max(0,i32(ceil(log2(max(1.0,max(pixels.x,pixels.y)))))));
    let dims = vec2<i32>(textureDimensions(pyramid,mip));
    let a = clamp(vec2<i32>(floor(lo*vec2<f32>(dims))),vec2<i32>(0),dims-vec2<i32>(1));
    let b = clamp(vec2<i32>(floor(hi*vec2<f32>(dims))),vec2<i32>(0),dims-vec2<i32>(1));
    let depth = min(min(textureLoad(pyramid,a,mip).x,textureLoad(pyramid,vec2<i32>(b.x,a.y),mip).x),min(textureLoad(pyramid,vec2<i32>(a.x,b.y),mip).x,textureLoad(pyramid,b,mip).x));
    return nearest >= depth - 0.00001;
}

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x != 0u || view.counts.x == 0u { return; }
    var stack: array<u32,32>; stack[0] = 0u; var count = 1u;
    for (var visit = 0u; visit < 4095u; visit++) {
        if count == 0u { return; }
        count--; let index = stack[count];
        let node = nodes[index];
        if !visible(node) { continue; }
        if node.links.w == 1u {
            // Mesh instances and tile descriptors share deduplicated request bits.
            let bit = 514u + node.links.z;
            if output[bit] == 0u {
                output[bit] = 1u;
                let slot = output[0]; output[0] = slot + 1u;
                if slot < 512u { output[2u+slot] = node.links.z; }
                else { output[1] = 1u; }
            }
        } else {
            if count + 2u > 32u { output[1] = 1u; return; }
            stack[count] = node.links.x; stack[count+1u] = node.links.y; count += 2u;
        }
    }
    if count != 0u { output[1] = 1u; }
}
