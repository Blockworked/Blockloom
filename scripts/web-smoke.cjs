// Headless smoke run of a single-file web build: opens the page straight
// from disk, waits for it to unpack, clicks to play, and checks the world
// built, its scripts opened, and the world moves. Movement is read from the
// player's `game_actors()`, since headless Chromium can't screenshot a
// WebGPU canvas.
//
//   node scripts/web-smoke.cjs <game.html> [--scripts N] [--moves ACTOR]
//
// Needs Playwright (`npm i -g playwright`, found through NODE_PATH) and a
// Chromium with WebGPU; software Vulkan (lavapipe/SwiftShader) is enough.

const path = require("path");
const { chromium } = require("playwright");

const args = process.argv.slice(2);
const page_file = args.find((arg) => !arg.startsWith("--"));
const option = (name) => {
  const at = args.indexOf(name);
  return at >= 0 ? args[at + 1] : undefined;
};
if (!page_file) {
  console.error("usage: node scripts/web-smoke.cjs <game.html> [--scripts N] [--moves ACTOR]");
  process.exit(2);
}
const want_scripts = option("--scripts");
const moves = option("--moves");

function fail(message) {
  console.error("web smoke: FAILED: " + message);
  process.exit(1);
}

(async () => {
  const browser = await chromium.launch({
    executablePath: process.env.CHROMIUM || undefined,
    // Chromium's Vulkan path (lavapipe in CI); its SwiftShader default loses
    // the device on start here.
    args: ["--enable-unsafe-webgpu", "--ignore-gpu-blocklist", "--enable-features=Vulkan", "--use-vulkan=native", "--use-angle=vulkan"],
  });
  const page = await browser.newPage({ viewport: { width: 800, height: 600 } });
  const log = [];
  page.on("console", (message) => log.push(`[${message.type()}] ${message.text()}`));
  page.on("pageerror", (error) => log.push(`[pageerror] ${error.message}`));
  const dump = () => console.error(log.filter((line) => !line.includes("un-applied commands")).slice(-40).join("\n"));

  await page.goto("file://" + path.resolve(page_file));
  const ready = await page
    .waitForFunction(
      () => window.blockloomReady || document.getElementById("problem").style.display === "block",
      null,
      { timeout: 120000 },
    )
    .then(() => page.evaluate(() => !!window.blockloomReady))
    .catch(() => false);
  if (!ready) {
    dump();
    fail(await page.evaluate(() => document.getElementById("problem").textContent || "never unpacked"));
  }
  await page.click("#play");

  const started = Date.now();
  let built;
  while (Date.now() - started < 120000) {
    built = log.find((line) => line.includes("built ") && line.includes(" actors"));
    if (built) break;
    await page.waitForTimeout(250);
  }
  if (!built) {
    dump();
    fail("the world never built");
  }
  console.log("web smoke: " + built.replace(/^\[\w+\]\s*/, ""));
  if (want_scripts !== undefined) {
    const opened = /opened (\d+) scripts/.exec(built);
    if (!opened || opened[1] !== want_scripts) {
      dump();
      fail(`expected ${want_scripts} scripts open`);
    }
  }

  // Warm-up holds the green flag until pipelines compile, which on a
  // software adapter can take a while, so poll rather than sample once.
  const actors = () => page.evaluate(() => JSON.parse(window.blockloom.game_actors()));
  const errors = () => log.filter((line) => /panic|\[pageerror\]|blockloom: /.test(line));
  const before = await actors();
  if (moves !== undefined) {
    const id = Object.keys(before).find((id) => before[id].name === moves);
    if (!id) fail(`no actor called ${moves}: ${JSON.stringify(before)}`);
    const where = (all) => JSON.stringify(all[id] && all[id].position);
    let now = where(before);
    const until = Date.now() + 60000;
    while (now === where(before) && Date.now() < until && !errors().length) {
      await page.waitForTimeout(1000);
      now = where(await actors());
    }
    if (now === where(before) && !errors().length) fail(`${moves} stood still at ${now}`);
    console.log(`web smoke: ${moves} moved from ${where(before)} to ${now}`);
  } else {
    await page.waitForTimeout(3000);
  }
  if (errors().length) {
    dump();
    fail("errors in the console:\n" + errors().join("\n"));
  }
  console.log("web smoke: ok, the game is running");
  await browser.close();
})().catch((error) => fail(error.stack || String(error)));
