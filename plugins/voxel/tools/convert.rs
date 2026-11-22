//! Moves a project's 0.3.x voxel `edits` lines into a `.blockworld` asset.
//! Usage: blockloom-voxel-convert <project folder> [name]. Run it with the editor closed.

use blockloom_plugin_sdk::serde_json::{self, Value};
use std::path::Path;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(folder) = args.first() else {
        eprintln!("usage: blockloom-voxel-convert <project folder> [name]");
        std::process::exit(2);
    };
    let name = args.get(1).map_or("world", String::as_str);
    match convert(Path::new(folder), name) {
        Ok(message) => println!("{message}"),
        Err(why) => {
            eprintln!("blockloom-voxel-convert: {why}");
            std::process::exit(1);
        }
    }
}

fn convert(folder: &Path, name: &str) -> Result<String, String> {
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_alphanumeric() || "-_ ".contains(c))
    {
        return Err(format!("{name:?} is not a plain file name"));
    }
    let index = folder.join("project.blockloom");
    let text = std::fs::read_to_string(&index).map_err(|e| format!("{}: {e}", index.display()))?;
    let mut project: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let record = project["plugin_resources"]
        .as_array_mut()
        .and_then(|list| {
            list.iter_mut()
                .find(|r| r["plugin"] == "com.blockworked.voxel" && r["type_id"] == "world")
        })
        .ok_or("the project has no voxel world resource")?;
    let payload = &mut record["payload"];
    let edits = payload["edits"].as_array().map_or(0, Vec::len);
    if edits == 0 {
        return Err("the voxel world has no edit lines to convert".into());
    }
    if payload["world_file"]
        .as_str()
        .is_some_and(|f| !f.is_empty())
    {
        return Err("the voxel world already has a world file".into());
    }
    let (bytes, sections, skipped) = blockloom_voxel::convert_edits(payload)?;
    for why in &skipped {
        eprintln!("skipped {why}");
    }
    let relative = format!("assets/worlds/{name}.blockworld");
    let path = folder.join(&relative);
    if path.exists() {
        return Err(format!("{relative} already exists"));
    }
    std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    std::fs::write(&path, &bytes).map_err(|e| format!("{relative}: {e}"))?;
    payload["world_file"] = Value::String(relative.clone());
    payload["edits"] = Value::Array(Vec::new());
    let out = serde_json::to_string_pretty(&project).map_err(|e| e.to_string())?;
    let temp = index.with_extension("blockloom.tmp");
    std::fs::write(&temp, out + "\n").map_err(|e| e.to_string())?;
    std::fs::rename(&temp, &index).map_err(|e| e.to_string())?;
    Ok(format!(
        "{edits} edit lines -> {relative} ({sections} sections, {} bytes)",
        bytes.len()
    ))
}
