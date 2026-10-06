//! Asking, in a terminal: which of teleprompt's uses to set up, and
//! whether to install what a command needs when it finds it missing.
//! Nothing here asks unless a person is at a terminal: a script, a pipe
//! or `--format json` gets the report it always did.

use std::fmt;
use std::io::IsTerminal;

use inquire::{Confirm, MultiSelect};

use teleprompt_setup::{download_mb, resolve, Goal, Setup, Tool, GOALS};

/// Whether a person is at a terminal to answer.
pub fn interactive() -> bool {
    std::io::stdin().is_terminal()
        && std::io::stdout().is_terminal()
        && std::io::stderr().is_terminal()
}

/// A goal as the list shows it: what it is for, and what it still needs.
struct Choice {
    goal: &'static Goal,
    shown: String,
}

impl fmt::Display for Choice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.shown)
    }
}

/// Asks which of teleprompt's uses to set up: their names, as `setup`
/// takes them, and what they need. A use that listens is offered only in
/// the build that can.
pub fn choose(setup: &Setup) -> Result<(Vec<String>, Vec<&'static Tool>), String> {
    let listening = cfg!(feature = "listen");
    if !listening {
        eprintln!(
            "Following your voice, drafting from what you said and transcribing \
             conversations need teleprompt built with speech models \
             (`--features listen`), so they are not offered here.\n"
        );
    }
    let offered: Vec<&'static Goal> = GOALS.iter().filter(|g| listening || !g.listens).collect();
    let width = offered.iter().map(|g| g.label.len()).max().unwrap_or(0) + 3;
    let choices: Vec<Choice> = offered
        .into_iter()
        .map(|goal| Choice {
            goal,
            shown: format!("{:<width$}{}", goal.label, state(setup, goal)),
        })
        .collect();
    // What every video needs, ticked when it is missing.
    let defaults: Vec<usize> = choices
        .iter()
        .enumerate()
        .filter(|(_, c)| c.goal.name == "render" && !missing(setup, c.goal).is_empty())
        .map(|(i, _)| i)
        .collect();
    let picked = MultiSelect::new("What do you want to do with teleprompt?", choices)
        .with_default(&defaults)
        .with_page_size(GOALS.len())
        .with_help_message("space chooses, enter goes on")
        .prompt()
        .map_err(|e| format!("nothing chosen: {e}"))?;
    let names: Vec<String> = picked.iter().map(|c| c.goal.name.to_string()).collect();
    if names.is_empty() {
        return Ok((names, Vec::new()));
    }
    let tools = resolve(crate::cli::registry(), &names)?;
    Ok((names, tools))
}

/// What a goal still needs, as the list says it.
fn state(setup: &Setup, goal: &Goal) -> String {
    let gone = missing(setup, goal);
    if gone.is_empty() {
        return "installed".to_string();
    }
    // Programs by name; models, which nobody knows by name, by count.
    let (models, programs): (Vec<&Tool>, Vec<&Tool>) = gone
        .iter()
        .partition(|t| download_mb(crate::cli::registry(), t.name).is_some());
    let mut parts: Vec<String> = programs.iter().map(|t| t.name.to_string()).collect();
    match models.len() {
        0 => {}
        1 => parts.push("a model".to_string()),
        n => parts.push(format!("{n} models")),
    }
    match megabytes(&gone) {
        0 => format!("needs {}", parts.join(", ")),
        mb => format!("needs {} · {mb} MB", parts.join(", ")),
    }
}

fn missing(setup: &Setup, goal: &Goal) -> Vec<&'static Tool> {
    let names: Vec<String> = vec![goal.name.to_string()];
    resolve(crate::cli::registry(), &names)
        .unwrap_or_default()
        .into_iter()
        .filter(|t| setup.missing(t))
        .collect()
}

fn megabytes(tools: &[&Tool]) -> u32 {
    tools
        .iter()
        .filter_map(|t| download_mb(crate::cli::registry(), t.name))
        .sum()
}

/// Asks whether to install `tools`, those of them that are missing, saying
/// what each is and its license; and installs them if so. The commands it
/// ran.
pub fn confirm_install(
    setup: &Setup,
    tools: &[&'static Tool],
    reporter: &dyn teleprompt_core::Reporter,
) -> Result<Vec<String>, String> {
    let gone: Vec<&'static Tool> = tools
        .iter()
        .copied()
        .filter(|t| setup.missing(t) && setup.command(t).is_some())
        .collect();
    if gone.is_empty() {
        return Ok(Vec::new());
    }
    eprintln!();
    for t in &gone {
        eprintln!("  {:<18} {} ({})", t.name, t.what, t.license);
    }
    let size = match megabytes(&gone) {
        0 => String::new(),
        mb => format!(", about {mb} MB"),
    };
    let question = format!("Install {} of them{size}?", gone.len());
    let yes = Confirm::new(&question)
        .with_default(true)
        .with_help_message("each is under its own license; teleprompt ships none of them")
        .prompt()
        .unwrap_or(false);
    if yes {
        setup.install(&gone, reporter)
    } else {
        Ok(Vec::new())
    }
}

/// A tool as a question names it: a model by what it is, not its id.
fn spoken(name: &str) -> &str {
    match name {
        "speech-model" => "the speech model",
        "punctuation-model" => "the punctuation model",
        "speaker-model" => "the speaker models",
        other => other,
    }
}

/// When a command finds `names` missing: offers to install them `to` do
/// what they are for, and says whether they are there now. Asks nobody who
/// is not at a terminal.
pub fn offer(reporter: &dyn teleprompt_core::Reporter, names: &[&str], to: &str) -> bool {
    if !interactive() {
        return false;
    }
    let names: Vec<String> = names.iter().map(|n| (*n).to_string()).collect();
    let Ok(tools) = resolve(crate::cli::registry(), &names) else {
        return false;
    };
    let setup = Setup::detect(crate::cli::registry());
    let gone: Vec<&'static Tool> = tools
        .into_iter()
        .filter(|t| setup.missing(t) && setup.command(t).is_some())
        .collect();
    if gone.is_empty() {
        return false;
    }
    let shown: Vec<&str> = gone.iter().map(|t| spoken(t.name)).collect();
    let size = match megabytes(&gone) {
        0 => String::new(),
        mb => format!(" ({mb} MB)"),
    };
    let licenses: Vec<String> = gone
        .iter()
        .map(|t| format!("{}: {}", t.name, t.license))
        .collect();
    let question = format!("Install {}{size} to {to}?", shown.join(" and "));
    let yes = Confirm::new(&question)
        .with_default(true)
        .with_help_message(&licenses.join("; "))
        .prompt()
        .unwrap_or(false);
    if !yes {
        return false;
    }
    match setup.install(&gone, reporter) {
        Ok(_) => true,
        Err(e) => {
            eprintln!("{e}");
            false
        }
    }
}
