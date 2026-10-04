//! Block registry: ids, names, colors, textures and custom models.
//!
//! Blocks are defined by JSON. Each definition names an id (1..255) or a new
//! name, a color, an optional emission, optional per-face textures (project
//! asset paths) and an optional model. Inline definitions live in the
//! world's `blocks` list; files under `assets/blocks/*.json` are imported
//! into that list by the editor, so the portable module only ever parses
//! the inline text.

use blockloom_plugin_sdk::Value;
use std::collections::BTreeMap;

/// A custom model for one block.
#[derive(Debug, Clone, PartialEq)]
pub enum BlockModel {
    /// A full cube, the default.
    Cube,
    /// One of the shaped cells (`slab`, `stair north`, ...).
    Shape(crate::grid::Shape),
    /// Raw triangles in a unit cell (0..1 per axis), with normals and
    /// optional uvs (one per vertex; planar x/y when absent).
    Mesh {
        positions: Vec<[f32; 3]>,
        normals: Vec<[f32; 3]>,
        indices: Vec<u32>,
        uvs: Vec<[f32; 2]>,
    },
}

/// One block definition.
#[derive(Debug, Clone, PartialEq)]
pub struct BlockDef {
    pub id: u8,
    pub name: String,
    pub color: String,
    pub emission: f64,
    /// Texture per face: top, bottom, side (or all). Project asset paths.
    pub textures: BTreeMap<String, String>,
    pub model: BlockModel,
}

impl BlockDef {
    /// The texture for one face: that face's entry, else `all`. Top never
    /// falls back to side, and side never to top.
    pub fn texture_face(&self, face: &str) -> Option<&str> {
        self.textures
            .get(face)
            .or_else(|| self.textures.get("all"))
            .map(String::as_str)
    }
}

/// The registry: id to definition, plus name lookup.
#[derive(Debug, Clone, Default)]
pub struct Registry {
    by_id: BTreeMap<u8, BlockDef>,
}

impl Registry {
    /// Built-in blocks, matching the palette order.
    pub fn built_ins() -> Vec<BlockDef> {
        let names = ["stone", "dirt", "grass", "sand", "wood", "leaves", "glow"];
        names
            .iter()
            .enumerate()
            .map(|(i, name)| BlockDef {
                id: (i + 1) as u8,
                name: (*name).to_string(),
                color: String::new(),
                emission: 0.0,
                textures: BTreeMap::new(),
                model: BlockModel::Cube,
            })
            .collect()
    }

    /// Parse inline JSON definitions over the built-ins.
    pub fn parse(defs: &[String]) -> Result<Registry, String> {
        let mut reg = Registry::default();
        for def in Self::built_ins() {
            reg.by_id.insert(def.id, def);
        }
        for (n, line) in defs.iter().enumerate() {
            let def = parse_one(line).map_err(|why| format!("block {}: {why}", n + 1))?;
            if def.id == 0 {
                return Err(format!("block {}: id 0 is air", n + 1));
            }
            if let Some(other) = reg.by_id.get(&def.id)
                && other.name != def.name
                && Self::built_ins().iter().any(|b| b.id == def.id)
            {
                // Replacing a built-in keeps its id but takes the new look.
            }
            // Names must stay unique.
            if reg
                .by_id
                .values()
                .any(|b| b.id != def.id && b.name == def.name)
            {
                return Err(format!("block {}: name {} is taken", n + 1, def.name));
            }
            reg.by_id.insert(def.id, def);
        }
        Ok(reg)
    }

    pub fn get(&self, id: u8) -> Option<&BlockDef> {
        self.by_id.get(&id)
    }

    pub fn id_of(&self, name: &str) -> Option<u8> {
        self.by_id
            .values()
            .find(|b| b.name.eq_ignore_ascii_case(name))
            .map(|b| b.id)
    }

    pub fn ids(&self) -> Vec<u8> {
        self.by_id.keys().copied().collect()
    }

    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.by_id.len()
    }
}

fn parse_one(line: &str) -> Result<BlockDef, String> {
    let v: Value = blockloom_plugin_sdk::serde_json::from_str(line).map_err(|e| e.to_string())?;
    let name = v["name"]
        .as_str()
        .ok_or("a block needs a name")?
        .to_string();
    if name.is_empty() || name.len() > 64 {
        return Err("a block name needs 1 to 64 characters".into());
    }
    let id = if let Some(id) = v.get("id").filter(|v| !v.is_null()) {
        id.as_u64()
            .filter(|id| (1..=255).contains(id))
            .map(|id| id as u8)
            .ok_or("a block id is 1 to 255")?
    } else if let Some(id) = Registry::built_ins()
        .iter()
        .find(|b| b.name == name)
        .map(|b| b.id)
    {
        id
    } else {
        return Err(format!("new block {name} needs an id from 1 to 255"));
    };
    let color = v["color"].as_str().unwrap_or("").to_string();
    if !color.is_empty() {
        crate::palette::linear(&color)?;
    }
    let emission = v["emission"].as_f64().unwrap_or(0.0);
    if !emission.is_finite() || !(0.0..=100.0).contains(&emission) {
        return Err("emission is 0 to 100".into());
    }
    let mut textures = BTreeMap::new();
    if let Some(obj) = v.get("textures").and_then(|t| t.as_object()) {
        for (face, path) in obj {
            let path = path.as_str().ok_or("a texture path is text")?;
            if !["top", "bottom", "side", "all"].contains(&face.as_str()) {
                return Err(format!("texture face {face} is top, bottom, side or all"));
            }
            if path.is_empty() || path.len() > 256 {
                return Err("a texture path needs 1 to 256 characters".into());
            }
            textures.insert(face.clone(), path.to_string());
        }
    }
    let model = parse_model(&v)?;
    Ok(BlockDef {
        id,
        name,
        color,
        emission,
        textures,
        model,
    })
}

fn parse_model(v: &Value) -> Result<BlockModel, String> {
    let m = v.get("model").unwrap_or(&Value::Null);
    if m.is_null() {
        return Ok(BlockModel::Cube);
    }
    if let Some(name) = m.as_str() {
        if name == "cube" {
            return Ok(BlockModel::Cube);
        }
        return Ok(BlockModel::Shape(crate::grid::Shape::from_name(name)?));
    }
    let kind = m["kind"].as_str().unwrap_or("cube");
    match kind {
        "cube" => Ok(BlockModel::Cube),
        "shape" => {
            let shape = m["shape"].as_str().unwrap_or("slab");
            Ok(BlockModel::Shape(crate::grid::Shape::from_name(shape)?))
        }
        "mesh" => {
            let pos = m["positions"]
                .as_array()
                .ok_or("a mesh model needs positions")?;
            if pos.len() % 3 != 0 || pos.is_empty() || pos.len() > 3 * 1024 {
                return Err("mesh positions are xyz triples, at most 1024 vertices".into());
            }
            let positions: Vec<[f32; 3]> = pos
                .chunks(3)
                .map(|c| {
                    [
                        c[0].as_f64().unwrap_or(0.0) as f32,
                        c[1].as_f64().unwrap_or(0.0) as f32,
                        c[2].as_f64().unwrap_or(0.0) as f32,
                    ]
                })
                .collect();
            if positions.iter().flatten().any(|v| !v.is_finite()) {
                return Err("mesh positions must be finite".into());
            }
            let normals = if let Some(arr) = m.get("normals").and_then(|n| n.as_array()) {
                if arr.len() != pos.len() {
                    return Err("mesh normals match positions".into());
                }
                arr.chunks(3)
                    .map(|c| {
                        [
                            c[0].as_f64().unwrap_or(0.0) as f32,
                            c[1].as_f64().unwrap_or(0.0) as f32,
                            c[2].as_f64().unwrap_or(1.0) as f32,
                        ]
                    })
                    .collect()
            } else {
                vec![[0.0, 1.0, 0.0]; positions.len()]
            };
            let indices = if let Some(arr) = m.get("indices").and_then(|i| i.as_array()) {
                if arr.len() % 3 != 0 || arr.is_empty() {
                    return Err("mesh indices are triangles".into());
                }
                arr.iter()
                    .map(|i| {
                        i.as_u64()
                            .filter(|i| (*i as usize) < positions.len())
                            .map(|i| i as u32)
                            .ok_or("a mesh index is past its vertices".to_string())
                    })
                    .collect::<Result<Vec<_>, _>>()?
            } else {
                (0..positions.len() as u32).collect()
            };
            let uvs = if let Some(arr) = m.get("uvs").and_then(|u| u.as_array()) {
                if arr.len() != positions.len() * 2 {
                    return Err("mesh uvs are one xy pair per vertex".into());
                }
                let uvs: Vec<[f32; 2]> = arr
                    .chunks(2)
                    .map(|c| {
                        [
                            c[0].as_f64().unwrap_or(0.0) as f32,
                            c[1].as_f64().unwrap_or(0.0) as f32,
                        ]
                    })
                    .collect();
                if uvs.iter().flatten().any(|v| !v.is_finite()) {
                    return Err("mesh uvs must be finite".into());
                }
                uvs
            } else {
                Vec::new()
            };
            Ok(BlockModel::Mesh {
                positions,
                normals,
                indices,
                uvs,
            })
        }
        other => Err(format!("unknown model kind {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_ins_cover_the_palette() {
        let reg = Registry::parse(&[]).unwrap();
        assert_eq!(reg.len(), 7);
        assert_eq!(reg.id_of("Grass"), Some(3));
    }

    #[test]
    fn a_json_file_defines_a_textured_block_with_a_model() {
        let reg = Registry::parse(&[
            r##"{"name":"mossy_stone","id":8,"color":"#6a7a5a","textures":{"all":"assets/textures/mossy_stone.png"},"model":"cube"}"##.to_string(),
            r##"{"name":"lantern","id":9,"textures":{"all":"assets/textures/lantern.png"},"model":{"kind":"mesh","positions":[0,0,0, 1,0,0, 0,1,0],"normals":[0,0,1, 0,0,1, 0,0,1],"indices":[0,1,2]}}"##.to_string(),
        ])
        .unwrap();
        assert_eq!(reg.len(), 9);
        let mossy = reg.get(8).unwrap();
        assert_eq!(
            mossy.texture_face("top"),
            Some("assets/textures/mossy_stone.png")
        );
        assert!(matches!(reg.get(9).unwrap().model, BlockModel::Mesh { .. }));
    }

    #[test]
    fn bad_definitions_are_refused() {
        assert!(Registry::parse(&["not json".to_string()]).is_err());
        assert!(Registry::parse(&[r#"{"name":"","id":8}"#.to_string()]).is_err());
        assert!(Registry::parse(&[r#"{"name":"x","id":0}"#.to_string()]).is_err());
        assert!(Registry::parse(&[r#"{"name":"stone","id":8}"#.to_string()]).is_err());
        assert!(Registry::parse(&[r#"{"name":"new","id":300}"#.to_string()]).is_err());
    }

    #[test]
    fn face_textures_fall_back_to_all_but_never_sideways() {
        let reg = Registry::parse(&[
            r##"{"name":"tuft","id":10,"textures":{"top":"t.png","side":"s.png"}}"##.to_string(),
        ])
        .unwrap();
        let tuft = reg.get(10).unwrap();
        assert_eq!(tuft.texture_face("top"), Some("t.png"));
        assert_eq!(tuft.texture_face("side"), Some("s.png"));
        assert_eq!(tuft.texture_face("bottom"), None);
    }

    #[test]
    fn mesh_model_uvs_match_vertices_or_fail() {
        let ok = r##"{"name":"frame","id":11,"model":{"kind":"mesh","positions":[0,0,0, 1,0,0, 0,1,0],"uvs":[0,0, 1,0, 0,1]}}"##;
        let reg = Registry::parse(&[ok.to_string()]).unwrap();
        match &reg.get(11).unwrap().model {
            BlockModel::Mesh { uvs, .. } => assert_eq!(uvs.len(), 3),
            _ => panic!("a mesh model"),
        }
        let bad = r##"{"name":"bad","id":12,"model":{"kind":"mesh","positions":[0,0,0, 1,0,0, 0,1,0],"uvs":[0,0]}}"##;
        assert!(Registry::parse(&[bad.to_string()]).is_err());
    }
}
