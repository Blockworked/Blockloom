//! The HDR frame: every world camera renders linear FP16 from the sky through
//! the lights to post, and only the tonemapper at the very end turns that into
//! display values. This module holds what rides on that: the Game view's
//! exposure debug views, which read the scene before it is tonemapped.

use bevy::core_pipeline::fullscreen_material::{FullscreenMaterial, FullscreenMaterialPlugin};
use bevy::core_pipeline::tonemapping::tonemapping;
use bevy::core_pipeline::{Core2d, Core2dSystems};
use bevy::ecs::schedule::{ScheduleConfigs, ScheduleLabel};
use bevy::ecs::system::BoxedSystem;
use bevy::prelude::*;
use bevy::render::RenderApp;
use bevy::render::extract_component::ExtractComponent;
use bevy::render::render_resource::ShaderType;
use bevy::shader::ShaderRef;
use blockloom_protocol::DebugView;

pub fn register(app: &mut App) {
    bevy::asset::embedded_asset!(app, "shaders/hdr_debug.wesl");
    app.init_resource::<HdrDebug>().add_plugins((
        FullscreenMaterialPlugin::<HdrDebugView3d>::default(),
        FullscreenMaterialPlugin::<HdrDebugView2d>::default(),
    ));
}

/// The debug view the editor asked for. An editor preference, re-sent with
/// the scene view to every world that comes up.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HdrDebug(pub DebugView);

impl HdrDebug {
    /// False color replaces the image outright, so the tonemapper has to
    /// stand aside or it would bend the bands' colors.
    pub fn bypasses_tonemapping(self) -> bool {
        self.0 == DebugView::FalseColor
    }

    fn mode(self) -> Option<u32> {
        match self.0 {
            DebugView::Lit => None,
            DebugView::FalseColor => Some(1),
            DebugView::Clipping => Some(2),
        }
    }

    /// Puts the matching pass on a world camera, or takes it off.
    pub fn apply(self, camera: &mut EntityCommands, is_3d: bool) {
        camera.remove::<(HdrDebugView3d, HdrDebugView2d)>();
        let Some(mode) = self.mode() else {
            return;
        };
        if is_3d {
            camera.insert(HdrDebugView3d::new(mode));
        } else {
            camera.insert(HdrDebugView2d::new(mode));
        }
    }
}

fn shader() -> ShaderRef {
    ShaderRef::Path(
        bevy::asset::AssetPath::from_path_buf(bevy::asset::embedded_path!(
            "shaders/hdr_debug.wesl"
        ))
        .with_source("embedded"),
    )
}

/// One material per dimension, since a fullscreen material names the one
/// schedule it runs in. The fields match the shader's `HdrDebug`.
macro_rules! debug_view {
    ($name:ident) => {
        #[derive(
            Component, ExtractComponent, Clone, Copy, Default, Debug, PartialEq, ShaderType,
        )]
        #[extract_app(RenderApp)]
        pub struct $name {
            mode: u32,
            /// Exposed luminance that counts as clipped.
            paper_white: f32,
            // Uniforms are sized in 16s on some backends.
            pad: Vec2,
        }

        impl $name {
            fn new(mode: u32) -> Self {
                Self {
                    mode,
                    paper_white: 1.0,
                    pad: Vec2::ZERO,
                }
            }
        }
    };
}

debug_view!(HdrDebugView3d);
debug_view!(HdrDebugView2d);

impl FullscreenMaterial for HdrDebugView3d {
    fn fragment_shader() -> ShaderRef {
        shader()
    }
}

impl FullscreenMaterial for HdrDebugView2d {
    fn fragment_shader() -> ShaderRef {
        shader()
    }

    fn schedule() -> impl ScheduleLabel + Clone {
        Core2d
    }

    fn schedule_configs(system: ScheduleConfigs<BoxedSystem>) -> ScheduleConfigs<BoxedSystem> {
        system
            .in_set(Core2dSystems::PostProcess)
            .before(tonemapping)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_debug_shader_compiles() {
        blockloom_core::shader_lib::validate(include_str!("shaders/hdr_debug.wesl"), &[])
            .unwrap_or_else(|error| panic!("{error}"));
    }

    #[test]
    fn only_false_color_bypasses_the_tonemapper() {
        assert!(HdrDebug(DebugView::FalseColor).bypasses_tonemapping());
        assert!(!HdrDebug(DebugView::Clipping).bypasses_tonemapping());
        assert!(!HdrDebug(DebugView::Lit).bypasses_tonemapping());
    }

    #[test]
    fn a_debug_view_lands_on_the_camera_for_its_dimension() {
        let mut app = App::new();
        let camera = app.world_mut().spawn_empty().id();
        let mut commands = app.world_mut().commands();
        HdrDebug(DebugView::Clipping).apply(&mut commands.entity(camera), false);
        app.world_mut().flush();
        assert_eq!(
            app.world()
                .get::<HdrDebugView2d>(camera)
                .map(|view| view.mode),
            Some(2)
        );
        assert!(app.world().get::<HdrDebugView3d>(camera).is_none());

        let mut commands = app.world_mut().commands();
        HdrDebug(DebugView::Lit).apply(&mut commands.entity(camera), false);
        app.world_mut().flush();
        assert!(app.world().get::<HdrDebugView2d>(camera).is_none());
    }
}
