//! The `terminal` scene, served by VHS tapes.
//!
//! The block body is real VHS tape syntax and stays runnable by `vhs`
//! itself. See `docs/design.md#adapters` for what is refused and why a shot
//! is `Exact` or `Estimated`.

use teleprompt_core::attrs::parse_duration_ms;
use teleprompt_core::{Diagnostic, Hash};

use teleprompt_scene::{
    validate_commands, BlockSource, CommandError, Measured, SceneCompiler, Shot, Validated,
};

/// The VHS adapter.
#[derive(Debug)]
pub struct VhsScene;

/// VHS's default gap between keystrokes, so an unannotated tape estimates
/// correctly.
const DEFAULT_TYPING_SPEED_MS: u64 = 50;

/// VHS's default bound on `Wait`, so teleprompt schedules against the bound
/// VHS enforces.
const DEFAULT_WAIT_TIMEOUT_MS: u64 = 5_000;

/// Keys VHS presses; chords are built from these (see [`is_key`]).
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

/// Settings that change how the terminal looks, never how long it takes.
/// Listed so a misspelling such as `Set TypingSped` is an error rather than
/// a tape typing at a speed nobody chose.
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

/// The mark, spelled as a VHS comment so the tape still runs under `vhs`
/// (`docs/design.md#marks`).
const MARK: &str = "# mark";

/// A `Set` that teleprompt reads, or one it only passes through.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Setting {
    TypingSpeed(u64),
    WaitTimeout(u64),
    /// Real, applied by the terminal, and costing no time.
    Cosmetic,
}

/// `Hide` stops the recording and `Show` resumes it; the shell keeps going
/// either way, which is how a tape does unseen setup.
const HIDE: &str = "Hide";
const SHOW: &str = "Show";

/// What one tape line contributes.
///
/// Every trait method reads lines through [`classify`], so `validate` and
/// `estimate` cannot disagree about a line; if they did, `check` would pass
/// a video of the wrong length.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Blank, a comment, or a command with no duration.
    Nothing,
    Mark,
    /// What follows runs unseen and costs the shot no time.
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

    if let Some(err) = refused(cmd) {
        return Err(err);
    }

    match cmd {
        "Sleep" => {
            let Some(value) = parts.next() else {
                return Err(CommandError::new(
                    "`Sleep` needs a duration",
                    "e.g. `Sleep 2s`",
                ));
            };
            // Rejected, not ignored: a line that passes `check` and then
            // contributes nothing is the worst outcome.
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
            Ok(Command::Type { text, speed: at })
        }

        "Set" => setting(parts.next(), parts.next()),

        w if w.starts_with("Wait") => wait(w, at, line[head.len()..].trim()),

        key if is_key(key) => keys(key, parts.next(), at),

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

        other => Err(CommandError::new(
            format!("unknown VHS command `{other}`"),
            match capitalisation_of(other) {
                // The likeliest mistake, so name the fix.
                Some(correct) => format!("VHS commands are capitalised: write `{correct}`"),
                None => "see the VHS tape reference for the command list".to_string(),
            },
        )),
    }
}

/// `<key> [count]`, pressed `count` times.
fn keys(key: &str, count: Option<&str>, speed: Option<u64>) -> Result<Command, CommandError> {
    let count = match count {
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
        speed,
    })
}

/// The error for a VHS command that teleprompt refuses in a tape, or `None`
/// if `cmd` is not one of them.
fn refused(cmd: &str) -> Option<CommandError> {
    let (message, help) = match cmd {
        // teleprompt writes the tape's `Output`; another would be a second,
        // unscheduled artifact.
        "Output" => (
            "`Output` is set by teleprompt, not by the tape",
            "remove the line; the compiler routes frames to the timeline",
        ),
        // A sourced tape is spliced in at run time, after `shots` has split
        // the block, so its marks would be ignored.
        "Source" => (
            "`Source` runs another tape inline, which hides its marks from the shot split",
            "load the other tape with the fence's `include=` attribute instead",
        ),
        // A scene's blocks share one shell whose environment is settled
        // before the first runs, so a block cannot change it.
        "Env" => (
            "`Env` is set by teleprompt, not by the tape",
            "put it under `scene.<name>.env` in the project config",
        ),
        // Refused for the reason `Output` is.
        "Screenshot" => (
            "`Screenshot` is written by teleprompt, not by the tape",
            "remove the line; every shot's last frame is already kept",
        ),
        _ => return None,
    };
    Some(CommandError::new(message, help))
}

/// `Set <name> <value>`. An unknown name is an error: see
/// [`COSMETIC_SETTINGS`].
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

        // The shell is shared by the scene's blocks and started before the
        // first runs, so a block cannot choose it.
        "Shell" => Err(CommandError::new(
            "`Set Shell` is set by teleprompt, not by the tape",
            "remove the line; configure the shell under `scene.terminal`",
        )),

        "PlaybackSpeed" => Err(CommandError::new(
            "`Set PlaybackSpeed` re-times the recording after the fact, which would slide the \
             narration out from under it",
            "pace the shot against its narration with `policy=stretch-action` on the fence",
        )),

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
    // Here `at` is a timeout, not a typing speed.
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
/// key name or a single character. A lone `C` is a typo, not a keystroke.
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
        .find(|c| c.eq_ignore_ascii_case(other))
}

/// Reads one quoted string, returning it unescaped with the rest of the line.
/// VHS accepts double quotes, single quotes, and backticks. Scanned, not
/// stripped at both ends, so an escaped quote does not end the string.
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

/// True when a mark-separated chunk carries something to execute. Comments
/// and `Set` lines (even `TypingSpeed`) spend no time, so a chunk of only
/// those is no shot rather than a zero-length one.
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
            // Each shot carries the `Set` lines in force when it starts, so
            // `estimate` depends only on `shot.source` and a changed setting
            // invalidates exactly the later shots it affects.
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
            let hash = Hash::of(source.trim().as_bytes());
            shots.push(Shot::numbered(block_id, index, source, hash));
        }

        Ok(shots)
    }

    /// Exact for a shot whose timing the tape states in full; `Estimated`,
    /// at the timeout, for one containing a `Wait`, whose real length is
    /// however long the command underneath takes.
    fn estimate(&self, shot: &Shot) -> Measured {
        let mut speed = DEFAULT_TYPING_SPEED_MS;
        let mut timeout = DEFAULT_WAIT_TIMEOUT_MS;
        let mut total: u64 = 0;
        let mut waited = false;
        // A shot's duration is how long something is on screen, so hidden
        // commands cost nothing; every duration goes through `visible`.
        let mut hidden = false;

        for line in shot.source.lines() {
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
                // A paste arrives at once: one keystroke.
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

    /// Re-times a tape by scaling every `Sleep` and keystroke speed by one
    /// factor, so a stretched shot is the same performance played slower
    /// rather than followed by a wait.
    ///
    /// Rounding is settled once at the end: the few milliseconds the scaled
    /// lines miss by become one trailing `Sleep`, or come off the last one.
    fn retime(&self, shot: &Shot, target_ms: u64) -> Option<String> {
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
                // A per-command speed scales per command.
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

        // Without this, a tape typing at the default speed would not
        // stretch its keystrokes.
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
                // Overshot by rounding: take it off the last sleep.
                source = shorten_last_sleep(&source, reached - target_ms)?;
            }
        }
        Some(source)
    }
}

/// `Type@100ms "hi"` with its `@` replaced and the rest copied through.
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
