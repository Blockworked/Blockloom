/* C binding for the frozen blockloom:script world, over the same three core
 * wasm imports the Rust guest uses. Verb numbers come from verbs.rs (a core
 * drift test holds them to abi.rs). Freestanding: no libc, no WASI.
 *
 * clang --target=wasm32 -O2 -nostdlib -Wl,--no-entry -Wl,--export-memory \
 *       -I c -o script.wasm templates/minimal.c
 */
#ifndef BLOCKLOOM_H
#define BLOCKLOOM_H

typedef unsigned int u32;
typedef int i32;

#define BL_ABI_VERSION 47u
#define BL_OK 0u
#define BL_TOO_LONG 2u

#define BL_READ_POSITION 1u
#define BL_READ_ROTATION 2u
#define BL_READ_SCALE 3u
#define BL_READ_VISIBLE 4u
#define BL_READ_TIMER 5u
#define BL_READ_KEY_DOWN 6u
#define BL_READ_MOUSE 7u
#define BL_READ_MOUSE_DOWN 8u
#define BL_READ_MOUSE_DELTA 16u
#define BL_READ_TOUCHING 9u
#define BL_READ_DISTANCE_TO 10u
#define BL_READ_HAS_COMPONENT 11u
#define BL_READ_FIELD 12u
#define BL_READ_POSITION_OF 13u
#define BL_READ_IS_CLONE 14u
#define BL_READ_ACTOR_COUNT 15u
#define BL_READ_MOUSE_LOCKED 17u
#define BL_READ_UI_VALUE 18u
#define BL_READ_GAME_PAUSED 19u
#define BL_READ_UI_SHOWN 20u
#define BL_READ_UI_EXISTS 21u
#define BL_READ_SOUND_PLAYING 22u
#define BL_READ_BUS_VOLUME 23u
#define BL_READ_LOCAL_POSITION 24u
#define BL_READ_LOCAL_POSITION_OF 25u
#define BL_READ_IS_TRIGGER 26u
#define BL_READ_COLLISION_LAYER 27u
#define BL_READ_RAY_DISTANCE 28u
#define BL_READ_CIRCLE_HIT_OF 29u
#define BL_READ_ACTION_DOWN 30u
#define BL_READ_ACTION_PRESSED 31u
#define BL_READ_ACTION_RELEASED 32u
#define BL_READ_ACTION_VALUE 33u
#define BL_READ_TOUCH_COUNT 34u
#define BL_READ_TOUCH 35u
#define BL_READ_GAMEPAD_CONNECTED 36u
#define BL_READ_GAMEPAD_AXIS 37u
#define BL_READ_GAMEPAD_BUTTON 38u
#define BL_READ_MOUSE_BUTTON 39u
#define BL_READ_ATMOSPHERE 40u
#define BL_READ_IS_TWEENING 41u
#define BL_READ_ANIM_FRAME 42u
#define BL_READ_ANIM_PLAYING 43u
#define BL_READ_CASTS_SHADOWS 44u
#define BL_READ_WATER 45u
#define BL_READ_UNDERWATER 46u
#define BL_READ_PARTICLES 47u
#define BL_READ_FRAME_TIME 49u
#define BL_READ_DRAW_CALLS 50u
#define BL_READ_DLSS_AVAILABLE 51u
#define BL_READ_CUTSCENE_TIME 52u
#define BL_READ_TILE_AT 48u
#define BL_READ_PLUGIN 53u
#define BL_READ_QUERY 54u
#define BL_READ_CONTROLLER 55u
#define BL_READ_VELOCITY 56u
#define BL_READ_ANGULAR_VELOCITY 57u
#define BL_READ_MASS 58u
#define BL_READ_GROUNDED 59u
#define BL_READ_DATA 60u
#define BL_READ_VARIABLE 61u
#define BL_READ_LIST_LENGTH 62u
#define BL_READ_LIST_ITEM 63u
#define BL_TEXT_CURRENT_QUALITY 18u
#define BL_TEXT_ACTOR_NAME 1u
#define BL_TEXT_FIELD 2u
#define BL_TEXT_ACTOR_ID 3u
#define BL_TEXT_PARENT 4u
#define BL_TEXT_NEW_ACTOR 5u
#define BL_TEXT_UI_VALUE 6u
#define BL_TEXT_UI_TEXT 7u
#define BL_TEXT_UI_FOCUS 8u
#define BL_TEXT_RAY_HIT 9u
#define BL_TEXT_CIRCLE_HIT 10u
#define BL_TEXT_CURRENT_CLIP 11u
#define BL_TEXT_ACTIVE_VOLUMES 12u
#define BL_TEXT_EVENT 13u
#define BL_TEXT_ROOM 14u
#define BL_TEXT_ENTERED_ROOM 15u
#define BL_TEXT_CURRENT_SCENE 16u
#define BL_TEXT_SCENE_NAMES 17u
#define BL_TEXT_CURRENT_WEATHER 19u
#define BL_TEXT_CUTSCENE_NAME 20u
#define BL_TEXT_DATA 25u
#define BL_TEXT_VARIABLE 26u
#define BL_TEXT_LIST_ITEM 27u
#define BL_TEXT_SAVE_SLOT 28u
#define BL_TEXT_SAVE_SLOTS 29u
#define BL_TEXT_LANGUAGE 30u
#define BL_TEXT_LOCALE_TEXT 31u
#define BL_TEXT_PLUGIN 21u
#define BL_TEXT_QUERY 22u
#define BL_TEXT_CONTROLLER 23u
#define BL_ACT_MOVE 1u
#define BL_ACT_GO_TO 2u
#define BL_ACT_CHANGE_POSITION 3u
#define BL_ACT_TURN 4u
#define BL_ACT_SET_ROTATION 5u
#define BL_ACT_POINT_TOWARDS 6u
#define BL_ACT_SET_SCALE 7u
#define BL_ACT_APPLY_IMPULSE 8u
#define BL_ACT_SET_VELOCITY 9u
#define BL_ACT_SAY 10u
#define BL_ACT_SET_VISIBLE 11u
#define BL_ACT_SET_COLOR 12u
#define BL_ACT_BROADCAST 13u
#define BL_ACT_SET_FIELD 14u
#define BL_ACT_SET_FIELD_TEXT 15u
#define BL_ACT_ATTACH 16u
#define BL_ACT_DETACH 17u
#define BL_ACT_SET_CAMERA_VIEW 18u
#define BL_ACT_LOG 19u
#define BL_ACT_STOP_ALL 20u
#define BL_ACT_SET_PARENT 21u
#define BL_ACT_CREATE_CLONE 22u
#define BL_ACT_CREATE_ACTOR 23u
#define BL_ACT_DELETE_ACTOR 24u
#define BL_ACT_SET_MOUSE_LOCKED 25u
#define BL_ACT_SET_CAMERA_PITCH 26u
#define BL_ACT_SET_CAMERA_FOV 36u
#define BL_ACT_UI_SHOW 27u
#define BL_ACT_UI_SET 28u
#define BL_ACT_UI_SET_TEXT 29u
#define BL_ACT_UI_HIDE 30u
#define BL_ACT_UI_DELETE 31u
#define BL_ACT_SET_PAUSED 32u
#define BL_ACT_UI_FOCUS 33u
#define BL_ACT_UI_THEME 34u
#define BL_ACT_SAVE_VARIABLE 35u
#define BL_ACT_NAVIGATE_TO 37u
#define BL_ACT_PLAY_SOUND 38u
#define BL_ACT_STOP_SOUND 39u
#define BL_ACT_SET_SOUND_VOLUME 40u
#define BL_ACT_SET_SOUND_PITCH 41u
#define BL_ACT_SET_BUS_VOLUME 42u
#define BL_ACT_SET_TRIGGER 43u
#define BL_ACT_SET_COLLISION_LAYER 44u
#define BL_ACT_SET_COLLISION_MASK 45u
#define BL_ACT_RUMBLE_GAMEPAD 46u
#define BL_ACT_BIND_ACTION 47u
#define BL_ACT_CLEAR_ACTION_BINDINGS 48u
#define BL_ACT_SET_RENDER_SETTING 110u
#define BL_ACT_SET_EXPOSURE 49u
#define BL_ACT_SET_LIGHT_INTENSITY 50u
#define BL_ACT_TWEEN_SCALE 51u
#define BL_ACT_TWEEN_ROTATION 52u
#define BL_ACT_TWEEN_COLOR 53u
#define BL_ACT_STOP_TWEENS 54u
#define BL_ACT_PLAY_ANIMATION 55u
#define BL_ACT_STOP_ANIMATION 56u
#define BL_ACT_SET_ANIMATION_SPEED 57u
#define BL_ACT_SET_EMISSIVE_STRENGTH 58u
#define BL_ACT_SET_HDR_OUTPUT 59u
#define BL_ACT_SET_PEAK_BRIGHTNESS 60u
#define BL_ACT_ENABLE_VOLUME 61u
#define BL_ACT_SET_VOLUME_WEIGHT 62u
#define BL_ACT_CAPTURE_PROBES 63u
#define BL_ACT_SET_SHADOW_DISTANCE 64u
#define BL_ACT_SET_LIGHT_SHADOWS 65u
#define BL_ACT_SET_RAY_TRACING 66u
#define BL_ACT_SET_GI_BOUNCES 67u
#define BL_ACT_SET_GI_SAMPLES 68u
#define BL_ACT_SET_FOG_DENSITY 69u
#define BL_ACT_SET_AURORA 70u
#define BL_ACT_STRIKE_LIGHTNING 71u
#define BL_ACT_SET_LIGHTNING_RATE 72u
#define BL_ACT_SET_WIND 73u
#define BL_ACT_SET_CLOUD_DRIFT 74u
#define BL_ACT_SET_CLOUDS 75u
#define BL_ACT_SET_CLOUD_LAYER 76u
#define BL_ACT_SET_WATER 77u
#define BL_ACT_FIRE_ANIMATION_TRIGGER 78u
#define BL_ACT_SET_RIG_SLOT 79u
#define BL_ACT_SET_SLOT_TINT 80u
#define BL_ACT_SET_IK_TARGET 81u
#define BL_ACT_SET_SPRITE_DIAL 82u
#define BL_ACT_BURST_PARTICLES 83u
#define BL_ACT_SET_EMITTER_DIAL 84u
#define BL_ACT_SET_EMITTER_PLAYING 85u
#define BL_ACT_PAINT_TILE 86u
#define BL_ACT_SET_PARALLAX 87u
#define BL_ACT_SWITCH_SCENE 88u
#define BL_ACT_SET_TIME_OF_DAY 89u
#define BL_ACT_ADVANCE_TIME 90u
#define BL_ACT_SET_PRECIPITATION 91u
#define BL_ACT_BLEND_WEATHER 92u
#define BL_ACT_PLAY_CUTSCENE 93u
#define BL_ACT_SKIP_CUTSCENE 94u
#define BL_ACT_CAMERA_SHAKE 95u
#define BL_ACT_SET_TIME_SCALE 96u
#define BL_ACT_HITSTOP 97u
#define BL_ACT_SET_LETTERBOX 98u
#define BL_ACT_FADE_SCREEN 99u
#define BL_ACT_PLUGIN_CALL 100u
#define BL_ACT_ADD_FORCE 101u
#define BL_ACT_PHYSICS_QUERY 102u
#define BL_ACT_CONTROLLER 103u
#define BL_ACT_SET_DATA 104u
#define BL_ACT_SET_DATA_TEXT 105u
#define BL_ACT_CLEAR_DATA 106u
#define BL_ACT_GO_TO_OTHER 111u
#define BL_ACT_CHANGE_POSITION_OTHER 112u
#define BL_ACT_MOVE_OTHER 113u
#define BL_ACT_TURN_OTHER 114u
#define BL_ACT_SET_ROTATION_OTHER 115u
#define BL_ACT_SET_SCALE_OTHER 116u
#define BL_ACT_POINT_TOWARDS_OTHER 117u
#define BL_ACT_SET_VISIBLE_OTHER 118u
#define BL_ACT_SET_COLOR_OTHER 119u
#define BL_ACT_SAY_OTHER 120u
#define BL_ACT_APPLY_IMPULSE_OTHER 121u
#define BL_ACT_SET_VELOCITY_OTHER 122u
#define BL_ACT_ADD_FORCE_OTHER 123u
#define BL_ACT_SET_VARIABLE 124u
#define BL_ACT_SET_VARIABLE_TEXT 125u
#define BL_ACT_LIST_ADD 126u
#define BL_ACT_LIST_ADD_TEXT 127u
#define BL_ACT_LIST_INSERT 128u
#define BL_ACT_LIST_INSERT_TEXT 129u
#define BL_ACT_LIST_REPLACE 130u
#define BL_ACT_LIST_REPLACE_TEXT 131u
#define BL_ACT_LIST_DELETE 132u
#define BL_ACT_LIST_CLEAR 133u
#define BL_ACT_SWITCH_SAVE_SLOT 134u
#define BL_ACT_DELETE_SAVE_SLOT 135u
#define BL_ACT_SET_LANGUAGE 136u
#define BL_EVENT_MESSAGE 1u
#define BL_EVENT_KEY 2u
#define BL_EVENT_ACTION 3u
#define BL_EVENT_CLICKED 4u
#define BL_EVENT_TOUCHED 5u
#define BL_EVENT_COLLISION 6u
#define BL_EVENT_PARTICLES 7u
#define BL_EVENT_ANIMATION_ENDED 8u
#define BL_EVENT_ANIMATION_MARKER 9u
#define BL_EVENT_UI_CLICKED 10u
#define BL_EVENT_UI_CHANGED 11u
#define BL_EVENT_UI 12u
#define BL_EVENT_ENTERED_ROOM 13u
#define BL_EVENT_QUALITY_DROPPED 16u
#define BL_EVENT_SCENE_STARTED 14u
#define BL_EVENT_SCENE_ENDED 15u
#define BL_EVENT_WEATHER 17u
#define BL_EVENT_CUTSCENE_SIGNAL 18u
#define BL_EVENT_CUTSCENE_ENDED 19u
#define BL_EVENT_PLUGIN 20u
#define BL_EVENT_CONTACT 21u

/* The call record the imports take; 56 bytes, field for field the host's
 * WasmCall. */
typedef struct {
    u32 a_ptr, a_len, b_ptr, b_len, c_ptr, c_len;
    u32 numbers, count;
    double arg;
    u32 out, out_cap, out_len, pad;
} bl_call;

#define BL_IMPORT(name) __attribute__((import_module("blockloom"), import_name(name)))
BL_IMPORT("read_number") u32 bl_import_read_number(i32 ctx, u32 what, bl_call *call);
BL_IMPORT("read_text") u32 bl_import_read_text(i32 ctx, u32 what, bl_call *call);
BL_IMPORT("act") u32 bl_import_act(i32 ctx, u32 what, bl_call *call);

#define BL_EXPORT(name) __attribute__((export_name(name)))

static i32 bl_ctx;

static u32 bl_strlen(const char *s) {
    u32 n = 0;
    while (s[n]) n++;
    return n;
}

/* Call at the top of every entry point with its first argument. */
static void bl_enter(i32 ctx) { bl_ctx = ctx; }

/* One numeric read; returns 1 and fills *out when the host has the reading. */
static int bl_read_number(u32 what, const char *a, const char *b, double arg, double *out) {
    bl_call call = {0};
    call.a_ptr = (u32)(unsigned long)a;
    call.a_len = bl_strlen(a);
    call.b_ptr = (u32)(unsigned long)b;
    call.b_len = bl_strlen(b);
    call.arg = arg;
    call.out = (u32)(unsigned long)out;
    return bl_import_read_number(bl_ctx, what, &call) == BL_OK;
}

/* One world write; it applies later, as the same effect the blocks produce. */
static void bl_act(u32 what, const char *a, const char *b, const char *c,
                   const double *numbers, u32 count) {
    bl_call call = {0};
    call.a_ptr = (u32)(unsigned long)a;
    call.a_len = bl_strlen(a);
    call.b_ptr = (u32)(unsigned long)b;
    call.b_len = bl_strlen(b);
    call.c_ptr = (u32)(unsigned long)c;
    call.c_len = bl_strlen(c);
    call.numbers = (u32)(unsigned long)numbers;
    call.count = count;
    bl_import_act(bl_ctx, what, &call);
}

static void bl_say(const char *text) { bl_act(BL_ACT_SAY, text, "", "", 0, 0); }

static double bl_axis(u32 axis) {
    double out = 0.0;
    bl_read_number(BL_READ_POSITION, "", "", (double)axis, &out);
    return out;
}

static void bl_change_position(u32 axis, double by) {
    double n[2] = {(double)axis, by};
    bl_act(BL_ACT_CHANGE_POSITION, "", "", "", n, 2);
}

static void bl_set_data(const char *key, double value) {
    double n[1] = {value};
    bl_act(BL_ACT_SET_DATA, key, "", "", n, 1);
}

static void bl_switch_save_slot(const char *slot) {
    bl_act(BL_ACT_SWITCH_SAVE_SLOT, slot, "", "", 0, 0);
}

static void bl_set_language(const char *language) {
    bl_act(BL_ACT_SET_LANGUAGE, language, "", "", 0, 0);
}

#endif
