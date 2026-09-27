//! A draft written out as a script.

use crate::{Block, Draft, Mode, Options};

pub(crate) fn render(draft: &Draft, options: &Options) -> String {
    let mut out = format!("---\nteleprompt: 1\n---\n\n# {}\n", options.title);
    let mut part = 0;
    for beat in &draft.beats {
        if let Some(line) = &beat.line {
            out.push('\n');
            out.push_str(&wrap(&line.text));
            out.push('\n');
        }
        for block in &beat.blocks {
            part += 1;
            out.push_str(&format!(
                "\n```teleprompt {}\n```\n",
                info(block, options, part)
            ));
        }
    }
    out
}

/// The fence's attributes: `hold` is the default, so it is left unsaid, as
/// a person writing the script would.
fn info(block: &Block, options: &Options, part: usize) -> String {
    let include = format!("{}#{part}", options.include);
    let include = if include.contains(char::is_whitespace) {
        format!("\"{include}\"")
    } else {
        include
    };
    let mut info = format!("scene={} include={include}", options.scene);
    if block.mode == Mode::Concurrent {
        info.push_str(" policy=concurrent");
    }
    if let Some(cue) = &block.cue {
        info.push_str(&format!(" cue=\"{cue}\""));
    }
    info
}

/// A paragraph broken at spaces to fit 76 columns, as scripts are written;
/// Markdown reads the breaks as spaces.
fn wrap(text: &str) -> String {
    let mut out = String::new();
    let mut width = 0;
    for word in text.split(' ') {
        if width > 0 && width + 1 + word.len() > 76 {
            out.push('\n');
            width = 0;
        } else if width > 0 {
            out.push(' ');
            width += 1;
        }
        out.push_str(word);
        width += word.len();
    }
    out
}
