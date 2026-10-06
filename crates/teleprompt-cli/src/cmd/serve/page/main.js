// ---- The monitor in a window of its own -----------------------------------

const monitorChannel = "BroadcastChannel" in window ? new BroadcastChannel("teleprompt-monitor") : null;
let popped = false;           // the monitor is in its own window

let shown = {};               // what screen() was last asked to show

/** The monitor showing `title` over `clip` from `at` seconds in (still, if
 *  `paused`), or over a `slate` that says why there is no picture. */
function screen({ title = "", time = "", slate = "", missing = false, clip = null, at = null, paused = false }) {
  shown = { title, slate, missing, clip };
  $("shot-title").textContent = title;
  $("shot-time").textContent = time;
  $("slate").textContent = slate;
  $("slate").classList.toggle("missing", missing);
  const video = $("clip");
  const progress = $("progress").firstElementChild;
  if (!clip) {
    video.pause();
    video.hidden = true;
    progress.style.width = "0";
    return broadcast();
  }
  video.hidden = false;
  const seek = () => {
    const to = Math.min(at, video.duration || at);
    if (paused ? video.currentTime !== to : Math.abs(video.currentTime - to) > 0.3) video.currentTime = to;
  };
  if (video.getAttribute("src") !== clip) {
    progress.style.width = "0";
    video.src = clip;
    if (at) video.addEventListener("loadedmetadata", () => {
      if (video.getAttribute("src") === clip) seek(), broadcast();
    }, { once: true });
  } else if (at !== null) seek();
  if (paused) video.pause();
  else video.play().catch(() => {});
  broadcast();
}

/** What the monitor shows, for its own window to show too. */
function broadcast() {
  if (!monitorChannel || VIEW || !popped) return;
  const video = $("clip");
  monitorChannel.postMessage({ ...shown, time: $("shot-time").textContent, at: video.currentTime, paused: video.paused });
}

function popOut() {
  if (!open("/?view=monitor", "teleprompt-monitor", "popup,width=800,height=520")) {
    return status("Could not open a window for the screen", true);
  }
  popped = true;
  document.body.classList.add("no-monitor");
  drawRibbons();
}

/** In the monitor's own window: shows what the prompter's monitor shows. */
function showMonitor() {
  document.body.classList.add("monitor-view");
  document.title = "Teleprompt · Screen";
  monitorChannel?.addEventListener("message", ({ data }) => {
    if (data !== "hello") screen(data);
  });
  monitorChannel?.postMessage("hello");
}

// ---- The script as its file now reads ------------------------------------

/** Looks again at the script between takes: a line reworded or a shot
 *  moved in the author's editor shows here too. */
async function watch() {
  const away = () => document.hidden || busy() || editing
    || drag || making || voicing || $("panel").open || $("review").open;
  if (away()) return;
  let fresh;
  try {
    fresh = await getScript();
  } catch {
    return;
  }
  if (away()) return;
  showBroken(fresh.error);
  const shape = (s) => JSON.stringify([s.lines.map((l) => l.text), s.shots, s.timeline ?? null]);
  if (shape(fresh) === shape(script)) {
    script.length_ms = fresh.length_ms;
    markRecorded(fresh.lines);
    return keys();
  }
  // A save while the video plays: it plays on from what the save moved,
  // so what was changed is heard at once.
  const wasPlaying = !!reading;
  halt();
  await loadScript(fresh);
  if (manifest) {
    await fetchManifest(true);
    if (wasPlaying) moved.length ? playMoved() : playOrStop();
  }
  keys();
}

/** The errors of a save that does not compile, over the glass; none hides them. */
function showBroken(errors) {
  $("broken").hidden = !errors?.length;
  $("broken").querySelector("pre").textContent = errors?.join("\n") ?? "";
}

function resize(by) {
  const now = parseFloat(getComputedStyle(document.documentElement).getPropertyValue("--size"));
  const size = Math.min(120, Math.max(24, now + by));
  document.documentElement.style.setProperty("--size", `${size}px`);
  try { localStorage.setItem("size", size); } catch {}
  status(`Text size ${size}`);
  glide();
  drawRibbons();
}

function mirror() {
  const on = document.body.classList.toggle("mirrored");
  if (on && editMode) setEdit(false);
  $("mirror").setAttribute("aria-pressed", on);
  try { localStorage.setItem("mirrored", on); } catch {}
}

// This reader's size and mirroring, as they left them.
try {
  const size = parseFloat(localStorage.getItem("size"));
  if (size >= 24 && size <= 120) document.documentElement.style.setProperty("--size", `${size}px`);
  if (localStorage.getItem("mirrored") === "true") mirror();
} catch {}

for (const el of document.querySelectorAll("#help kbd.rec")) el.textContent = RECORD_KEY;
for (const el of document.querySelectorAll("#help kbd.cmd")) el.textContent = MAC ? "⌘" : "Ctrl";
$("smaller").addEventListener("click", () => resize(-4));
$("larger").addEventListener("click", () => resize(4));
$("mirror").addEventListener("click", mirror);
$("keys-help").addEventListener("click", () => $("help").showModal());

$("record").addEventListener("click", recordOrKeep);
$("edit").addEventListener("click", () => setEdit(!editMode));
$("video").addEventListener("click", playOrStop);
$("capture").addEventListener("click", () => make("capture"));
$("build").addEventListener("click", () => make("build"));
$("pop").addEventListener("click", popOut);
document.addEventListener("pointermove", (e) => { if (drag) dragTo(e.clientX, e.clientY); });
document.addEventListener("pointerup", () => { if (drag) dropAt(); });
document.addEventListener("pointercancel", () => { if (drag) endDrag(); });
// In Edit mode, a word hovered is the moment it is said: the monitor shows it.
$("script").addEventListener("pointerover", (e) => {
  const word = editMode && !drag && e.target.closest?.(".w");
  if (word) scrub(moment(Number(word.dataset.l), Number(word.dataset.w)));
});
$("glass").addEventListener("pointerleave", () => { if (editMode && !drag) scrub(null); });
$("clip").addEventListener("timeupdate", (e) => {
  const v = e.target;
  if (!v.duration) return;
  $("progress").firstElementChild.style.width = `${(v.currentTime / v.duration) * 100}%`;
  if (!editMode) $("shot-time").textContent = `${v.currentTime.toFixed(1)} s / ${v.duration.toFixed(1)} s`;
  broadcast();
});
/** The first line and the last can both reach the reading line. */
function margins() {
  const height = $("scroller").clientHeight;
  $("script").style.paddingTop = `${height * READING}px`;
  $("script").style.paddingBottom = `${height * (1 - READING)}px`;
  glide();
  drawRibbons();
}
if (VIEW === "monitor") {
  showMonitor();
} else {
  $("clip").addEventListener("ended", next);
  $("clip").addEventListener("error", (e) => {
    // Only the clip that should be on screen; a load cut short is no error.
    const clip = playing && shots.get(playing)?.clip;
    if (!clip || !e.target.src.endsWith(clip)) return;
    screen({ title: name(playing), slate: "This browser cannot play the clip. Chrome, Edge and Safari can.", missing: true });
  });
  // A monitor window just opened asks what to show.
  monitorChannel?.addEventListener("message", ({ data }) => {
    if (data !== "hello") return;
    popped = true;
    document.body.classList.add("no-monitor");
    drawRibbons();
    broadcast();
  });
  addEventListener("resize", margins);
  margins();
  document.fonts?.ready.then(drawRibbons);
  start();
}

/** Opens what `?open=` names, read as `?narrator=you|voice` says, or the
 *  script the server has open; with none, the welcome. `?setup=a,b`
 *  offers to set those up, saying `?why=`. */
async function start() {
  if (PARAMS.has("open")) return openScript(PARAMS.get("open"), PARAMS.get("narrator") === "voice");
  const r = await fetch(`${API}/script`).catch(() => null);
  if (r?.status === 404) {
    // In an app, the app's own welcome lists the scripts; the page waits
    // to be asked to open one, or to set teleprompt up.
    if (IN_APP) {
      document.body.classList.add("home");
      status("");
    } else await showHome();
  } else {
    await loadScript(r ? await r.json() : null);
    offerSaid();
    if (voiced) {
      voiceStatus();
      voiceUnmade();
    } else status(READY);
    keys();
  }
  setInterval(() => { if (!homeShown) watch(); }, 4000);
  if (PARAMS.has("setup")) openSetup(PARAMS.get("setup").split(",").filter(Boolean), PARAMS.get("why"));
}
