//! Spelling rules for the names a plugin invents. Names that persist
//! (plugin ids, type ids) are identities; display names are metadata.

/// A reverse-domain plugin id: two or more dot-separated segments of
/// lowercase letters, digits, `-` and `_`, each starting with a letter.
pub fn validate_plugin_id(id: &str) -> Result<(), String> {
    if id.len() > 128 {
        return Err("plugin id is longer than 128 characters".to_string());
    }
    let segments: Vec<&str> = id.split('.').collect();
    if segments.len() < 2 {
        return Err(format!(
            "\"{id}\" is not a reverse-domain id like com.example.thing"
        ));
    }
    for segment in segments {
        let mut chars = segment.chars();
        let ok = chars.next().is_some_and(|c| c.is_ascii_lowercase())
            && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_');
        if !ok {
            return Err(format!(
                "\"{id}\": segment \"{segment}\" must start with a lowercase letter and use only lowercase letters, digits, '-' and '_'"
            ));
        }
    }
    Ok(())
}

/// A name inside one plugin (a component type, a block, a command): letters,
/// digits and `_`, starting with a letter.
pub fn validate_type_id(id: &str) -> Result<(), String> {
    let mut chars = id.chars();
    let ok = id.len() <= 64
        && chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
    if ok {
        Ok(())
    } else {
        Err(format!(
            "\"{id}\" must be 1-64 letters, digits or '_' and start with a letter"
        ))
    }
}

/// `plugin/type`, the one spelling a component or command is addressed by.
pub fn qualified(plugin: &str, type_id: &str) -> String {
    format!("{plugin}/{type_id}")
}

/// The inverse of [`qualified`].
pub fn split_qualified(name: &str) -> Option<(&str, &str)> {
    name.split_once('/')
}

/// A path inside a package: relative, forward slashes, no `..`.
pub fn validate_package_path(path: &str) -> Result<(), String> {
    if path.is_empty() || path.starts_with('/') || path.contains('\\') || path.contains(':') {
        return Err(format!("\"{path}\" is not a relative package path"));
    }
    if path
        .split('/')
        .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(format!("\"{path}\" escapes or repeats a package folder"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_ids_are_reverse_domain() {
        assert!(validate_plugin_id("com.blockworked.voxel").is_ok());
        assert!(validate_plugin_id("com.example.hello-world_2").is_ok());
        assert!(validate_plugin_id("voxel").is_err());
        assert!(validate_plugin_id("Com.example").is_err());
        assert!(validate_plugin_id("com..example").is_err());
        assert!(validate_plugin_id("com.1example").is_err());
    }

    #[test]
    fn type_ids_are_identifiers() {
        assert!(validate_type_id("Chunk_2").is_ok());
        assert!(validate_type_id("2d").is_err());
        assert!(validate_type_id("a/b").is_err());
        assert!(validate_type_id("").is_err());
    }

    #[test]
    fn package_paths_stay_inside() {
        assert!(validate_package_path("runtime/x86_64-unknown-linux-gnu/lib.so").is_ok());
        for bad in ["", "/etc", "../x", "a/../b", "a//b", "a\\b", "c:x"] {
            assert!(validate_package_path(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn qualified_names_round_trip() {
        let name = qualified("com.example.a", "Health");
        assert_eq!(split_qualified(&name), Some(("com.example.a", "Health")));
    }
}
