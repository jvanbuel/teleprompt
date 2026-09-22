//! The `terminal` scene, served by VHS tapes.
//!
//! The block body is real VHS tape syntax and stays runnable by `vhs` itself
//! — including the mark, which is spelled as a comment for exactly that
//! reason (§7.5). teleprompt reuses the language; the runtime is its own PTY.
//!
//! A tape mostly states its own timing: every `Sleep` is written down and
//! every keystroke costs `Set TypingSpeed`. So `estimate` is exact for a shot
//! that contains only those, and `Estimated` for one containing a `Wait`,
//! whose real length is a fact about the command underneath rather than about
//! the tape. See [`VhsScene::estimate`].

// Rust guideline compliant 2026-07-18

use teleprompt_core::attrs::parse_duration_ms;
use teleprompt_core::{Diagnostic, Hash};

use teleprompt_scene::{
    validate_commands, BlockSource, CommandError, Measured, SceneCompiler, Shot, Validated,
};

/// The VHS adapter.
#[derive(Debug)]
pub struct VhsScene;

/// VHS's own default gap between keystrokes, overridable per tape with
/// `Set TypingSpeed` or per command with `Type@100ms`. Matching the tool's
/// default is what makes an unannotated tape estimate correctly.
const DEFAULT_TYPING_SPEED_MS: u64 = 50;

/// VHS's own default bound on `Wait`, overridable with `Set WaitTimeout` or
/// per command with `Wait@30s`. Matching the tool's default is what makes the
/// bound teleprompt schedules against the bound VHS would enforce.
const DEFAULT_WAIT_TIMEOUT_MS: u64 = 5_000;

/// Keys VHS presses. Modifier chords are built from these and from single
/// characters — see [`is_key`].
const KEYS: &[&str] = &[
    "Enter",
    "Tab",
    "Space",
    "Backspace",
    "Delete",
    "Escape",
    "Esc",
    "Up",
    "Down",
    "Left",
    "Right",
    "PageUp",
    "PageDown",
    "Home",
    "End",
    "Insert",
];

const MODIFIERS: &[&str] = &["Ctrl", "Alt", "Shift"];

/// Commands that change the tape's state but consume no wall clock.
/// Commands that are real, cost no time, and teleprompt has nothing to do
/// about. Deliberately short: every other VHS command is either read, or
/// refused with a reason. A command accepted here and dropped by the
/// capture is a video that is wrong rather than missing, which is the
/// failure `Set`'s unknown-name rejection exists to prevent — and it was
/// one for as long as nothing ran a tape.
const INSTANT: &[&str] = &[];

/// Settings teleprompt passes through without reading: they change what the
/// terminal looks like, never how long it takes. Named all the same, so a
/// misspelling is reported rather than silently having no effect — `Set
/// TypingSped 10ms` used to pass `check` and then type at the default speed,
/// which is the "wrong length while `check` reports success" failure this
/// adapter's own `Command` doc warns about, arriving by a different door.
const COSMETIC_SETTINGS: &[&str] = &[
    "BorderRadius",
    "CursorBlink",
    "FontFamily",
    "FontSize",
    "Framerate",
    "Height",
    "LetterSpacing",
    "LineHeight",
    "Margin",
    "MarginFill",
    "Padding",
    "Theme",
    "WaitPattern",
    "Width",
    "WindowBar",
    "WindowBarSize",
];

/// The mark, spelled as a VHS comment.
///
/// VHS has no yield-point concept, and adding a `Mark` command would make the
/// body a teleprompt dialect rather than a tape — forfeiting the only reason
/// to point `include=` at a real `.tape` file, which is that `vhs demo.tape`
/// runs it and VHS's own tooling understands it. As a comment it is inert.
const MARK: &str = "# mark";

/// A `Set` that teleprompt reads, or one it only passes through.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Setting {
    TypingSpeed(u64),
    WaitTimeout(u64),
    /// Real, applied by the terminal, and costing no time.
    Cosmetic,
}

/// Commands that run but are not watched running. `Hide` stops the
/// recording, `Show` resumes it; the shell keeps going either way. It is
/// how a tape does its setup — `cd`, `export`, `clear` — without every
/// demo opening on somebody navigating to a directory.
const HIDE: &str = "Hide";
const SHOW: &str = "Show";

/// What one tape line contributes.
///
/// This is the adapter's single notion of content, and all three trait
/// methods go through it — the same discipline `MockScene` documents. A
/// `Type@100ms` that `validate` accepts and `estimate` does not understand
/// is not a parse bug, it is a video that is the wrong length while `check`
/// reports success.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Blank, a comment, or a command with no duration.
    Nothing,
    Mark,
    /// Stop recording. What follows runs and is not seen, and — this is
    /// the part that has to agree between measuring and capturing — costs
    /// the shot no time, because nothing is on screen for it to cost.
    Hide,
    Show,
    /// `Require <program>`: refuse to record unless it is there.
    Require(String),
    /// `Copy "text"`, and the `Paste` that sends it.
    Copy(String),
    Paste,
    Sleep(u64),
    /// `speed` is a `Type@<duration>` override; `None` means "whatever
    /// `Set TypingSpeed` last established".
    ///
    /// The text rides along rather than just its length, because this enum
    /// is the adapter's single notion of what a tape line contains and a
    /// capture backend has to send the characters somewhere. Two parsers
    /// for one language is a tape that `check` accepts and nothing can
    /// run.
    Type {
        text: String,
        speed: Option<u64>,
    },
    Keys {
        /// The key as the tape spelled it — `Enter`, `Ctrl+C`, `Down`.
        key: String,
        count: u64,
        speed: Option<u64>,
    },
    /// `timeout` is a `Wait@<duration>` override; `None` means "whatever
    /// `Set WaitTimeout` last established".
    Wait {
        timeout: Option<u64>,
    },
    Setting(Setting),
}

/// The lone place a tape line is interpreted.
///
/// Public because it is the lone place: the capture backend that runs a
/// tape reads the same enum the compiler measures, so a line the two would
/// disagree about cannot exist.
pub fn classify(line: &str) -> Result<Command, CommandError> {
    let line = line.trim();

    if line == MARK {
        return Ok(Command::Mark);
    }
    if line.is_empty() || line.starts_with('#') {
        return Ok(Command::Nothing);
    }

    let mut parts = line.split_whitespace();
    let head = parts.next().expect("a trimmed non-empty line has a token");

    // `Type@100ms "hello"` — VHS hangs the per-command speed off the name.
    let (cmd, at) = match head.split_once('@') {
        Some((cmd, dur)) => {
            let ms =
                parse_duration_ms(dur).map_err(|m| CommandError::new(m, "e.g. `Type@100ms`"))?;
            (cmd, Some(ms))
        }
        None => (head, None),
    };

    match cmd {
        "Sleep" => {
            let Some(value) = parts.next() else {
                return Err(CommandError::new(
                    "`Sleep` needs a duration",
                    "e.g. `Sleep 2s`",
                ));
            };
            // Trailing garbage is rejected rather than ignored, for the
            // reason `MockScene` gives: a line that passes `check` and then
            // contributes nothing is the worst of both outcomes.
            if let Some(extra) = parts.next() {
                return Err(CommandError::new(
                    format!("`Sleep` takes one duration, found trailing `{extra}`"),
                    "e.g. `Sleep 2s`",
                ));
            }
            parse_duration_ms(value)
                .map(Command::Sleep)
                .map_err(|m| CommandError::new(m, "e.g. `Sleep 2s`"))
        }

        "Type" => {
            let (text, tail) = quoted(cmd, line[head.len()..].trim())?;
            reject_trailing(cmd, tail)?;
            // A tape long enough to overflow this could not fit in memory,
            // but `as` would wrap silently if one ever did.
            Ok(Command::Type { text, speed: at })
        }

        "Set" => setting(parts.next(), parts.next()),

        // teleprompt owns framing and encoding: a tape that writes its own
        // file would produce a second, unscheduled artifact beside the one
        // the timeline expects.
        "Output" => Err(CommandError::new(
            "`Output` is set by teleprompt, not by the tape",
            "remove the line; the compiler routes frames to the timeline",
        )),

        // `Source` splices another tape in at run time, long after `shots`
        // has decided where the shots are — so any mark inside the sourced
        // tape is invisible to the split that was supposed to honour it.
        "Source" => Err(CommandError::new(
            "`Source` runs another tape inline, which hides its marks from the shot split",
            "load the other tape with the fence's `include=` attribute instead",
        )),

        // `Wait` blocks until the shell prompt returns, so its real duration
        // is however long the command underneath takes — a number that is
        // nowhere in the tape. Its timeout is the one thing the tape does
        // state, so that is what it contributes, and the shot carrying it
        // reports `Estimated` rather than `Exact`.
        w if w.starts_with("Wait") => wait(w, at, line[head.len()..].trim()),

        key if is_key(key) => {
            let count = match parts.next() {
                None => 1,
                Some(value) => value.parse::<u64>().map_err(|_| {
                    CommandError::new(
                        format!("`{key}` takes an optional repeat count, found `{value}`"),
                        "e.g. `Enter 3`",
                    )
                })?,
            };
            Ok(Command::Keys {
                key: key.to_string(),
                count,
                speed: at,
            })
        }

        HIDE => reject_trailing(cmd, line[head.len()..].trim()).map(|()| Command::Hide),
        SHOW => reject_trailing(cmd, line[head.len()..].trim()).map(|()| Command::Show),

        "Require" => {
            let Some(program) = parts.next() else {
                return Err(CommandError::new(
                    "`Require` needs a program name",
                    "e.g. `Require flowrs`",
                ));
            };
            if let Some(extra) = parts.next() {
                return Err(CommandError::new(
                    format!("`Require` takes one program, found trailing `{extra}`"),
                    "e.g. `Require flowrs`",
                ));
            }
            Ok(Command::Require(program.to_string()))
        }

        "Copy" => {
            let (text, tail) = quoted(cmd, line[head.len()..].trim())?;
            reject_trailing(cmd, tail)?;
            Ok(Command::Copy(text))
        }
        "Paste" => reject_trailing(cmd, line[head.len()..].trim()).map(|()| Command::Paste),

        // The environment belongs to the scene, not to the tape — the same
        // reason `Set Shell` does. A scene is a session: its blocks share
        // one shell, and that shell's environment is settled before the
        // first of them runs, so a variable set halfway through a
        // walkthrough could not mean what it says.
        "Env" => Err(CommandError::new(
            "`Env` is set by teleprompt, not by the tape",
            "put it under `scene.<name>.env` in the project config",
        )),

        // teleprompt owns what a capture writes, for the reason `Output`
        // gives: a tape that writes its own file produces a second,
        // unscheduled artifact beside the one the timeline expects.
        "Screenshot" => Err(CommandError::new(
            "`Screenshot` is written by teleprompt, not by the tape",
            "remove the line; every shot's last frame is already kept",
        )),

        instant if INSTANT.contains(&instant) => Ok(Command::Nothing),

        other => Err(CommandError::new(
            format!("unknown VHS command `{other}`"),
            match capitalisation_of(other) {
                // By far the likeliest mistake, and "unknown VHS command
                // `type`" sends the author looking for a missing feature
                // rather than a missing shift key.
                Some(correct) => format!("VHS commands are capitalised: write `{correct}`"),
                None => "see the VHS tape reference for the command list".to_string(),
            },
        )),
    }
}

/// `Set <name> <value>`.
///
/// An unrecognised name is an error rather than a pass-through. The
/// pass-through is the tempting reading — VHS has settings teleprompt does
/// not care about, and ignoring them keeps the adapter out of the way — but
/// it cannot tell `Set Padding 20` from `Set TypingSped 10ms`, and the second
/// is a tape that types at a speed its author did not choose.
fn setting(name: Option<&str>, value: Option<&str>) -> Result<Command, CommandError> {
    let (Some(name), Some(value)) = (name, value) else {
        return Err(match name {
            Some(name) => CommandError::new(
                format!("`Set {name}` needs a value"),
                "e.g. `Set FontSize 32`",
            ),
            None => CommandError::new("`Set` needs a name and a value", "e.g. `Set FontSize 32`"),
        });
    };

    let duration = |ms: fn(u64) -> Setting| {
        parse_duration_ms(value)
            .map(|v| Command::Setting(ms(v)))
            .map_err(|m| {
                CommandError::new(
                    format!("`Set {name}`: {m}"),
                    format!("e.g. `Set {name} 40ms`"),
                )
            })
    };

    match name {
        "TypingSpeed" => duration(Setting::TypingSpeed),
        "WaitTimeout" => duration(Setting::WaitTimeout),

        // teleprompt executes the tape against its own PTY (§7.5), so a tape
        // that picks its own shell picks one teleprompt is not driving.
        "Shell" => Err(CommandError::new(
            "`Set Shell` is set by teleprompt, not by the tape",
            "remove the line; configure the shell under `scene.terminal`",
        )),

        // Re-timing the finished recording would slide the narration out from
        // under the action it was scheduled against — which is the one thing
        // the whole scheduler exists to prevent.
        "PlaybackSpeed" => Err(CommandError::new(
            "`Set PlaybackSpeed` re-times the recording after the fact, which would slide the \
             narration out from under it",
            "pace the shot against its narration with `policy=stretch-action` on the fence",
        )),

        // A GIF-only setting, and a shot inside a video never loops.
        "LoopOffset" => Err(CommandError::new(
            "`Set LoopOffset` chooses which frame a looping GIF starts on, and a shot inside a \
             video never loops",
            "remove the line",
        )),

        n if COSMETIC_SETTINGS.contains(&n) => Ok(Command::Setting(Setting::Cosmetic)),

        other => Err(CommandError::new(
            format!("unknown setting `{other}`"),
            format!("teleprompt understands: {}", settings_list().join(", ")),
        )),
    }
}

/// `Wait`, `Wait+Screen`, or `Wait+Command`, with an optional `/regex/`.
fn wait(cmd: &str, at: Option<u64>, rest: &str) -> Result<Command, CommandError> {
    let scope = &cmd["Wait".len()..];
    if !matches!(scope, "" | "+Screen" | "+Command") {
        return Err(CommandError::new(
            format!("`{cmd}` is not a scope `Wait` understands"),
            "write `Wait`, `Wait+Screen`, or `Wait+Command`",
        ));
    }
    // The `@` on a `Wait` is its timeout, not a typing speed — the one place
    // the two readings of `@` differ, and the reason `classify` hands `at`
    // through rather than interpreting it at the split.
    let pattern_ok =
        rest.is_empty() || (rest.len() >= 2 && rest.starts_with('/') && rest.ends_with('/'));
    if !pattern_ok {
        return Err(CommandError::new(
            format!("`{cmd}`'s pattern must be a regular expression between slashes"),
            "e.g. `Wait+Screen /\\$ $/`",
        ));
    }
    Ok(Command::Wait { timeout: at })
}

/// Whether `name` is a key press: a bare key name, or modifiers ending in a
/// key name or a single character.
///
/// A lone character is not a key — `C` on a line of its own is a typo, not a
/// keystroke — and a chord's tail is checked rather than assumed, so
/// `Ctrl+Etner` is reported instead of silently pressed.
fn is_key(name: &str) -> bool {
    let mut parts = name.split('+').peekable();
    let mut modifiers = 0;
    while let Some(part) = parts.next() {
        if parts.peek().is_none() {
            return !part.is_empty()
                && (KEYS.contains(&part) || (modifiers > 0 && part.chars().count() == 1));
        }
        if !MODIFIERS.contains(&part) {
            return false;
        }
        modifiers += 1;
    }
    false
}

fn settings_list() -> Vec<&'static str> {
    let mut all = COSMETIC_SETTINGS.to_vec();
    all.push("TypingSpeed");
    all.push("WaitTimeout");
    all.sort_unstable();
    all
}

/// The command `other` would have been, had it been capitalised.
fn capitalisation_of(other: &str) -> Option<&'static str> {
    ["Sleep", "Type", "Set", "Output", "Source", "Wait"]
        .into_iter()
        .chain(KEYS.iter().copied())
        .chain(INSTANT.iter().copied())
        .find(|c| c.eq_ignore_ascii_case(other))
}

/// Reads one quoted string, returning it unescaped with the rest of the line.
/// VHS accepts double quotes, single quotes, and backticks.
///
/// Scanned rather than stripped at both ends: `strip_suffix` cannot tell a
/// closing quote from an escaped one, so `Type "say \"hi\""` measured two
/// characters too many, and a trailing argument after the string was folded
/// into it instead of being reported.
fn quoted<'a>(cmd: &str, rest: &'a str) -> Result<(String, &'a str), CommandError> {
    let help = format!("e.g. `{cmd} \"npm install\"`");
    let mut chars = rest.char_indices();
    let Some((_, open)) = chars.next() else {
        return Err(CommandError::new(
            format!("`{cmd}` needs a quoted string"),
            help,
        ));
    };
    if !matches!(open, '"' | '\'' | '`') {
        return Err(CommandError::new(
            format!("`{cmd}`'s argument must be quoted, found `{rest}`"),
            help,
        ));
    }

    let mut out = String::new();
    let mut escaped = false;
    for (i, c) in chars {
        if escaped {
            // Only the escapes a shell demo needs; anything else keeps its
            // backslash, so `Type "C:\path"` types what it says.
            match c {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                other => {
                    if other != open && other != '\\' {
                        out.push('\\');
                    }
                    out.push(other);
                }
            }
            escaped = false;
            continue;
        }
        match c {
            '\\' => escaped = true,
            c if c == open => return Ok((out, &rest[i + c.len_utf8()..])),
            c => out.push(c),
        }
    }

    Err(CommandError::new(
        format!("`{cmd}`'s string is never closed"),
        help,
    ))
}

fn reject_trailing(cmd: &str, tail: &str) -> Result<(), CommandError> {
    if tail.trim().is_empty() {
        return Ok(());
    }
    Err(CommandError::new(
        format!("`{cmd}` has trailing `{}`", tail.trim()),
        "one command per line",
    ))
}

/// True when a mark-separated chunk carries something to execute.
///
/// A chunk of nothing but comments and `Set` lines is empty, not a
/// zero-duration shot (ruling F13). `TypingSpeed` counts as a setting here
/// even though it carries a duration: it configures later typing rather than
/// spending any time itself, and a chunk holding only settings is exactly the
/// phantom shot F13 rules out.
fn has_content(chunk: &[&str]) -> bool {
    chunk
        .iter()
        .any(|l| !matches!(classify(l), Ok(Command::Nothing | Command::Setting(_))))
}

impl SceneCompiler for VhsScene {
    fn kind(&self) -> &'static str {
        "vhs"
    }

    fn validate(&self, src: &BlockSource) -> Result<Validated, Vec<Diagnostic>> {
        validate_commands(src, classify)
    }

    fn shots(&self, v: &Validated, block_id: &str) -> Result<Vec<Shot>, Vec<Diagnostic>> {
        let lines: Vec<&str> = v.body.lines().collect();
        let mut settings: Vec<&str> = Vec::new();
        let mut shots: Vec<Shot> = Vec::new();

        for chunk in lines.split(|l| matches!(classify(l), Ok(Command::Mark))) {
            // `Set` lines established before a mark still govern the tape
            // after it, so each shot carries the settings in force when it
            // starts. That keeps `estimate` a pure function of `shot.source`,
            // and puts the settings in the hash — so raising TypingSpeed
            // invalidates exactly the later shots whose timing it changes.
            let source = settings
                .iter()
                .chain(chunk.iter())
                .copied()
                .collect::<Vec<_>>()
                .join("\n");

            settings.extend(
                chunk
                    .iter()
                    .filter(|l| l.trim_start().starts_with("Set "))
                    .copied(),
            );

            if !has_content(chunk) {
                continue;
            }

            let index = shots.len();
            shots.push(Shot {
                id: format!("{block_id}#{index}"),
                hash: Hash::of(source.trim().as_bytes()),
                source,
                index,
            });
        }

        Ok(shots)
    }

    /// Exact for a shot whose timing the tape states in full, `Estimated` for
    /// one containing a `Wait`.
    ///
    /// The distinction is per shot rather than per adapter. `Sleep` and
    /// `Set TypingSpeed` are exact, and a shot built from those alone needs
    /// no measuring pass — which is what lets `plan` and `diff` report a
    /// terminal scene's pacing offline, with no terminal anywhere. `Wait` is
    /// the exception: it blocks until the shell prompt returns, so its length
    /// is whatever `cargo build` takes, and that number is nowhere in the
    /// tape. Its timeout is the bound the tape does state, so the shot
    /// contributes that and says `Estimated` — the honest signal that M1's
    /// measuring pass has something to improve here and nothing to improve on
    /// the shot next to it.
    fn estimate(&self, shot: &Shot) -> Measured {
        let mut speed = DEFAULT_TYPING_SPEED_MS;
        let mut timeout = DEFAULT_WAIT_TIMEOUT_MS;
        let mut total: u64 = 0;
        let mut waited = false;
        // Hidden commands cost the shot nothing. They run — the `cd` really
        // happens — but a shot's duration is how long something is *on
        // screen*, and nothing hidden is. Counting them would size the slot
        // for work the viewer never sees and leave the narration waiting
        // through it.
        let mut hidden = false;

        for line in shot.source.lines() {
            // Hidden commands run and cost the shot nothing, so every
            // duration below goes through `visible`.
            let mut visible = |ms: u64| {
                if !hidden {
                    total = total.saturating_add(ms);
                }
            };
            match classify(line) {
                Ok(Command::Hide) => hidden = true,
                Ok(Command::Show) => hidden = false,
                Ok(Command::Sleep(ms)) => visible(ms),
                Ok(Command::Setting(Setting::TypingSpeed(ms))) => speed = ms,
                Ok(Command::Setting(Setting::WaitTimeout(ms))) => timeout = ms,
                Ok(Command::Type { text, speed: at }) => {
                    // A tape long enough to overflow this could not fit in
                    // memory, but `as` would wrap silently if one ever did.
                    let chars = u64::try_from(text.chars().count()).unwrap_or(u64::MAX);
                    visible(chars.saturating_mul(at.unwrap_or(speed)));
                }
                Ok(Command::Keys {
                    count, speed: at, ..
                }) => visible(count.saturating_mul(at.unwrap_or(speed))),
                // A paste arrives all at once, so it costs one keystroke
                // rather than one per character.
                Ok(Command::Paste) => visible(speed),
                Ok(Command::Wait { timeout: at }) => {
                    waited = true;
                    visible(at.unwrap_or(timeout));
                }
                // Not a catch-all: a tape command added later must fail to
                // compile here rather than silently estimate as zero.
                Ok(
                    Command::Nothing
                    | Command::Mark
                    | Command::Require(_)
                    | Command::Copy(_)
                    | Command::Setting(Setting::Cosmetic),
                )
                | Err(_) => {}
            }
        }

        if waited {
            Measured::Estimated(total)
        } else {
            Measured::Exact(total)
        }
    }

    /// Re-times a tape by scaling everything that takes time in it.
    ///
    /// Every `Sleep`, and the speed of every keystroke, moves by the same
    /// factor, so a stretched shot is the same performance played slower
    /// rather than the same performance followed by a wait. That is what
    /// `stretch-action` means when it says the action fills the sentence
    /// above it, and the difference is visible: a tape that types at its
    /// authored speed and then sits still for ten seconds is the frozen
    /// frame again, wearing a `Sleep`.
    ///
    /// Rounding is settled at the end rather than per line. Milliseconds
    /// are integers and a factor is not, so a dozen scaled values can miss
    /// the target by a few milliseconds between them; the remainder becomes
    /// one trailing `Sleep`, which is the only line here that is not just
    /// the author's own, scaled.
    fn retime(&self, shot: &Shot, target_ms: u64) -> Option<String> {
        // Only a shot that states its own timing can be promised to a
        // length. A `Wait` is as long as the command takes.
        let current = match self.estimate(shot) {
            Measured::Exact(ms) => ms,
            _ => return None,
        };
        if current == 0 || target_ms == 0 {
            return None;
        }

        let factor = target_ms as f64 / current as f64;
        let scale = |ms: u64| ((ms as f64 * factor).round() as u64).max(1);

        let mut out: Vec<String> = Vec::new();
        let mut stated_speed = false;
        for line in shot.source.lines() {
            let trimmed = line.trim();
            match classify(trimmed) {
                Ok(Command::Sleep(ms)) => out.push(format!("Sleep {}ms", scale(ms))),
                Ok(Command::Setting(Setting::TypingSpeed(ms))) => {
                    stated_speed = true;
                    out.push(format!("Set TypingSpeed {}ms", scale(ms)));
                }
                // `Type@100ms "…"` and `Enter@50ms` carry their own speed,
                // which the tape states per command and which therefore
                // scales per command.
                Ok(
                    Command::Type {
                        speed: Some(at), ..
                    }
                    | Command::Keys {
                        speed: Some(at), ..
                    },
                ) => {
                    out.push(rescale_at(trimmed, scale(at)));
                }
                _ => out.push(line.to_string()),
            }
        }

        // A tape that never said how fast it types is typing at the default,
        // and the default is not the author's to leave alone here: without
        // this line the keystrokes would be the one thing that did not
        // stretch.
        if !stated_speed {
            out.insert(
                0,
                format!("Set TypingSpeed {}ms", scale(DEFAULT_TYPING_SPEED_MS)),
            );
        }

        let mut source = out.join("\n");
        source.push('\n');

        let scaled = Shot {
            id: shot.id.clone(),
            source: source.clone(),
            hash: shot.hash,
            index: shot.index,
        };
        if let Measured::Exact(reached) = self.estimate(&scaled) {
            if let Some(remainder) = target_ms.checked_sub(reached) {
                if remainder > 0 {
                    source.push_str(&format!("Sleep {remainder}ms\n"));
                }
            } else {
                // Overshot by rounding: take it back off the last sleep
                // rather than leaving the shot longer than it was promised.
                source = shorten_last_sleep(&source, reached - target_ms)?;
            }
        }
        Some(source)
    }
}

/// `Type@100ms "hi"` with its `@` replaced. The text after the command is
/// the author's and is copied through untouched.
fn rescale_at(line: &str, ms: u64) -> String {
    match line.split_once(char::is_whitespace) {
        Some((head, rest)) => {
            let cmd = head.split('@').next().unwrap_or(head);
            format!("{cmd}@{ms}ms {rest}")
        }
        None => {
            let cmd = line.split('@').next().unwrap_or(line);
            format!("{cmd}@{ms}ms")
        }
    }
}

/// Takes `excess` milliseconds off the tape's last `Sleep`, or gives up if
/// there is no sleep long enough to take it from.
fn shorten_last_sleep(source: &str, excess: u64) -> Option<String> {
    let mut lines: Vec<String> = source.lines().map(str::to_string).collect();
    let last = lines
        .iter()
        .rposition(|l| matches!(classify(l.trim()), Ok(Command::Sleep(ms)) if ms > excess))?;
    if let Ok(Command::Sleep(ms)) = classify(lines[last].trim()) {
        lines[last] = format!("Sleep {}ms", ms - excess);
    }
    let mut out = lines.join("\n");
    out.push('\n');
    Some(out)
}
