// Component-model shim for the C# guest: exports the world's `entry`
// interface into managed code and offers the imports to P/Invoke as plain C.
#include <stdlib.h>
#include <string.h>
#include <stdio.h>
#include <mono/metadata/assembly.h>
#include <mono/metadata/class.h>
#include <mono/metadata/image.h>
#include <mono/metadata/object.h>
#include <mono/metadata/exception.h>
#include "gen/script.h"

extern int initialize_runtime(void);
extern const char *dotnet_wasi_getentrypointassemblyname(void);
extern void mono_print_unhandled_exception(MonoObject *exc);

static MonoImage *image;
static MonoMethod *m_start, *m_tick, *m_frame, *m_ui, *m_stop, *m_destroy, *m_event;

static MonoMethod *find(MonoClass *klass, const char *name, int argc) {
    MonoMethod *method = mono_class_get_method_from_name(klass, name, argc);
    if (!method) {
        fprintf(stderr, "Blockloom.Host.%s not found\n", name);
        __builtin_trap();
    }
    return method;
}

static void ensure(void) {
    if (image) return;
    initialize_runtime();
    MonoAssembly *assembly = mono_assembly_open(dotnet_wasi_getentrypointassemblyname(), NULL);
    if (!assembly) {
        fprintf(stderr, "guest assembly not found\n");
        __builtin_trap();
    }
    image = mono_assembly_get_image(assembly);
    MonoClass *host = mono_class_from_name(image, "Blockloom", "Host");
    if (!host) {
        fprintf(stderr, "Blockloom.Host not found\n");
        __builtin_trap();
    }
    m_start = find(host, "Start", 0);
    m_tick = find(host, "Tick", 1);
    m_frame = find(host, "Frame", 1);
    m_ui = find(host, "Ui", 1);
    m_stop = find(host, "Stop", 0);
    m_destroy = find(host, "Destroy", 0);
    m_event = find(host, "OnEvent", 5);
}

static void invoke(MonoMethod *method, void **args) {
    MonoObject *exc = NULL;
    mono_runtime_invoke(method, NULL, args, &exc);
    if (exc) {
        mono_print_unhandled_exception(exc);
        __builtin_trap();
    }
}

void exports_blockloom_script_entry_start(void) { ensure(); invoke(m_start, NULL); }
void exports_blockloom_script_entry_tick(float dt) { ensure(); void *a[] = {&dt}; invoke(m_tick, a); }
void exports_blockloom_script_entry_frame(float dt) { ensure(); void *a[] = {&dt}; invoke(m_frame, a); }
void exports_blockloom_script_entry_ui(float dt) { ensure(); void *a[] = {&dt}; invoke(m_ui, a); }
void exports_blockloom_script_entry_stop(void) { ensure(); invoke(m_stop, NULL); }
void exports_blockloom_script_entry_destroy(void) { ensure(); invoke(m_destroy, NULL); }
void exports_blockloom_script_entry_on_event(uint32_t kind, double n0, double n1, double n2, double n3) {
    ensure();
    void *a[] = {&kind, &n0, &n1, &n2, &n3};
    invoke(m_event, a);
}

// Imports for managed code. Text goes in as NUL-terminated UTF-8.
static script_string_t str(const char *s) {
    script_string_t out = {(uint8_t *)s, s ? strlen(s) : 0};
    return out;
}

// Copies a host string into buf (truncated to cap), frees it, returns its full length.
static int take(script_string_t *text, char *buf, int cap) {
    int full = (int)text->len, n = full < cap ? full : cap;
    memcpy(buf, text->ptr, n);
    script_string_free(text);
    return full;
}

// 0 = ok, 1 = missing.
int bl_read(uint32_t what, const char *a, const char *b, double arg, double *out) {
    script_string_t sa = str(a), sb = str(b);
    blockloom_script_sensors_read_error_t err;
    return blockloom_script_sensors_read(what, &sa, &sb, arg, out, &err) ? 0 : 1 + err;
}

// Copies into buf. Returns the length, or -1 missing, -2 too long for the host.
int bl_read_text(uint32_t what, const char *a, const char *b, char *buf, int cap) {
    script_string_t sa = str(a), sb = str(b), ret;
    blockloom_script_texts_read_error_t err;
    if (!blockloom_script_texts_read_text(what, &sa, &sb, &ret, &err)) return err == BLOCKLOOM_SCRIPT_TEXTS_READ_ERROR_TOO_LONG ? -2 : -1;
    return take(&ret, buf, cap);
}

void bl_act(uint32_t what, const char *a, const char *b, const char *c, const double *numbers, int count) {
    script_string_t sa = str(a), sb = str(b), sc = str(c);
    script_list_f64_t list = {(double *)numbers, (size_t)count};
    blockloom_script_acts_act(what, &sa, &sb, &sc, &list);
}

// Per-actor state: 0 = ok, 1 = missing.
int bl_state_get_number(const char *key, double *out) {
    script_string_t k = str(key);
    blockloom_script_state_read_error_t err;
    return blockloom_script_state_get_number(&k, out, &err) ? 0 : 1;
}
int bl_state_get_text(const char *key, char *buf, int cap) {
    script_string_t k = str(key), ret;
    blockloom_script_state_read_error_t err;
    return blockloom_script_state_get_text(&k, &ret, &err) ? take(&ret, buf, cap) : -1;
}
void bl_state_set_number(const char *key, double value) { script_string_t k = str(key); blockloom_script_state_set_number(&k, value); }
void bl_state_set_text(const char *key, const char *value) { script_string_t k = str(key), v = str(value); blockloom_script_state_set_text(&k, &v); }
void bl_state_clear(const char *key) { script_string_t k = str(key); blockloom_script_state_clear(&k); }

// Project variables.
double bl_var_get_number(const char *name) { script_string_t n = str(name); return blockloom_script_vars_get_number(&n); }
int bl_var_get_text(const char *name, char *buf, int cap) {
    script_string_t n = str(name), ret;
    blockloom_script_vars_get_text(&n, &ret);
    return take(&ret, buf, cap);
}
void bl_var_set_number(const char *name, double value) { script_string_t n = str(name); blockloom_script_vars_set_number(&n, value); }
void bl_var_set_text(const char *name, const char *value) { script_string_t n = str(name), v = str(value); blockloom_script_vars_set_text(&n, &v); }

// Project lists.
int bl_list_len(const char *name) { script_string_t n = str(name); return (int)blockloom_script_lists_len(&n); }
int bl_list_get_number(const char *name, int index, double *out) {
    script_string_t n = str(name);
    blockloom_script_lists_read_error_t err;
    return blockloom_script_lists_get_number(&n, (uint32_t)index, out, &err) ? 0 : 1;
}
int bl_list_get_text(const char *name, int index, char *buf, int cap) {
    script_string_t n = str(name), ret;
    blockloom_script_lists_read_error_t err;
    return blockloom_script_lists_get_text(&n, (uint32_t)index, &ret, &err) ? take(&ret, buf, cap) : -1;
}
void bl_list_add_number(const char *name, double value) { script_string_t n = str(name); blockloom_script_lists_add_number(&n, value); }
void bl_list_add_text(const char *name, const char *value) { script_string_t n = str(name), v = str(value); blockloom_script_lists_add_text(&n, &v); }
void bl_list_insert_number(const char *name, int index, double value) { script_string_t n = str(name); blockloom_script_lists_insert_number(&n, (uint32_t)index, value); }
void bl_list_insert_text(const char *name, int index, const char *value) { script_string_t n = str(name), v = str(value); blockloom_script_lists_insert_text(&n, (uint32_t)index, &v); }
void bl_list_replace_number(const char *name, int index, double value) { script_string_t n = str(name); blockloom_script_lists_replace_number(&n, (uint32_t)index, value); }
void bl_list_replace_text(const char *name, int index, const char *value) { script_string_t n = str(name), v = str(value); blockloom_script_lists_replace_text(&n, (uint32_t)index, &v); }
void bl_list_delete(const char *name, int index) { script_string_t n = str(name); blockloom_script_lists_delete(&n, (uint32_t)index); }
void bl_list_clear(const char *name) { script_string_t n = str(name); blockloom_script_lists_clear(&n); }
