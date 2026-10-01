import blockloom::clouds::CloudUniforms;
@group(0) @binding(0) var<uniform> c: CloudUniforms;
@group(0) @binding(1) var raw_color: texture_2d<f32>;
@group(0) @binding(2) var raw_depth: texture_2d<f32>;
@group(0) @binding(3) var history: texture_2d<f32>;
@group(0) @binding(4) var history_depth: texture_2d<f32>;
@group(0) @binding(5) var linear: sampler;
struct Output { @location(0) color: vec4<f32>, @location(1) depths: vec4<f32> }
fn world(uv:vec2<f32>,d:f32)->vec3<f32>{
    let p=c.world_from_clip*vec4<f32>(uv*vec2<f32>(2.0,-2.0)+vec2<f32>(-1.0,1.0),d,1.0);
    return p.xyz/p.w;
}
// Folds this frame's march into the history. The history is clipped to what
// the pixel's neighbours show now, so a jittered, noisy far march converges
// instead of being pulled back to its latest sample.
@fragment fn fragment(@builtin(position) pixel:vec4<f32>)->Output {
    let size=vec2<i32>(textureDimensions(raw_color));
    let at=vec2<i32>(pixel.xy);
    let middle=textureLoad(raw_depth,at,0);
    var out:Output; out.color=textureLoad(raw_color,at,0); out.depths=middle;
    let distance=middle.x*1000.0;
    // Neighbours that see the same scene depth, for the spread and the blur.
    var sum=vec4<f32>(0.0); var square=vec4<f32>(0.0); var count=0.0;
    for(var y=-1;y<=1;y++){ for(var x=-1;x<=1;x++){
        let tap=clamp(at+vec2<i32>(x,y),vec2<i32>(0),size-vec2<i32>(1));
        let depths=textureLoad(raw_depth,tap,0);
        if abs(depths.y-middle.y)<=max(0.01,middle.y*0.02) {
            let color=textureLoad(raw_color,tap,0);
            sum+=color; square+=color*color; count+=1.0;
        }
    } }
    let mean=sum/count;
    let spread=sqrt(max(square/count-mean*mean,vec4<f32>(0.0)));
    // Far clouds are sparse in samples and slow in detail: let the blur in.
    let current=mix(out.color,mean,smoothstep(15000.0,80000.0,distance));
    out.color=current;
    if c.previous_camera.w>0.0 {
        let full=min(at*2,vec2<i32>(c.screen.xy)-vec2<i32>(1));
        let uv=(vec2<f32>(full)+0.5)/c.screen.xy;
        let ray=normalize(world(uv,0.00001)-c.camera.xyz);
        let p=c.camera.xyz+ray*distance-(c.drift.xyz-c.previous_drift.xyz);
        let clip=c.previous_clip*vec4<f32>(p,1.0);
        let old_uv=clip.xy/clip.w*vec2<f32>(0.5,-0.5)+0.5;
        if clip.w>0.0 && all(old_uv>vec2<f32>(0.0)) && all(old_uv<vec2<f32>(1.0)) {
            let old_d=textureSampleLevel(history_depth,linear,old_uv,0.0).xy*1000.0;
            let expected=length(p-c.previous_camera.xyz);
            let scene=middle.y*1000.0;
            if abs(old_d.x-expected)<max(150.0,distance*0.08) && abs(old_d.y-scene)<max(10.0,scene*0.02) {
                let old=textureSampleLevel(history,linear,old_uv,0.0);
                let clipped=clamp(old,mean-spread*1.5,mean+spread*1.5);
                out.color=mix(current,clipped,mix(0.9,0.95,smoothstep(10000.0,60000.0,distance)));
            }
        }
    }
    return out;
}
