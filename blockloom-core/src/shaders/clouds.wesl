// Shared cloud density and lighting parameters.
struct CloudUniforms {
    world_from_clip: mat4x4<f32>,
    previous_clip: mat4x4<f32>,
    camera: vec4<f32>,
    previous_camera: vec4<f32>,
    layer: vec4<f32>,
    shape: vec4<f32>,
    detail: vec4<f32>,
    profile: vec4<f32>,
    phase: vec4<f32>,
    lighting: vec4<f32>,
    sun: vec4<f32>,
    sun_color: vec4<f32>,
    moon: vec4<f32>,
    moon_color: vec4<f32>,
    drift: vec4<f32>,
    erosion_drift: vec4<f32>,
    shear: vec4<f32>,
    screen: vec4<f32>,
    steps: vec4<u32>,
    shadow: vec4<f32>,
    previous_drift: vec4<f32>,
}
fn density(p: vec3<f32>, c: CloudUniforms, shape_tex: texture_3d<f32>, detail_tex: texture_3d<f32>, repeat: sampler) -> f32 {
    let h = (p.y-c.layer.x) / (c.layer.y-c.layer.x);
    if h <= 0.0 || h >= 1.0 || c.shape.x <= 0.0 { return 0.0; }
    let q = (p-c.drift.xyz-vec3<f32>(c.shear.x,0.0,c.shear.y)*h) / c.layer.z;
    let n = textureSampleLevel(shape_tex,repeat,q,0.0);
    let cover = clamp(c.shape.x + c.profile.x*c.shape.z*smoothstep(0.55,1.0,h),0.0,1.0);
    let base = smoothstep(c.shape.w,c.detail.w,clamp((n.x-(1.0-cover))/max(cover,0.001),0.0,1.0));
    let top = mix(0.35,1.0,c.shape.z);
    let height = smoothstep(0.0,c.profile.y,h)*(1.0-smoothstep(top-c.profile.z,top,h));
    let dq=(p-c.erosion_drift.xyz)*c.detail.x/c.layer.z;
    let curl=textureSampleLevel(detail_tex,repeat,dq.yzx,0.0).yzx-textureSampleLevel(detail_tex,repeat,dq.zxy,0.0).zxy;
    let e = textureSampleLevel(detail_tex,repeat,dq+curl*0.15,0.0).xyz;
    let erosion = dot(e,vec3<f32>(0.625,0.25,0.125))*c.detail.y*mix(0.5,1.0,c.shape.z);
    return max(base*height-erosion*(1.0-base*0.5),0.0)*c.shape.y*0.004;
}
