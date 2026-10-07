//! What this plugin needs that teleprompt does not ship, as
//! `teleprompt setup` lists and installs it.

use teleprompt_scene::core::tool::{Found, Manager, Tool};

pub static XVFB: Tool = Tool {
    name: "Xvfb",
    what: "the virtual display an x11 scene's app runs on",
    license: "MIT",
    home: "https://www.x.org",
    guide: None,
    found: Found::Program("Xvfb"),
    install: &[
        (Manager::Apt, "sudo apt-get install -y xvfb"),
        (Manager::Dnf, "sudo dnf install -y xorg-x11-server-Xvfb"),
        (
            Manager::Pacman,
            "sudo pacman -S --needed --noconfirm xorg-server-xvfb",
        ),
    ],
    download_mb: None,
};

pub static XDOTOOL: Tool = Tool {
    name: "xdotool",
    what: "presses keys and moves the pointer in an x11 scene",
    license: "BSD-3-Clause",
    home: "https://github.com/jordansissel/xdotool",
    guide: None,
    found: Found::Program("xdotool"),
    install: &[
        (Manager::Apt, "sudo apt-get install -y xdotool"),
        (Manager::Dnf, "sudo dnf install -y xdotool"),
        (
            Manager::Pacman,
            "sudo pacman -S --needed --noconfirm xdotool",
        ),
    ],
    download_mb: None,
};
