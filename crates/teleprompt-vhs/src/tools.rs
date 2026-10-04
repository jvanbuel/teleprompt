//! What this plugin needs that teleprompt does not ship, as
//! `teleprompt setup` lists and installs it.

use teleprompt_plugin::tool::{Found, Manager, Tool};

pub static VHS: Tool = Tool {
    name: "vhs",
    what: "records terminal tapes",
    license: "MIT",
    home: "https://github.com/charmbracelet/vhs",
    guide: None,
    found: Found::Program("vhs"),
    install: &[
        (Manager::Brew, "brew install vhs"),
        (Manager::Pacman, "sudo pacman -S --needed --noconfirm vhs"),
        (
            Manager::Go,
            "go install github.com/charmbracelet/vhs@latest",
        ),
    ],
    download_mb: None,
};

pub static TTYD: Tool = Tool {
    name: "ttyd",
    what: "the terminal vhs records",
    license: "MIT",
    home: "https://github.com/tsl0922/ttyd",
    guide: None,
    found: Found::Program("ttyd"),
    install: &[
        (Manager::Brew, "brew install ttyd"),
        (Manager::Apt, "sudo apt-get install -y ttyd"),
        (Manager::Dnf, "sudo dnf install -y ttyd"),
        (Manager::Pacman, "sudo pacman -S --needed --noconfirm ttyd"),
    ],
    download_mb: None,
};
