// ---- Capture and build ----------------------------------------------------

/** A progress event as the tally bar says it, and how far along it is. */
function progressSays(e) {
  const done = e.done ?? e.done_ms ?? 0, of = e.of ?? e.of_ms ?? 0;
  const fraction = of ? Math.min(1, done / of) : 0;
  const count = `${done} of ${of}`;
  if (e.stage === "voice") return [`Voicing lines · ${count}`, fraction];
  if (e.stage === "capture") return [`Capturing ${(e.shot ?? "").split("#")[0]} · ${count}`, fraction];
  if (e.stage === "render") return [`Rendering · ${Math.round(fraction * 100)}%`, fraction];
  return [`${e.stage} · ${count}`, fraction];
}

/** Captures the shots not yet captured, or changed since, and for a build
 *  renders the video: one job at a time, never during a take. */
async function make(job) {
  if (making || listening || paused || counting || reading) return;
  making = job;
  $("working").hidden = false;
  $("working").firstElementChild.style.width = "0";
  status(job === "build" ? "Building the video…" : "Capturing the shots…");
  keys();
  try {
    const r = await fetch(`${API}/make?job=${job}`, { method: "POST" });
    const last = await events(r, (e) => {
      const [says, fraction] = progressSays(e);
      status(says);
      $("working").firstElementChild.style.width = `${fraction * 100}%`;
    });
    if (last.event === "failed") throw new Error(last.errors.join("\n"));
    const said = last.video ? `Built ${last.video.split("/").pop()}` : "Every shot is captured";
    status(last.video ? `Built ${last.video}` : said);
    toast(said);
  } catch (e) {
    status(`Could not ${job}: ${String(e.message).split("\n")[0]}`, true);
    toast(`Could not ${job}`);
  } finally {
    making = null;
    $("working").hidden = true;
    keys();
  }
  // New clips for the monitor, and the shots' lengths as captured.
  await loadScript();
}

/** Reads a long job's answer: each progress event to `progress` as it
 *  comes, and the last event, which says how the job went. */
async function events(r, progress) {
  if (!r.ok) throw new Error((await r.text()).trim() || r.statusText);
  const reader = r.body.getReader(), text = new TextDecoder();
  let buffer = "", last = null;
  for (;;) {
    const { value, done } = await reader.read();
    if (value) buffer += text.decode(value, { stream: true });
    let nl;
    while ((nl = buffer.indexOf("\n")) >= 0) {
      const line = buffer.slice(0, nl).trim();
      buffer = buffer.slice(nl + 1);
      if (!line) continue;
      const e = JSON.parse(line);
      if (e.event === "progress") progress(e);
      else last = e;
    }
    if (done) break;
  }
  if (!last) throw new Error("the server stopped without saying how it went");
  return last;
}

// ---- The welcome: a script to open, and who narrates ---------------------

let homeShown = false;

/** Tells the app around the page, if there is one, what the page did: a
 *  script opened, the welcome shown, setup closed. */
function tellShell(message) {
  try { window.webkit?.messageHandlers?.teleprompt?.postMessage(JSON.stringify(message)); } catch {}
}

/** The project's scripts, to open one: at start when none is open, or from
 *  the help to open another. */
async function showHome() {
  if (listening || paused || counting || making || reading || editing) {
    return status("Finish what is under way first");
  }
  let home;
  try {
    const r = await fetch(`${API}/home`);
    if (!r.ok) return status("This prompter serves one script; run teleprompt serve to open others", true);
    home = await r.json();
  } catch {
    return status("The server is not answering", true);
  }
  $("help").close();
  homeShown = true;
  document.body.classList.add("home");
  $("home").hidden = false;
  document.title = "Teleprompt";
  $("home-title").textContent = home.project ? `Teleprompt · ${home.project}` : "Teleprompt";
  $("home-back").hidden = !home.opened;

  // You read when this teleprompt can hear you; a voice reads otherwise.
  const you = document.querySelector('#narrator input[value="you"]');
  let chosen = PARAMS.get("narrator");
  try { chosen ??= localStorage.getItem("narrator"); } catch {}
  you.disabled = !home.can_hear;
  if (!home.can_hear || (!home.hears && chosen !== "you")) chosen = "voice";
  document.querySelector(`#narrator input[value="${chosen === "voice" ? "voice" : "you"}"]`).checked = true;
  narratorNote(home);

  const list = $("scripts");
  list.replaceChildren(...home.scripts.map((s) => {
    const item = document.createElement("li");
    const open = document.createElement("button");
    const title = document.createElement("span");
    title.textContent = s.name;
    open.append(title);
    if (s.name === home.opened) {
      const now = document.createElement("span");
      now.className = "open-now";
      now.textContent = "Open";
      open.append(now);
    }
    open.title = s.path;
    open.addEventListener("click", () => openScript(s.path, narrator() === "voice"));
    item.append(open);
    return item;
  }));
  $("no-scripts").hidden = home.scripts.length > 0;
  status(home.scripts.length ? "Choose a script" : "No scripts here");
  list.querySelector("button")?.focus();
}

function narrator() {
  return document.querySelector("#narrator input:checked")?.value ?? "you";
}

/** Why you cannot read, or what reading needs first. */
function narratorNote(home) {
  const note = $("narrator-note");
  note.hidden = false;
  if (!home.can_hear) note.textContent = "This teleprompt was built without speech models, so a voice reads.";
  else if (!home.hears && narrator() === "you") note.textContent = "Following your voice needs the speech model: opening a script offers to install it.";
  else note.hidden = true;
}

function hideHome() {
  homeShown = false;
  document.body.classList.remove("home");
  $("home").hidden = true;
  if (script.name) document.title = script.name;
  if (voiced) voiceStatus();
  else status(READY);
}

/** Opens `path` in place of what is open, then shows it; what it needs
 *  first, it offers to set up. */
async function openScript(path, voice) {
  $("home-errors").textContent = "";
  status(`Opening ${path.split("/").pop()}…`);
  let r, answer;
  try {
    r = await fetch(`${API}/open?script=${encodeURIComponent(path)}${voice ? "&voice=1" : ""}`, { method: "POST" });
    answer = await r.json().catch(() => ({ errors: [r.statusText] }));
  } catch {
    return status("The server is not answering", true);
  }
  if (r.ok) {
    try { localStorage.setItem("narrator", voice ? "voice" : "you"); } catch {}
    tellShell({ event: "opened", path, voice });
    // The page as it starts with a script open, on what this one was asked.
    const kept = new URLSearchParams(PARAMS);
    for (const k of ["open", "setup"]) kept.delete(k);
    kept.set("narrator", voice ? "voice" : "you");
    return location.replace(`${location.pathname}?${kept}`);
  }
  if (answer.needs) {
    if (!homeShown && !IN_APP) await showHome();
    return openSetup([answer.needs], answer.errors?.[0], () => openScript(path, voice));
  }
  if (IN_APP) {
    status(`Could not open ${path.split("/").pop()}`, true);
    return tellShell({ event: "open-failed", path, errors: answer.errors ?? [r.statusText] });
  }
  if (!homeShown) await showHome();
  $("home-errors").textContent = (answer.errors ?? [r.statusText]).join("\n");
  status(`Could not open ${path.split("/").pop()}`, true);
}

// ---- Setting teleprompt up ------------------------------------------------

let uses = [];                // what the server can set up, as it last said
let installing = false;
let afterSetup = null;        // what to do once what was wanted is installed
let installed = [];           // what was installed since the dialog opened

/** What teleprompt can be set up to do, with `wanted` ticked; `then` runs
 *  once it is installed. */
async function openSetup(wanted = [], why = null, then = null) {
  afterSetup = then;
  if (!$("setup").open) installed = [];
  $("setup-why").hidden = !why;
  $("setup-why").textContent = why ?? "";
  if (!$("setup").open) $("setup").showModal();
  await listUses(wanted);
}

async function listUses(wanted = []) {
  setupSays("Looking at what is here…");
  try {
    const r = await fetch(`${API}/setup`);
    if (!r.ok) throw new Error((await r.text()).trim() || r.statusText);
    uses = (await r.json()).uses;
  } catch (e) {
    return setupSays(`Could not tell what is here: ${e.message}`, true);
  }
  $("uses").replaceChildren(...uses.map((u) => useRow(u, wanted.includes(u.name))));
  setupSays("");
  chosenChanged();
}

function useRow(u, wanted) {
  const item = document.createElement("li");
  const label = document.createElement("label");
  const box = document.createElement("input");
  box.type = "checkbox";
  box.value = u.name;
  box.disabled = u.installed || !u.available || installing;
  box.checked = wanted && !box.disabled;
  box.addEventListener("change", chosenChanged);
  const title = document.createElement("span");
  title.textContent = u.label;
  const state = document.createElement("span");
  state.className = `use-state${u.installed ? " ok" : ""}`;
  state.textContent = useState(u);
  label.append(box, title, state);
  const more = document.createElement("details");
  const summary = document.createElement("summary");
  summary.textContent = "What it uses";
  more.append(summary, ...u.tools.map((t) => {
    const p = document.createElement("p");
    const head = document.createElement("strong");
    head.textContent = t.name;
    p.append(head, ` ${t.what}. ${t.license}. `);
    if (t.installed) p.append("Installed.");
    else if (t.command) {
      const code = document.createElement("code");
      code.textContent = t.command;
      p.append(code);
    } else p.append(t.guide ?? "Nothing here can install it.");
    return p;
  }));
  item.append(label, more);
  return item;
}

/** What a use needs here, as its row says it. */
function useState(u) {
  if (!u.available) return "Needs teleprompt built with speech models";
  if (u.installed) return "Installed";
  const missing = u.tools.filter((t) => t.installed === false);
  const programs = missing.filter((t) => t.download_mb == null).map((t) => t.name);
  const models = missing.length - programs.length;
  if (models) programs.push(models === 1 ? "a model" : `${models} models`);
  let says = `Needs ${programs.join(", ")}`;
  if (u.download_mb) says += ` · ${u.download_mb} MB`;
  if (missing.some((t) => t.password)) says += " · asks for your password";
  return says;
}

function chosen() {
  return [...document.querySelectorAll("#uses input:checked")].map((i) => i.value);
}

/** The Install button says what the uses ticked download, each model once. */
function chosenChanged() {
  const names = chosen();
  const seen = new Map();
  for (const t of uses.filter((u) => names.includes(u.name)).flatMap((u) => u.tools)) {
    if (t.installed === false && t.download_mb) seen.set(t.name, t.download_mb);
  }
  const mb = [...seen.values()].reduce((a, b) => a + b, 0);
  $("setup-install").disabled = installing || !names.length;
  $("setup-install").textContent = mb ? `Install · ${mb} MB` : "Install";
}

function setupSays(text, bad = false) {
  $("setup-says").textContent = text;
  $("setup-says").classList.toggle("bad", bad);
}

/** A tool as a sentence names it. */
function spoken(tool) {
  return { "speech-model": "the speech model", "punctuation-model": "the punctuation model",
    "speaker-model": "the speaker models" }[tool] ?? tool;
}

async function install() {
  const names = chosen();
  if (!names.length || installing) return;
  installing = true;
  for (const box of document.querySelectorAll("#uses input")) box.disabled = true;
  $("setup-close").disabled = true;
  chosenChanged();
  $("setup-bar").hidden = false;
  const bar = $("setup-bar").firstElementChild;
  bar.style.width = "0";
  setupSays("Installing…");
  let ok = false;
  try {
    const r = await fetch(`${API}/setup?uses=${names.map(encodeURIComponent).join(",")}`, { method: "POST" });
    const last = await events(r, (e) => {
      if (e.stage !== "install") return;
      if (e.state === "start") setupSays(`Installing ${spoken(e.tool)}…`);
      if (e.state === "downloading") {
        setupSays(`Downloading ${spoken(e.tool)} · ${Math.min(e.mb, e.of)} of ${e.of} MB`);
        bar.style.width = `${Math.min(1, e.mb / Math.max(1, e.of)) * 100}%`;
      }
      if (e.state === "done") setupSays(`Installed ${spoken(e.tool)}.`);
    });
    if (last.event === "failed") throw new Error(last.errors.join("\n"));
    installed.push(...names);
    ok = true;
  } catch (e) {
    setupSays(`Could not install: ${e.message}`, true);
  } finally {
    installing = false;
    $("setup-close").disabled = false;
    $("setup-bar").hidden = true;
  }
  if (!ok) return listUses(names).then(() => setupSays($("setup-says").textContent || "Could not install", true));
  await listUses();
  setupSays("Installed.");
  if (afterSetup) {
    const then = afterSetup;
    afterSetup = null;
    $("setup").close();
    then();
  }
}

$("setup-open").addEventListener("click", () => openSetup());
$("setup-install").addEventListener("click", install);
$("setup-close").addEventListener("click", () => $("setup").close());
$("setup").addEventListener("cancel", (e) => { if (installing) e.preventDefault(); });
$("setup").addEventListener("close", () => {
  afterSetup = null;
  tellShell({ event: "setup", installed });
});
$("home-back").addEventListener("click", hideHome);
// In an app, its own welcome has the scripts.
$("help-scripts").addEventListener("click", () => {
  if (!IN_APP) return showHome();
  $("help").close();
  tellShell({ event: "scripts" });
});
$("help-setup").addEventListener("click", () => { $("help").close(); openSetup(); });
for (const radio of document.querySelectorAll("#narrator input")) {
  radio.addEventListener("change", async () => {
    try { narratorNote(await (await fetch(`${API}/home`)).json()); } catch {}
  });
}

