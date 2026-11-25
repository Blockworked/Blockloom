//! What language a script asset is written in, and how Play builds it.
//!
//! Rust is compiled by [`super::compile_for`] into a native library (plus a
//! wasm module for the sandbox). Every other language is a WebAssembly guest:
//! its own toolchain turns the source into one `.wasm` beside the Rust
//! builds, which the runtime loads as a component or a core module. The
//! toolchains are not Blockloom's, so each one says how to install itself
//! rather than failing with whatever the shell printed.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Where a guest build's `.wasm` lands, under the project's build folder.
pub const GUEST_DIR: &str = "guest";

/// A script asset's language, by file extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Language {
    /// `.rs`, compiled to a native library (and a wasm module).
    Rust,
    /// `.c`, one clang run into a core wasm module.
    C,
    /// `.py`, a component through componentize-py.
    Python,
    /// `.ts`, compiled to JavaScript and then a component through jco.
    TypeScript,
    /// `.js`/`.mjs`, a component through jco.
    JavaScript,
    /// `.go`, TinyGo plus wasm-tools.
    Go,
    /// `.cs`, .NET on wasi-wasm.
    CSharp,
    /// `.kt`, Kotlin/Wasm WASI.
    Kotlin,
    /// `.wasm`, already built by somebody else's toolchain.
    Prebuilt,
}

impl Language {
    /// The language of a script path, or `None` for an extension no language
    /// claims.
    pub fn of(relative: &str) -> Option<Self> {
        let lower = relative.to_ascii_lowercase();
        let (_, extension) = lower.rsplit_once('.')?;
        Some(match extension {
            "rs" => Self::Rust,
            "c" => Self::C,
            "py" => Self::Python,
            "ts" => Self::TypeScript,
            "js" | "mjs" => Self::JavaScript,
            "go" => Self::Go,
            "cs" => Self::CSharp,
            "kt" => Self::Kotlin,
            "wasm" => Self::Prebuilt,
            _ => return None,
        })
    }

    /// Every language a script can be written in, Rust first.
    pub fn every() -> &'static [Self] {
        &[
            Self::Rust,
            Self::C,
            Self::Python,
            Self::TypeScript,
            Self::JavaScript,
            Self::Go,
            Self::CSharp,
            Self::Kotlin,
        ]
    }

    /// The language called `name`: its own name, or its extension.
    pub fn named(name: &str) -> Option<Self> {
        let wanted = name.trim().trim_start_matches('.').to_ascii_lowercase();
        if wanted.is_empty() {
            return None;
        }
        Self::of(&format!("x.{wanted}")).or_else(|| {
            Self::every()
                .iter()
                .copied()
                .chain([Self::Prebuilt])
                .find(|language| language.name().to_ascii_lowercase() == wanted)
        })
    }

    /// What it is called in a message or a menu.
    pub fn name(self) -> &'static str {
        match self {
            Self::Rust => "Rust",
            Self::C => "C",
            Self::Python => "Python",
            Self::TypeScript => "TypeScript",
            Self::JavaScript => "JavaScript",
            Self::Go => "Go",
            Self::CSharp => "C#",
            Self::Kotlin => "Kotlin",
            Self::Prebuilt => "WebAssembly",
        }
    }

    /// The extension a new script of this language gets.
    pub fn extension(self) -> &'static str {
        match self {
            Self::Rust => "rs",
            Self::C => "c",
            Self::Python => "py",
            Self::TypeScript => "ts",
            Self::JavaScript => "mjs",
            Self::Go => "go",
            Self::CSharp => "cs",
            Self::Kotlin => "kt",
            Self::Prebuilt => "wasm",
        }
    }

    /// Whether Play compiles this language itself (Rust) rather than through
    /// a guest toolchain.
    pub fn is_rust(self) -> bool {
        self == Self::Rust
    }

    /// Whether a path in this language is already built.
    pub fn is_prebuilt(self) -> bool {
        self == Self::Prebuilt
    }

    /// The template a new script of this language starts from, as the guest
    /// kit names it.
    pub fn template(self) -> String {
        format!("minimal.{}", self.extension())
    }

    /// The template file's name in the guest kit.
    fn template_file(self) -> String {
        format!("templates/{}", self.template())
    }

    /// The programs its toolchain needs, in the order they run.
    pub fn programs(self) -> &'static [&'static str] {
        match self {
            Self::Rust | Self::Prebuilt => &[],
            Self::C => &["clang"],
            Self::Python => &["componentize-py"],
            Self::TypeScript => &["npx"],
            Self::JavaScript => &["npx"],
            Self::Go => &["tinygo", "wasm-tools"],
            Self::CSharp => &["dotnet", "wit-bindgen"],
            Self::Kotlin => &["gradle", "wasm-tools"],
        }
    }

    /// How to get the toolchain, shown when a program is missing.
    pub fn install_hint(self) -> &'static str {
        match self {
            Self::Rust => "a Rust toolchain ships with Blockloom",
            Self::Prebuilt => "nothing to install: the file is already built",
            Self::C => "install clang (the LLVM toolchain) and make sure it is on PATH",
            Self::Python => "pip install componentize-py",
            Self::TypeScript | Self::JavaScript => {
                "install Node.js; jco is fetched by npx on first use"
            }
            Self::Go => "install TinyGo and wasm-tools (cargo install wasm-tools)",
            Self::CSharp => "install the .NET 10 SDK and wit-bindgen",
            Self::Kotlin => "install Gradle (a JDK with it) and wasm-tools",
        }
    }

    /// The first program of its toolchain this machine hasn't got, if any.
    pub fn missing_program(self) -> Option<&'static str> {
        self.programs()
            .iter()
            .copied()
            .find(|program| !on_path(program))
    }

    /// Whether this machine can build this language right now, or why not.
    pub fn available(self) -> Result<(), String> {
        match self.missing_program() {
            None => Ok(()),
            Some(program) => Err(format!(
                "{} scripts need {program}, which isn't on this machine: {}",
                self.name(),
                self.install_hint()
            )),
        }
    }
}

fn on_path(program: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| {
        let candidate = dir.join(program);
        candidate.is_file() || candidate.with_extension("exe").is_file()
    })
}

/// The guest kit: the per-language build scripts, bindings and templates of
/// `blockloom-script-guest`. `BLOCKLOOM_SCRIPT_GUEST` names it, else it is
/// `script-guest/` beside the editor in an install, else the repo's own
/// folder in a development build.
pub fn guest_kit() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("BLOCKLOOM_SCRIPT_GUEST").map(PathBuf::from)
        && dir.is_dir()
    {
        return Some(dir);
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(parent) = exe.parent()
    {
        let beside = parent
            .ancestors()
            .take(2)
            .map(|dir| dir.join("script-guest"))
            .find(|dir| dir.is_dir());
        if beside.is_some() {
            return beside;
        }
    }
    let repo = crate::android::workspace_root()?.join("blockloom-script-guest");
    repo.is_dir().then_some(repo)
}

/// The source a new script in `language` starts from: the guest kit's own
/// template, with the actor's name in its first line.
pub fn starter(language: Language, actor: &str) -> Result<String, String> {
    let kit = guest_kit().ok_or_else(|| {
        format!(
            "a {} script starts from Blockloom's script guest kit, which isn't beside the editor: set BLOCKLOOM_SCRIPT_GUEST",
            language.name()
        )
    })?;
    let path = kit.join(language.template_file());
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(text.replacen("Minimal", &format!("{actor}'s minimal"), 1))
}

/// Where a guest language's build writes its module.
pub fn guest_wasm_path(project_dir: &Path, relative: &str) -> PathBuf {
    project_dir
        .join(super::BUILD_DIR)
        .join(GUEST_DIR)
        .join(format!("{}.wasm", super::crate_name_of(relative)))
}

/// Builds one guest-language script into its `.wasm`, reusing the last build
/// while the source, the ABI and the toolchain are the same.
pub fn build_guest(project_dir: &Path, relative: &str) -> Result<PathBuf, String> {
    let language = Language::of(relative)
        .ok_or_else(|| format!("\"{relative}\" has no language Blockloom can build"))?;
    if language.is_rust() || language.is_prebuilt() {
        return Err(format!("{} scripts aren't built this way", language.name()));
    }
    language.available()?;
    let source_path = project_dir.join(relative);
    let source =
        std::fs::read(&source_path).map_err(|e| format!("{}: {e}", source_path.display()))?;
    let out = guest_wasm_path(project_dir, relative);
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    let stamp = out.with_extension("stamp");
    let wanted = guest_stamp(language, &source);
    if out.is_file() && std::fs::read_to_string(&stamp).is_ok_and(|held| held == wanted) {
        return Ok(out);
    }
    let _ = std::fs::remove_file(&stamp);
    let kit = guest_kit().ok_or_else(|| {
        format!(
            "{} scripts need Blockloom's script guest kit, which isn't beside the editor: set BLOCKLOOM_SCRIPT_GUEST",
            language.name()
        )
    })?;
    run_guest_build(language, &kit, &source_path, &out)?;
    if !out.is_file() {
        return Err(format!(
            "the {} toolchain reported success but wrote no {}",
            language.name(),
            out.display()
        ));
    }
    std::fs::write(&stamp, wanted).map_err(|e| format!("{}: {e}", stamp.display()))?;
    Ok(out)
}

/// What a guest build has to match to be reused.
fn guest_stamp(language: Language, source: &[u8]) -> String {
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    source.hash(&mut hash);
    format!(
        "abi {}\nlanguage {}\nsource {} bytes {:016x}\n",
        super::abi::ABI_VERSION,
        language.name(),
        source.len(),
        hash.finish()
    )
}

/// Runs the language's own toolchain. C is one clang run, since it needs no
/// component tooling; the rest go through the kit's build script, which is
/// the same one the guest's own tests use.
fn run_guest_build(
    language: Language,
    kit: &Path,
    source: &Path,
    out: &Path,
) -> Result<(), String> {
    let mut command = match language {
        Language::C => {
            let mut clang = Command::new("clang");
            clang
                .arg("--target=wasm32")
                .arg("-O2")
                .arg("-nostdlib")
                .arg("-Wl,--no-entry")
                .arg("-Wl,--export-memory")
                .arg("-I")
                .arg(kit.join("c"))
                .arg("-o")
                .arg(out)
                .arg(source);
            clang
        }
        _ => {
            let script = kit.join(script_folder(language)).join("build.sh");
            if !script.is_file() {
                return Err(format!(
                    "the script guest kit has no {} build script at {}",
                    language.name(),
                    script.display()
                ));
            }
            let mut shell = Command::new("bash");
            shell.arg(&script).arg(source).arg(out);
            shell
        }
    };
    let output = command
        .output()
        .map_err(|e| format!("couldn't start the {} toolchain: {e}", language.name()))?;
    if output.status.success() {
        return Ok(());
    }
    let mut said = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if said.is_empty() {
        said = String::from_utf8_lossy(&output.stdout).trim().to_string();
    }
    Err(said)
}

/// Which folder of the kit holds a language's build script.
fn script_folder(language: Language) -> &'static str {
    match language {
        Language::CSharp => "csharp",
        Language::Kotlin => "kotlin",
        Language::Python => "python",
        Language::TypeScript | Language::JavaScript => "js",
        Language::Go => "go",
        Language::C | Language::Rust | Language::Prebuilt => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_language_answers_to_its_name_or_its_extension() {
        assert_eq!(Language::named("python"), Some(Language::Python));
        assert_eq!(Language::named("C#"), Some(Language::CSharp));
        assert_eq!(Language::named(".go"), Some(Language::Go));
        assert_eq!(Language::named("mjs"), Some(Language::JavaScript));
        assert_eq!(Language::named("fortran"), None);
        assert_eq!(Language::named(""), None);
    }

    #[test]
    fn a_scripts_language_is_its_extension() {
        assert_eq!(Language::of("assets/scripts/a.rs"), Some(Language::Rust));
        assert_eq!(Language::of("assets/scripts/a.PY"), Some(Language::Python));
        assert_eq!(
            Language::of("assets/scripts/a.mjs"),
            Some(Language::JavaScript)
        );
        assert_eq!(
            Language::of("assets/scripts/a.wasm"),
            Some(Language::Prebuilt)
        );
        assert_eq!(Language::of("assets/scripts/a.txt"), None);
        assert_eq!(Language::of("assets/scripts/noextension"), None);
    }

    #[test]
    fn every_language_names_itself_and_its_toolchain() {
        for language in Language::every() {
            assert!(!language.name().is_empty());
            assert!(!language.extension().is_empty());
            assert!(!language.install_hint().is_empty());
            assert_eq!(
                Language::of(&format!("a.{}", language.extension())).is_some(),
                true
            );
            if !language.is_rust() {
                assert!(!language.programs().is_empty(), "{}", language.name());
            }
        }
    }

    #[test]
    fn a_missing_toolchain_says_how_to_get_it() {
        // Nothing is on an empty PATH, so every guest language reports itself.
        temp_env_path("", || {
            for language in Language::every() {
                if language.is_rust() {
                    continue;
                }
                let error = language.available().expect_err(language.name());
                assert!(error.contains(language.name()));
                assert!(error.contains(language.install_hint()));
            }
        });
    }

    fn temp_env_path(value: &str, body: impl FnOnce()) {
        let held = std::env::var_os("PATH");
        // Safe here: these tests don't read PATH from another thread.
        unsafe { std::env::set_var("PATH", value) };
        body();
        match held {
            Some(path) => unsafe { std::env::set_var("PATH", path) },
            None => unsafe { std::env::remove_var("PATH") },
        }
    }
}
