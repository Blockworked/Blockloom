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
    // Single-bind stacked noise atlas: each page's XY scale into the
    // shared image plus its depth slab (offset, extent) in 0-1. The shape
    // and detail volumes live in one 3D texture, so the march and the
    // shadow pass bind once instead of once per volume.
    noise_shape: vec4<f32>,
    noise_detail: vec4<f32>,
}
fn atlas_uv(q: vec3<f32>, page: vec4<f32>, dims: vec3<f32>) -> vec3<f32> {
    // Pages share one atlas with no gutter between them, so clamp the
    // sample half a texel inside the page: linear filtering at the wrap
    // would otherwise read the neighbouring page and step the density
    // along every tile boundary plane. The bake wraps its lattice, so the
    // clamped edges still meet seamlessly.
    let origin = vec3<f32>(0.0, 0.0, page.z * dims.z);
    let size = vec3<f32>(page.xy * dims.xy, page.w * dims.z);
    let half = vec3<f32>(0.5);
    let local = clamp(fract(q) * size, half, max(size - half, half));
    return (origin + local) / dims;
}
// How far a sample's footprint (metres: the march step or the pixel's width
// there, whichever is wider) outgrows a feature: 0 resolves it, 1 has lost it.
fn lost(footprint: f32, feature: f32) -> f32 {
    return smoothstep(0.5, 2.0, footprint / max(feature, 1.0));
}
// Mean of the erosion channels, what a filtered-out detail settles to.
const EROSION_MEAN = 0.45;
fn density(p: vec3<f32>, c: CloudUniforms, noise_tex: texture_3d<f32>, repeat: sampler, footprint: f32) -> f32 {
    let h = (p.y-c.layer.x) / (c.layer.y-c.layer.x);
    if h <= 0.0 || h >= 1.0 || c.shape.x <= 0.0 { return 0.0; }
    let dims = vec3<f32>(textureDimensions(noise_tex));
    let q = (p-c.drift.xyz-vec3<f32>(c.shear.x,0.0,c.shear.y)*h) / c.layer.z;
    let n = textureSampleLevel(noise_tex,repeat,atlas_uv(q,c.noise_shape,dims),0.0);
    // Past what the march can resolve, soften the shape's edges towards a
    // plain ramp (its finest octave is a 32nd of the tile) instead of
    // aliasing, and let erosion settle to its mean.
    let soft = lost(footprint, c.layer.z / 32.0);
    let cover = clamp(c.shape.x + c.profile.x*c.shape.z*smoothstep(0.55,1.0,h),0.0,1.0);
    let base = smoothstep(mix(c.shape.w,0.0,soft),mix(c.detail.w,1.0,soft),clamp((n.x-(1.0-cover))/max(cover,0.001),0.0,1.0));
    let top = mix(0.35,1.0,c.shape.z);
    let height = smoothstep(0.0,c.profile.y,h)*(1.0-smoothstep(top-c.profile.z,top,h));
    var e = vec3<f32>(EROSION_MEAN);
    let kept = 1.0 - lost(footprint, c.layer.z / max(c.detail.x,0.001) / 8.0);
    if kept > 0.0 {
        let dq=(p-c.erosion_drift.xyz)*c.detail.x/c.layer.z;
        let curl=textureSampleLevel(noise_tex,repeat,atlas_uv(dq.yzx,c.noise_detail,dims),0.0).yzx-textureSampleLevel(noise_tex,repeat,atlas_uv(dq.zxy,c.noise_detail,dims),0.0).zxy;
        e = mix(e,textureSampleLevel(noise_tex,repeat,atlas_uv(dq+curl*0.15,c.noise_detail,dims),0.0).xyz,kept);
    }
    let erosion = dot(e,vec3<f32>(0.625,0.25,0.125))*c.detail.y*mix(0.5,1.0,c.shape.z);
    return max(base*height-erosion*(1.0-base*0.5),0.0)*c.shape.y*0.004;
}
