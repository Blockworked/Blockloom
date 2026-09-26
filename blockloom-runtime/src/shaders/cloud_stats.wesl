@group(0) @binding(0) var depths:texture_2d<f32>;
@group(0) @binding(1) var<storage,read_write> result:vec4<f32>;
var<workgroup> counts:array<vec2<f32>,64>;
@compute @workgroup_size(64)
fn measure(@builtin(local_invocation_index) lane:u32) {
    let size=textureDimensions(depths);let pixels=size.x*size.y;
    var sum=vec2<f32>(0.0);
    for(var i=lane;i<pixels;i+=64u){sum+=textureLoad(depths,vec2<i32>(vec2<u32>(i%size.x,i/size.x)),0).zw;}
    counts[lane]=sum;workgroupBarrier();
    if lane==0u {
        var total=vec2<f32>(0.0);
        for(var i=0u;i<64u;i++){total+=counts[i];}
        result=vec4<f32>(total/f32(pixels),1.0,0.0);
    }
}
