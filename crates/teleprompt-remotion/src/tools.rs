//! What this plugin needs that teleprompt does not ship, as
//! `teleprompt setup` lists and installs it.

use teleprompt_scene::core::tool::{Found, Tool};

pub static REMOTION: Tool = Tool {
    name: "remotion",
    what: "renders compositions for Remotion scenes",
    license: "the Remotion License: free for individuals and small teams; \
              a larger company needs a company license (https://www.remotion.dev/license)",
    home: "https://www.remotion.dev",
    guide: Some(
        "Remotion renders from a project of your own: run `npm install` in it, \
         and name it in the scene's `project` setting.",
    ),
    found: Found::Unknowable,
    install: &[],
    download_mb: None,
};
