// ---- Edit mode: the shots on the glass --------------------------------

const hasTimeline = () => (script.timeline?.shots?.length ?? 0) > 0;
const leads = (shot) => shot.shot.endsWith("#0");
const seconds = (ms) => `${(ms / 1000).toFixed(1)}s`;

/** A scene's colour: steady for its name, muted to sit under the ink. */
const HUES = ["#7aa3ff", "#d49cff", "#5ed4e0", "#f28c82", "#6ee8a6", "#d9dbe0"];
function hue(scene) {
  let n = 0;
  for (const b of new TextEncoder().encode(scene)) n = (Math.imul(n, 31) + b) >>> 0;
  return HUES[n % HUES.length];
}

/** Each word's start and end in a line `length` ms long, by its letters:
 *  where the compiler places a cue when the voice gave no word timings. */
function shares(text, length) {
  const weights = text.split(/\s+/).filter(Boolean).map((w) => [...w].length + 1);
  const total = Math.max(1, weights.reduce((a, b) => a + b, 0));
  let at = 0;
  return weights.map((w) => {
    const start = Math.floor((at * length) / total);
    at += w;
    return [start, Math.floor((at * length) / total)];
  });
}

/** Line `id`: where it is in the script, its words, and when it is said. */
function spanOf(id) {
  const l = script.lines.findIndex((line) => line.id === id);
  const said = script.timeline.lines.find((line) => line.id === id);
  return l < 0 || !said ? null : { l, text: script.lines[l].text, start: said.start_ms, end: said.end_ms };
}

/** Every shot that goes with a line: the words it plays over, none for
 *  one held after its line, and how long it runs on past the line. */
function ribbons() {
  return script.timeline.shots.flatMap((shot, i) => {
    const span = shot.line && spanOf(shot.line);
    if (!span) return [];
    const said = shares(span.text, span.end - span.start);
    let from = said.findIndex(([s]) => span.start + s >= shot.start_ms);
    if (from < 0) from = said.length;
    let last = -1;
    said.forEach(([s], k) => { if (span.start + s < shot.end_ms) last = k; });
    const to = last < 0 ? from : Math.max(last + 1, from);
    const past = Math.max(0, shot.end_ms - Math.max(span.end, shot.start_ms));
    return [{ shot: i, line: span.l, from, to, past }];
  });
}

/** When word `w` of line `l` is said. */
function moment(l, w) {
  const span = script.lines[l] && spanOf(script.lines[l].id);
  if (!span) return 0;
  return span.start + (shares(span.text, span.end - span.start)[w]?.[0] ?? 0);
}

function setEdit(on) {
  if (on && (listening || paused || counting)) return status("Keep or discard this take first");
  if (on && document.body.classList.contains("mirrored")) {
    return status("Mirrored text cannot be edited: M turns mirroring off");
  }
  if (on && !hasTimeline()) return;
  if (on && reading) stopReading();
  if (on && editing) endReword(false);
  editMode = on;
  $("panel").close();
  drawRibbons();
  if (!on) scrub(null);
  if (on) status("Drag a shot onto a word to start it there, or its grip to set how long it runs");
  else if (voiced) voiceStatus();
  else status(READY);
  keys();
}

/** The shots, drawn over the glass where they play. */
function drawRibbons() {
  $("ribbons")?.remove();
  if (!editMode || !hasTimeline() || VIEW) return;
  const root = $("script");
  const layer = document.createElement("div");
  layer.id = "ribbons";
  root.append(layer);
  const box = root.getBoundingClientRect();
  const rect = (el) => {
    const r = el.getBoundingClientRect();
    return { x: r.left - box.left, y: r.top - box.top, w: r.width, h: r.height };
  };
  const place = (el, x, y, w) => {
    el.style.left = `${x}px`;
    el.style.top = `${y}px`;
    if (w !== undefined) el.style.width = `${w}px`;
  };
  // Each line's pause fills from its last word rightwards.
  const after = new Map();
  for (const r of ribbons()) {
    const shot = script.timeline.shots[r.shot];
    const piece = document.createElement("div");
    piece.className = leads(shot) ? "piece leads" : "piece";
    piece.style.setProperty("--hue", hue(shot.scene));
    piece.dataset.shot = r.shot;
    piece.title = `${name(shot.shot)} · ${shot.scene} · ${seconds(shot.end_ms - shot.start_ms)}`;
    layer.append(piece);
    const bars = [];
    for (const word of words[r.line].slice(r.from, r.to)) {
      const w = rect(word), bar = bars.at(-1);
      if (bar && Math.abs(bar.y - w.y) < 2) bar.w = w.x + w.w - bar.x;
      else bars.push(w);
    }
    let end = null;
    for (const b of bars) {
      const bar = document.createElement("span");
      bar.className = "bar";
      place(bar, b.x, b.y + b.h + 3, b.w);
      piece.append(bar);
      end = { x: b.x + b.w, y: b.y + b.h + 3 + 2.5 };
    }
    if (r.from === r.to || r.past > 0) {
      const last = rect(words[r.line].at(-1));
      const pill = document.createElement("span");
      pill.className = "pill";
      pill.textContent = r.from === r.to ? `${shot.block} · ${seconds(r.past)}` : `+${seconds(r.past)}`;
      piece.append(pill);
      const w = pill.offsetWidth;
      let x = after.get(r.line) ?? last.x + last.w + 14, y = last.y + (last.h - 24) / 2;
      if (x + w > root.clientWidth - 16) {
        x = rect(lines[r.line].children[1]).x;
        y = last.y + last.h + 3;
      }
      after.set(r.line, x + w + 8);
      place(pill, x, y);
      end = { x: x + w, y: y + 12 };
    }
    if (end && shot.timed && leads(shot)) {
      const grip = document.createElement("span");
      grip.className = "grip";
      grip.title = "Drag along the line to set how long it runs";
      place(grip, end.x - 3, end.y - 8);
      piece.append(grip);
    }
  }
  layer.addEventListener("pointerdown", grab);
}

/** A press on a block's first shot: its ribbon moves it, its grip stretches it. */
function grab(e) {
  const part = e.target.closest(".bar, .pill, .grip");
  const piece = part?.closest(".piece.leads");
  if (!piece || e.button !== 0) return;
  e.preventDefault();
  e.stopPropagation();
  drag = { shot: Number(piece.dataset.shot), grip: part.classList.contains("grip"), moved: false,
           piece, from: [e.clientX, e.clientY] };
}

/** The line and word under (`x`, `y`): `word` null past a line's last
 *  word, or below it, for the pause after it. */
function ontoAt(x, y) {
  const el = document.elementFromPoint(x, y);
  const word = el?.closest?.("#script .w");
  if (word) return { line: Number(word.dataset.l), word: Number(word.dataset.w) };
  const line = el?.closest?.("#script .line");
  if (!line) return null;
  const l = lines.indexOf(line);
  const spans = words[l];
  if (!spans?.length) return null;
  const last = spans.at(-1).getBoundingClientRect();
  if (y > last.bottom || (y >= last.top && x > last.right)) return { line: l, word: null };
  // Between words: the nearest on the row.
  let best = null, dist = Infinity;
  spans.forEach((s, k) => {
    const r = s.getBoundingClientRect();
    if (y < r.top || y > r.bottom) return;
    const d = x < r.left ? r.left - x : x > r.right ? x - r.right : 0;
    if (d < dist) { dist = d; best = k; }
  });
  return best === null ? { line: l, word: null } : { line: l, word: best };
}

/** What dropping the dragged shot at `onto` asks of the script, or null. */
function editFor(d, onto) {
  if (!onto) return null;
  const shot = script.timeline.shots[d.shot];
  const id = script.lines[onto.line].id, home = shot.line === id;
  if (d.grip) return onto.word === null ? null : stretchedTo(shot, onto.line, onto.word);
  if (onto.word !== null) {
    return home ? { type: "cue", block: shot.block, word: onto.word }
      : { type: "move", block: shot.block, after: id, word: onto.word };
  }
  return home ? { type: "hold", block: shot.block } : { type: "move", block: shot.block, after: id, word: null };
}

/** A grip dropped on word `w` of its own line: as long as up to that
 *  word's end. Null for another line, or a shot that states no length. */
function stretchedTo(shot, l, w) {
  if (script.lines[l].id !== shot.line || !shot.timed) return null;
  const span = spanOf(shot.line);
  const until = span.start + shares(span.text, span.end - span.start)[w][1];
  const now = shot.end_ms - shot.start_ms, then = until - shot.start_ms;
  if (now <= 0 || then <= 0 || Math.abs(now - then) < 50) return null;
  return { type: "stretch", block: shot.block, by: Math.round((then / now) * 1000) / 1000 };
}

/** What a drop will do, in the author's words; `home` is the shot's line. */
function describe(edit, home) {
  const text = (id) => script.lines.find((l) => l.id === id)?.text.split(/\s+/).filter(Boolean) ?? [];
  const word = (id, n) => (text(id)[n] ?? "").replace(/^[^\p{L}\p{N}]+|[^\p{L}\p{N}]+$/gu, "");
  const first = (id) => `“${text(id).slice(0, 3).join(" ")}…”`;
  switch (edit.type) {
    case "cue": return edit.word === 0 ? "Start with the line" : `Start on “${word(home, edit.word)}”`;
    case "hold": return "After the line";
    case "move": return edit.word === null ? `Move after ${first(edit.after)}`
      : `Move to ${first(edit.after)} on “${word(edit.after, edit.word)}”`;
    default: return `Stretch ×${edit.by.toFixed(2)}`;
  }
}

/** Follows the pointer with where the shot would land, and what that does. */
function dragTo(x, y) {
  if (!drag.moved && Math.hypot(x - drag.from[0], y - drag.from[1]) < 4) return;
  drag.moved = true;
  document.body.classList.add("dragging");
  drag.piece.classList.add("dragged");
  const onto = ontoAt(x, y);
  drag.edit = editFor(drag, onto);
  $("drop-target")?.remove();
  if (onto && drag.edit) {
    const box = $("script").getBoundingClientRect();
    const mark = document.createElement("div");
    mark.id = "drop-target";
    const r = (onto.word === null ? words[onto.line].at(-1) : words[onto.line][onto.word]).getBoundingClientRect();
    const [left, top, w, h] = onto.word === null
      ? [r.right + 6, r.top + 6, 4, r.height - 12]
      : [r.left, r.bottom + 3, r.width, 5];
    Object.assign(mark.style, { left: `${left - box.left}px`, top: `${top - box.top}px`, width: `${w}px`, height: `${h}px` });
    $("script").append(mark);
  }
  let chip = $("drop-chip");
  if (!chip) {
    chip = document.createElement("div");
    chip.id = "drop-chip";
    document.body.append(chip);
  }
  chip.textContent = drag.edit ? describe(drag.edit, script.timeline.shots[drag.shot].line) : "Not here";
  Object.assign(chip.style, { left: `${x}px`, top: `${y}px` });
}

function endDrag() {
  $("drop-target")?.remove();
  $("drop-chip")?.remove();
  drag?.piece.classList.remove("dragged");
  document.body.classList.remove("dragging");
  drag = null;
}

function dropAt() {
  const { moved, edit, shot } = drag;
  endDrag();
  if (moved && edit) sendEdit(edit, describe(edit, script.timeline.shots[shot].line));
}

/** The monitor at `ms` into the video, still: the shot on screen then, as
 *  far into it as that is. `null` puts back what it showed. */
function scrub(ms) {
  if (ms === null) {
    if (!playing) slate();
    return;
  }
  const video = $("clip");
  const shot = script.timeline.shots.findLast((s) => s.start_ms <= ms && ms < s.end_ms);
  $("progress").firstElementChild.style.width = "0";
  if (!shot) {
    video.pause();
    video.hidden = true;
    $("slate").classList.remove("missing");
    $("slate").textContent = `Nothing on screen at ${seconds(ms)}`;
    $("shot-title").textContent = "";
    $("shot-time").textContent = "";
    return broadcast();
  }
  const clip = shots.get(shot.shot)?.clip ?? script.shots.find((s) => s.shot === shot.shot)?.clip;
  $("shot-title").textContent = name(shot.shot);
  $("shot-time").textContent = `${seconds(ms)} · ${seconds(ms - shot.start_ms)} into the shot`;
  if (!clip) {
    video.pause();
    video.hidden = true;
    $("slate").textContent = "This shot was never captured. Capture records it.";
    $("slate").classList.add("missing");
    return broadcast();
  }
  $("slate").textContent = "";
  $("slate").classList.remove("missing");
  video.hidden = false;
  video.pause();
  const into = (ms - shot.start_ms) / 1000;
  if (video.getAttribute("src") !== clip) {
    video.addEventListener("loadedmetadata", () => {
      video.currentTime = Math.min(into, video.duration || into);
      broadcast();
    }, { once: true });
    video.src = clip;
  } else {
    video.currentTime = Math.min(into, video.duration || into);
  }
  broadcast();
}

// ---- Retakes: the lines reworded since their takes, recorded again -----

const staleLines = () => (voiced ? [] : script.lines.flatMap((line, l) => (line.stale ? [l] : [])));

function retake() {
  if (voiced || listening || paused || counting || retakes) return;
  const due = staleLines();
  if (due.length === 0) return toast("No line has been reworded since its take");
  retakes = { lines: due, at: 0 };
  take(due[0]);
}

/** Whether the reader has read the line being re-recorded: moved on past
 *  it, or, on the script's last line, reached its end. */
function queuedRead() {
  const l = retakes.lines[retakes.at];
  return at.line > l || (l + 1 === words.length && at.line === l && at.word >= words[l].length);
}

/** A re-recorded line kept: on to the next, or done. */
function retakeNext() {
  retakes.at += 1;
  if (retakes.at >= retakes.lines.length) {
    const n = retakes.lines.length;
    retakes = null;
    return status(`Re-recorded ${n} line${n === 1 ? "" : "s"}`);
  }
  const l = retakes.lines[retakes.at];
  setTimeout(() => { if (retakes) take(l); }, 600);
}

