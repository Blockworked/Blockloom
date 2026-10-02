//! Background child processes used by desktop tooling.

use std::ffi::OsStr;
use std::process::Command;

pub fn background_command(program: impl AsRef<OsStr>) -> Command {
    let command = Command::new(program);
    #[cfg(windows)]
    let command = {
        let mut command = command;
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW avoids stealing focus.
        command
    };
    command
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn background_child_has_no_console() {
        let output = background_command(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "process::tests::check_child_console",
                "--nocapture",
            ])
            .env("BLOCKLOOM_TEST_CHILD_CONSOLE", "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
    }

    #[test]
    fn check_child_console() {
        if std::env::var_os("BLOCKLOOM_TEST_CHILD_CONSOLE").is_none() {
            return;
        }
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetConsoleWindow() -> *mut std::ffi::c_void;
        }
        assert!(unsafe { GetConsoleWindow() }.is_null());
    }
}
