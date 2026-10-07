//! What this plugin needs that teleprompt does not ship, as
//! `teleprompt setup` lists and installs it.

use teleprompt_plugin::core::tool::{Found, Manager, Tool};

pub static SLIDEV: Tool = Tool {
    name: "slidev",
    what: "exports slides for Slidev scenes, in this project",
    license: "MIT",
    home: "https://sli.dev",
    guide: None,
    found: Found::Package("@slidev/cli"),
    install: &[(
        Manager::Npm,
        "npm install --save-dev @slidev/cli playwright-chromium",
    )],
    download_mb: None,
};
