//! The source `plugin-new template=engine` writes, compiled and run here so
//! it never drifts from the engine API.

#[path = "../templates/lib.rs"]
mod template;

use blockloom_engine::prelude::*;

#[test]
fn the_template_builds_into_an_app() {
    let mut app = App::new();
    app.init_resource::<RunState>();
    template::blockloom_module().build(&mut app);
    assert!(app.world().contains_resource::<template::Ticks>());
}
