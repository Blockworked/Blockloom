/* Minimal C guest: the same contract as minimal.rs and the sandbox's WAT
 * fixture, through the C binding. `start` says hello, `tick` reads its x and
 * steps +1 along it, `event` answers. Built with:
 *
 *   clang --target=wasm32 -O2 -nostdlib -Wl,--no-entry -Wl,--export-memory \
 *         -I c -o minimal.wasm templates/minimal.c
 */
#include "blockloom.h"

BL_EXPORT("blockloom_script_abi") u32 blockloom_script_abi(void) { return BL_ABI_VERSION; }

BL_EXPORT("blockloom_script_start") void script_start(i32 ctx, i32 api) {
    (void)api;
    bl_enter(ctx);
    bl_say("hello from wasm");
    bl_set_data("ticks", 0.0);
    bl_switch_save_slot("Slot 2");
    bl_set_language("fr");
}

BL_EXPORT("blockloom_script_tick") void script_tick(i32 ctx, i32 api, float dt) {
    (void)api;
    (void)dt;
    bl_enter(ctx);
    double x = bl_axis(0);
    bl_set_data("ticks", 1.0);
    bl_set_data("x_seen", x);
    bl_change_position(0, 1.0);
}

BL_EXPORT("blockloom_script_event")
void script_event(i32 ctx, i32 api, i32 kind, double n0, double n1, double n2, double n3) {
    (void)api; (void)kind; (void)n0; (void)n1; (void)n2; (void)n3;
    bl_enter(ctx);
    bl_say("event heard");
}
