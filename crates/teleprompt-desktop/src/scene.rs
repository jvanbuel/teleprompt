//! The compile-time half, the same for every desktop: each platform's
//! plugin registers it under its own scene plugin name.

use teleprompt_core::{BlockId, Diagnostic, Hash};
use teleprompt_plugin::scene::{
    validate_commands, BlockSource, Measured, SceneCompiler, Shot, Validated,
};

use crate::script::{
    classify, timing, Action, DEFAULT_POINTER_SPEED_MS, DEFAULT_TYPING_SPEED_MS, MARK,
};

/// A desktop scene's blocks, under the plugin name `kind`.
#[derive(Debug, Clone, Copy)]
pub struct DesktopScene {
    pub kind: &'static str,
}

/// Exact where every line states its time; `Estimated` at the timeout
/// where a `Wait` depends on the app.
pub fn length(source: &str) -> Measured {
    match timing(source.lines()) {
        (ms, false) => Measured::Exact(ms),
        (ms, true) => Measured::Estimated(ms),
    }
}

impl SceneCompiler for DesktopScene {
    fn kind(&self) -> &'static str {
        self.kind
    }

    fn validate(&self, src: &BlockSource) -> Result<Validated, Vec<Diagnostic>> {
        validate_commands(src, classify)
    }

    /// Split at `# mark`. Each shot carries the `Set` lines in force when it
    /// starts, so its timing depends on its own source alone.
    fn shots(&self, v: &Validated, block_id: &BlockId) -> Result<Vec<Shot>, Vec<Diagnostic>> {
        let lines: Vec<&str> = v.body.lines().collect();
        let mut settings: Vec<&str> = Vec::new();
        let mut shots = Vec::new();
        for chunk in lines.split(|l| matches!(classify(l), Ok(Action::Mark))) {
            let source = settings
                .iter()
                .chain(chunk)
                .copied()
                .collect::<Vec<_>>()
                .join("\n");
            settings.extend(chunk.iter().copied().filter(|l| is_setting(l)));
            if !chunk.iter().any(|l| does_something(l)) {
                continue;
            }
            let (hash, length) = (Hash::of(source.trim().as_bytes()), length(&source));
            shots.push(Shot::numbered(block_id, shots.len(), source, hash).lasting(length));
        }
        Ok(shots)
    }

    /// Plays the same actions slower or faster: every pause, key and glide
    /// scaled by one factor, the rounding settled in a last `Sleep`.
    fn retime(&self, shot: &Shot, target_ms: u64) -> Option<String> {
        let Measured::Exact(current) = length(&shot.source) else {
            return None;
        };
        if current == 0 || target_ms == 0 {
            return None;
        }
        let factor = target_ms as f64 / current as f64;
        let scale = |ms: u64| ((ms as f64 * factor).round() as u64).max(1);

        // The defaults, stated, so they scale too; a block's own `Set`
        // lines follow and win.
        let mut out = vec![
            format!("Set TypingSpeed {}ms", scale(DEFAULT_TYPING_SPEED_MS)),
            format!("Set PointerSpeed {}ms", scale(DEFAULT_POINTER_SPEED_MS)),
        ];
        for line in shot.source.lines() {
            let trimmed = line.trim();
            out.push(match classify(trimmed) {
                Ok(Action::Sleep(ms)) => format!("Sleep {}ms", scale(ms)),
                Ok(Action::TypingSpeed(ms)) => format!("Set TypingSpeed {}ms", scale(ms)),
                Ok(Action::PointerSpeed(ms)) => format!("Set PointerSpeed {}ms", scale(ms)),
                Ok(
                    Action::Type {
                        speed: Some(at), ..
                    }
                    | Action::Press {
                        speed: Some(at), ..
                    }
                    | Action::Pointer {
                        speed: Some(at), ..
                    },
                ) => with_speed(trimmed, scale(at)),
                _ => line.to_string(),
            });
        }
        let reached = timing(out.iter().map(String::as_str)).0;
        if reached < target_ms {
            out.push(format!("Sleep {}ms", target_ms - reached));
        } else if reached > target_ms {
            shorten_last_sleep(&mut out, reached - target_ms)?;
        }
        Some(out.join("\n") + "\n")
    }

    fn select(&self, body: &str, fragment: &str) -> Result<String, String> {
        teleprompt_plugin::scene::select_marked(body, MARK, fragment)
    }
}

fn is_setting(line: &str) -> bool {
    matches!(
        classify(line),
        Ok(Action::TypingSpeed(_) | Action::PointerSpeed(_) | Action::WaitTimeout(_))
    )
}

/// Whether a line acts: a part of only comments and settings is no shot.
fn does_something(line: &str) -> bool {
    !is_setting(line) && !matches!(classify(line), Ok(Action::Nothing | Action::Mark) | Err(_))
}

/// `Click@400ms 1 2` with its speed replaced.
fn with_speed(line: &str, ms: u64) -> String {
    let (head, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
    let name = head.split('@').next().unwrap_or(head);
    format!("{name}@{ms}ms {rest}").trim_end().to_string()
}

fn shorten_last_sleep(lines: &mut [String], excess: u64) -> Option<()> {
    let last = lines
        .iter()
        .rposition(|l| matches!(classify(l), Ok(Action::Sleep(ms)) if ms > excess))?;
    if let Ok(Action::Sleep(ms)) = classify(&lines[last]) {
        lines[last] = format!("Sleep {}ms", ms - excess);
    }
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use teleprompt_plugin::scene::BodyOrigin;

    const SCENE: DesktopScene = DesktopScene { kind: "x11" };

    fn shots(body: &str) -> Vec<Shot> {
        let src = BlockSource {
            scene: "app".into(),
            body: body.into(),
            origin: BodyOrigin::Included { path: "t".into() },
        };
        let v = SCENE.validate(&src).unwrap_or_else(|d| panic!("{d:?}"));
        SCENE.shots(&v, &BlockId::new("tour")).unwrap()
    }

    #[test]
    fn marks_split_a_block_and_settings_carry_forward() {
        let s = shots("Set TypingSpeed 80ms\nType \"ab\"\n# mark\nEnter\n# mark\n# nothing\n");
        assert_eq!(s.len(), 2, "a part with only a comment is no shot");
        assert!(
            s[1].source.starts_with("Set TypingSpeed 80ms"),
            "{}",
            s[1].source
        );
        assert_eq!(s[1].length, Measured::Exact(80));
    }

    #[test]
    fn a_wait_makes_a_shot_a_bound() {
        let s = shots("Wait@3s \"Teleprompt\"\nSleep 1s\n");
        assert_eq!(s[0].length, Measured::Estimated(4000));
        assert_eq!(
            SCENE.retime(&s[0], 8000),
            None,
            "an app's own time cannot be scaled"
        );
    }

    #[test]
    fn retiming_plays_the_same_actions_at_another_pace() {
        let s = shots("Type \"abcd\"\nClick@300ms 10 20\nSleep 1s\n");
        let before = s[0].length.duration_ms().unwrap();
        let source = SCENE.retime(&s[0], before * 2).unwrap();
        assert_eq!(length(&source), Measured::Exact(before * 2), "{source}");
        assert!(source.contains("Click@600ms 10 20"), "{source}");
        assert!(source.contains("Sleep 2000ms"), "{source}");
    }

    #[test]
    fn a_retime_that_rounds_over_comes_off_the_last_sleep() {
        let s = shots("Type \"abc\"\nSleep 1s\n");
        for target in [1001, 1333, 2999] {
            let source = SCENE.retime(&s[0], target).unwrap();
            assert_eq!(timing(source.lines()).0, target, "{source}");
        }
    }

    #[test]
    fn every_bad_line_is_reported() {
        let src = BlockSource {
            scene: "app".into(),
            body: "Clik 1 2\nType hi\nEnter\n".into(),
            origin: BodyOrigin::Included { path: "t".into() },
        };
        assert_eq!(SCENE.validate(&src).unwrap_err().len(), 2);
    }
}
