import blockloom::hash::hash_cell3;
import blockloom::clouds::CloudUniforms;
@group(0) @binding(0) var<uniform> c: CloudUniforms;
@group(0) @binding(1) var output_noise: texture_storage_3d<rgba8unorm,write>;
fn cell_hash(cell: vec3<i32>, period: i32) -> vec3<f32> {
    let wrapped = ((cell % vec3<i32>(period))+vec3<i32>(period))%vec3<i32>(period);
    return hash_cell3(wrapped + vec3<i32>(i32(c.steps.w & 65535u)));
}
fn worley(p: vec3<f32>, period: i32) -> f32 {
    let cell = vec3<i32>(floor(p));
    var d = 3.0;
    for (var z=-1; z<=1; z++) { for (var y=-1; y<=1; y++) { for (var x=-1; x<=1; x++) {
        let offset = vec3<i32>(x,y,z);
        let delta = vec3<f32>(offset)+cell_hash(cell+offset,period)-fract(p);
        d = min(d,dot(delta,delta));
    } } }
    return 1.0-clamp(sqrt(d),0.0,1.0);
}
fn perlin(p: vec3<f32>, period: i32) -> f32 {
    let cell=vec3<i32>(floor(p)); let f=fract(p); let s=f*f*f*(f*(f*6.0-15.0)+10.0);
    var n=0.0;
    for(var z=0;z<2;z++){ for(var y=0;y<2;y++){ for(var x=0;x<2;x++){
        let o=vec3<i32>(x,y,z); let v=vec3<f32>(o);
        let g=normalize(cell_hash(cell+o,period)*2.0-1.0+vec3<f32>(0.0001));
        let w=mix(1.0-s,s,v);
        n+=dot(g,f-v)*w.x*w.y*w.z;
    } } }
    return n*0.5+0.5;
}
@compute @workgroup_size(4,4,4)
fn bake(@builtin(global_invocation_id) id: vec3<u32>) {
    let size=textureDimensions(output_noise);
    if any(id>=size) { return; }
    let uv=(vec3<f32>(id)+0.5)/vec3<f32>(size);
    let w=vec3<f32>(worley(uv*4.0,4),worley(uv*8.0,8),worley(uv*16.0,16));
    let p=perlin(uv*4.0,4)*0.625+perlin(uv*8.0,8)*0.25+perlin(uv*16.0,16)*0.125;
    let pw=clamp(p+(1.0-p)*dot(w,vec3<f32>(0.625,0.25,0.125)),0.0,1.0);
    var result=vec4<f32>(pw,w);
    if size.x==32u { result=vec4<f32>(w,1.0); }
    textureStore(output_noise,vec3<i32>(id),result);
}
