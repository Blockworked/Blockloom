// Project symbols for scripts: actor names, input actions, custom component
// fields and shared variables as checked constants, so a rename breaks
// compile instead of silently reading zero at runtime.
//
// Generated per project at build time and appended to the `blockloom` crate
// (`super::build_prelude`), plus the analysis crate rust-analyzer indexes
// (`super::ide`). A folder with no readable project gets an empty module, so
// unit tests and fresh projects still compile.

use std::path::Path;

/// Rust source for `pub mod symbols` from a loaded project.
pub fn project_symbols(project: &crate::project::Project) -> String {
    let mut names: Vec<String> = Vec::new();
    for scene in &project.scenes {
        for actor in &scene.actors {
            if !names.contains(&actor.name) {
                names.push(actor.name.clone());
            }
        }
    }
    let mut ids: Vec<String> = Vec::new();
    for scene in &project.scenes {
        for actor in &scene.actors {
            if !ids.contains(&actor.id) {
                ids.push(actor.id.clone());
            }
        }
    }
    let mut actions: Vec<String> = Vec::new();
    for scene in &project.scenes {
        for action in &scene.world.input.actions {
            if !actions.contains(&action.name) {
                actions.push(action.name.clone());
            }
        }
    }
    let mut customs: Vec<(String, Vec<String>)> = Vec::new();
    for scene in &project.scenes {
        for actor in &scene.actors {
            for component in actor.components.iter() {
                if let crate::components::ActorComponent::Custom { name, fields } = component {
                    let entry = match customs.iter_mut().find(|entry| entry.0 == *name) {
                        Some(entry) => entry,
                        None => {
                            customs.push((name.clone(), Vec::new()));
                            customs.last_mut().expect("just pushed")
                        }
                    };
                    for field in fields {
                        if !entry.1.contains(&field.name) {
                            entry.1.push(field.name.clone());
                        }
                    }
                }
            }
        }
    }
    let variables: Vec<String> = project.globals.iter().map(|v| v.name.clone()).collect();
    let lists: Vec<String> = project
        .global_lists
        .iter()
        .map(|l| l.name.clone())
        .collect();
    render(&names, &ids, &actions, &customs, &variables, &lists)
}

/// Rust source for `pub mod symbols` from a project folder. Unreadable means
/// an empty module, never an error: tests and fresh folders still compile.
pub fn project_symbols_for_dir(project_dir: &Path) -> (String, String) {
    let (source, fingerprint) = match crate::project::read_project_dir(project_dir) {
        Ok(project) => {
            let source = project_symbols(&project);
            let fingerprint = fingerprint_of(&project);
            (source, fingerprint)
        }
        Err(_) => (
            render(&[], &[], &[], &[], &[], &[]),
            "no-project".to_string(),
        ),
    };
    (source, fingerprint)
}

/// What the build stamps carry: actor renames, new actions and new fields
/// must rebuild every script, since the constants changed.
fn fingerprint_of(project: &crate::project::Project) -> String {
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    for scene in &project.scenes {
        for actor in &scene.actors {
            actor.id.hash(&mut hash);
            actor.name.hash(&mut hash);
            for component in actor.components.iter() {
                if let crate::components::ActorComponent::Custom { name, fields } = component {
                    name.hash(&mut hash);
                    for field in fields {
                        field.name.hash(&mut hash);
                    }
                }
            }
        }
        for action in &scene.world.input.actions {
            action.name.hash(&mut hash);
        }
    }
    for v in &project.globals {
        v.name.hash(&mut hash);
    }
    for l in &project.global_lists {
        l.name.hash(&mut hash);
    }
    format!("symbols {:016x}\n", hash.finish())
}

fn render(
    names: &[String],
    ids: &[String],
    actions: &[String],
    customs: &[(String, Vec<String>)],
    variables: &[String],
    lists: &[String],
) -> String {
    let mut out = String::from(
        "/// Checked names from this project: actors, input actions, custom\n\
         /// component fields and shared variables. A rename here breaks compile\n\
         /// instead of silently reading zero - prefer these over string literals.\n\
         pub mod symbols {\n",
    );
    out.push_str("    /// Every actor name in the project, as authored.\n    pub mod actors {\n");
    let mut taken = std::collections::HashSet::new();
    for name in names {
        let ident = unique_ident(name, &mut taken);
        out.push_str(&format!(
            "        /// Actor `\"{name}\"`.\n        pub const {ident}: &str = \"{name}\";\n"
        ));
    }
    if names.is_empty() {
        out.push_str("        // No actors yet.\n");
    }
    out.push_str("    }\n");
    out.push_str("    /// Every actor id in the project. Names are what scripts usually\n    /// want; ids are here for the cases that need them.\n    pub mod actor_ids {\n");
    let mut taken = std::collections::HashSet::new();
    for id in ids {
        let ident = unique_ident(id, &mut taken);
        out.push_str(&format!(
            "        /// Actor id `\"{id}\"`.\n        pub const {ident}: &str = \"{id}\";\n"
        ));
    }
    if ids.is_empty() {
        out.push_str("        // No actors yet.\n");
    }
    out.push_str("    }\n");
    out.push_str("    /// Every input action in the project.\n    pub mod actions {\n");
    let mut taken = std::collections::HashSet::new();
    for action in actions {
        let ident = unique_ident(action, &mut taken);
        out.push_str(&format!(
            "        /// Action `\"{action}\"`.\n        pub const {ident}: &str = \"{action}\";\n"
        ));
    }
    if actions.is_empty() {
        out.push_str("        // No input actions yet.\n");
    }
    out.push_str("    }\n");
    out.push_str("    /// Every custom component and its fields.\n    pub mod components {\n");
    if customs.is_empty() {
        out.push_str("        // No custom components yet.\n");
    }
    for (name, fields) in customs {
        let module = module_ident(name);
        out.push_str(&format!(
            "        /// Custom component `\"{name}\"`.\n        pub mod {module} {{\n"
        ));
        out.push_str(&format!("            pub const NAME: &str = \"{name}\";\n"));
        let mut taken = std::collections::HashSet::new();
        for field in fields {
            let ident = unique_ident(field, &mut taken);
            out.push_str(&format!("            /// Field `\"{field}\"` of `\"{name}\"`.\n            pub const {ident}: &str = \"{field}\";\n"));
        }
        out.push_str("        }\n");
    }
    out.push_str("    }\n");
    out.push_str("    /// Shared block variables (`Project.globals`).\n    pub mod variables {\n");
    let mut taken = std::collections::HashSet::new();
    for name in variables {
        let ident = unique_ident(name, &mut taken);
        out.push_str(&format!(
            "        /// Variable `\"{name}\"`.\n        pub const {ident}: &str = \"{name}\";\n"
        ));
    }
    if variables.is_empty() {
        out.push_str("        // No shared variables yet.\n");
    }
    out.push_str("    }\n");
    out.push_str("    /// Shared block lists (`Project.global_lists`).\n    pub mod lists {\n");
    let mut taken = std::collections::HashSet::new();
    for name in lists {
        let ident = unique_ident(name, &mut taken);
        out.push_str(&format!(
            "        /// List `\"{name}\"`.\n        pub const {ident}: &str = \"{name}\";\n"
        ));
    }
    if lists.is_empty() {
        out.push_str("        // No shared lists yet.\n");
    }
    out.push_str("    }\n");
    out.push_str("}\n");
    out
}

/// `Ball-1` to `BALL_1`: uppercase, non-alphanumeric to underscores, leading
/// digits prefixed, empties named `SYM`, collisions suffixed.
fn unique_ident(name: &str, taken: &mut std::collections::HashSet<String>) -> String {
    let mut ident: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect();
    ident = ident
        .split('_')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("_");
    if ident.is_empty() {
        ident = "SYM".to_string();
    }
    if ident.starts_with(|c: char| c.is_ascii_digit()) {
        ident = format!("_{ident}");
    }
    let mut candidate = ident.clone();
    let mut n = 2u32;
    while taken.contains(&candidate) {
        candidate = format!("{ident}_{n}");
        n += 1;
    }
    taken.insert(candidate.clone());
    candidate
}

fn module_ident(name: &str) -> String {
    let mut ident: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    ident = ident
        .split('_')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("_");
    if ident.is_empty() {
        ident = "sym".to_string();
    }
    if ident.starts_with(|c: char| c.is_ascii_digit()) {
        ident = format!("_{ident}");
    }
    if is_rust_keyword(&ident) {
        ident = format!("{ident}_");
    }
    ident
}

fn is_rust_keyword(word: &str) -> bool {
    matches!(
        word,
        "as" | "break"
            | "const"
            | "continue"
            | "crate"
            | "else"
            | "enum"
            | "extern"
            | "false"
            | "fn"
            | "for"
            | "if"
            | "impl"
            | "in"
            | "let"
            | "loop"
            | "match"
            | "mod"
            | "move"
            | "mut"
            | "pub"
            | "ref"
            | "return"
            | "self"
            | "static"
            | "struct"
            | "super"
            | "trait"
            | "true"
            | "type"
            | "unsafe"
            | "use"
            | "where"
            | "while"
            | "async"
            | "await"
            | "dyn"
            | "abstract"
            | "become"
            | "box"
            | "do"
            | "final"
            | "macro"
            | "override"
            | "priv"
            | "typeof"
            | "unsized"
            | "virtual"
            | "yield"
            | "try"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idents_stay_valid_and_unique() {
        let mut taken = std::collections::HashSet::new();
        assert_eq!(unique_ident("Ball", &mut taken), "BALL");
        assert_eq!(unique_ident("Ball", &mut taken), "BALL_2");
        assert_eq!(unique_ident("1up", &mut taken), "_1UP");
        assert_eq!(unique_ident("---", &mut taken), "SYM");
        assert_eq!(module_ident("Health"), "health");
        assert_eq!(module_ident("type"), "type_");
    }

    #[test]
    fn render_lists_everything_once() {
        let out = render(
            &["Ball".to_string(), "Ball".to_string()],
            &["a1".to_string()],
            &["jump".to_string()],
            &[("Health".to_string(), vec!["hp".to_string()])],
            &["score".to_string()],
            &[],
        );
        assert!(out.contains("pub const BALL: &str = \"Ball\";"));
        assert!(out.contains("pub const BALL_2: &str = \"Ball\";"));
        assert!(out.contains("pub const JUMP: &str = \"jump\";"));
        assert!(out.contains("pub mod health"));
        assert!(out.contains("pub const HP: &str = \"hp\";"));
        assert!(out.contains("pub const SCORE: &str = \"score\";"));
    }

    #[test]
    fn generated_symbols_compile_against_rustc() {
        // The generated module must be valid Rust: compile a tiny crate that
        // uses every section, skipping when this machine has no toolchain.
        if super::super::toolchain_version().is_err() {
            return;
        }
        let out = render(
            &["Ball".to_string(), "Player 1".to_string()],
            &["a1".to_string()],
            &["jump".to_string()],
            &[(
                "Health".to_string(),
                vec!["hp".to_string(), "type".to_string()],
            )],
            &["score".to_string()],
            &["log".to_string()],
        );
        let dir = std::env::temp_dir().join(format!("blockloom-symbols-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a temp dir");
        let source = dir.join("symbols_check.rs");
        std::fs::write(
            &source,
            format!(
                "{out}\nfn main() {{\n    let _ = symbols::actors::BALL;\n    let _ = symbols::actors::PLAYER_1;\n    let _ = symbols::actor_ids::A1;\n    let _ = symbols::actions::JUMP;\n    let _ = symbols::components::health::NAME;\n    let _ = symbols::components::health::HP;\n    let _ = symbols::components::health::TYPE;\n    let _ = symbols::variables::SCORE;\n    let _ = symbols::lists::LOG;\n}}\n"
            ),
        )
        .expect("the check source");
        let output = super::super::rustc_command()
            .arg("--edition")
            .arg("2024")
            .arg("--crate-type")
            .arg("bin")
            .arg("-o")
            .arg(dir.join("symbols_check"))
            .arg(&source)
            .output()
            .expect("rustc runs");
        assert!(
            output.status.success(),
            "generated symbols didn't compile:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn typed_names_parse_back_in_core() {
        use super::super::typed::*;
        for (mode, parsed) in [
            (ForceMode::Force.name(), crate::physics::ForceMode::Force),
            (
                ForceMode::Acceleration.name(),
                crate::physics::ForceMode::Acceleration,
            ),
            (
                ForceMode::Impulse.name(),
                crate::physics::ForceMode::Impulse,
            ),
            (
                ForceMode::VelocityChange.name(),
                crate::physics::ForceMode::VelocityChange,
            ),
        ] {
            assert_eq!(crate::physics::ForceMode::parse(mode), Some(parsed));
        }
        assert_eq!(
            crate::wind::WindProperty::parse(WindDial::Direction.name()),
            Some(crate::wind::WindProperty::Direction)
        );
        assert_eq!(
            crate::wind::WindProperty::parse(WindDial::Speed.name()),
            Some(crate::wind::WindProperty::Speed)
        );
        assert_eq!(
            crate::wind::WindProperty::parse(WindDial::Gust.name()),
            Some(crate::wind::WindProperty::Gust)
        );
        assert_eq!(
            crate::wind::WindProperty::parse(WindDial::Storm.name()),
            Some(crate::wind::WindProperty::Storm)
        );
        assert_eq!(
            crate::water::WaterProperty::parse(WaterDial::Level.name()),
            Some(crate::water::WaterProperty::Level)
        );
        assert_eq!(
            crate::water::WaterProperty::parse(WaterDial::Chop.name()),
            Some(crate::water::WaterProperty::Chop)
        );
        assert_eq!(
            crate::water::WaterProperty::parse(WaterDial::Foam.name()),
            Some(crate::water::WaterProperty::Foam)
        );
        assert_eq!(
            crate::clouds::CloudProperty::parse(CloudDial::Coverage.name()),
            Some(crate::clouds::CloudProperty::Coverage)
        );
        assert_eq!(
            crate::clouds::CloudProperty::parse(CloudDial::Density.name()),
            Some(crate::clouds::CloudProperty::Density)
        );
        assert_eq!(
            crate::clouds::CloudProperty::parse(CloudDial::Type.name()),
            Some(crate::clouds::CloudProperty::Type)
        );
        assert_eq!(
            crate::director::PrecipitationKind::parse(Precipitation::Rain.name()),
            Some(crate::director::PrecipitationKind::Rain)
        );
        assert_eq!(
            crate::director::PrecipitationKind::parse(Precipitation::Snow.name()),
            Some(crate::director::PrecipitationKind::Snow)
        );
        for easing in [
            Easing::Linear,
            Easing::EaseIn,
            Easing::EaseOut,
            Easing::EaseInOut,
            Easing::Bounce,
            Easing::Elastic,
        ] {
            assert!(crate::animation::TweenEasing::parse(easing.name()).is_some());
        }
    }
}
