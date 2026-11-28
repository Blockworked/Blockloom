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

    /// Plays a clip on one of my Animation component's bone layers over the state machine, fading in by the crossfade. A clip that plays once fades back out at its end.
    pub fn play_layer(&self, clip: &str, layer: &str) {
        self.verb("play_layer", &[PluginArg::from(clip), PluginArg::from(layer)]);
    }

    /// Fades one of my animation layers out by the crossfade, handing its bones back to the state machine.
    pub fn stop_layer(&self, layer: &str) {
        self.verb("stop_layer", &[PluginArg::from(layer)]);
    }

    /// How much of its bones one of my animation layers takes from the state machine, 0 to 1, easing there by the crossfade.
    pub fn set_layer_weight(&self, layer: &str, weight: f64) {
        self.verb("set_layer_weight", &[PluginArg::from(layer), PluginArg::from(weight)]);
    }

    /// Turns my model's head toward another actor every frame, as far as my Animation component's IK look limit allows, until stop looking.
    pub fn look_at(&self, target: &str) {
        self.verb("look_at", &[PluginArg::from(target)]);
    }

    /// Turns my model's head toward a point in the world, as far as my Animation component's IK look limit allows, until stop looking.
    pub fn look_at_point(&self, x: f64, y: f64, z: f64) {
        self.verb("look_at_point", &[PluginArg::from(x), PluginArg::from(y), PluginArg::from(z)]);
    }

    /// Hands my model's head back to its animation.
    pub fn stop_looking(&self) {
        self.verb("stop_looking", &[]);
    }

    /// Ends the running playmode test as passed. Outside a test run it does nothing.
    pub fn pass_test(&self) {
        self.verb("pass_test", &[]);
    }

    /// Ends the running playmode test as failed, giving the reason. Outside a test run it does nothing.
    pub fn fail_test(&self, reason: &str) {
        self.verb("fail_test", &[PluginArg::from(reason)]);
    }

    /// Fails the running playmode test with message when condition is false, and carries on when it is true.
    pub fn expect(&self, condition: bool, message: &str) {
        self.verb("expect", &[PluginArg::from(condition), PluginArg::from(message)]);
    }

    /// Hands my model's ragdoll bones to physics, keeping how fast I was moving, until get up. My own body stops colliding meanwhile.
    pub fn go_limp(&self) {
        self.verb("go_limp", &[]);
    }

    /// Ends my ragdoll where it lies: my body comes back there and the animation blends back in over the ragdoll's get up time.
    pub fn get_up(&self) {
        self.verb("get_up", &[]);
    }

    /// Reads one property of a component on an actor by its registry path, as the run has it now.
    pub fn property(&self, property: &str, actor: &str) -> Option<String> {
        self.verb_text("property", &[PluginArg::from(property), PluginArg::from(actor)])
    }

    /// Whether a screen the scene or game lists is open, by its file name without the extension or its path.
    pub fn screen_open(&self, screen: &str) -> Option<bool> {
        self.verb_number("screen_open", &[PluginArg::from(screen)]).map(|n| n != 0.0)
    }

    /// Opens this run to LAN guests on the port in Project settings, up to its guest limit; each guest plays the next player. Needs multiplayer on in Project settings.
    pub fn host_lan(&self) {
        self.verb("host_lan", &[]);
    }

    /// Closes this run to LAN guests. They go back to their own game; the run carries on.
    pub fn close_lan(&self) {
        self.verb("close_lan", &[]);
    }

    /// Leaves this run and joins a LAN game of the same build as the next player: a number from the games found (1 is the first), a game's name, or an invite.
    pub fn join_lan(&self, game: &str) {
        self.verb("join_lan", &[PluginArg::from(game)]);
    }

    /// How many LAN games of this build are announcing themselves nearby. Hosts announce when Project settings say so.
    pub fn lan_game_count(&self) -> Option<f64> {
        self.verb_number("lan_game_count", &[])
    }

    /// The name of one LAN game found, 1 for the first; empty past the end.
    pub fn lan_game_name(&self, index: f64) -> Option<String> {
        self.verb_text("lan_game_name", &[PluginArg::from(index)])
    }

    /// Whether this run is open to LAN guests.
    pub fn hosting_lan(&self) -> Option<bool> {
        self.verb_number("hosting_lan", &[]).map(|n| n != 0.0)
    }
}
