// Cube records hold merged quads; smooth offsets hold disjoint triangle spans.
@group(0) @binding(0) var<storage, read> samples: array<i32>;
@group(0) @binding(1) var<storage, read> palette: array<f32>;
@group(0) @binding(2) var<storage, read> params: array<f32>;
@group(0) @binding(3) var<storage, read_write> vertices: array<f32>;
@group(0) @binding(4) var<storage, read> offsets: array<u32>;
const corners = array<vec3<i32>,8>(vec3(0,0,0),vec3(1,0,0),vec3(0,1,0),vec3(1,1,0),vec3(0,0,1),vec3(1,0,1),vec3(0,1,1),vec3(1,1,1));
const tets = array<vec4<u32>,6>(vec4(0u,1u,3u,7u),vec4(0u,3u,2u,7u),vec4(0u,2u,6u,7u),vec4(0u,6u,4u,7u),vec4(0u,4u,5u,7u),vec4(0u,5u,1u,7u));
fn sample_index(p: vec3<i32>) -> u32 {
    let n = vec3<i32>(i32(params[0]),i32(params[1]),i32(params[2]));
    let c = clamp(p,vec3(0),n-vec3(1));
    return u32((c.z*n.y+c.y)*n.x+c.x)*2u;
}
fn density(p: vec3<i32>) -> f32 { return f32(samples[sample_index(p)]); }
fn material(p: vec3<i32>) -> u32 { return u32(samples[sample_index(p)+1u]); }
fn point(c: vec3<i32>) -> vec3<f32> { return vec3<f32>(c)+vec3(params[8],params[9],params[10])-vec3(1.0)+vec3(0.5); }
fn edge(a0: vec3<i32>, b0: vec3<i32>) -> vec3<f32> {
    var a=a0; var b=b0;
    if a.x>b.x || (a.x==b.x && a.y>b.y) || (a.x==b.x && a.y==b.y && a.z>b.z) { a=b0; b=a0; }
    let t=density(a)/(density(a)-density(b));
    return point(a)+t*(point(b)-point(a));
}
fn normal_at(p: vec3<f32>, fallback: vec3<f32>) -> vec3<f32> {
    let lattice=p-vec3(0.5)-vec3(params[8],params[9],params[10])+vec3(1.0);
    let cell=vec3<i32>(floor(lattice)); let f=fract(lattice);
    var n=vec3(0.0);
    for(var c=0u;c<8u;c=c+1u) {
        let corner=corners[c];
        let w=select(vec3(1.0)-f,f,corner==vec3(1));
        let s=cell+corner;
        n+=(w.x*w.y*w.z)*vec3(density(s+vec3(1,0,0))-density(s-vec3(1,0,0)),density(s+vec3(0,1,0))-density(s-vec3(0,1,0)),density(s+vec3(0,0,1))-density(s-vec3(0,0,1)));
    }
    if length(n)>1e-8 { return normalize(n); }
    return fallback;
}
fn vertex(slot:u32, p:vec3<f32>, n:vec3<f32>, m:u32) {
    let i=slot*10u; let q=p*params[3];
    vertices[i]=q.x; vertices[i+1u]=q.y; vertices[i+2u]=q.z;
    vertices[i+3u]=n.x;vertices[i+4u]=n.y;vertices[i+5u]=n.z;
    vertices[i+6u]=palette[m*4u];vertices[i+7u]=palette[m*4u+1u];vertices[i+8u]=palette[m*4u+2u];vertices[i+9u]=1.0;
}
@compute @workgroup_size(64) fn main(@builtin(global_invocation_id) id:vec3<u32>) {
    if id.x>=u32(params[12]) { return; }
    if params[11]==0.0 {
        let record=offsets[id.x*2u]; let dimensions=offsets[id.x*2u+1u];let m=(dimensions>>16u)&255u;
        let face=(record>>24u)&7u;
        let axis=face/2u; let sign=select(-1,1,(face%2u)==1u);
        let u=(axis+1u)%3u; let v=(axis+2u)%3u;
        var p=vec3(0.0);
        p[axis]=f32(record&255u);
        p[u]=f32((record>>8u)&255u);p[v]=f32((record>>16u)&255u);
        let width=f32(dimensions&255u);let height=f32((dimensions>>8u)&255u);
        var quad:array<vec3<f32>,4>;quad[0]=p;quad[1]=p;quad[1][u]+=width;quad[2]=quad[1];quad[2][v]+=height;quad[3]=p;quad[3][v]+=height;
        var n=vec3(0.0);n[axis]=f32(sign);
        let order=array<u32,6>(0u,1u,2u,0u,2u,3u);
        for(var i=0u;i<6u;i=i+1u) {
            var q=order[i];if sign<0 {q=order[(i/3u)*3u+select(i%3u,3u-i%3u,i%3u!=0u)];}
            vertex(id.x*6u+i,quad[q],n,m);
        }
        return;
    }
    let dims=vec3<u32>(u32(params[4]),u32(params[5]),u32(params[6]));
    let at=vec3<i32>(i32(id.x%dims.x),i32((id.x/dims.x)%dims.y),i32(id.x/(dims.x*dims.y)))+vec3(1);
    let start=offsets[id.x];let end=offsets[id.x+1u];
    if start==end { return; }
    for(var w=0u;w<360u;w=w+1u) {
        if w>=(end-start)*10u { break; }
        vertices[start*10u+w]=0.0;
    }
    var cursor=start;
    for(var t=0u;t<6u;t=t+1u) {
        var inside:array<vec3<i32>,4>;var outside:array<vec3<i32>,4>;var ni=0u;var no=0u;
        for(var c=0u;c<4u;c=c+1u) {
            let p=at+corners[tets[t][c]];
            if density(p)<0.0 {inside[ni]=p;ni+=1u;} else {outside[no]=p;no+=1u;}
        }
        if ni==0u || ni==4u {continue;}
        let m=material(inside[0]);if m==0u || palette[m*4u+3u]!=params[7] {continue;}
        var poly:array<vec3<f32>,4>;var count=3u;
        if ni==1u {poly[0]=edge(inside[0],outside[0]);poly[1]=edge(inside[0],outside[1]);poly[2]=edge(inside[0],outside[2]);}
        else if ni==2u {poly[0]=edge(inside[0],outside[0]);poly[1]=edge(inside[0],outside[1]);poly[2]=edge(inside[1],outside[1]);poly[3]=edge(inside[1],outside[0]);count=4u;}
        else {poly[0]=edge(inside[0],outside[0]);poly[1]=edge(inside[1],outside[0]);poly[2]=edge(inside[2],outside[0]);}
        let direction=point(outside[0])-point(inside[0]);
        let first=cursor;
        cursor+=(count-2u)*3u;
        for(var tri=0u;tri<2u;tri=tri+1u) {
            if tri+2u>=count {continue;}
            var a=poly[0];var b=poly[tri+1u];var c=poly[tri+2u];var n=cross(b-a,c-a);
            if dot(n,direction)<0.0 {let temp=b;b=c;c=temp;n=-n;}
            if length(n)<=1e-8 {continue;}
            n=normalize(n);let slot=first+tri*3u;
            vertex(slot,a,normal_at(a,n),m);vertex(slot+1u,b,normal_at(b,n),m);vertex(slot+2u,c,normal_at(c,n),m);
        }
    }
}
