//! What this plugin needs that teleprompt does not ship, as
//! `teleprompt setup` lists and installs it.

use teleprompt_scene::core::tool::{Found, Manager, Tool};

pub static PLAYWRIGHT: Tool = Tool {
    name: "playwright",
    what: "drives the browser for Playwright scenes, in this project",
    license: "Apache-2.0; the browsers it downloads have their own",
    home: "https://playwright.dev",
    guide: None,
    found: Found::Package("playwright"),
    install: &[(
        Manager::Npm,
        "npm install --save-dev playwright && npx playwright install chromium",
    )],
    download_mb: None,
};
