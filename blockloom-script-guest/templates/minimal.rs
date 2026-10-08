// Minimal guest: the same contract as the sandbox's WAT fixture, written
// against the Rust guest binding instead of raw imports. `start` says hello,
// `tick` reads its x and steps +1 along it, `event` answers. Built with:
//
// ```sh
// cargo build --target wasm32-unknown-unknown
// ```
//
// or with two `rustc` runs (the guest `rlib`, then this file as a `cdylib`
// over it), which is what the sandbox test does.

use blockloom_script_guest::{Event, export_script, lifecycle, locale, saves, state, vars};

fn start() {
    lifecycle::say("hello from wasm");
    state::set_number("ticks", 0.0);
    saves::switch_save_slot("Slot 2");
    locale::set_language("fr");
}

fn tick(_dt: f32) {
    let x = lifecycle::x();
    state::set_number("ticks", state::get_number("ticks") + 1.0);
    vars::set_number("shared", f64::from(x));
    lifecycle::change_position(0, 1.0);
}

fn event(_event: &Event) {
    lifecycle::say("event heard");
}

export_script!(start = start, tick = tick, event = event);
