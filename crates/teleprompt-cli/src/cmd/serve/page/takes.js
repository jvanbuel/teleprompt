// ---- The script, its shots, and a take ------------------------------------

async function loadScript(given = null) {
  if (editing) endReword(false);
  script = given ?? await (await fetch(`${API}/script`)).json();
  voiced = !!script.voice && !script.voice.listens;
  if (script.name) { $("title").textContent = script.name; document.title = script.name; }
  const root = $("script");
  root.replaceChildren();
  $("rundown").replaceChildren();
  shots = new Map();
  words = script.lines.map(({ text }, l) => {
    const line = document.createElement("div");
    line.className = "line";
    line.tabIndex = 0;
    line.setAttribute("role", "button");
    const fromHere = () => {
      if (editing || editMode) return;
      if (voiced) return lineClicked(l);
      if (listening || paused || counting) return status("Keep or discard this take first");
      take(l);
    };
    line.addEventListener("click", fromHere);
    line.addEventListener("keydown", (e) => {
      if (editing || (voiced && e.key === " ")) return;
      if ((e.key === "Enter" || e.key === " ") && !e.ctrlKey && !e.metaKey && !e.shiftKey) {
        e.preventDefault();
        e.stopPropagation();
        fromHere();
      }
    });
    const gutter = document.createElement("span");
    gutter.className = "gutter";
    const who = document.createElement("span");
    who.className = "who";
    who.setAttribute("aria-hidden", "true");
    gutter.append(who, String(l + 1));
    const body = document.createElement("span");
    const spans = text.split(/\s+/).filter(Boolean).map((w, k) => {
      const s = document.createElement("span");
      s.className = "w";
      s.textContent = w;
      s.dataset.l = l;
      s.dataset.w = k;
      body.append(s, " ");
      return s;
    });
    line.setAttribute("aria-label", `Line ${l + 1}: ${text}`);
    line.append(gutter, body);
    root.append(line);
    return spans;
  });
  lines = [...root.children];
  for (const shot of script.shots) markShot(shot);
  markRecorded(script.lines);
  document.body.classList.toggle("no-monitor", script.shots.length === 0 || popped);
  // A project's script can be captured and built, and its shots dragged.
  for (const job of ["capture", "build"]) $(job).hidden = !script.timeline;
  $("edit").hidden = !hasTimeline();
  for (const row of document.querySelectorAll("#help .has-timeline")) row.hidden = !hasTimeline();
  if (editMode && !hasTimeline()) setEdit(false);
  showBroken(script.error);
  const marked = new Set(moved.map((m) => m.id));
  lines.forEach((el, l) => el.classList.toggle("moved", marked.has(script.lines[l]?.id)));
  show(at, true);
  slate();
  drawRibbons();
}

function markRecorded(scriptLines) {
  script.lines = scriptLines;
  lines.forEach((el, l) => {
    const audio = voiced && scriptLines[l]?.audio;
    const speaker = scriptLines[l]?.speaker;
    const recorded = !!scriptLines[l]?.recorded;
    el.classList.toggle("recorded", recorded);
    el.classList.toggle("heard", !!scriptLines[l]?.said);
    // A speaker's initial, or the narrator's waveform; a take has its tick.
    el.classList.toggle("voiced", !speaker && !!audio && audio.source === "voice" && audio.ready);
    el.classList.toggle("unvoiced", !speaker && !!audio && audio.source === "voice" && !audio.ready);
    el.classList.toggle("unmade", !!audio && !audio.ready);
    const who = el.querySelector(".who");
    who.textContent = speaker && !recorded ? speaker[0].toUpperCase() : "";
    who.style.color = speaker ? speakerColour(speaker) : "";
    el.setAttribute("aria-label", `Line ${l + 1}${speaker ? `, said by ${speaker}` : ""}: ${scriptLines[l]?.text ?? ""}`);
  });
}

/** Speaker `name`'s colour, chosen by the name's letters so it is the
 *  same every time (apps/DESIGN.md). */
const SPEAKER_COLOURS = ["#c58af9", "#4dd0c8", "#f28bd0", "#d7b98e"];
function speakerColour(name) {
  let sum = 0;
  for (const b of new TextEncoder().encode(name)) sum += b;
  return SPEAKER_COLOURS[sum % SPEAKER_COLOURS.length];
}

const key = ({ id, said }) => `${id}\n${said}`;

/** The first line whose take said other words, not yet turned down. */
function heardOtherwise() {
  const l = script.lines.findIndex((line) => line.said && !declined.has(key(line)));
  return l < 0 ? null : l;
}

/** Tells of the lines newly heard saying other words, once: in a toast,
 *  or, while a toast offers Undo, in the status line. */
function offerSaid(quiet = false) {
  const fresh = script.lines.map((line, l) => [line, l]).filter(([line]) => line.said && !offered.has(key(line)));
  if (fresh.length === 0) return;
  for (const [line] of fresh) offered.add(key(line));
  const said = fresh.length === 1
    ? `Line ${fresh[0][1] + 1} was said in other words. Press W to review`
    : `${fresh.length} lines were said in other words. Press W to review`;
  if (quiet) status(`${$("status").textContent.replace(/\.?$/, ".")} ${said}`);
  else toast(said);
}

/** Shows the next line said in other words, to keep what was said or not. */
function reviewSaid() {
  if (listening || paused || counting || $("review").open) return;
  const l = heardOtherwise();
  if (l === null) return toast("Every line reads as it was said");
  const line = script.lines[l];
  $("review-title").textContent = `Keep what you said on line ${l + 1}?`;
  $("review-diff").replaceChildren(...line.said_diff.flatMap(({ kind, words }) => {
    const el = document.createElement(kind === "gone" ? "del" : kind === "new" ? "ins" : "span");
    el.textContent = words;
    return [el, " "];
  }));
  $("review").dataset.line = l;
  $("review").showModal();
}

$("review-said").addEventListener("click", async () => {
  const line = script.lines[$("review").dataset.line];
  $("review").close();
  try {
    await openSession();
    ws.send(JSON.stringify({ type: "keep_said", line: line.id }));
  } catch (e) {
    status(e.message, true);
  }
});
/** Keeping the script, by its button or Escape: not asked again this visit. */
function keepScript() {
  declined.add(key(script.lines[$("review").dataset.line]));
  $("review").close();
  keys();
}
$("review-script").addEventListener("click", keepScript);
$("review").addEventListener("cancel", (e) => { e.preventDefault(); keepScript(); });

/** A diamond in the text where a shot starts, and its row in the rundown. */
function markShot({ shot, at: cue, clip }) {
  const marker = document.createElement("span");
  marker.className = "shot" + (clip ? "" : " missing");
  marker.textContent = "◆";
  const said = words[cue.line]?.[cue.word];
  if (said) said.before(marker);
  else lines[cue.line]?.lastElementChild.append(marker);
  const row = document.createElement("li");
  row.innerHTML = `<span class="dot"></span><span></span><span class="where"></span>`;
  row.children[1].textContent = name(shot);
  $("rundown").append(row);
  shots.set(shot, { clip, marker, row, at: cue });
  rundown();
}

function rundown() {
  for (const [id, { clip, marker, row, at: cue }] of shots) {
    const state = playing === id ? "on-screen" : !clip ? "not-captured" : started.has(id) ? "aired" : "";
    row.className = state;
    marker.classList.toggle("aired", started.has(id) && playing !== id);
    row.querySelector(".where").textContent =
      state === "on-screen" ? "on screen" : state === "not-captured" ? "not captured"
      : state === "aired" ? "played"
      : cue.word === 0 ? `line ${cue.line + 1}` : `line ${cue.line + 1}, word ${cue.word + 1}`;
  }
}

/** Plays `ids` in turn, cutting whatever is playing: the reader has moved
 *  on, and the screen follows the reader. */
function play(ids) {
  if (!ids || ids.length === 0) return;
  for (const id of ids) started.add(id);
  queue = [...ids];
  next();
}

function next() {
  const video = $("clip");
  playing = queue.shift() ?? null;
  rundown();
  if (playing === null) return slate();
  const { clip } = shots.get(playing);
  $("shot-title").textContent = name(playing);
  if (clip) {
    $("slate").textContent = "";
    video.hidden = false;
    video.src = clip;
    video.play().catch(() => {});
  } else {
    video.hidden = true;
    $("slate").textContent = "This shot was never captured. Run teleprompt capture to record it.";
    $("slate").classList.add("missing");
    const shot = playing;
    setTimeout(() => { if (playing === shot) next(); }, 2000);
  }
  broadcast();
}

/** The monitor with nothing playing: what comes next. */
function slate() {
  const video = $("clip");
  video.pause();
  video.hidden = true;
  $("slate").classList.remove("missing");
  $("shot-title").textContent = "";
  $("shot-time").textContent = "";
  $("progress").firstElementChild.style.width = "0";
  const upcoming = [...shots].find(([id, s]) => s.clip && !started.has(id)
    && (s.at.line > at.line || (s.at.line === at.line && s.at.word >= at.word)));
  $("slate").textContent = upcoming ? `Next: ${name(upcoming[0])}, at line ${upcoming[1].at.line + 1}`
    : started.size ? "Every shot has played." : "Each shot plays here as your reading reaches it.";
  broadcast();
}

/** Dims what is said and the lines to come, marks the next word, and
 *  glides its line to the reading line; `quiet` for the script drawn
 *  again, which says nothing of where the reader is. */
function show(next, quiet = false) {
  at = next;
  words.forEach((spans, l) => spans.forEach((s, w) => {
    s.classList.toggle("said", l < at.line || (l === at.line && w < at.word));
    s.classList.toggle("next", l === at.line && w === at.word);
  }));
  lines.forEach((el, l) => {
    el.classList.toggle("current", l === at.line);
    if (l === at.line) el.setAttribute("aria-current", "location");
    else el.removeAttribute("aria-current");
  });
  if (at.line >= words.length && words.length && !quiet) status("End of script");
  glide();
}

let glideGoal = 0, gliding = false;
function glide() {
  const scroller = $("scroller");
  const target = words[at.line]?.[at.word] ?? words[at.line]?.at(-1) ?? lines.at(-1)?.lastElementChild;
  if (!target) return;
  const rect = target.getBoundingClientRect(), box = scroller.getBoundingClientRect();
  const centre = rect.top - box.top + rect.height / 2;
  glideGoal = Math.max(0, scroller.scrollTop + centre - box.height * READING);
  if (gliding) return;
  gliding = true;
  let last = performance.now();
  const reduce = matchMedia("(prefers-reduced-motion: reduce)").matches;
  const step = (now) => {
    const dt = (now - last) / 1000;
    last = now;
    const d = glideGoal - scroller.scrollTop;
    if (Math.abs(d) < 0.5 || reduce) {
      scroller.scrollTop = glideGoal;
      gliding = false;
      return;
    }
    scroller.scrollTop += d * (1 - Math.exp(-dt / GLIDE));
    requestAnimationFrame(step);
  };
  requestAnimationFrame(step);
}

/** Sends the audio not yet sent, as one binary message. */
function send() {
  if (!ws || ws.readyState !== WebSocket.OPEN || pending.length === 0) return;
  ws.send(Float32Array.from(pending).buffer);
  pending = [];
}

/** Opens the session socket; its messages drive the page from then on. */
function openSession() {
  if (ws && ws.readyState === WebSocket.OPEN) return Promise.resolve();
  return new Promise((resolve, reject) => {
    const socket = new WebSocket(`ws://${location.host}${API}/session`);
    socket.binaryType = "arraybuffer";
    socket.onopen = () => {
      ws = socket;
      resolve();
    };
    socket.onerror = () => reject(new Error(
      "Could not open the session: another prompter may have it open, or the server stopped. Close it and reload."));
    socket.onclose = () => {
      if (ws === socket) status("Lost the server. Reload once it is running again.", true);
      ws = null;
      listening = false;
      keys();
    };
    socket.onmessage = (e) => onMessage(JSON.parse(e.data));
  });
}

let toastTimer = null;
/** Says `text` on the glass; with `action`, a button to act on it for longer. */
function toast(text, action = null) {
  const el = $("toast");
  el.replaceChildren(text);
  if (action) {
    const button = document.createElement("button");
    button.textContent = action.label;
    button.addEventListener("click", () => { el.classList.remove("on"); action.run(); });
    el.append(button);
  }
  el.classList.add("on");
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => el.classList.remove("on"), action ? 8000 : 4000);
}

let undoable = false;         // the last take kept lines, which undo puts back

function lineNames(ids) {
  const n = ids.map((id) => script.lines.findIndex((line) => line.id === id) + 1).filter((n) => n > 0);
  return n.length === 1 ? `line ${n[0]}` : `lines ${n.join(", ")}`;
}

/** Puts back what the last take kept replaced; or, when an edit was the
 *  last thing done, what the edit changed. */
function undo() {
  if (!undoable || editMode) return undoEdit();
  if (!ws || listening || paused || counting) return;
  undoable = false;
  ws.send(JSON.stringify({ type: "undo" }));
}

/** Ends the take, or the count before it, keeping nothing. */
function discard() {
  retakes = null;
  if (counting) {
    counting = false;
    status(READY);
    return keys();
  }
  if (!ws || !(listening || paused)) return;
  listening = false;
  paused = false;
  if (takeSince !== null) { takeTime += performance.now() - takeSince; takeSince = null; }
  pending = [];
  ws.send(JSON.stringify({ type: "discard" }));
  level(0);
  keys();
}

async function onMessage(message) {
  if (message.type === "reached") {
    show(message);
    play(message.play);
    if (!playing) slate();
    // Re-recording a reworded line: kept once the reader has read it.
    if (retakes && listening && queuedRead()) keep();
  } else if (message.type === "stopped") {
    const { saved } = message;
    undoable = saved.length > 0;
    if (saved.length === 0) {
      status("Nothing kept: a line is kept once it is read to its end");
      toast("Nothing kept: read a line to its end");
    } else {
      status(`Kept ${lineNames(saved)}. ${MAC ? "⌘Z" : "Ctrl+Z"} puts back what it replaced`);
      toast(`Kept ${lineNames(saved)}`, { label: "Undo", run: undo });
    }
    markRecorded((await (await fetch(`${API}/script`)).json()).lines);
    offerSaid(undoable);
    if (retakes) retakeNext();
    keys();
  } else if (message.type === "discarded") {
    status("Take discarded: nothing kept");
    toast("Take discarded");
  } else if (message.type === "undone") {
    const was = message.lines.length === 1 ? "it was" : "they were";
    const put = message.lines.length ? `Put back ${lineNames(message.lines)} as ${was}` : "Nothing to put back";
    status(put);
    toast(put);
    markRecorded((await (await fetch(`${API}/script`)).json()).lines);
    keys();
  } else if (message.type === "kept_said") {
    const l = script.lines.findIndex((line) => line.id === message.line);
    await loadScript();
    status(`Line ${l + 1} now reads as you said it`);
    toast(`Line ${l + 1} now reads as you said it`);
    keys();
  } else if (message.type === "edited") {
    const said = lastEdit ?? "Edited";
    lastEdit = null;
    undoEdits += 1;
    await loadScript();
    if (!editMode) lines[at.line]?.focus({ preventScroll: true });
    toast(said, { label: "Undo", run: undoEdit });
    if (editMode) status(said);
    voiceStatus();
    voiceUnmade();
    keys();
  } else if (message.type === "edit_undone") {
    undoEdits = Math.max(0, undoEdits - 1);
    await loadScript();
    toast("Undone");
    status("Undone");
    voiceStatus();
    voiceUnmade();
    keys();
  } else if (message.type === "error") {
    lastEdit = null;
    status(message.message, true);
  }
}

const WORKLET = `
class Tap extends AudioWorkletProcessor {
  process(inputs) {
    const ch = inputs[0][0];
    if (ch) this.port.postMessage(ch.slice());
    return true;
  }
}
registerProcessor("tap", Tap);`;

let mic = null;               // opened once, on the first take

/** Opens the microphone. What it hears is the take, so the browser is
 *  asked not to clean it up. */
async function openMic() {
  if (mic) return;
  const stream = await navigator.mediaDevices.getUserMedia({
    audio: { channelCount: 1, echoCancellation: false, noiseSuppression: false, autoGainControl: false },
  });
  const ctx = new AudioContext();
  rate = ctx.sampleRate;
  const url = URL.createObjectURL(new Blob([WORKLET], { type: "text/javascript" }));
  await ctx.audioWorklet.addModule(url);
  const tap = new AudioWorkletNode(ctx, "tap");
  tap.port.onmessage = (e) => {
    if (!listening) return;
    let power = 0;
    for (const v of e.data) { pending.push(v); power += v * v; }
    level(Math.sqrt(power / e.data.length));
  };
  ctx.createMediaStreamSource(stream).connect(tap);
  setInterval(send, SEND_EVERY_MS);
  mic = stream;
}

function level(rms) {
  $("meter").firstElementChild.style.transform = `scaleX(${Math.min(1, Math.sqrt(rms))})`;
}

/** A new take from line `from`, after a count of three; the server drops
 *  any take not kept. */
async function take(from = 0) {
  if (counting) return;
  if (editMode) setEdit(false);
  halt();
  try {
    await openMic();
    await openSession();
  } catch (e) {
    status(e.name === "NotAllowedError" ? "The browser may not use the microphone: allow it for this page." : e.message, true);
    return;
  }
  listening = false;
  paused = false;
  undoable = false;
  pending = [];
  queue = [];
  playing = null;
  started = new Set();
  show({ line: from, word: 0 });
  rundown();
  slate();
  counting = true;
  const what = retakes ? `Re-recording ${retakes.at + 1} of ${retakes.lines.length}: line ${from + 1}`
    : `Recording from line ${from + 1}`;
  status(`${what} in…`);
  keys();
  for (const n of COUNT ? [3, 2, 1] : []) {
    $("countdown").textContent = n;
    $("countdown").classList.add("on");
    await new Promise((r) => setTimeout(r, COUNT_MS));
    if (!counting) break;
  }
  $("countdown").classList.remove("on");
  if (!counting) return;
  counting = false;
  ws.send(JSON.stringify({ type: "start", from, rate }));
  listening = true;
  takeTime = 0;
  takeSince = performance.now();
  status(what);
  keys();
}

/** Ends the take, keeping the lines read in full. */
function keep() {
  if (counting) {
    retakes = null;
    counting = false;
    status(READY);
    return keys();
  }
  if (!ws || !(listening || paused)) return;
  listening = false;
  paused = false;
  if (takeSince !== null) { takeTime += performance.now() - takeSince; takeSince = null; }
  send();
  ws.send(JSON.stringify({ type: "stop" }));
  status("Keeping the take…");
  level(0);
  keys();
}

function togglePause() {
  if (!(listening || paused)) return;
  paused = !paused;
  listening = !paused;
  if (paused) { takeTime += performance.now() - takeSince; takeSince = null; }
  else takeSince = performance.now();
  status(paused ? "Paused" : "Recording");
  keys();
}

setInterval(() => {
  const ms = reading ? performance.now() - reading.since
    : takeTime + (takeSince === null ? 0 : performance.now() - takeSince);
  const tenths = Math.floor(ms / 100);
  $("timecode").textContent =
    `${String(Math.floor(tenths / 600)).padStart(2, "0")}:${String(Math.floor(tenths / 10) % 60).padStart(2, "0")}.${tenths % 10}`;
}, 100);

document.addEventListener("keydown", (e) => {
  // Typing in the panel or a line's editor is typing; they take their own keys.
  if (e.target.closest?.("input, textarea")) return;
  if (homeShown || $("setup").open || !script.lines.length) {
    if (homeShown && e.key === "Escape" && !$("home-back").hidden) hideHome();
    return;
  }
  if ($("panel").open && e.key === "Escape") { e.preventDefault(); return $("panel").close(); }
  if ($("panel").open && e.target.closest?.("#panel")) return;
  const command = MAC ? e.metaKey : e.ctrlKey;
  if (e.key === " " && e.shiftKey && command) {
    e.preventDefault();
    return recordOrKeep();
  }
  if ($("review").open || $("help").open) return;
  if (command && !e.shiftKey && (e.key === "Enter" || e.key === "z")) {
    e.preventDefault();
    return e.key === "Enter" ? keep() : undo();
  }
  if (e.metaKey || e.ctrlKey || e.altKey) return;
  if (voiced && e.key === " ") { e.preventDefault(); return playOrStop(); }
  if (e.key === "F2") { e.preventDefault(); return reword(at.line < words.length ? at.line : 0); }
  const handled = {
    "Escape": reading ? () => stopReading() : discard,
    "p": togglePause,
    "w": reviewSaid,
    "m": mirror,
    "?": () => $("help").showModal(),
    "s": () => { document.body.classList.toggle("no-monitor"); drawRibbons(); },
    "e": () => setEdit(!editMode),
    "r": retake,
    "v": playOrStop,
    "j": playMoved,
    "ArrowLeft": () => skip(-5000),
    "ArrowRight": () => skip(5000),
    "+": () => resize(4), "=": () => resize(4), "-": () => resize(-4),
  }[e.key];
  if (!handled) return;
  e.preventDefault();
  handled();
});

function recordOrKeep() {
  if (voiced) playOrStop();
  else if (listening || paused || counting) keep();
  else take(at.line < words.length ? at.line : 0);
}

