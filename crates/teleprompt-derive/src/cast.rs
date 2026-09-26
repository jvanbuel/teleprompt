//! A terminal session as asciinema records it.

use serde_json::Value;

/// What a recorded session holds that a script needs: when each keystroke
/// arrived, and when the terminal wrote anything, in milliseconds from the
/// start of the recording.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Trace {
    pub input: Vec<(u64, String)>,
    pub output: Vec<u64>,
}

/// Reads an asciicast, version 2 (absolute times) or 3 (intervals). It
/// must have recorded the keyboard.
pub fn read_cast(text: &str) -> Result<Trace, String> {
    let mut lines = text.lines().enumerate();
    let header: Value = lines
        .next()
        .and_then(|(_, l)| serde_json::from_str(l).ok())
        .ok_or("the cast has no header line")?;
    let relative = match header.get("version").and_then(Value::as_u64) {
        Some(2) => false,
        Some(3) => true,
        Some(v) => {
            return Err(format!(
                "asciicast version {v} is not supported; record with asciinema 2 or 3"
            ))
        }
        None => return Err("the cast's header has no version".to_string()),
    };

    let mut trace = Trace::default();
    let mut clock = 0.0;
    for (i, line) in lines {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (time, kind, data) =
            event(line).ok_or_else(|| format!("line {} is not an event: {line}", i + 1))?;
        clock = if relative { clock + time } else { time };
        let ms = (clock * 1000.0).round() as u64;
        match kind.as_str() {
            "i" => trace.input.push((ms, data)),
            "o" => trace.output.push(ms),
            _ => {}
        }
    }
    if trace.input.is_empty() {
        return Err(
            "the cast has no keystrokes: record it with `asciinema rec --stdin` \
             (asciinema 2) or `--capture-input` (asciinema 3)"
                .to_string(),
        );
    }
    Ok(trace)
}

fn event(line: &str) -> Option<(f64, String, String)> {
    let v: Value = serde_json::from_str(line).ok()?;
    let [time, kind, data] = v.as_array()?.as_slice() else {
        return None;
    };
    Some((
        time.as_f64()?,
        kind.as_str()?.to_string(),
        data.as_str()?.to_string(),
    ))
}
