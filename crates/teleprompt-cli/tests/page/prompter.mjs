// The prompter page, driven as a person would in Chromium, against a real
// `teleprompt serve` following a recorded reading of apps/fixtures/tour,
// which Chromium plays as its microphone. Served without a script, the
// page welcomes, lists what setup can install, and opens the script
// chosen. Then it records a take, plays the video as it will play, drags
// the shots on the glass and undoes a drag, builds the video, and
// re-records a line reworded in the file, keeping what was said.
//
//   TELEPROMPT_BIN=…/teleprompt TELEPROMPT_MODEL=…/zipformer node prompter.mjs
//
// Needs Chromium (TELEPROMPT_CHROMIUM, or Chrome or Chromium on the PATH),
// ffmpeg, and `npm install` here for playwright-core. Screenshots are left
// in $UI_SHOTS (default: a temporary directory).

import { spawn, execFileSync } from "node:child_process";
import { cpSync, existsSync, mkdirSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { createInterface } from "node:readline";
import { chromium } from "playwright-core";

const bin = process.env.TELEPROMPT_BIN, model = process.env.TELEPROMPT_MODEL;
if (!bin || !model) {
  console.error("set TELEPROMPT_BIN (built with --features listen) and TELEPROMPT_MODEL");
  process.exit(2);
}
const repo = join(dirname(fileURLToPath(import.meta.url)), "../../../..");
const work = mkdtempSync(join(tmpdir(), "teleprompt-page-"));
const shots = process.env.UI_SHOTS ?? join(work, "shots");
mkdirSync(shots, { recursive: true });
const project = join(work, "project"), md = join(project, "scripts/tour.md");
cpSync(join(repo, "apps/fixtures/tour"), project, { recursive: true });
execFileSync(bin, ["capture", "scripts/tour.md"], { cwd: project, stdio: "ignore" });
// A microphone does not stop at the last word: room tone after it, so the
// recognizer hears the reading end.
const reading = join(work, "reading.wav");
execFileSync("ffmpeg", ["-v", "error", "-i",
  join(repo, "crates/teleprompt-listen/tests/fixtures/two-lines.wav"), "-af", "apad=pad_dur=3", reading]);

const executablePath = process.env.TELEPROMPT_CHROMIUM
  ?? ["chromium", "chromium-browser", "google-chrome"].map((name) => {
    try { return execFileSync("sh", ["-c", `command -v ${name}`]).toString().trim(); } catch { return ""; }
  }).find(Boolean);
if (!executablePath) {
  console.error("no Chromium: set TELEPROMPT_CHROMIUM");
  process.exit(2);
}

let failed = 0;
const check = (ok, said, why = "") => {
  console.log(ok ? `ok: ${said}` : `FAIL: ${said}${why ? `\n${why}` : ""}`);
  if (!ok) failed = 1;
};

/** `teleprompt serve` on a port the OS picks, and where it listens;
 *  with no script, the page's welcome. */
async function serve(script = "scripts/tour.md") {
  const server = spawn(bin, ["--format", "json", "serve", ...(script ? [script] : []), "--model", model, "--port", "0"],
    { cwd: project, stdio: ["ignore", "pipe", "inherit"] });
  const [line] = await createInterface({ input: server.stdout })[Symbol.asyncIterator]().next()
    .then(({ value }) => [value]);
  return { server, url: JSON.parse(line).url };
}

/** A browser whose microphone plays the reading, from when it is opened. */
async function browse(url) {
  const browser = await chromium.launch({ executablePath, args: [
    "--use-fake-ui-for-media-stream", "--use-fake-device-for-media-stream",
    `--use-file-for-fake-audio-capture=${reading}%noloop`, "--autoplay-policy=no-user-gesture-required",
  ] });
  const page = await browser.newPage({ viewport: { width: 1400, height: 860 } });
  page.on("pageerror", (e) => check(false, "the page runs without errors", e.message));
  await page.goto(`${url}/?countdown=0`);
  await page.waitForFunction(() => document.querySelectorAll("#script .line").length > 0);
  return { browser, page };
}

const status = (page) => page.textContent("#status");
const until = (page, test, ms = 30000) => page.waitForFunction(test, null, { timeout: ms, polling: 100 });

/** Drags `from`'s centre onto `to`'s, in steps as a hand moves. */
async function drag(page, from, to) {
  const a = await page.locator(from).first().boundingBox(), b = await page.locator(to).boundingBox();
  await page.mouse.move(a.x + a.width / 2, a.y + a.height / 2);
  await page.mouse.down();
  for (let i = 1; i <= 12; i++) {
    await page.mouse.move(a.x + a.width / 2 + ((b.x + b.width / 2) - (a.x + a.width / 2)) * i / 12,
      a.y + a.height / 2 + ((b.y + b.height / 2) - (a.y + a.height / 2)) * i / 12);
  }
  const said = await page.textContent("#drop-chip").catch(() => null);
  await page.mouse.up();
  await until(page, () => document.getElementById("toast").classList.contains("on"));
  await page.waitForTimeout(800);
  return said;
}

// With no script, the page welcomes: it lists the project's scripts, says
// what teleprompt can be set up to do, and opens the script chosen.
{
  const { server, url } = await serve(null);
  const browser = await chromium.launch({ executablePath });
  try {
    const page = await browser.newPage({ viewport: { width: 1400, height: 860 } });
    page.on("pageerror", (e) => check(false, "the welcome runs without errors", e.message));
    await page.goto(`${url}/?countdown=0`);
    await until(page, () => document.querySelectorAll("#scripts button").length > 0, 10000).catch(() => {});
    const listed = await page.$$eval("#scripts button", (b) => b.map((x) => x.textContent));
    check(await page.isVisible("#home") && listed.join() === "tour.md", `the welcome lists the scripts (${listed})`);
    check(await page.isChecked('#narrator input[value="you"]'), "you read, where the speech model is here");
    await page.screenshot({ path: join(shots, "00-welcome.png") });
    await page.click("#setup-open");
    await until(page, () => document.querySelectorAll("#uses li").length > 0, 20000).catch(() => {});
    const uses = await page.$$eval("#uses label > span:not(.use-state)", (s) => s.map((x) => x.textContent));
    check(uses.includes("Render videos") && await page.isDisabled("#setup-install"),
      `Set up lists the uses, nothing ticked (${uses.length})`);
    await page.screenshot({ path: join(shots, "00-setup.png") });
    await page.click("#setup-close");
    await page.click("#scripts button");
    await until(page, () => document.querySelectorAll("#script .line").length > 0, 20000).catch(() => {});
    check(!(await page.isVisible("#home")) && (await page.$$("#script .line")).length > 0,
      "a script chosen opens in the prompter", await status(page));
  } finally {
    await browser.close();
    server.kill();
  }
}

const { server, url } = await serve();
try {
  // A take: the reading followed to the end and kept, a shot on screen on the way.
  let { browser, page } = await browse(url);
  await page.keyboard.press("Control+Shift+Space");
  // What a shot on the monitor looks like is the browser's: a Chromium
  // without H.264 shows the clip's slate. Its title is the page's.
  let shown = "";
  for (let i = 0; i < 150 && !shown; i++) {
    shown = await page.textContent("#shot-title");
    await page.waitForTimeout(100);
  }
  check(shown === "welcome-a", `a shot went on the monitor as the reading reached it (${shown})`);
  await until(page, () => document.querySelectorAll("#script .line")[1].querySelector(".w:last-child.said"));
  await page.screenshot({ path: join(shots, "01-read.png") });
  await page.keyboard.press("Control+Shift+Space");
  await until(page, () => document.getElementById("status").textContent.startsWith("Kept"));
  for (const line of ["welcome", "deploy"]) {
    check(existsSync(join(project, `takes/${line}.json`)), `${line} was kept as a take`);
  }

  // V plays the video as it will play, from the manifest: the takes just
  // kept, the clock running, and the held shot on screen once line 1 ends.
  await page.keyboard.press("v");
  await until(page, () => /^Playing the video/.test(document.getElementById("status").textContent), 60000)
    .catch(() => {});
  await until(page, () => document.getElementById("shot-title").textContent, 20000).catch(() => {});
  const clock = await page.textContent("#timecode"), title = await page.textContent("#shot-title");
  check(/^Playing the video/.test(await status(page)) && clock >= "00:01.0" && title === "welcome-a",
    `V plays the video, its shots where the plan puts them (${clock}, ${title})`, await status(page));
  await page.screenshot({ path: join(shots, "01-video.png") });
  await page.keyboard.press("v");
  await page.waitForTimeout(300);
  check(!/^Playing/.test(await status(page)), "V again stops it", await status(page));

  // Edit mode: a word hovered shows its moment; the shots' ribbons dragged
  // write the script, and Ctrl+Z puts it back.
  await page.keyboard.press("e");
  await page.locator('#script .w[data-l="1"][data-w="7"]').hover();
  await page.waitForTimeout(500);
  check(await page.textContent("#shot-title") === "deploy-a", "a word hovered in Edit mode shows its shot");
  await page.screenshot({ path: join(shots, "02-edit.png") });
  const stretched = await drag(page, '.piece[data-shot="1"] .grip', '#script .w[data-l="1"][data-w="8"]');
  const afterStretch = readFileSync(md, "utf8");
  check(/cue="streams progress" stretch=0\./.test(afterStretch), `a shot's grip dragged is a stretch (${stretched})`, afterStretch);
  const cued = await drag(page, '.piece[data-shot="0"] .pill', '#script .w[data-l="0"][data-w="2"]');
  const afterCue = readFileSync(md, "utf8");
  check((afterCue.match(/cue=/g) ?? []).length === 2, `a shot dragged onto its line is cued there (${cued})`, afterCue);
  await page.keyboard.press("Control+z");
  await until(page, () => document.getElementById("status").textContent === "Undone");
  check(readFileSync(md, "utf8") === afterStretch, "Ctrl+Z undoes the last drag");
  await drag(page, '.piece[data-shot="0"] .pill', '#script .w[data-l="0"][data-w="5"]');
  check(/^```teleprompt scene=mock policy=concurrent cue="show you"$/m.test(readFileSync(md, "utf8")),
    "a held shot dragged onto a word of its line starts there", readFileSync(md, "utf8"));
  await page.screenshot({ path: join(shots, "03-dragged.png") });

  // Build: the shots the drags changed captured again, then the video.
  await page.keyboard.press("e");
  await page.click("#build");
  await until(page, () => /^(Built|Could not)/.test(document.getElementById("status").textContent), 180000);
  check(existsSync(join(project, "build/tour.en.mp4")), "Build made the video", await status(page));
  await page.screenshot({ path: join(shots, "04-built.png") });
  await browser.close();

  // A line reworded in the file is picked up, re-recorded on its own with
  // R, and what was said kept with W.
  writeFileSync(md, readFileSync(md, "utf8").replace("Let me show you around.", "Let me show you around the office!"));
  ({ browser, page } = await browse(url));
  await page.keyboard.press("r");
  await until(page, () => /^Re-recorded/.test(document.getElementById("status").textContent), 40000)
    .catch(() => {});
  check(/^Re-recorded 1 line/.test(await status(page)), "R re-recorded the reworded line, and only it", await status(page));
  await until(page, () => document.querySelector("#script .line.heard"), 10000).catch(() => {});
  await page.keyboard.press("w");
  await page.waitForTimeout(500);
  await page.keyboard.press("Enter");
  await until(page, () => /reads as you said it/.test(document.getElementById("status").textContent), 10000)
    .catch(() => {});
  await page.screenshot({ path: join(shots, "05-kept-said.png") });
  const welcome = JSON.parse(readFileSync(join(project, "takes/welcome.json"), "utf8"));
  check(welcome.text === "Welcome to Acme. Let me show you around!" && /^Welcome to Acme\. Let me show you around! \{#welcome\}$/m.test(readFileSync(md, "utf8")),
    "W kept what was said", `${welcome.text}\n${readFileSync(md, "utf8")}`);
  check(/Deployment is one command/.test(readFileSync(join(project, "takes/deploy.json"), "utf8")),
    "the line not reworded kept its take");
  await browser.close();
} finally {
  server.kill();
}
process.exit(failed);
