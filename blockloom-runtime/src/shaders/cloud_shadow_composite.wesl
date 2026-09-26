import blockloom::clouds::CloudUniforms;
@group(0) @binding(0) var<uniform> c:CloudUniforms;
@group(0) @binding(1) var scene:texture_2d<f32>;
@if(!MULTISAMPLED) @group(0) @binding(2) var depth:texture_depth_2d;
@if(MULTISAMPLED) @group(0) @binding(2) var depth:texture_depth_multisampled_2d;
@group(0) @binding(3) var shadow:texture_2d<f32>;
@group(0) @binding(4) var linear:sampler;
fn shadow_for(world:vec3<f32>,light:vec4<f32>,channel:u32)->f32 {
    if light.y<=0.01 || light.w<=0.0 {return 1.0;}
    let ground=world.xz-light.xz*world.y/light.y;
    let suv=(ground-c.camera.xz)/c.shadow.x+0.5;
    if any(suv<vec2<f32>(0.0)) || any(suv>vec2<f32>(1.0)){return 1.0;}
    let edge=smoothstep(0.0,0.05,min(min(suv.x,suv.y),min(1.0-suv.x,1.0-suv.y)));
    let trans=textureSampleLevel(shadow,linear,suv,0.0)[channel];
    return mix(1.0,trans,c.shadow.y*edge);
}
@fragment fn fragment(@builtin(position) pixel:vec4<f32>)->@location(0) vec4<f32>{
    let pos=vec2<i32>(pixel.xy);let color=textureLoad(scene,pos,0);let d=textureLoad(depth,pos,0);
    if d<=0.0 || c.shadow.y<=0.0 {return color;}
    let uv=pixel.xy/c.screen.xy;
    let homogeneous=c.world_from_clip*vec4<f32>(uv*vec2<f32>(2.0,-2.0)+vec2<f32>(-1.0,1.0),d,1.0);
    let world=homogeneous.xyz/homogeneous.w;
    if world.y>=c.layer.x {return color;}
    let weights=vec2<f32>(dot(c.sun_color.rgb,vec3<f32>(0.2126,0.7152,0.0722)),dot(c.moon_color.rgb,vec3<f32>(0.2126,0.7152,0.0722)));
    if weights.x+weights.y<=0.0 {return color;}
    let through=dot(vec2<f32>(shadow_for(world,c.sun,0u),shadow_for(world,c.moon,1u)),weights)/(weights.x+weights.y);
    return vec4<f32>(color.rgb*through,color.a);
}
