//! 2D post (`blockloom_core::post2d`): one fullscreen pass over the 2D
//! world camera, after the tonemapper and the post stack's display-value
//! pass and before the interface, so the HUD stays crisp.

use crate::engine::Engine;
use crate::post::PostLdrPass;
use crate::world::WorldCamera;
use bevy::core_pipeline::fullscreen_material::{FullscreenMaterial, FullscreenMaterialPlugin};
use bevy::core_pipeline::{Core2d, Core2dSystems};
use bevy::ecs::schedule::{ScheduleConfigs, ScheduleLabel};
use bevy::ecs::system::BoxedSystem;
use bevy::prelude::*;
use bevy::render::RenderApp;
use bevy::render::extract_component::ExtractComponent;
use bevy::render::render_resource::ShaderType;
use bevy::shader::ShaderRef;
use bevy::ui_render::ui_pass;
use blockloom_core::post2d::{MAX_PALETTE, Post2d, parse_srgb};

pub fn register(app: &mut App) {
    bevy::asset::embedded_asset!(app, "shaders/post2d.wesl");
    app.add_plugins(FullscreenMaterialPlugin::<Post2dPass>::default())
        .add_systems(Update, sync_post2d.run_if(is_2d_world));
}

fn is_2d_world(dimension: Res<crate::engine::Dimension>) -> bool {
    dimension.0 == blockloom_core::scene::Mode::TwoD
}

/// The uniform; matches `Post2d` in `post2d.wesl`.
#[derive(Component, ExtractComponent, Clone, Copy, Debug, PartialEq, ShaderType)]
#[extract_app(RenderApp)]
pub struct Post2dPass {
    a: Vec4,
    b: Vec4,
    c: Vec4,
    d: Vec4,
    e: Vec4,
    palette: [Vec4; MAX_PALETTE],
}

impl Default for Post2dPass {
    fn default() -> Self {
        Self::of(&Post2d::default())
    }
}

impl Post2dPass {
    pub fn of(post: &Post2d) -> Self {
        let mut palette = [Vec4::ZERO; MAX_PALETTE];
        let mut count = 0;
        for hex in post.palette.iter().take(MAX_PALETTE) {
            if let Some(c) = parse_srgb(hex) {
                palette[count] = Vec4::new(c[0], c[1], c[2], 1.0);
                count += 1;
            }
        }
        let outline = parse_srgb(&post.outline.color).unwrap_or([0.0; 3]);
        Self {
            a: Vec4::new(
                post.pixelation.max(1) as f32,
                post.levels as f32,
                post.dither,
                post.split,
            ),
            b: Vec4::new(post.outline.strength, post.outline.threshold, 0.0, 0.0),
            c: Vec4::new(outline[0], outline[1], outline[2], 1.0),
            d: Vec4::new(
                post.crt.scanlines,
                post.crt.curvature,
                post.crt.vignette,
                post.crt.mask,
            ),
            e: Vec4::new(count as f32, 0.0, 0.0, 0.0),
            palette,
        }
    }
}

fn shader() -> ShaderRef {
    ShaderRef::Path(
        bevy::asset::AssetPath::from_path_buf(bevy::asset::embedded_path!("shaders/post2d.wesl"))
            .with_source("embedded"),
    )
}

impl FullscreenMaterial for Post2dPass {
    fn fragment_shader() -> ShaderRef {
        shader()
    }

    fn schedule() -> impl ScheduleLabel + Clone {
        Core2d
    }

    fn schedule_configs(system: ScheduleConfigs<BoxedSystem>) -> ScheduleConfigs<BoxedSystem> {
        system
            .in_set(Core2dSystems::PostProcess)
            .after(PostLdrPass)
            .before(ui_pass)
    }
}

/// The project's 2D post with what blocks have set this run laid over it.
pub fn effective(engine: &Engine) -> Post2d {
    let mut post = engine.project.world.post2d.clone();
    for (dial, value) in &engine.look2d.post {
        post.set(*dial, value);
    }
    post
}

/// Puts the pass on the world camera while any effect is on, and takes it
/// off when none is.
fn sync_post2d(
    engine: NonSend<Engine>,
    cameras: Query<(Entity, Option<&Post2dPass>), With<WorldCamera>>,
    mut commands: Commands,
) {
    let post = effective(&engine);
    for (entity, held) in &cameras {
        if !post.is_on() {
            if held.is_some() {
                commands.entity(entity).remove::<Post2dPass>();
            }
            continue;
        }
        let next = Post2dPass::of(&post);
        if held != Some(&next) {
            commands.entity(entity).insert(next);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use blockloom_core::blocks::Look2dDial;

    #[test]
    fn the_uniform_carries_each_setting() {
        let mut post = Post2d::default();
        post.pixelation = 4;
        post.levels = 8;
        post.palette = vec!["#ff0000".into(), "#00ff00".into()];
        post.crt.scanlines = 0.5;
        let pass = Post2dPass::of(&post);
        assert_eq!(pass.a.x, 4.0);
        assert_eq!(pass.a.y, 8.0);
        assert_eq!(pass.e.x, 2.0);
        assert_eq!(pass.palette[1], Vec4::new(0.0, 1.0, 0.0, 1.0));
        assert_eq!(pass.d.x, 0.5);
        assert_eq!(Post2dPass::of(&Post2d::default()).a.x, 1.0);
    }

    #[test]
    fn blocks_override_the_project_look() {
        let mut look = crate::light2d::Look2d::default();
        look.set(Look2dDial::Pixelation, "6");
        look.set(Look2dDial::Pixelation, "3");
        assert_eq!(look.post.len(), 1);
        let mut post = Post2d::default();
        for (d, v) in &look.post {
            post.set(*d, v);
        }
        assert_eq!(post.pixelation, 3);
    }

    /// The shader's source must pass naga.
    #[test]
    fn the_post_shader_compiles() {
        blockloom_core::shader_lib::validate(include_str!("shaders/post2d.wesl"), &[])
            .unwrap_or_else(|error| panic!("{error}"));
    }
}
