//! `teleprompt voice clone`: a voice from the author's own takes, made by
//! a backend that clones voices (Voicebox, in this build),
//! so the lines not recorded are spoken in the voice of those that are.
//! Each take goes with the words it says, which cloning needs to be
//! faithful; the takes are clean recordings of known text, which is what
//! it asks for.

use serde::Serialize;
use teleprompt_voice::takes::Takes;
use teleprompt_voice::VoiceSample;

use teleprompt_project::project::Project;

/// Shorter takes teach a voice little.
const SHORTEST_MS: u64 = 1_500;
/// Enough to clone from; more only makes the upload longer.
const MOST: usize = 6;

#[derive(Debug, Serialize)]
pub struct CloneReport {
    /// The backend that made it, which `voice.backend` names.
    pub backend: String,
    pub voice: String,
    pub id: String,
    /// The takes it was made from, longest first.
    pub takes: Vec<String>,
    pub seconds: f64,
}

impl CloneReport {
    pub fn render(&self) -> String {
        format!(
            "made voice `{}` from {} takes ({:.0} s): {}\n\n\
             To speak in it, in teleprompt.toml:\n\n  [voice]\n  backend = \"{}\"\n  voice = \"{}\"\n",
            self.voice,
            self.takes.len(),
            self.seconds,
            self.takes.join(", "),
            self.backend,
            self.voice
        )
    }
}

/// Clones `name` from the project's takes.
pub async fn run_clone(
    project: &Project,
    name: &str,
    language: &str,
) -> Result<CloneReport, String> {
    let takes = Takes::load(&project.takes_dir()).map_err(|e| e.to_string())?;
    let mut usable: Vec<(&str, u64, &str)> = takes
        .iter()
        .filter(|(_, t)| t.duration_ms >= SHORTEST_MS)
        .map(|(id, t)| (id, t.duration_ms, t.text.as_str()))
        .collect();
    if usable.is_empty() {
        return Err(format!(
            "no takes of {} s or longer to clone a voice from: record some lines first, \
             with `teleprompt serve` or the app",
            SHORTEST_MS / 1000
        ));
    }
    usable.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    usable.truncate(MOST);
    let samples = usable
        .iter()
        .map(|(id, _, text)| {
            Ok(VoiceSample {
                file: format!("{id}.wav"),
                wav: takes.read(id).map_err(|e| e.to_string())?,
                text: (*text).to_string(),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let (backend, profile) = project
        .backends()
        .clone_voice(name, language, &samples)
        .await?;
    Ok(CloneReport {
        backend,
        voice: profile.name,
        id: profile.id,
        takes: usable.iter().map(|(id, _, _)| (*id).to_string()).collect(),
        seconds: usable.iter().map(|(_, ms, _)| *ms).sum::<u64>() as f64 / 1000.0,
    })
}

/// `voice`'s arguments: what to do with a voice.
#[derive(clap::Args)]
pub struct Args {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(clap::Subcommand)]
pub enum Command {
    /// Make a voice from your takes, so the lines you have not recorded
    /// are spoken in your voice. Clone only your own voice, or one you
    /// have permission to
    Clone {
        /// What to call it: `voice.voice` names it
        name: String,
        /// The language of your takes
        #[arg(long, default_value = "en")]
        language: String,
    },
}

pub fn run(args: Args, format: crate::output::Format) -> crate::cli::Run {
    let Command::Clone { name, language } = args.command;
    let project = crate::cli::project_here()?;
    let report = crate::cli::runtime()?
        .block_on(run_clone(&project, &name, &language))
        .map_err(crate::cli::runtime_failure)?;
    crate::cli::emit(format, &report, &report.render());
    Ok(crate::output::Outcome::Ok)
}
