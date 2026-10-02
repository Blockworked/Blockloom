//! Triangles out of a model file, for collision only: positions and indices, with
//! the scene's node transforms baked in. Materials, normals, UVs, skins and
//! animation are not read; a skinned mesh collides in its bind pose.

use super::{CookError, RawMesh, fail};
use crate::pipeline::{ModelFormat, detect_format};
use base64::Engine as _;
use glam::{Mat4, Quat, Vec3};
use serde_json::Value;

/// Reads the triangles of a model. `resolve` fetches a file next to the model
/// (an external `.bin`) by its relative name.
pub fn load_mesh(
    name: &str,
    bytes: &[u8],
    resolve: &dyn Fn(&str) -> Option<Vec<u8>>,
) -> Result<RawMesh, CookError> {
    match detect_format(name, bytes) {
        ModelFormat::Obj => load_obj(name, bytes),
        ModelFormat::Glb => {
            let (json, bin) = split_glb(name, bytes)?;
            load_gltf(name, &json, bin, resolve)
        }
        ModelFormat::Gltf => {
            let text = std::str::from_utf8(bytes)
                .map_err(|_| CookError::Message(format!("{name} isn't UTF-8 glTF JSON")))?;
            let json = serde_json::from_str(text)
                .map_err(|e| CookError::Message(format!("{name}: bad glTF JSON: {e}")))?;
            load_gltf(name, &json, None, resolve)
        }
        ModelFormat::Fbx => fail(format!(
            "{name} is an FBX file; collision reads .gltf, .glb and .obj"
        )),
        ModelFormat::Unknown => fail(format!(
            "{name} isn't a model Blockloom reads collision from (.gltf, .glb, .obj)"
        )),
    }
}

fn split_glb<'a>(name: &str, bytes: &'a [u8]) -> Result<(Value, Option<&'a [u8]>), CookError> {
    if bytes.len() < 20 {
        return fail(format!("{name} is too short to be a .glb"));
    }
    let total = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
    let end = total.min(bytes.len());
    let mut at = 12;
    let mut json = None;
    let mut bin = None;
    while at + 8 <= end {
        let len = u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
        let kind = &bytes[at + 4..at + 8];
        let body_end = (at + 8).saturating_add(len);
        if body_end > end {
            return fail(format!(
                "{name}: a .glb chunk runs past the end of the file"
            ));
        }
        let body = &bytes[at + 8..body_end];
        match kind {
            b"JSON" => json = Some(body),
            b"BIN\0" if bin.is_none() => bin = Some(body),
            _ => {}
        }
        at = body_end.next_multiple_of(4);
    }
    let json = json.ok_or_else(|| CookError::Message(format!("{name}: no JSON chunk")))?;
    let text = std::str::from_utf8(json)
        .map_err(|_| CookError::Message(format!("{name}: bad glTF JSON chunk")))?;
    let value = serde_json::from_str(text)
        .map_err(|e| CookError::Message(format!("{name}: bad glTF JSON: {e}")))?;
    Ok((value, bin))
}

fn array<'a>(json: &'a Value, key: &str) -> &'a [Value] {
    json.get(key)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

fn uint(value: &Value, key: &str) -> Option<usize> {
    value.get(key).and_then(Value::as_u64).map(|v| v as usize)
}

fn load_gltf(
    name: &str,
    json: &Value,
    glb_bin: Option<&[u8]>,
    resolve: &dyn Fn(&str) -> Option<Vec<u8>>,
) -> Result<RawMesh, CookError> {
    let required = |extension: &str| {
        array(json, "extensionsRequired")
            .iter()
            .any(|e| e.as_str() == Some(extension))
    };
    for unsupported in ["KHR_draco_mesh_compression", "EXT_meshopt_compression"] {
        if required(unsupported) {
            return fail(format!(
                "{name} needs {unsupported}, which collision cooking does not read; export it uncompressed"
            ));
        }
    }
    let mut buffers: Vec<Vec<u8>> = Vec::new();
    for (n, buffer) in array(json, "buffers").iter().enumerate() {
        let data = match buffer.get("uri").and_then(Value::as_str) {
            None if n == 0 => glb_bin.map(<[u8]>::to_vec),
            None => None,
            Some(uri) => match uri.strip_prefix("data:") {
                Some(rest) => rest.split_once(";base64,").and_then(|(_, data)| {
                    base64::engine::general_purpose::STANDARD.decode(data).ok()
                }),
                None => resolve(&uri.replace("%20", " ")),
            },
        };
        let data = data
            .ok_or_else(|| CookError::Message(format!("{name}: buffer {n} could not be read")))?;
        if let Some(len) = uint(buffer, "byteLength")
            && data.len() < len
        {
            return fail(format!("{name}: buffer {n} is shorter than it says"));
        }
        buffers.push(data);
    }

    let nodes = array(json, "nodes");
    let scene_roots: Vec<usize> = {
        let scenes = array(json, "scenes");
        let chosen = uint(json, "scene")
            .and_then(|i| scenes.get(i))
            .or(scenes.first());
        match chosen {
            Some(scene) => array(scene, "nodes")
                .iter()
                .filter_map(|n| n.as_u64().map(|n| n as usize))
                .collect(),
            None => {
                // No scene: every node nothing else lists as a child is a root.
                let children: std::collections::HashSet<usize> = nodes
                    .iter()
                    .flat_map(|n| array(n, "children"))
                    .filter_map(|c| c.as_u64().map(|c| c as usize))
                    .collect();
                (0..nodes.len()).filter(|n| !children.contains(n)).collect()
            }
        }
    };

    let mut out = RawMesh::default();
    let mut stack: Vec<(usize, Mat4, usize)> = scene_roots
        .into_iter()
        .rev()
        .map(|n| (n, Mat4::IDENTITY, 0))
        .collect();
    let mut visited = 0usize;
    while let Some((n, parent, depth)) = stack.pop() {
        visited += 1;
        if depth > 64 || visited > nodes.len().saturating_mul(4).max(64) {
            return fail(format!("{name}: the node tree loops or is too deep"));
        }
        let Some(node) = nodes.get(n) else {
            return fail(format!(
                "{name}: a scene names node {n}, which does not exist"
            ));
        };
        let world = parent * node_matrix(node);
        if let Some(mesh) = uint(node, "mesh") {
            append_mesh(name, json, &buffers, mesh, &world, &mut out)?;
        }
        for child in array(node, "children").iter().rev() {
            if let Some(child) = child.as_u64() {
                stack.push((child as usize, world, depth + 1));
            }
        }
    }
    if out.indices.is_empty() {
        return fail(format!("{name} has no triangles to collide with"));
    }
    Ok(out)
}

fn node_matrix(node: &Value) -> Mat4 {
    if let Some(m) = node.get("matrix").and_then(Value::as_array)
        && m.len() == 16
    {
        let mut a = [0f32; 16];
        for (slot, v) in a.iter_mut().zip(m) {
            *slot = v.as_f64().unwrap_or(0.0) as f32;
        }
        return Mat4::from_cols_array(&a);
    }
    let vec3 = |key: &str, default: Vec3| {
        node.get(key)
            .and_then(Value::as_array)
            .filter(|a| a.len() == 3)
            .map(|a| Vec3::new(f(&a[0]), f(&a[1]), f(&a[2])))
            .unwrap_or(default)
    };
    let rotation = node
        .get("rotation")
        .and_then(Value::as_array)
        .filter(|a| a.len() == 4)
        .map(|a| Quat::from_xyzw(f(&a[0]), f(&a[1]), f(&a[2]), f(&a[3])).normalize())
        .unwrap_or(Quat::IDENTITY);
    Mat4::from_scale_rotation_translation(
        vec3("scale", Vec3::ONE),
        rotation,
        vec3("translation", Vec3::ZERO),
    )
}

fn f(v: &Value) -> f32 {
    v.as_f64().unwrap_or(0.0) as f32
}

fn append_mesh(
    name: &str,
    json: &Value,
    buffers: &[Vec<u8>],
    mesh: usize,
    world: &Mat4,
    out: &mut RawMesh,
) -> Result<(), CookError> {
    let Some(mesh) = array(json, "meshes").get(mesh) else {
        return fail(format!("{name}: a node names a mesh that does not exist"));
    };
    // A mirrored node turns triangles inside out; put the winding back.
    let mirrored = world.determinant() < 0.0;
    for primitive in array(mesh, "primitives") {
        let mode = uint(primitive, "mode").unwrap_or(4);
        if mode < 4 {
            continue;
        }
        let Some(position) = primitive
            .get("attributes")
            .and_then(|a| a.get("POSITION"))
            .and_then(Value::as_u64)
        else {
            continue;
        };
        let points = read_vec3(name, json, buffers, position as usize)?;
        let base = out.positions.len() as u32;
        out.positions.extend(
            points
                .iter()
                .map(|p| world.transform_point3(Vec3::from(*p)).to_array()),
        );
        let raw: Vec<u32> = match uint(primitive, "indices") {
            Some(accessor) => read_indices(name, json, buffers, accessor)?,
            None => (0..points.len() as u32).collect(),
        };
        let tris: Vec<[u32; 3]> = match mode {
            4 => raw.chunks_exact(3).map(|t| [t[0], t[1], t[2]]).collect(),
            5 => (0..raw.len().saturating_sub(2))
                .map(|i| {
                    if i % 2 == 0 {
                        [raw[i], raw[i + 1], raw[i + 2]]
                    } else {
                        [raw[i + 1], raw[i], raw[i + 2]]
                    }
                })
                .collect(),
            _ => (1..raw.len().saturating_sub(1))
                .map(|i| [raw[0], raw[i], raw[i + 1]])
                .collect(),
        };
        for t in tris {
            if t.iter().any(|i| *i as usize >= points.len()) {
                return fail(format!(
                    "{name}: a triangle names a vertex that is not there"
                ));
            }
            let t = if mirrored { [t[0], t[2], t[1]] } else { t };
            out.indices.extend(t.map(|i| base + i));
        }
    }
    Ok(())
}

struct View<'a> {
    bytes: &'a [u8],
    count: usize,
    component: usize,
    components: usize,
    stride: usize,
    kind: usize,
    normalized: bool,
}

fn view<'a>(
    name: &str,
    json: &Value,
    buffers: &'a [Vec<u8>],
    accessor: usize,
) -> Result<View<'a>, CookError> {
    let bad = |why: &str| CookError::Message(format!("{name}: accessor {accessor} {why}"));
    let a = array(json, "accessors")
        .get(accessor)
        .ok_or_else(|| bad("does not exist"))?;
    if a.get("sparse").is_some() {
        return Err(bad("is sparse, which collision cooking does not read"));
    }
    let count = uint(a, "count").ok_or_else(|| bad("has no count"))?;
    let kind = uint(a, "componentType").ok_or_else(|| bad("has no component type"))?;
    let component = match kind {
        5120 | 5121 => 1,
        5122 | 5123 => 2,
        5125 | 5126 => 4,
        _ => return Err(bad("has an unknown component type")),
    };
    let components = match a.get("type").and_then(Value::as_str) {
        Some("SCALAR") => 1,
        Some("VEC2") => 2,
        Some("VEC3") => 3,
        Some("VEC4") => 4,
        _ => return Err(bad("has an unsupported type")),
    };
    let Some(buffer_view) = uint(a, "bufferView") else {
        return Err(bad(
            "has no buffer view (all zeros accessors are not supported)",
        ));
    };
    let bv = array(json, "bufferViews")
        .get(buffer_view)
        .ok_or_else(|| bad("names a missing buffer view"))?;
    let buffer = uint(bv, "buffer").unwrap_or(0);
    let data = buffers
        .get(buffer)
        .ok_or_else(|| bad("names a missing buffer"))?;
    let start = uint(bv, "byteOffset").unwrap_or(0) + uint(a, "byteOffset").unwrap_or(0);
    let stride = uint(bv, "byteStride").unwrap_or(component * components);
    let end = if count == 0 {
        start
    } else {
        start + stride * (count - 1) + component * components
    };
    if end > data.len() {
        return Err(bad("runs past the end of its buffer"));
    }
    Ok(View {
        bytes: &data[start..end.max(start)],
        count,
        component,
        components,
        stride,
        kind,
        normalized: a
            .get("normalized")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    })
}

impl View<'_> {
    fn float(&self, row: usize, column: usize) -> f32 {
        let at = row * self.stride + column * self.component;
        let b = &self.bytes[at..at + self.component];
        let raw = match self.kind {
            5126 => return f32::from_le_bytes(b.try_into().unwrap()),
            5120 => i8::from_le_bytes([b[0]]) as f32,
            5121 => b[0] as f32,
            5122 => i16::from_le_bytes([b[0], b[1]]) as f32,
            5123 => u16::from_le_bytes([b[0], b[1]]) as f32,
            _ => u32::from_le_bytes(b.try_into().unwrap()) as f32,
        };
        if !self.normalized {
            return raw;
        }
        match self.kind {
            5120 => (raw / 127.0).max(-1.0),
            5121 => raw / 255.0,
            5122 => (raw / 32767.0).max(-1.0),
            5123 => raw / 65535.0,
            _ => raw,
        }
    }

    fn index(&self, row: usize) -> u32 {
        let at = row * self.stride;
        let b = &self.bytes[at..at + self.component];
        match self.component {
            1 => b[0] as u32,
            2 => u16::from_le_bytes([b[0], b[1]]) as u32,
            _ => u32::from_le_bytes(b.try_into().unwrap()),
        }
    }
}

fn read_vec3(
    name: &str,
    json: &Value,
    buffers: &[Vec<u8>],
    accessor: usize,
) -> Result<Vec<[f32; 3]>, CookError> {
    let v = view(name, json, buffers, accessor)?;
    if v.components != 3 {
        return fail(format!("{name}: POSITION is not a VEC3"));
    }
    Ok((0..v.count)
        .map(|i| [v.float(i, 0), v.float(i, 1), v.float(i, 2)])
        .collect())
}

fn read_indices(
    name: &str,
    json: &Value,
    buffers: &[Vec<u8>],
    accessor: usize,
) -> Result<Vec<u32>, CookError> {
    let v = view(name, json, buffers, accessor)?;
    if v.components != 1 || v.kind == 5126 || v.kind == 5120 || v.kind == 5122 {
        return fail(format!(
            "{name}: the index accessor is not an unsigned integer list"
        ));
    }
    Ok((0..v.count).map(|i| v.index(i)).collect())
}

fn load_obj(name: &str, bytes: &[u8]) -> Result<RawMesh, CookError> {
    let text = String::from_utf8_lossy(bytes);
    let mut out = RawMesh::default();
    for (n, line) in text.lines().enumerate() {
        let line = line.split('#').next().unwrap_or("").trim();
        let mut words = line.split_whitespace();
        match words.next() {
            Some("v") => {
                let v: Vec<f32> = words.take(3).filter_map(|w| w.parse().ok()).collect();
                if v.len() != 3 {
                    return fail(format!("{name}:{}: a vertex needs three numbers", n + 1));
                }
                out.positions.push([v[0], v[1], v[2]]);
            }
            Some("f") => {
                let mut face = Vec::new();
                for word in words {
                    let first = word.split('/').next().unwrap_or("");
                    let i: i64 = first.parse().map_err(|_| {
                        CookError::Message(format!("{name}:{}: bad face index {first:?}", n + 1))
                    })?;
                    let index = if i < 0 {
                        out.positions.len() as i64 + i
                    } else {
                        i - 1
                    };
                    if index < 0 || index as usize >= out.positions.len() {
                        return fail(format!(
                            "{name}:{}: a face names a vertex that is not defined yet",
                            n + 1
                        ));
                    }
                    face.push(index as u32);
                }
                if face.len() < 3 {
                    return fail(format!("{name}:{}: a face needs three vertices", n + 1));
                }
                for k in 1..face.len() - 1 {
                    out.indices.extend([face[0], face[k], face[k + 1]]);
                }
            }
            _ => {}
        }
    }
    if out.indices.is_empty() {
        return fail(format!("{name} has no faces to collide with"));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn none(_: &str) -> Option<Vec<u8>> {
        None
    }

    fn quad_gltf(extra_node: &str, buffer: &str) -> String {
        format!(
            r#"{{
              "asset": {{"version": "2.0"}},
              "scene": 0,
              "scenes": [{{"nodes": [0]}}],
              "nodes": [{{"mesh": 0 {extra_node}}}],
              "meshes": [{{"primitives": [{{"attributes": {{"POSITION": 0}}, "indices": 1}}]}}],
              "accessors": [
                {{"bufferView": 0, "componentType": 5126, "count": 4, "type": "VEC3"}},
                {{"bufferView": 1, "componentType": 5123, "count": 6, "type": "SCALAR"}}
              ],
              "bufferViews": [
                {{"buffer": 0, "byteOffset": 0, "byteLength": 48}},
                {{"buffer": 0, "byteOffset": 48, "byteLength": 12}}
              ],
              "buffers": [{{"byteLength": 60, "uri": "{buffer}"}}]
            }}"#
        )
    }

    fn quad_bytes() -> Vec<u8> {
        let mut bytes = Vec::new();
        for p in [
            [0.0f32, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
        ] {
            for v in p {
                bytes.extend(v.to_le_bytes());
            }
        }
        for i in [0u16, 1, 2, 0, 2, 3] {
            bytes.extend(i.to_le_bytes());
        }
        bytes
    }

    fn data_uri() -> String {
        format!(
            "data:application/octet-stream;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(quad_bytes())
        )
    }

    #[test]
    fn a_gltf_quad_loads_with_its_node_transform_baked_in() {
        let json = quad_gltf(
            r#", "translation": [10, 0, 0], "scale": [2, 2, 2]"#,
            &data_uri(),
        );
        let mesh = load_mesh("quad.gltf", json.as_bytes(), &none).unwrap();
        assert_eq!(mesh.indices.len(), 6);
        assert_eq!(mesh.positions[2], [12.0, 2.0, 0.0]);
    }

    #[test]
    fn a_mirrored_node_keeps_its_triangles_facing_out() {
        let json = quad_gltf(r#", "scale": [-1, 1, 1]"#, &data_uri());
        let mesh = load_mesh("quad.gltf", json.as_bytes(), &none).unwrap();
        assert_eq!(&mesh.indices[..3], &[0, 2, 1]);
    }

    #[test]
    fn an_external_buffer_is_fetched_by_name() {
        let json = quad_gltf("", "quad.bin");
        let bin = quad_bytes();
        let mesh = load_mesh("quad.gltf", json.as_bytes(), &|n| {
            (n == "quad.bin").then(|| bin.clone())
        })
        .unwrap();
        assert_eq!(mesh.indices.len(), 6);
        let missing = load_mesh("quad.gltf", json.as_bytes(), &none).unwrap_err();
        assert!(missing.to_string().contains("buffer 0"), "{missing}");
    }

    #[test]
    fn a_glb_reads_its_binary_chunk() {
        let json = quad_gltf("", "").replace(r#", "uri": """#, "");
        let mut json = json.into_bytes();
        while json.len() % 4 != 0 {
            json.push(b' ');
        }
        let bin = quad_bytes();
        let mut out = Vec::new();
        out.extend(b"glTF");
        out.extend(2u32.to_le_bytes());
        out.extend(((12 + 8 + json.len() + 8 + bin.len()) as u32).to_le_bytes());
        out.extend((json.len() as u32).to_le_bytes());
        out.extend(b"JSON");
        out.extend(&json);
        out.extend((bin.len() as u32).to_le_bytes());
        out.extend(b"BIN\0");
        out.extend(&bin);
        let mesh = load_mesh("quad.glb", &out, &none).unwrap();
        assert_eq!(mesh.positions.len(), 4);
        assert_eq!(mesh.indices.len(), 6);
    }

    #[test]
    fn compressed_or_broken_files_say_what_is_wrong() {
        let draco = quad_gltf("", &data_uri()).replace(
            r#""scene": 0"#,
            r#""scene": 0, "extensionsRequired": ["KHR_draco_mesh_compression"]"#,
        );
        let err = load_mesh("m.gltf", draco.as_bytes(), &none).unwrap_err();
        assert!(err.to_string().contains("KHR_draco"), "{err}");
        assert!(load_mesh("m.gltf", b"{not json", &none).is_err());
        assert!(load_mesh("m.fbx", b"Kaydara FBX Binary  \0", &none).is_err());
    }

    #[test]
    fn an_obj_triangulates_polygons_and_reads_negative_indices() {
        let obj =
            "# a quad\nv 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nf 1/1/1 2/2/1 3/3/1 4/4/1\nf -4 -3 -2\n";
        let mesh = load_mesh("quad.obj", obj.as_bytes(), &none).unwrap();
        assert_eq!(mesh.positions.len(), 4);
        assert_eq!(mesh.indices, vec![0, 1, 2, 0, 2, 3, 0, 1, 2]);
        let bad = load_mesh("bad.obj", b"v 0 0 0\nf 1 2 3\n", &none).unwrap_err();
        assert!(bad.to_string().contains("not defined"), "{bad}");
    }
}
