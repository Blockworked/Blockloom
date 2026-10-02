//! What a plugin gives the renderer beyond meshes: WESL library modules and
//! instanced draws.
//!
//! A shader module is a `.wesl` file in the package. The world registers it
//! as `blockloom::<module_name>`, so a project's surface shader imports it
//! like it imports Blockloom's own library. Instances are many copies of one
//! of the plugin's meshes, drawn in one batch.

use crate::id::{validate_package_path, validate_type_id};
use serde::{Deserialize, Serialize};

/// The most copies one instanced draw may hold.
pub const MAX_INSTANCES: usize = 65_536;

/// A shader module a package ships.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShaderSchema {
    pub name: String,
    /// The `.wesl` file, relative to the package root.
    pub file: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
}

impl ShaderSchema {
    pub fn check_definition(&self) -> Result<(), String> {
        validate_type_id(&self.name)?;
        validate_package_path(&self.file).map_err(|e| format!("shader {}: {e}", self.name))?;
        if !self.file.ends_with(".wesl") {
            return Err(format!(
                "shader {}: {} is not a .wesl file",
                self.name, self.file
            ));
        }
        Ok(())
    }
}

/// The name a plugin's shader module is imported by, under the `blockloom`
/// package: `com.example.fx` and `foam` become `plugin_com_example_fx_foam`.
pub fn module_name(plugin: &str, shader: &str) -> String {
    let id: String = plugin
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    format!("plugin_{id}_{shader}")
}

/// One shader module as a world loads it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LoadoutShader {
    /// The import name, from [`module_name`].
    pub module: String,
    pub source: String,
}

/// Many copies of one mesh the plugin already submitted, drawn together.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InstanceData {
    /// Names the set within its plugin; a later set of the same name replaces it.
    pub name: String,
    /// The plugin's mesh each copy draws.
    pub mesh: String,
    /// x, y, z per copy.
    pub positions: Vec<f32>,
    /// Turn about the up axis per copy, in radians; empty for none.
    #[serde(default)]
    pub yaw: Vec<f32>,
    /// Uniform scale per copy; empty for 1.
    #[serde(default)]
    pub scales: Vec<f32>,
}

impl InstanceData {
    pub fn count(&self) -> usize {
        self.positions.len() / 3
    }

    pub fn check(&self) -> Result<(), String> {
        let name = &self.name;
        if name.is_empty() || self.mesh.is_empty() {
            return Err("an instance set needs a name and a mesh".to_string());
        }
        if !self.positions.len().is_multiple_of(3) {
            return Err(format!(
                "instances {name}: positions are not a multiple of 3"
            ));
        }
        let count = self.count();
        if count > MAX_INSTANCES {
            return Err(format!(
                "instances {name}: {count} copies, at most {MAX_INSTANCES}"
            ));
        }
        if !self.yaw.is_empty() && self.yaw.len() != count {
            return Err(format!("instances {name}: one yaw per copy, or none"));
        }
        if !self.scales.is_empty() && self.scales.len() != count {
            return Err(format!("instances {name}: one scale per copy, or none"));
        }
        let finite = self
            .positions
            .iter()
            .chain(&self.yaw)
            .chain(&self.scales)
            .all(|v| v.is_finite());
        if !finite {
            return Err(format!("instances {name}: a value is not finite"));
        }
        if self.scales.iter().any(|s| *s <= 0.0) {
            return Err(format!("instances {name}: a scale is not positive"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set() -> InstanceData {
        InstanceData {
            name: "forest".into(),
            mesh: "tree".into(),
            positions: vec![0., 0., 0., 1., 0., 1.],
            yaw: vec![],
            scales: vec![],
        }
    }

    #[test]
    fn module_names_are_one_identifier() {
        assert_eq!(
            module_name("com.example.fx", "foam"),
            "plugin_com_example_fx_foam"
        );
    }

    #[test]
    fn a_shader_is_a_wesl_file_inside_the_package() {
        let ok = ShaderSchema {
            name: "foam".into(),
            file: "shaders/foam.wesl".into(),
            description: String::new(),
        };
        assert!(ok.check_definition().is_ok());
        for file in ["shaders/foam.wgsl", "../foam.wesl", "/foam.wesl"] {
            let bad = ShaderSchema {
                file: file.into(),
                ..ok.clone()
            };
            assert!(bad.check_definition().is_err(), "{file}");
        }
    }

    #[test]
    fn instances_agree_with_their_count() {
        assert!(set().check().is_ok());
        assert_eq!(set().count(), 2);
        let mut bad = set();
        bad.yaw = vec![0.0];
        assert!(bad.check().is_err());
        let mut bad = set();
        bad.scales = vec![1.0, 0.0];
        assert!(bad.check().is_err());
        let mut bad = set();
        bad.positions.push(1.0);
        assert!(bad.check().is_err());
    }
}
