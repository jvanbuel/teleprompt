//! A draft written out as a script.

use crate::{Block, Draft, Mode, Options};

pub(crate) fn render(draft: &Draft, options: &Options) -> String {
    let scene = &options.scene;
    let mut out = format!(
        "---\nteleprompt: 1\nscene:\n  {scene}:\n    adapter: vhs\n---\n\n# {}\n",
        options.title
    );
    for beat in &draft.beats {
        if let Some(line) = &beat.line {
            out.push('\n');
            out.push_str(&line.text);
            out.push('\n');
        }
        for block in &beat.blocks {
            out.push_str(&format!(
                "\n```teleprompt {}\n{}```\n",
                info(block, scene),
                block.tape
            ));
        }
    }
    out
}

/// The fence's attributes: `hold` is the default, so it is left unsaid, as
/// a person writing the script would.
fn info(block: &Block, scene: &str) -> String {
    let mut info = format!("scene={scene}");
    if block.mode == Mode::Concurrent {
        info.push_str(" policy=concurrent");
    }
    if let Some(cue) = &block.cue {
        info.push_str(&format!(" cue=\"{cue}\""));
    }
    info
}
