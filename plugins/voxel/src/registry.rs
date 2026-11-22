//! Block registry: ids, names, colors, textures and custom models.
//!
//! A definition with `properties` is a block type with states: each
//! combination of property values takes its own id, counted up from `id` with
//! the last property changing fastest, and `variants` (`{when, ...}`) lay a
//! model, textures, color or emission over the states they match. A state is
//! named `name[prop=value,...]`; the bare name is the default state.
//!
//! Blocks are defined by JSON. Each definition names an id (1..65534) or a new
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
    pub id: crate::grid::Block,
    pub name: String,
    pub color: String,
    pub emission: f64,
    /// Texture per face: top, bottom, side (or all). Project asset paths.
    pub textures: BTreeMap<String, String>,
    pub model: BlockModel,
    /// The block type this state belongs to (the name for a stateless block).
    pub type_name: String,
    /// This state's property values, in the type's property order.
    pub state: Vec<(String, String)>,
    pub tags: Vec<String>,
    pub layer: Layer,
}

/// How a block draws: solid, alpha-tested or blended (used by M3 rendering).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Layer {
    #[default]
    Opaque,
    Cutout,
    Translucent,
}

impl Layer {
    pub fn name(self) -> &'static str {
        match self {
            Layer::Opaque => "opaque",
            Layer::Cutout => "cutout",
            Layer::Translucent => "translucent",
        }
    }
}

/// A block type's states: its first id, properties and default state.
#[derive(Debug, Clone, PartialEq)]
pub struct BlockType {
    pub name: String,
    pub base: crate::grid::Block,
    pub properties: Vec<(String, Vec<String>)>,
    pub default: crate::grid::Block,
}

impl BlockType {
    pub fn states(&self) -> usize {
        self.properties.iter().map(|(_, v)| v.len()).product()
    }

    /// The id for a state: given values, the default's for the rest.
    pub fn id_for(&self, values: &[(String, String)]) -> Result<crate::grid::Block, String> {
        let mut index = 0usize;
        let defaults = self.values_of(self.default);
        for (k, (prop, options)) in self.properties.iter().enumerate() {
            let value = match values.iter().find(|(p, _)| p == prop) {
                Some((_, v)) => v.as_str(),
                None => defaults[k].1.as_str(),
            };
            let at = options
                .iter()
                .position(|o| o == value)
                .ok_or_else(|| format!("{} has no {prop}={value}", self.name))?;
            index = index * options.len() + at;
        }
        if let Some((p, _)) = values
            .iter()
            .find(|(p, _)| !self.properties.iter().any(|(q, _)| q == p))
        {
            return Err(format!("{} has no property {p}", self.name));
        }
        Ok(self.base + index as crate::grid::Block)
    }

    pub fn values_of(&self, id: crate::grid::Block) -> Vec<(String, String)> {
        let mut index = usize::from(id - self.base);
        let mut out = vec![(String::new(), String::new()); self.properties.len()];
        for (k, (prop, options)) in self.properties.iter().enumerate().rev() {
            out[k] = (prop.clone(), options[index % options.len()].clone());
            index /= options.len();
        }
        out
    }
}

/// `name[a=b,c=d]` for a state, the bare name for no properties.
pub fn state_name(name: &str, values: &[(String, String)]) -> String {
    if values.is_empty() {
        return name.to_string();
    }
    let inner: Vec<String> = values.iter().map(|(k, v)| format!("{k}={v}")).collect();
    format!("{name}[{}]", inner.join(","))
}

/// Splits `name[a=b,...]` into the name and its values.
pub fn parse_state(text: &str) -> Result<(&str, Vec<(String, String)>), String> {
    let Some((name, rest)) = text.split_once('[') else {
        return Ok((text, Vec::new()));
    };
    let inner = rest.strip_suffix(']').ok_or("a block state ends with ]")?;
    let mut values = Vec::new();
    for pair in inner.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        let (k, v) = pair
            .split_once('=')
            .ok_or_else(|| format!("{pair} is not prop=value"))?;
        values.push((k.trim().to_string(), v.trim().to_string()));
    }
    Ok((name, values))
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
    by_id: BTreeMap<crate::grid::Block, BlockDef>,
    types: BTreeMap<String, BlockType>,
}

impl Registry {
    /// Built-in blocks, matching the palette order.
    pub fn built_ins() -> Vec<BlockDef> {
        let names = ["stone", "dirt", "grass", "sand", "wood", "leaves", "glow"];
        names
            .iter()
            .enumerate()
            .map(|(i, name)| BlockDef {
                id: (i + 1) as crate::grid::Block,
                name: (*name).to_string(),
                color: String::new(),
                emission: 0.0,
                textures: BTreeMap::new(),
                model: BlockModel::Cube,
                type_name: (*name).to_string(),
                state: Vec::new(),
                tags: Vec::new(),
                layer: Layer::Opaque,
            })
            .collect()
    }

    /// Parse inline JSON definitions over the built-ins.
    pub fn parse(defs: &[String]) -> Result<Registry, String> {
        let mut reg = Registry::default();
        for def in Self::built_ins() {
            reg.by_id.insert(def.id, def);
        }
        let mut defined = std::collections::BTreeSet::new();
        for (n, line) in defs.iter().enumerate() {
            let (states, ty) = parse_one(line).map_err(|why| format!("block {}: {why}", n + 1))?;
            if reg.types.contains_key(&states[0].type_name) {
                return Err(format!(
                    "block {}: name {} is taken",
                    n + 1,
                    states[0].type_name
                ));
            }
            let ids: std::collections::BTreeSet<_> = states.iter().map(|d| d.id).collect();
            // Names must stay unique (a built-in may be redefined in place).
            if reg
                .by_id
                .values()
                .any(|b| !ids.contains(&b.id) && b.type_name == states[0].type_name)
            {
                return Err(format!(
                    "block {}: name {} is taken",
                    n + 1,
                    states[0].type_name
                ));
            }
            for def in states {
                if def.id == 0 {
                    return Err(format!("block {}: id 0 is air", n + 1));
                }
                // A built-in's id may be taken over; another line's may not.
                if !defined.insert(def.id) {
                    return Err(format!("block {}: id {} is taken", n + 1, def.id));
                }
                reg.by_id.insert(def.id, def);
            }
            if let Some(ty) = ty {
                reg.types.insert(ty.name.clone(), ty);
            }
        }
        Ok(reg)
    }

    pub fn get(&self, id: crate::grid::Block) -> Option<&BlockDef> {
        self.by_id.get(&id)
    }

    /// A block's id by name, or a state's by `name[prop=value,...]`.
    pub fn id_of(&self, name: &str) -> Option<crate::grid::Block> {
        self.state_id(name).ok()
    }

    /// Like `id_of`, saying why a state doesn't resolve.
    pub fn state_id(&self, text: &str) -> Result<crate::grid::Block, String> {
        let (name, values) = parse_state(text)?;
        if let Some(ty) = self
            .types
            .values()
            .find(|t| t.name.eq_ignore_ascii_case(name))
        {
            return ty.id_for(&values);
        }
        let id = self
            .by_id
            .values()
            .find(|b| b.name.eq_ignore_ascii_case(name))
            .map(|b| b.id)
            .ok_or_else(|| format!("no block called {name}"))?;
        if !values.is_empty() {
            return Err(format!("{name} has no properties"));
        }
        Ok(id)
    }

    pub fn types(&self) -> impl Iterator<Item = &BlockType> {
        self.types.values()
    }

    #[cfg(test)]
    pub fn block_type(&self, name: &str) -> Option<&BlockType> {
        self.types.get(name)
    }

    /// The type a state id belongs to, if it has properties.
    #[cfg(test)]
    pub fn type_of(&self, id: crate::grid::Block) -> Option<&BlockType> {
        self.by_id
            .get(&id)
            .and_then(|def| self.types.get(&def.type_name))
    }

    pub fn ids(&self) -> Vec<crate::grid::Block> {
        self.by_id.keys().copied().collect()
    }

    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.by_id.len()
    }
}

const MAX_PROPERTIES: usize = 8;
const MAX_VALUES: usize = 32;
const MAX_STATES: usize = 4096;

fn plain_word(text: &str, what: &str) -> Result<String, String> {
    if text.is_empty()
        || text.len() > 32
        || !text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(format!(
            "{what} {text:?} needs 1 to 32 letters, digits, _ or -"
        ));
    }
    Ok(text.to_string())
}

/// What a definition (or a variant over it) says about looks.
fn parse_look(v: &Value, def: &mut BlockDef) -> Result<(), String> {
    if let Some(color) = v.get("color").and_then(Value::as_str) {
        if !color.is_empty() {
            crate::palette::linear(color)?;
        }
        def.color = color.to_string();
    }
    if let Some(emission) = v.get("emission").filter(|e| !e.is_null()) {
        let emission = emission.as_f64().unwrap_or(f64::NAN);
        if !emission.is_finite() || !(0.0..=100.0).contains(&emission) {
            return Err("emission is 0 to 100".into());
        }
        def.emission = emission;
    }
    if let Some(obj) = v.get("textures").and_then(|t| t.as_object()) {
        for (face, path) in obj {
            let path = path.as_str().ok_or("a texture path is text")?;
            if !["top", "bottom", "side", "all"].contains(&face.as_str()) {
                return Err(format!("texture face {face} is top, bottom, side or all"));
            }
            if path.is_empty() || path.len() > 256 {
                return Err("a texture path needs 1 to 256 characters".into());
            }
            def.textures.insert(face.clone(), path.to_string());
        }
    }
    if v.get("model").is_some_and(|m| !m.is_null()) {
        def.model = parse_model(v)?;
    }
    Ok(())
}

fn parse_one(line: &str) -> Result<(Vec<BlockDef>, Option<BlockType>), String> {
    let v: Value = blockloom_plugin_sdk::serde_json::from_str(line).map_err(|e| e.to_string())?;
    let name = v["name"]
        .as_str()
        .ok_or("a block needs a name")?
        .to_string();
    if name.is_empty() || name.len() > 64 || name.contains(['[', ']', '=', ',']) {
        return Err("a block name needs 1 to 64 characters and no [ ] = ,".into());
    }
    let id = if let Some(id) = v.get("id").filter(|v| !v.is_null()) {
        id.as_u64()
            .filter(|id| (1..=u64::from(crate::grid::MAX_BLOCK)).contains(id))
            .map(|id| id as crate::grid::Block)
            .ok_or("a block id is 1 to 65534")?
    } else if let Some(id) = Registry::built_ins()
        .iter()
        .find(|b| b.name == name)
        .map(|b| b.id)
    {
        id
    } else {
        return Err(format!("new block {name} needs an id from 1 to 65534"));
    };
    let tags = match v.get("tags").filter(|t| !t.is_null()) {
        None => Vec::new(),
        Some(t) => t
            .as_array()
            .ok_or("tags is a list")?
            .iter()
            .map(|t| plain_word(t.as_str().unwrap_or(""), "a tag").map(|t| t.to_lowercase()))
            .collect::<Result<Vec<_>, _>>()?,
    };
    let layer = match v["layer"].as_str().unwrap_or("opaque") {
        "opaque" => Layer::Opaque,
        "cutout" => Layer::Cutout,
        "translucent" => Layer::Translucent,
        other => return Err(format!("layer {other} is opaque, cutout or translucent")),
    };
    let mut base = BlockDef {
        id,
        name: name.clone(),
        color: String::new(),
        emission: 0.0,
        textures: BTreeMap::new(),
        model: BlockModel::Cube,
        type_name: name.clone(),
        state: Vec::new(),
        tags,
        layer,
    };
    parse_look(&v, &mut base)?;
    let Some(props) = v.get("properties").filter(|p| !p.is_null()) else {
        return Ok((vec![base], None));
    };
    let props = props
        .as_object()
        .ok_or("properties maps a name to its values")?;
    if props.is_empty() || props.len() > MAX_PROPERTIES {
        return Err(format!("a block type has 1 to {MAX_PROPERTIES} properties"));
    }
    let mut properties = Vec::new();
    for (prop, values) in props {
        let prop = plain_word(prop, "a property")?;
        let values = values
            .as_array()
            .ok_or_else(|| format!("property {prop} lists its values"))?
            .iter()
            .map(|v| match v {
                Value::Bool(b) => Ok(b.to_string()),
                Value::Number(n) => Ok(n.to_string()),
                other => plain_word(other.as_str().unwrap_or(""), "a value"),
            })
            .collect::<Result<Vec<_>, _>>()?;
        let unique: std::collections::BTreeSet<_> = values.iter().collect();
        if values.is_empty() || values.len() > MAX_VALUES || unique.len() != values.len() {
            return Err(format!(
                "property {prop} needs 1 to {MAX_VALUES} different values"
            ));
        }
        properties.push((prop, values));
    }
    let mut ty = BlockType {
        name: name.clone(),
        base: id,
        properties,
        default: id,
    };
    let states = ty.states();
    if states > MAX_STATES || usize::from(id) + states - 1 > usize::from(crate::grid::MAX_BLOCK) {
        return Err(format!(
            "{name} has {states} states from id {id}: at most {MAX_STATES}, ending by 65534"
        ));
    }
    if let Some(default) = v.get("default").filter(|d| !d.is_null()) {
        let values = default
            .as_object()
            .ok_or("default maps properties to values")?
            .iter()
            .map(|(k, v)| {
                let v = v.as_str().map_or_else(|| v.to_string(), str::to_string);
                (k.clone(), v)
            })
            .collect::<Vec<_>>();
        ty.default = ty.id_for(&values)?;
    }
    let variants = match v.get("variants").filter(|v| !v.is_null()) {
        None => Vec::new(),
        Some(list) => list.as_array().ok_or("variants is a list")?.clone(),
    };
    for variant in &variants {
        let when = variant["when"]
            .as_object()
            .ok_or("a variant needs a when object")?;
        for (prop, value) in when {
            let value = value
                .as_str()
                .map_or_else(|| value.to_string(), str::to_string);
            let known = ty
                .properties
                .iter()
                .find(|(p, _)| p == prop)
                .is_some_and(|(_, options)| options.contains(&value));
            if !known {
                return Err(format!(
                    "a variant names {prop}={value}, which {name} has not"
                ));
            }
        }
    }
    let mut out = Vec::with_capacity(states);
    for index in 0..states {
        let id = ty.base + index as crate::grid::Block;
        let values = ty.values_of(id);
        let mut def = base.clone();
        def.id = id;
        def.name = if id == ty.default {
            name.clone()
        } else {
            state_name(&name, &values)
        };
        for variant in &variants {
            let matches = variant["when"]
                .as_object()
                .unwrap()
                .iter()
                .all(|(prop, value)| {
                    let value = value
                        .as_str()
                        .map_or_else(|| value.to_string(), str::to_string);
                    values.iter().any(|(p, v)| p == prop && *v == value)
                });
            if matches {
                parse_look(variant, &mut def)?;
            }
        }
        def.state = values;
        out.push(def);
    }
    Ok((out, Some(ty)))
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
        assert!(Registry::parse(&[r#"{"name":"new","id":70000}"#.to_string()]).is_err());
    }

    #[test]
    fn a_block_type_gives_each_state_an_id_and_variants_pick_their_looks() {
        let line = r##"{"name":"oak_stairs","id":300,"tags":["stairs","Wooden"],"layer":"cutout",
            "properties":{"facing":["north","east","south","west"],"waterlogged":[false,true]},
            "default":{"facing":"south"},"color":"#8a6a3a",
            "variants":[{"when":{"facing":"east"},"model":"stair east"},
                        {"when":{"waterlogged":"true"},"color":"#4a6aaa"}]}"##;
        let reg = Registry::parse(&[line.to_string()]).unwrap();
        assert_eq!(reg.len(), 7 + 8);
        let ty = reg.block_type("oak_stairs").unwrap();
        assert_eq!(ty.states(), 8);
        // The last property changes fastest; the bare name is the default.
        assert_eq!(reg.id_of("oak_stairs"), Some(304));
        assert_eq!(
            reg.id_of("oak_stairs[facing=north,waterlogged=true]"),
            Some(301)
        );
        assert_eq!(reg.id_of("oak_stairs[waterlogged=true]"), Some(305));
        assert_eq!(reg.id_of("OAK_STAIRS[facing=east]"), Some(302));
        assert!(
            reg.state_id("oak_stairs[facing=up]")
                .unwrap_err()
                .contains("facing=up")
        );
        assert!(
            reg.state_id("oak_stairs[axis=x]")
                .unwrap_err()
                .contains("axis")
        );
        assert!(reg.state_id("stone[axis=x]").is_err());
        let east = reg.get(302).unwrap();
        assert_eq!(east.name, "oak_stairs[facing=east,waterlogged=false]");
        assert_eq!(
            east.model,
            BlockModel::Shape(crate::grid::Shape::from_name("stair east").unwrap())
        );
        assert_eq!(east.color, "#8a6a3a");
        assert_eq!(reg.get(303).unwrap().color, "#4a6aaa");
        assert_eq!(east.tags, ["stairs", "wooden"]);
        assert_eq!(east.layer, Layer::Cutout);
        assert_eq!(reg.type_of(307).unwrap().name, "oak_stairs");
        assert_eq!(
            ty.values_of(307)[0],
            ("facing".to_string(), "west".to_string())
        );
    }

    #[test]
    fn block_types_refuse_overlaps_and_bad_properties() {
        let ty = r#"{"name":"lamp","id":20,"properties":{"lit":[false,true]}}"#.to_string();
        let clash = r#"{"name":"post","id":21}"#.to_string();
        assert!(
            Registry::parse(&[ty.clone(), clash])
                .unwrap_err()
                .contains("id 21")
        );
        assert!(Registry::parse(&[ty.clone(), ty.clone()]).is_err());
        for bad in [
            r#"{"name":"a","id":30,"properties":{}}"#,
            r#"{"name":"a","id":30,"properties":{"x":[]}}"#,
            r#"{"name":"a","id":30,"properties":{"x":["a","a"]}}"#,
            r#"{"name":"a","id":65534,"properties":{"x":["a","b"]}}"#,
            r#"{"name":"a","id":30,"properties":{"x":["a"]},"default":{"x":"b"}}"#,
            r#"{"name":"a","id":30,"properties":{"x":["a"]},"variants":[{"when":{"y":"a"}}]}"#,
            r#"{"name":"a[b]","id":30}"#,
            r#"{"name":"a","id":30,"layer":"glass"}"#,
        ] {
            assert!(Registry::parse(&[bad.to_string()]).is_err(), "{bad}");
        }
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
