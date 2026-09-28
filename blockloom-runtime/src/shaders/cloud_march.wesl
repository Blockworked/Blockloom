import blockloom::blue_noise::blue_noise;
import blockloom::clouds::{CloudUniforms,density};
import blockloom::sky::{SKY_UNIT,cube_lookup};
import blockloom::scattering::{dual_lobe_phase,beer,powder};
@group(0) @binding(0) var<uniform> c: CloudUniforms;
@if(!MULTISAMPLED) @group(0) @binding(1) var depth_tex: texture_depth_2d;
@if(MULTISAMPLED) @group(0) @binding(1) var depth_tex: texture_depth_multisampled_2d;
@group(0) @binding(2) var noise_tex: texture_3d<f32>;
@group(0) @binding(3) var repeat: sampler;
@group(0) @binding(4) var history: texture_2d<f32>;
@group(0) @binding(5) var history_depth: texture_2d<f32>;
@group(0) @binding(6) var sky: texture_cube<f32>;
@group(0) @binding(7) var linear: sampler;
struct Output { @location(0) color: vec4<f32>, @location(1) depths: vec4<f32> }
fn world(uv:vec2<f32>,d:f32)->vec3<f32>{
    let p=c.world_from_clip*vec4<f32>(uv*vec2<f32>(2.0,-2.0)+vec2<f32>(-1.0,1.0),d,1.0);
    return p.xyz/p.w;
}
fn medium(p:vec3<f32>)->f32 { return density(p,c,noise_tex,repeat); }
fn incident(p:vec3<f32>,ray:vec3<f32>,light:vec4<f32>,color:vec3<f32>,jitter:f32,sigma:f32,step:f32,h:f32)->vec3<f32> {
    if max(max(color.x,color.y),color.z)<=0.0 {return vec3<f32>(0.0);}
    let direction=normalize(light.xyz);
    let light_step=(c.layer.y-c.layer.x)/f32(c.steps.y);
    var optical=0.0;
    if light.w>0.0 {
        for(var j=0u;j<c.steps.y;j++){optical+=medium(p+direction*(f32(j)+0.5+jitter*0.5)*light_step)*light_step;}
    }
    let phase=dual_lobe_phase(dot(ray,direction),c.phase.x,c.phase.y,c.phase.z);
    let lit=exp(-optical)*(1.0+c.phase.w*powder(sigma,step))+c.lighting.z*exp(-optical*0.25)*h*0.15;
    return color*phase*lit;
}
@fragment fn fragment(@builtin(position) pixel:vec4<f32>)->Output {
    let full=min(vec2<i32>(pixel.xy)*2,vec2<i32>(c.screen.xy)-1);
    let uv=(vec2<f32>(full)+0.5)/c.screen.xy;
    let d=textureLoad(depth_tex,full,0);
    let ray=normalize(world(uv,0.00001)-c.camera.xyz);
    var scene_distance=200000.0;
    if d>0.0 { scene_distance=length(world(uv,d)-c.camera.xyz); }
    var out:Output; out.color=vec4<f32>(0.0); out.depths=vec4<f32>(0.0,scene_distance/1000.0,0.0,0.0);
    if abs(ray.y)<0.00001 { return out; }
    let a=(c.layer.x-c.camera.y)/ray.y; let b=(c.layer.y-c.camera.y)/ray.y;
    let start=max(min(a,b),0.0); let end=min(max(a,b),scene_distance);
    if end<=start || start>200000.0 { return out; }
    // Distant, sparse weather needs fewer samples than a dense nearby bank.
    let detail=mix(1.0,0.5,clamp(start/100000.0,0.0,1.0))*mix(0.5,1.0,clamp(c.shape.x*c.shape.y,0.0,1.0));
    let count=max(8u,u32(f32(c.steps.x)*detail));
    let step=(min(end,200000.0)-start)/f32(count);
    let jitter=blue_noise(vec2<u32>(full),u32(c.camera.w));
    var trans=1.0; var rgb=vec3<f32>(0.0); var weighted=0.0; var weight=0.0;
    for(var i=0u;i<count;i++) {
        out.depths.z+=1.0;
        let distance=start+(f32(i)+jitter)*step; let p=c.camera.xyz+ray*distance;
        let sigma=medium(p);
        if sigma>0.000001 {
            out.depths.w+=1.0;
            let h=clamp((p.y-c.layer.x)/(c.layer.y-c.layer.x),0.0,1.0);
            let ambient=textureSampleLevel(sky,linear,cube_lookup(normalize(vec3<f32>(ray.x,0.5+h,ray.z))),0.0).rgb*SKY_UNIT*c.lighting.x;
            let bounce=mix(1.0-c.lighting.y,1.0,h);
            let belly=mix(vec3<f32>(0.65,0.75,1.0),vec3<f32>(1.0,0.95,0.9),h);
            let incoming=ambient*bounce*belly+incident(p,ray,c.sun,c.sun_color.rgb,jitter,sigma,step,h)+incident(p,ray,c.moon,c.moon_color.rgb,jitter,sigma,step,h);
            let alpha=1.0-beer(sigma,step); let w=trans*alpha;
            rgb+=incoming*w; weighted+=distance*w; weight+=w; trans*=1.0-alpha;
            if trans<c.layer.w { break; }
        }
    }
    out.color=vec4<f32>(rgb*c.lighting.w,1.0-trans);
    let distance=select((start+end)*0.5,weighted/max(weight,0.00001),weight>0.001);
    out.depths.x=distance/1000.0;
    if c.previous_camera.w>0.0 {
        let p=c.camera.xyz+ray*distance-(c.drift.xyz-c.previous_drift.xyz);
        let clip=c.previous_clip*vec4<f32>(p,1.0);
        let old_uv=clip.xy/clip.w*vec2<f32>(0.5,-0.5)+0.5;
        if clip.w>0.0 && all(old_uv>vec2<f32>(0.0)) && all(old_uv<vec2<f32>(1.0)) {
            let old_d=textureSampleLevel(history_depth,linear,old_uv,0.0).xy*1000.0;
            let expected=length(p-c.previous_camera.xyz);
            if abs(old_d.x-expected)<max(150.0,distance*0.08) && abs(old_d.y-scene_distance)<max(10.0,scene_distance*0.02) {
                let old=textureSampleLevel(history,linear,old_uv,0.0);
                out.color=mix(out.color,clamp(old,out.color-vec4<f32>(0.2),out.color+vec4<f32>(0.2)),0.85);
            }
        }
    }
    return out;
}
