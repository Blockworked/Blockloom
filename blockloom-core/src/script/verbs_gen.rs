// Generated from blockloom-core/src/verbs.rs; BLESS=1 cargo test -p blockloom-core --test verbs rewrites it.
// One typed method per block verb, over the generic `verb` calls.

impl Actor {
    /// Writes one property of a component on an actor for the rest of the run, by its registry path (Light.intensity, Look.visual.color).
    pub fn set_property(&self, property: &str, actor: &str, value: impl Into<PluginArg>) {
        self.verb("set_property", &[PluginArg::from(property), PluginArg::from(actor), value.into()]);
    }

    /// Sets the world's snow cover or surface wetness (0 to 1) for the rest of the run, over the director and volumes.
    pub fn set_surface(&self, property: &str, value: f64) {
        self.verb("set_surface", &[PluginArg::from(property), PluginArg::from(value)]);
    }

    /// Reads one property of a component on an actor by its registry path, as the run has it now.
    pub fn property(&self, property: &str, actor: &str) -> Option<String> {
        self.verb_text("property", &[PluginArg::from(property), PluginArg::from(actor)])
    }

    /// Whether a screen the scene or game lists is open, by its file name without the extension or its path.
    pub fn screen_open(&self, screen: &str) -> Option<bool> {
        self.verb_number("screen_open", &[PluginArg::from(screen)]).map(|n| n != 0.0)
    }
}
