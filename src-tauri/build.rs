fn main() {
    build_frontend();
    tauri_build::build();
}

// `frontendDist` (`../ui/dist`) doesn't exist until Vite runs, and a plain
// `cargo build` never goes through the Tauri CLI to trigger that - so build it
// here instead.
fn build_frontend() {
    let ui_dir = std::path::Path::new("..").join("ui");

    for watched in ["index.html", "src", "package.json", "vite.config.ts"] {
        println!("cargo:rerun-if-changed={}", ui_dir.join(watched).display());
    }

    // pnpm may be a `.cmd` shim (npm global) or `pnpm.exe` (WinGet/scoop) on
    // Windows; try both, then fall back to PATH resolution.
    let pnpm = if cfg!(windows) {
        ["pnpm.cmd", "pnpm.exe", "pnpm"]
            .iter()
            .find(|name| {
                std::process::Command::new("where")
                    .arg(name)
                    .output()
                    .is_ok_and(|out| out.status.success())
            })
            .copied()
            .unwrap_or("pnpm")
    } else {
        "pnpm"
    };

    let run = |args: &[&str]| {
        let status = std::process::Command::new(pnpm)
            .args(args)
            .current_dir(&ui_dir)
            .status()
            .unwrap_or_else(|e| {
                panic!("failed to run `pnpm {}` in {ui_dir:?}: {e}", args.join(" "))
            });
        if !status.success() {
            panic!(
                "`pnpm {}` in {ui_dir:?} failed with {status}",
                args.join(" ")
            );
        }
    };

    run(&["install"]);
    run(&["run", "build"]);
}
