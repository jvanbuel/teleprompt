//! What this plugin needs that teleprompt does not ship, as
//! `teleprompt setup` lists and installs it.

use teleprompt_plugin::core::tool::{Found, Manager, Tool};

pub static ASCIINEMA: Tool = Tool {
    name: "asciinema",
    what: "records terminal sessions for `record`",
    license: "GPL-3.0",
    home: "https://asciinema.org",
    guide: None,
    found: Found::Program("asciinema"),
    install: &[
        (Manager::Brew, "brew install asciinema"),
        (Manager::Apt, "sudo apt-get install -y asciinema"),
        (Manager::Dnf, "sudo dnf install -y asciinema"),
        (
            Manager::Pacman,
            "sudo pacman -S --needed --noconfirm asciinema",
        ),
        (Manager::Pipx, "pipx install asciinema"),
    ],
    download_mb: None,
};

pub static AGG: Tool = Tool {
    name: "agg",
    what: "draws asciinema casts as video",
    license: "Apache-2.0",
    home: "https://github.com/asciinema/agg",
    guide: None,
    found: Found::Program("agg"),
    install: &[
        (Manager::Brew, "brew install agg"),
        (
            Manager::Cargo,
            "cargo install --locked --git https://github.com/asciinema/agg",
        ),
    ],
    download_mb: None,
};
