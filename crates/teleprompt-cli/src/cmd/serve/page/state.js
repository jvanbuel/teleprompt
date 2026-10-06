"use strict";
// The prompter page, in parts that run in order as one script: this one
// (settings and state), takes, voice, edit, shell and main. `serve/routes.rs`
// puts them together, and the style, into index.html.
const API = "/api/v1";
const SEND_EVERY_MS = 100;   // how much audio each message carries, at most
const READING = 0.36;        // the reading line, as a fraction of the glass
const GLIDE = 0.22;          // how quickly the text settles on it, in seconds
const COUNT_MS = 650;        // one beat of the count before a take
const $ = (id) => document.getElementById(id);

let script = { lines: [], shots: [] };
let words = [];              // per line, the word spans, split as the server splits
let lines = [];              // the line elements
let shots = new Map();       // shot id to { clip, marker, row, at }
let queue = [];              // shots to play after the one playing
let playing = null;
let started = new Set();     // shots started this take
let at = { line: 0, word: 0 };
let listening = false, paused = false, counting = false;
let pending = [];            // samples not yet sent, at the microphone's rate
let rate = 0;                // the microphone's rate, once it is open
let ws = null;               // the session socket, once open
let takeTime = 0, takeSince = null;
let declined = new Set();    // "id\nsaid" of rewordings turned down this visit
let offered = new Set();     // and of those a toast has told of

// Read by a voice (`teleprompt serve --voice`) rather than following one.
let voiced = false;
let reading = null;           // { origin, until, since, nodes, shot, raf } while the video plays
let editing = null;           // { line, editor, body } while a line's words are edited
let undoEdits = 0;            // edits the server keeps to undo
let lastEdit = null;          // what the edit sent will be said as, until the server answers it

const PARAMS = new URLSearchParams(location.search);
// "monitor": only the screen, in a window of its own, for a second display.
const VIEW = PARAMS.get("view");
// `?countdown=0`: a take starts at once, without a count of three.
const COUNT = PARAMS.get("countdown") !== "0";
const IN_APP = PARAMS.get("shell") === "1";
if (IN_APP) document.body.classList.add("in-app");
let editMode = false;         // Edit mode: the shots on the glass, to drag
let drag = null;              // { shot, grip, moved, piece } while a shot is dragged
let retakes = null;           // { lines, at } while reworded lines are recorded again
let making = null;            // "capture" or "build", while one runs

function status(text, bad = false) {
  $("status").textContent = text;
  $("status").classList.toggle("bad", bad);
}

/** Starts and stops recording: ⌘⇧Space on a Mac, Ctrl+Shift+Space elsewhere. */
const MAC = /Mac|iPhone|iPad/.test(navigator.platform);
// What to do first, in the terms of the device: a phone has no keys.
const READY = matchMedia("(pointer: coarse)").matches
  ? "Tap Record to read from here, or tap a line"
  : `Press ${MAC ? "⌘⇧Space" : "Ctrl+Shift+Space"} to record from here, or click a line`;

const RECORD_KEY = MAC ? "⌘ ⇧ Space" : "Ctrl ⇧ Space";

/** The keys that matter now, as keycaps. */
function keys() {
  const retake = staleLines().length ? [["R", "Retake"]] : [];
  const pairs = editing ? [["Enter", "Keep"], ["Esc", "Leave it"]]
    : reading ? [[voiced ? "Space" : "V", "Stop"], ["← →", "5 s"], ["Esc", "Stop"]]
    : editMode ? [["Drag", "Move a shot"], [MAC ? "⌘Z" : "Ctrl Z", "Undo"], ["E", "Done"]]
    : voiced ? [["Space", "Play"], ["Click", "Direct a line"], ["F2", "Reword"], ["M", "Mirror"]]
    : counting ? [["Esc", "Cancel"]]
    : paused ? [[RECORD_KEY, "Keep take"], ["Esc", "Discard"], ["P", "Resume"]]
    : listening ? [[RECORD_KEY, "Keep take"], ["Esc", "Discard"], ["P", "Pause"]]
    : heardOtherwise() !== null ? [[RECORD_KEY, "Record"], ["W", "Review"], ...retake]
    : [[RECORD_KEY, "Record"], ...retake, ["V", "Play video"]];
  $("keys").replaceChildren(...pairs.map(([k, action]) => {
    const pair = document.createElement("span");
    const kbd = document.createElement("kbd");
    kbd.textContent = k;
    pair.append(kbd, action);
    return pair;
  }));
  const taking = listening || paused || counting;
  for (const job of ["capture", "build"]) {
    $(job).disabled = !!making || taking || !!reading;
    $(job).textContent = making === job ? (job === "build" ? "Building…" : "Capturing…")
      : job === "build" ? "Build" : "Capture";
  }
  $("edit").disabled = taking || !!reading;
  $("video").hidden = voiced || !script.voice;
  $("video").disabled = taking;
  $("video").textContent = reading ? "Stop" : "Play video";
  $("edit").setAttribute("aria-pressed", editMode);
  document.body.classList.toggle("editing", editMode);
  $("record").textContent = voiced ? (reading ? "Stop" : "Play")
    : counting ? "Cancel" : taking ? "Keep take" : "Record";
  $("record").title = voiced ? "Read aloud from the line you are on, or stop (Space)"
    : "Record from the line you are on";
  document.body.classList.toggle("voiced", voiced);
  document.body.classList.toggle("reading", !!reading);
  $("record").classList.toggle("keep", listening || paused);
  for (const el of [$("tally"), $("timecode")]) el.classList.toggle("on-air", listening);
  $("tally").setAttribute("aria-label", listening ? "On air" : paused ? "Paused" : "Off air");
  document.body.classList.toggle("on-air", listening);
  document.body.classList.toggle("paused", paused);
}

function name(shot) { return shot.replace(/#0$/, ""); }

