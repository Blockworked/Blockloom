# Android Mali Investigation

Status on 2026-09-30: native memory exhaustion remains unresolved. The black
screen was traced to the default cinematic fade, not a blank GPU render target.
Survival while black is not a successful stability test. Do not treat the
point/spot shadow gate as proven.

## Evidence

- EmberNoShadow's tombstone 27 records SIGABRT in the Render thread after 2,198
  seconds (36.6 minutes). Earlier 24-minute survival was an intermediate sample,
  not evidence of indefinite operation. The original panic log had rotated.
- A fresh Emberwatch baseline died after about 111 seconds. Logcat records failed
  malloc allocations, then `gpu_readback.rs:409` buffer mapping failure. Cleanup
  subsequently panics while destroying an acquired swapchain semaphore and aborts.
  The cleanup stack is secondary, not the original cause of memory exhaustion.
- Baseline pipeline counts plateau at 88 render / 46 compute. Gated runs plateau
  at 75 / 46. Runtime assets also plateau. The runtime specialization audit found
  no unbounded frame-varying keys, including `post::Guide` (a bounded enum).
- Before the preprocess cache, indexed bin-unpacking and uniform-allocation bind
  groups were each recreated thousands of times per five-second sample. The
  Mali-only cache removes this steady allocation churn after warmup. This is
  evidence for reuse, not proof that the native memory leak is fixed.
- GPU capture of Emberwatch's active 960 x 2142 window returned 2,056,320 black
  pixels. A minimal three-actor 3D project also rendered black. Foreground state
  was checked before Android screenshots.
- Stage captures were non-black through postprocessing and black after UI.
  The UI census identified a full-screen `cine-fade` with alpha 1. Derived
  `CinePlayer::default()` used an empty fade string, but only `"none"` disables
  the fade. The explicit default now uses `"none"`; regression tests cover a
  normal run and restarting after an active fade. Foreground screenshots and
  GPU readback verified visible title UI and gameplay. Temporary stage captures
  and verbose UI-node dumps were removed after diagnosis. The later cleanup also
  removed the watchdog, counting allocator, asset/pipeline census, terrain stream
  traces and dependency allocation/cache counters. Panic stacktraces and the
  runtime fixes remain; the measurements below describe the instrumented builds.
- The fade-fixed Emberwatch build was verified in foreground, then entered
  gameplay with "Begin the Vigil". It ran for about 11.5 minutes without an
  abort. A ten-minute memory series rose from 4,044,743 to 4,333,334 KiB PSS,
  with the last few minutes nearly flat. Pipelines plateaued at 77 / 46,
  tracked GPU allocations at about 867 MB and Rust allocations at 639 MB.
  Later inspection showed that an unattended attempt reaches "The Light Fades"
  after about 48 seconds and the project calls `PauseGame`. Most later samples
  therefore exercise rendering behind the game-over UI, not active simulation.
  In-game "Try again" broadcasts `new_vigil` and resumes the same process.
  Neither this comparison nor a stable game-over screen proves indefinite play.
- Tombstone 29 is a distinct SurfaceFlinger RenderEngine crash. Its abort message
  reports `VK_ERROR_DEVICE_LOST`, `CS_INHERIT_FAULT`, and fault VA `0x5ffffc6ff8`.
  This explains a system compositor restart during one debugger attempt; it
  does not establish that Emberwatch caused that fault.
- A Perfetto heap sample contains Mali command-buffer allocation/command-pool
  retention stacks. The profiler significantly stalled the app, so this capture
  cannot establish a healthy steady-state allocation slope or prove a leak.

The existing Android serial-render guard now includes nested schedules in the
render sub-app. This alone did not fix black frames. Bevy also finishes
pending command encoders on a task pool independently of schedule executors.

## Command Pool Candidate

An unproven Android/Mali-only candidate requests `RELEASE_RESOURCES` every 64
resets of each Vulkan command pool. The existing wgpu reset point already waits
for its command buffers to finish. Other platforms and vendors are unchanged.
The goal is to bound retained command storage suggested by the intrusive heap
profile, not to assert that the profile established a leak. Compare against the
fade-fixed run before treating this candidate as a fix.

The isolated candidate ran for about 11.5 minutes without a native abort or
readback failure. Its ten-minute series starts at 3,362,905 and ends at 3,622,562
KiB PSS, with periodic decreases and later samples around 3.3-3.5 GiB. The earlier
run ended near 4.13 GiB. Two in-game retries resumed simulation without restarting
the native process. The automatic game-over pause still limits active-play
coverage, and one pair of runs cannot exclude startup variability. Keep this
candidate experimental until repeated, longer active-play tests pass. Pipelines
were bounded at 76-77 render / 46 compute; tracked GPU memory stayed around
867 MB, with 1.411 GB reserved versus 1.546 GB in the earlier run.

Vulkan documents resource release in the [command-pool reset flags](https://docs.vulkan.org/refpages/latest/refpages/source/VkCommandPoolResetFlagBits.html)
and the required non-pending state in [vkResetCommandPool](https://docs.vulkan.org/refpages/latest/refpages/source/vkResetCommandPool.html).

## Controlled Builds

```sh
just build
printf '%s\n' \
  'open-project path=/home/treetrain1/Blockloom/projects/Emberwatch force=true' \
  'build-game path=/home/treetrain1/RustroverProjects/Blockloom/target/mali-test target=aarch64-linux-android' \
  | ./target/debug/blockloom-shell --no-state
adb install -r 'target/mali-test/Emberwatch (Android (arm64))/Emberwatch.apk'
adb logcat -c
adb shell am start -n com.blockloom.game.emberwatch/android.app.NativeActivity
adb logcat -v threadtime -s RustStdoutStderr:V blockloom:V
```

Dependency patches must invalidate the cached Android runtime library as well as
runtime source edits. The Android builder runs `scripts/prepare-patched-deps.sh`
before checking its cache. `runtime_sources_newer_than` includes `patches/`,
`.patched-deps/`, and the preparation script for this. Former vendor changes are
stored as diffs; see [patched dependencies](../patches/README.md).
Use `log::error!` for vendor diagnostics because the Bevy filter suppresses lower
wgpu levels. The direct `blockloom` Android log tag survives stdio disruption.

## Native Crash And Live Stacks

This device allows shell access to textual tombstones without root:

```sh
adb shell ls -lt /data/tombstones
adb pull /data/tombstones/tombstone_29 /tmp/blockloom-tombstone-29.txt
adb logcat -b crash -d
adb shell dumpsys activity exit-info com.blockloom.game.emberwatch
```

Symbolize library-relative PCs with the exact `.so` from the crashed APK, not a
subsequent build. The NDK's `llvm-symbolizer` accepts `--obj=<library>` followed
by the tombstone's relative hexadecimal addresses on stdin.

For live stacks, use a temporary debug-signed APK with
`android:debuggable="true"` in its generated manifest. This is a diagnostic APK
only; do not change the shipping manifest template. A `profileable` flag alone
does not enable `run-as`.

```sh
NDK=$HOME/Blockloom/android-sdk/ndk/27.3.13750724
HOST=$NDK/toolchains/llvm/prebuilt/linux-x86_64
APP=com.blockloom.game.emberwatch
adb push "$HOST/lib/clang/18/lib/linux/aarch64/lldb-server" /data/local/tmp/blockloom-lldb-server
adb shell run-as "$APP" cp /data/local/tmp/blockloom-lldb-server ./lldb-server
adb shell run-as "$APP" chmod 700 ./lldb-server
PID=$(adb shell pidof "$APP")
adb forward tcp:5039 tcp:5039
adb shell run-as "$APP" ./lldb-server gdbserver 127.0.0.1:5039 --attach "$PID"
```

Run the host debugger in another terminal. The NDK `lldb.sh` configures its Python
and shared-library paths. Minimal module loading avoids a lengthy remote module
scan before stack capture:

```sh
"$HOST/bin/lldb.sh" -b \
  -o 'settings set target.memory-module-load-level minimal' \
  -o 'gdb-remote 5039' \
  -o 'thread backtrace all' \
  -o 'process detach' \
  -o quit
adb shell run-as "$APP" cat /proc/"$PID"/maps
adb forward --remove tcp:5039
```

Subtract each library's load bias from raw LLDB PCs for offline symbolization.
Detaching resumes the app. Pausing for stack capture perturbs timing and is not a
stability test. Restore the ordinary APK and remove debugger files afterward.

## Test Hygiene

Do not reboot routinely. Do not infer foreground rendering from a background
screenshot. Check `dumpsys activity activities` for `mFocusedApp` first. Record
PID, elapsed time, PSS/RSS, native heap, graphics memory, pipeline totals, asset
totals and whether the image is actually visible. A flat asset count does not
imply flat driver memory. Uninstall disposable test apps after testing.
