// ---- Read by a voice ----------------------------------------------------

/** The word being said `ms` into a line whose words start at `starts`. */
function wordAt(starts, ms) {
  let w = 0;
  while (w + 1 < starts.length && starts[w + 1] <= ms) w++;
  return w;
}

function clock(ms) {
  const s = Math.floor(ms / 1000);
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

/** Who reads, how long the video runs, and how far the voice has got. */
function voiceStatus() {
  if (!voiced || reading || editing || editMode) return;
  const name = script.voice.name;
  const audio = script.lines.map((l) => l.audio).filter(Boolean);
  const made = audio.filter((a) => a.ready).length;
  if (made < audio.length) return status(`${name} is reading the lines: ${made} of ${audio.length}`);
  const tap = matchMedia("(pointer: coarse)").matches;
  status(`${name} · ${clock(script.length_ms ?? 0)} long. ${tap
    ? "Tap Play to play from here, or tap a line" : "Space plays the video from here; click a line to direct it"}`);
}

/** The script's lines again, for which the voice has made; the whole
 *  script if its words changed. */
async function refreshLines() {
  const fresh = await (await fetch(`${API}/script`)).json();
  const same = fresh.lines.length === script.lines.length
    && fresh.lines.every((l, i) => l.text === script.lines[i].text);
  if (!same && !reading && !editing) return loadScript();
  script.length_ms = fresh.length_ms;
  markRecorded(fresh.lines);
}

let voicing = false;
/** Has the voice make the lines it has yet to, one after another. */
async function voiceUnmade() {
  if (!voiced || voicing) return;
  voicing = true;
  const failed = new Set();
  try {
    for (;;) {
      const l = script.lines.findIndex((line, i) => line.audio && !line.audio.ready && !failed.has(i));
      if (l < 0) break;
      const r = await fetch(script.lines[l].audio.url);
      if (!r.ok) {
        failed.add(l);
        status(`The voice could not read line ${l + 1}: ${(await r.text()).trim()}`, true);
        continue;
      }
      await r.arrayBuffer();
      await refreshLines();
      voiceStatus();
    }
  } finally {
    voicing = false;
  }
}

// ---- The video, as it will play ------------------------------------------
// Played from the manifest `dub` publishes, as an outside renderer reads it:
// every line and shot where the manifest puts it, on one clock, each line at
// the tempo it will have. So it shows no timing the video will not have.

let manifest = null;          // the video as it will play, once asked for
let buffers = new Map();      // a line's audio_hash to its decoded audio
let ctx = null;               // the clock the video plays on
let moved = [];               // what the last save moved: { at, what, id }, in order

function videoClock() {
  if (!ctx) ctx = new AudioContext();
  if (ctx.state === "suspended") ctx.resume();
  return ctx;
}

/** The manifest as the script now reads, every line voiced first; after
 *  a save, also what it moved since the one before. `null`, and why in the
 *  status, if it cannot be made. */
async function fetchManifest(saved = false) {
  const unmade = script.lines.filter((l) => l.audio && !l.audio.ready).length;
  if (unmade) status(`Voicing ${unmade} line${unmade === 1 ? "" : "s"} for the video…`);
  let fresh;
  try {
    const r = await fetch(`${API}/manifest`);
    fresh = await r.json();
    if (!r.ok) throw new Error((fresh.errors ?? []).join("\n") || r.statusText);
    if (fresh.manifest_version !== 3) throw new Error(`a manifest v${fresh.manifest_version}; this page reads v3`);
  } catch (e) {
    status(`The video could not be made: ${String(e.message).split("\n")[0]}`, true);
    return null;
  }
  if (saved && manifest) moved = movedBetween(manifest, fresh);
  manifest = fresh;
  await Promise.all(fresh.lines.map(async (line) => {
    if (buffers.has(line.audio_hash)) return;
    const r = await fetch(`${API}/voice/${encodeURIComponent(line.id)}.wav?fit=1`);
    if (r.ok) buffers.set(line.audio_hash, await videoClock().decodeAudioData(await r.arrayBuffer()));
  }));
  if (saved) markMoved();
  return fresh;
}

/** What moved between two manifests, as places to play from: a line
 *  reworded or moved, a shot changed or moved. */
function movedBetween(before, after) {
  const was = (list, key, id) => list.find((x) => x[key] === id);
  return [
    ...after.lines.filter((l) => {
      const b = was(before.lines, "id", l.id);
      return !b || b.source_hash !== l.source_hash || b.start_ms !== l.start_ms || b.duration_ms !== l.duration_ms;
    }).map((l) => ({ at: l.start_ms, id: l.id, what: `line ${script.lines.findIndex((s) => s.id === l.id) + 1}` })),
    ...after.shots.filter((s) => {
      const b = was(before.shots, "shot", s.shot);
      return !b || b.capture_key !== s.capture_key || b.start_ms !== s.start_ms || b.duration_ms !== s.duration_ms;
    }).map((s) => ({ at: s.start_ms, id: s.shot, what: name(s.shot) })),
  ].sort((a, b) => a.at - b.at);
}

/** The lines a save moved, marked in the margin, and said. */
function markMoved() {
  const ids = new Set(moved.map((m) => m.id));
  lines.forEach((el, l) => el.classList.toggle("moved", ids.has(script.lines[l]?.id)));
  if (moved.length && !reading) {
    status(`Moved since the last save: ${moved.slice(0, 4).map((m) => m.what).join(", ")}${moved.length > 4 ? "…" : ""}. J plays from there`);
  }
}

/** Plays the video from `fromMs`, to `untilMs` or its end. */
async function playVideo(fromMs, untilMs = null) {
  if (listening || paused || counting || editing) return;
  halt();
  $("panel").close();
  const mine = reading = { until: untilMs, since: performance.now(), nodes: [], shot: undefined };
  keys();
  const m = await fetchManifest();
  if (reading !== mine) return;
  if (!m) return halt(), keys();
  const c = videoClock();
  const from = Math.min(Math.max(0, fromMs), m.duration_ms);
  for (const line of m.lines) {
    const buf = buffers.get(line.audio_hash);
    if (!buf || line.start_ms + line.duration_ms <= from) continue;
    if (untilMs !== null && line.start_ms >= untilMs) continue;
    const node = c.createBufferSource();
    node.buffer = buf;
    node.connect(c.destination);
    // Every line against one origin, so none drifts from the others.
    node.start(c.currentTime + Math.max(0, line.start_ms - from) / 1000, Math.max(0, from - line.start_ms) / 1000);
    mine.nodes.push(node);
  }
  mine.origin = c.currentTime - from / 1000;
  mine.since = performance.now() - from;
  status(untilMs === null ? `Playing the video from ${clock(from)}` : "Playing the line");
  frame();
}

/** Where the video is, in milliseconds. */
const videoAt = () => (videoClock().currentTime - reading.origin) * 1000;

/** One frame of the video: the word being said lit, and the shot on screen. */
function frame() {
  if (!reading || reading.origin === undefined) return;
  const t = videoAt();
  const end = reading.until ?? manifest.duration_ms;
  if (t >= end) {
    const only = reading.until !== null;
    halt();
    status(only ? "Played the line" : "Played to the end");
    return keys();
  }
  const line = manifest.lines.find((l) => t >= l.start_ms && t < l.start_ms + l.duration_ms);
  const l = line ? script.lines.findIndex((s) => s.id === line.id) : -1;
  if (l >= 0) {
    const spoken = script.lines[l];
    // The voice's own word timings, fitted to the line's length as played.
    const raw = spoken.audio?.duration_ms;
    const starts = spoken.audio?.words && raw
      ? spoken.audio.words.map((w) => (w * line.duration_ms) / raw)
      : shares(spoken.text, line.duration_ms).map(([s]) => s);
    const word = wordAt(starts, t - line.start_ms);
    if (l !== at.line || word !== at.word) show({ line: l, word });
  }
  screenAt(t);
  reading.raf = requestAnimationFrame(frame);
}

/** The monitor at `t`: the shot the manifest has on screen, as far into
 *  its clip as `t` is into the shot. */
function screenAt(t) {
  const shot = manifest.shots.findLast((s) => s.start_ms <= t && t < s.start_ms + s.duration_ms);
  const video = $("clip");
  const id = shot?.shot ?? null;
  if (id !== reading.shot) {
    reading.shot = id;
    playing = id;
    rundown();
    $("progress").firstElementChild.style.width = "0";
    $("shot-time").textContent = "";
    $("slate").classList.remove("missing");
    const clip = id && shots.get(id)?.clip;
    if (!shot || shot.scene === "pause" || !clip) {
      video.pause();
      video.hidden = true;
      $("shot-title").textContent = id ? name(id) : "";
      $("slate").textContent = !shot ? "No picture at this point"
        : shot.scene === "pause" ? `Holding for ${seconds(shot.duration_ms)}`
        : "This shot was never captured. Capture records it.";
      $("slate").classList.toggle("missing", !!shot && shot.scene !== "pause" && !clip);
      return broadcast();
    }
    $("shot-title").textContent = name(id);
    $("slate").textContent = "";
    video.hidden = false;
    video.src = clip;
    video.currentTime = (t - shot.start_ms) / 1000;
    video.play().catch(() => {});
    return broadcast();
  }
  // A clip that has drifted from the clock is put back on it.
  if (shot && !video.hidden && !video.seeking && Math.abs(video.currentTime * 1000 - (t - shot.start_ms)) > 300) {
    video.currentTime = (t - shot.start_ms) / 1000;
  }
}

/** Ends any playing, quietly. */
function halt() {
  if (!reading) return;
  cancelAnimationFrame(reading.raf);
  for (const node of reading.nodes ?? []) { try { node.stop(); } catch {} }
  reading = null;
  $("clip").pause();
  playing = null;
  rundown();
}

function stopReading(why = null, bad = false) {
  if (!reading) return;
  halt();
  if (why) status(why, bad);
  else status(`Stopped. ${voiced ? "Space" : "V"} plays on from here`);
  keys();
}

function playOrStop() {
  if (editing || listening || paused || counting) return;
  if (reading) return stopReading();
  readFrom(at.line < words.length ? at.line : 0);
}

/** Plays from line `l` to the end, or only it; `fresh` has the voice make
 *  it anew first. */
async function readFrom(l, only = false, fresh = false) {
  const line = script.lines[l];
  if (!line) return;
  if (fresh) {
    status(`Voicing line ${l + 1} again…`);
    const r = await fetch(`${line.audio.url}?fresh=1`);
    if (!r.ok) return status(`The voice failed: ${(await r.text()).trim()}`, true);
    await refreshLines();
  }
  const m = manifest && !fresh ? manifest : await fetchManifest();
  const placed = m?.lines.find((p) => p.id === line.id);
  if (!placed) return status(`Line ${l + 1} has no voice to read it`, true);
  playVideo(placed.start_ms, only ? placed.start_ms + placed.duration_ms : null);
}

/** Five seconds back or on, playing on. */
function skip(ms) {
  if (!reading || reading.origin === undefined) return;
  playVideo(Math.max(0, videoAt() + ms), reading.until);
}

/** Plays from the first thing the last save moved, with a run-in. */
function playMoved() {
  if (moved.length) playVideo(Math.max(0, moved[0].at - 400));
}

/** A line clicked, read by a voice: while it reads, it reads on from
 *  there; otherwise the line is chosen and its panel opens. */
function lineClicked(l) {
  if (reading) return readFrom(l);
  show({ line: l, word: 0 });
  openPanel(l);
}

function openPanel(l) {
  const line = script.lines[l];
  const panel = $("panel");
  const byVoice = line.audio?.source === "voice";
  $("panel-title").textContent = `Line ${l + 1}`;
  $("panel-who").textContent = line.audio?.source === "take" ? "Read from your take"
    : line.speaker ? `Said by ${line.speaker}` : `Read by ${script.voice.name}`;
  $("panel-voice").hidden = !byVoice;
  $("panel-again").hidden = !byVoice;
  $("panel-instruct").value = line.instruct ?? "";
  panel.dataset.line = l;
  panel.show();
  // Under the line, where it settles on the reading line.
  const box = $("glass").getBoundingClientRect();
  const lineBox = lines[l].getBoundingClientRect();
  const first = words[l][0]?.getBoundingClientRect() ?? lineBox;
  const shift = box.top + box.height * READING - (first.top + first.height / 2);
  const top = Math.min(lineBox.bottom + shift + 10, innerHeight - panel.offsetHeight - 12);
  const left = Math.min(Math.max(12, first.left - 16), innerWidth - panel.offsetWidth - 12);
  panel.style.top = `${Math.max(12, top)}px`;
  panel.style.left = `${left}px`;
  $("panel-listen").focus();
}

const panelLine = () => Number($("panel").dataset.line);
$("panel-listen").addEventListener("click", () => readFrom(panelLine(), true));
$("panel-on").addEventListener("click", () => readFrom(panelLine()));
$("panel-again").addEventListener("click", () => readFrom(panelLine(), true, true));
$("panel-reword").addEventListener("click", () => { const l = panelLine(); $("panel").close(); reword(l); });
$("panel-instruct").addEventListener("keydown", (e) => {
  if (e.key === "Escape") { e.preventDefault(); return $("panel").close(); }
  if (e.key !== "Enter") return;
  e.preventDefault();
  const l = panelLine(), line = script.lines[l];
  const text = e.target.value.trim();
  $("panel").close();
  if (text === (line.instruct ?? "")) return;
  sendEdit({ type: "instruct", line: line.id, text: text || null },
    text ? `Line ${l + 1} said ${text}` : `Line ${l + 1} said as the voice would`);
});
// A click away from the panel closes it.
document.addEventListener("pointerdown", (e) => {
  if ($("panel").open && !e.target.closest("#panel") && !e.target.closest(".line")) $("panel").close();
});

/** Sends `edit` over the session socket, to be said as `said` once made. */
async function sendEdit(edit, said) {
  try {
    await openSession();
  } catch (e) {
    return status(e.message, true);
  }
  lastEdit = said;
  undoable = false;
  ws.send(JSON.stringify(edit));
}

/** Puts back what the last edit changed: the server keeps the script as
 *  it was before each. */
async function undoEdit() {
  if (undoEdits === 0 || editing || listening || paused || counting) return;
  try {
    await openSession();
  } catch (e) {
    return status(e.message, true);
  }
  ws.send(JSON.stringify({ type: "undo_edit" }));
}

/** Line `l`'s words become editable where they stand: Enter keeps them,
 *  Escape leaves the line as it was. Not during a take, nor on mirrored
 *  glass, which reads backwards. */
function reword(l) {
  if (editing || !lines[l]) return;
  if (listening || paused || counting) return status("Keep or discard this take first");
  if (document.body.classList.contains("mirrored")) return status("Mirrored text cannot be edited: M turns mirroring off");
  halt();
  $("panel").close();
  show({ line: l, word: 0 });
  const body = lines[l].children[1];
  const editor = document.createElement("textarea");
  editor.className = "line-editor";
  editor.value = script.lines[l].text;
  editor.setAttribute("aria-label", `Line ${l + 1}`);
  const fit = () => { editor.style.height = "auto"; editor.style.height = `${editor.scrollHeight}px`; };
  editor.addEventListener("input", fit);
  editor.addEventListener("keydown", (e) => {
    if (e.key === "Enter") { e.preventDefault(); endReword(true); }
    else if (e.key === "Escape") { e.preventDefault(); endReword(false); }
  });
  editor.addEventListener("click", (e) => e.stopPropagation());
  body.hidden = true;
  lines[l].append(editor);
  editing = { line: l, editor, body };
  fit();
  editor.focus();
  editor.setSelectionRange(editor.value.length, editor.value.length);
  status(`Rewording line ${l + 1}: Enter keeps it, Esc leaves it`);
  keys();
}

function endReword(keepIt) {
  if (!editing) return;
  const { line: l, editor, body } = editing;
  editing = null;
  const text = editor.value.split(/\s+/).filter(Boolean).join(" ");
  editor.remove();
  body.hidden = false;
  lines[l].focus();
  const line = script.lines[l];
  if (keepIt && text && text !== line.text) {
    sendEdit({ type: "reword", line: line.id, text }, `Reworded line ${l + 1}`);
  }
  if (voiced) voiceStatus();
  else status(READY);
  keys();
}

