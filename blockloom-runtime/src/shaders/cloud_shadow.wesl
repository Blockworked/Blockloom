import blockloom::clouds::{CloudUniforms,density};
@group(0) @binding(0) var<uniform> c: CloudUniforms;
@group(0) @binding(1) var noise_tex: texture_3d<f32>;
@group(0) @binding(2) var repeat: sampler;
@group(0) @binding(3) var shadow_map: texture_storage_2d<rgba16float,write>;
fn optical_depth(xz:vec2<f32>,light:vec4<f32>)->f32 {
    let sun=normalize(light.xyz);var optical=0.0;
    if sun.y>0.01 && c.shadow.y>0.0 && light.w>0.0 {
        let step=(c.layer.y-c.layer.x)/8.0;
        for(var i=0u;i<8u;i++){
            let y=c.layer.x+(f32(i)+0.5)*step;
            let p=vec3<f32>(xz.x,0.0,xz.y)+sun*y/sun.y;
            optical+=density(p,c,noise_tex,repeat,0.0)*step/sun.y;
        }
    }
    return exp(-optical);
}
@compute @workgroup_size(8,8)
fn shadow(@builtin(global_invocation_id) id:vec3<u32>) {
    let size=textureDimensions(shadow_map);
    if any(id.xy>=size){return;}
    let xz=(vec2<f32>(id.xy)+0.5)/vec2<f32>(size)*c.shadow.x-c.shadow.x*0.5+c.camera.xz;
    textureStore(shadow_map,vec2<i32>(id.xy),vec4<f32>(optical_depth(xz,c.sun),optical_depth(xz,c.moon),0.0,1.0));
}
