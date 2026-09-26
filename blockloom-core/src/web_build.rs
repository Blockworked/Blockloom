//! The single-file web build: one `.html` carrying the wasm player, its JS
//! glue, the pack, every game file and each script's wasm module.
//!
//! Everything goes into one gzip'd archive, base64'd into the page, which the
//! page unpacks with the browser's own `DecompressionStream` and hands to the
//! player from memory (see `blockloom-runtime/src/web.rs`). Gzip first keeps
//! the file near the size the parts would be served compressed anyway;
//! base64 then adds a third, which is the price of opening from anywhere,
//! `file://` included, with no server.

use base64::Engine;
use flate2::Compression;
use flate2::write::GzEncoder;
use std::io::Write;

/// The staged web player's two files, as `just web-player` names them.
pub const PLAYER_WASM: &str = "blockloom_runtime_bg.wasm";
pub const PLAYER_GLUE: &str = "blockloom_runtime.js";

/// Where each part sits in the archive. Game files go under [`GAME`] with
/// their path in the game folder.
const ENTRY_WASM: &str = "player.wasm";
const ENTRY_GLUE: &str = "player.js";
const GAME: &str = "game/";

/// Packs `(path, bytes)` entries: a little-endian `u32` name length, the
/// name, a `u32` data length, the data - the shape the page's `unpack` reads.
pub fn archive(entries: &[(String, Vec<u8>)]) -> Result<Vec<u8>, String> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::best());
    for (name, data) in entries {
        let name_len = u32::try_from(name.len()).map_err(|_| format!("{name}: path too long"))?;
        let data_len = u32::try_from(data.len())
            .map_err(|_| format!("{name} is over 4 GB, too big for a web build"))?;
        encoder
            .write_all(&name_len.to_le_bytes())
            .and_then(|_| encoder.write_all(name.as_bytes()))
            .and_then(|_| encoder.write_all(&data_len.to_le_bytes()))
            .and_then(|_| encoder.write_all(data))
            .map_err(|e| format!("couldn't compress {name}: {e}"))?;
    }
    encoder
        .finish()
        .map_err(|e| format!("couldn't compress the web build: {e}"))
}

/// Reads an [`archive`] back, gzip and all. The page does this in JS; this is
/// its twin for tests.
#[cfg(test)]
pub fn unarchive(gzipped: &[u8]) -> Vec<(String, Vec<u8>)> {
    use std::io::Read;
    let mut bytes = Vec::new();
    flate2::read::GzDecoder::new(gzipped)
        .read_to_end(&mut bytes)
        .unwrap();
    let mut entries = Vec::new();
    let mut at = 0;
    let next = |at: &mut usize, len: usize| {
        let part = bytes[*at..*at + len].to_vec();
        *at += len;
        part
    };
    while at < bytes.len() {
        let len = u32::from_le_bytes(next(&mut at, 4).try_into().unwrap()) as usize;
        let name = String::from_utf8(next(&mut at, len)).unwrap();
        let len = u32::from_le_bytes(next(&mut at, 4).try_into().unwrap()) as usize;
        entries.push((name, next(&mut at, len)));
    }
    entries
}

/// Everything one page is made of.
pub struct Page<'a> {
    pub title: &'a str,
    /// A PNG, shown as the tab's icon.
    pub icon: &'a [u8],
    pub player_wasm: Vec<u8>,
    pub player_glue: Vec<u8>,
    /// `(path in the game folder, bytes)`, `game.pack` among them.
    pub files: Vec<(String, Vec<u8>)>,
}

/// The page itself.
pub fn page(page: Page) -> Result<String, String> {
    let mut entries = vec![
        (ENTRY_GLUE.to_string(), page.player_glue),
        (ENTRY_WASM.to_string(), page.player_wasm),
    ];
    entries.extend(
        page.files
            .into_iter()
            .map(|(path, bytes)| (format!("{GAME}{path}"), bytes)),
    );
    let data = base64::engine::general_purpose::STANDARD.encode(archive(&entries)?);
    let icon = base64::engine::general_purpose::STANDARD.encode(page.icon);
    Ok(TEMPLATE
        .replace("{{TITLE}}", &escape(page.title))
        .replace("{{ICON}}", &icon)
        .replace("{{DATA}}", &data))
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// The host page. `{{DATA}}` is the base64 archive; nothing in base64 can
/// close the script element it sits in.
const TEMPLATE: &str = r##"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1, user-scalable=no">
<title>{{TITLE}}</title>
<link rel="icon" href="data:image/png;base64,{{ICON}}">
<style>
html, body { margin: 0; height: 100%; background: #1b212c; overflow: hidden; }
#stage { position: fixed; inset: 0; }
#stage canvas { display: block; width: 100%; height: 100%; outline: none; touch-action: none; }
#cover { position: fixed; inset: 0; display: flex; flex-direction: column; align-items: center;
  justify-content: center; gap: 16px; background: #1b212c; color: #e8ecf3;
  font: 16px system-ui, sans-serif; cursor: default; }
#cover h1 { margin: 0; font-size: 28px; font-weight: 600; }
#cover button { font: inherit; font-size: 18px; padding: 12px 28px; border: 0; border-radius: 8px;
  background: #4f7cff; color: white; cursor: pointer; }
#cover button:disabled { background: #3a4252; color: #9aa3b5; cursor: progress; }
#problem { position: fixed; left: 16px; right: 16px; bottom: 16px; padding: 12px 16px;
  background: #3a1414; color: #ffd7d7; font: 14px sans-serif; white-space: pre-wrap;
  border: 1px solid #a33; border-radius: 8px; display: none; }
</style>
</head>
<body>
<div id="stage"><canvas id="blockloom-canvas" tabindex="0"></canvas></div>
<div id="cover"><h1>{{TITLE}}</h1><button id="play" disabled>Loading…</button></div>
<div id="problem"></div>
<script type="application/octet-stream" id="blockloom-data">{{DATA}}</script>
<script type="module">
const problem = (message) => {
  const box = document.getElementById("problem");
  box.textContent = "Blockloom: " + message;
  box.style.display = "block";
};
window.addEventListener("error", (event) => problem(event.message));
window.addEventListener("unhandledrejection", (event) => problem(String(event.reason)));

// The archive: gzip'd entries of a u32 name length, the name, a u32 data
// length and the data (see `web_build.rs`).
async function unpack() {
  const text = document.getElementById("blockloom-data").textContent.trim();
  const gzipped = await (await fetch("data:application/octet-stream;base64," + text)).blob();
  const stream = gzipped.stream().pipeThrough(new DecompressionStream("gzip"));
  const bytes = new Uint8Array(await new Response(stream).arrayBuffer());
  const view = new DataView(bytes.buffer);
  const names = new TextDecoder();
  const files = {};
  let at = 0;
  while (at < bytes.length) {
    const nameLength = view.getUint32(at, true); at += 4;
    const name = names.decode(bytes.subarray(at, at + nameLength)); at += nameLength;
    const length = view.getUint32(at, true); at += 4;
    files[name] = bytes.subarray(at, at + length); at += length;
  }
  return files;
}

try {
  // The player renders through WebGPU; say so plainly rather than failing
  // somewhere inside it.
  const adapter = navigator.gpu && await navigator.gpu.requestAdapter();
  if (!adapter) {
    throw new Error("This game needs WebGPU, which this browser doesn't offer. " +
      "Try a current Chrome, Edge, Firefox or Safari.");
  }
  const files = await unpack();
  const glue = URL.createObjectURL(new Blob([files["player.js"]], { type: "text/javascript" }));
  const player = await import(glue);
  await player.default({ module_or_path: files["player.wasm"] });
  const pack = new TextDecoder().decode(files["game/game.pack"]);
  // Game files by their path in the game folder; script libraries compiled
  // here, since a large module can't be compiled synchronously.
  const game = {};
  const scripts = {};
  for (const [name, data] of Object.entries(files)) {
    if (!name.startsWith("game/")) continue;
    const path = name.slice(5);
    if (path.startsWith(".blockloom/build/") && path.endsWith(".wasm")) {
      scripts[path] = await WebAssembly.compile(data);
    } else if (path !== "game.pack") {
      game[path] = data;
    }
  }
  const play = document.getElementById("play");
  play.textContent = "▶ Click to play";
  play.disabled = false;
  // Starting on a click is what lets the game make sound.
  play.addEventListener("click", () => {
    document.getElementById("cover").remove();
    const canvas = document.getElementById("blockloom-canvas");
    canvas.focus();
    player.start_game(pack, "#blockloom-canvas", game, scripts);
  }, { once: true });
  window.blockloom = player;
  window.blockloomReady = true;
} catch (error) {
  document.getElementById("play").textContent = "Can't start";
  problem(error && error.message ? error.message : String(error));
  console.error(error);
}
</script>
</body>
</html>
"##;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_archive_reads_back_entry_for_entry() {
        let entries = vec![
            ("player.js".to_string(), b"export default 1".to_vec()),
            ("game/assets/a b.png".to_string(), vec![0, 1, 2, 255]),
            ("game/empty".to_string(), Vec::new()),
        ];
        assert_eq!(unarchive(&archive(&entries).unwrap()), entries);
    }

    #[test]
    fn a_page_carries_everything_and_escapes_its_title() {
        let html = page(Page {
            title: "Pond <Game>",
            icon: b"png",
            player_wasm: b"\0asm".to_vec(),
            player_glue: b"glue".to_vec(),
            files: vec![("game.pack".to_string(), b"{}".to_vec())],
        })
        .unwrap();
        assert!(html.contains("<title>Pond &lt;Game&gt;</title>"));
        let start = html.find("id=\"blockloom-data\">").unwrap() + "id=\"blockloom-data\">".len();
        let end = start + html[start..].find("</script>").unwrap();
        let gzipped = base64::engine::general_purpose::STANDARD
            .decode(&html[start..end])
            .unwrap();
        let names: Vec<String> = unarchive(&gzipped).into_iter().map(|(n, _)| n).collect();
        assert_eq!(names, ["player.js", "player.wasm", "game/game.pack"]);
    }
}
