#[cfg(test)]
mod guard_tests {
    use super::*;

    extern "C" fn number(_: *mut c_void, _: u32, _: Str, _: Str, _: f64, _: *mut f64) -> u32 {
        MISSING
    }

    extern "C" fn text(
        _: *mut c_void,
        _: u32,
        _: Str,
        _: Str,
        _: *mut u8,
        _: usize,
        _: *mut usize,
    ) -> u32 {
        MISSING
    }

    extern "C" fn act(
        ctx: *mut c_void,
        what: u32,
        a: Str,
        _: Str,
        _: Str,
        _: *const f64,
        _: usize,
    ) {
        assert_eq!(what, ACT_LOG);
        unsafe { &mut *ctx.cast::<Vec<String>>() }.push(unsafe { a.as_str() }.to_string());
    }

    static API: HostApi = HostApi {
        abi: ABI_VERSION,
        read_number: number,
        read_text: text,
        act,
    };

    fn start(_: &Actor) {
        panic!("start failed");
    }
    fn tick(_: &Actor, _: f32) {
        panic!("tick failed: {}", 42);
    }
    fn event(_: &Actor, _: &Event) {
        std::panic::panic_any(42u32);
    }
    crate::export!(start = start, tick = tick, event = event);

    #[test]
    fn exported_callbacks_report_the_panic_site_and_keep_running() {
        let mut logs = Vec::<String>::new();
        let ctx = (&raw mut logs).cast();
        blockloom_script_start(ctx, &API);
        blockloom_script_tick(ctx, &API, 0.1);
        blockloom_script_event(ctx, &API, EVENT_CLICKED, 0.0, 0.0, 0.0, 0.0);
        blockloom_script_tick(ctx, &API, 0.1);
        assert_eq!(logs.len(), 4);
        for (log, callback, message) in [
            (&logs[0], "start", "start failed"),
            (&logs[1], "tick", "tick failed: 42"),
            (&logs[2], "event", "non-string panic payload"),
            (&logs[3], "tick", "tick failed: 42"),
        ] {
            assert!(
                log.starts_with(&format!("the script panicked in {callback} at ")),
                "{log}"
            );
            let function = format!("fn {callback}(");
            let line = include_str!("guard_tests.rs")
                .lines()
                .position(|line| line.trim_start().starts_with(&function))
                .unwrap()
                + 2;
            assert!(log.contains(&format!("guard_tests.rs:{line}:")), "{log}");
            assert!(log.ends_with(message), "{log}");
        }
    }

    #[test]
    fn nested_guards_and_threads_keep_their_own_panic_details() {
        let threads: Vec<_> = (0..4)
            .map(|id| {
                std::thread::spawn(move || {
                    let mut logs = Vec::<String>::new();
                    let actor = unsafe { Actor::from_raw((&raw mut logs).cast(), &API) };
                    guard(&actor, "outer", || {
                        guard(&actor, "inner", || panic!("inner {id}"));
                        panic!("outer {id}");
                    });
                    guard(&actor, "success", || {});
                    guard(&actor, "later", || panic!("later {id}"));
                    assert_eq!(logs.len(), 3);
                    assert!(logs[0].ends_with(&format!("inner {id}")));
                    assert!(logs[1].ends_with(&format!("outer {id}")));
                    assert!(logs[2].ends_with(&format!("later {id}")));
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
    }
}
