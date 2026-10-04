//! `teleprompt voice clone`: a Voicebox voice from the author's own takes,
//! so the lines not recorded are spoken in the voice of those that are.
//! Each take goes with the words it says, which cloning needs to be
//! faithful; the takes are clean recordings of known text, which is what
//! it asks for.

use serde::Serialize;
use teleprompt_voice::takes::Takes;
use teleprompt_voice_voicebox::Sample;

use crate::project::Project;

/// Shorter takes teach a voice little.
const SHORTEST_MS: u64 = 1_500;
/// Enough to clone from; more only makes the upload longer.
const MOST: usize = 6;

#[derive(Debug, Serialize)]
pub struct CloneReport {
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
             To speak in it, in teleprompt.toml:\n\n  [voice]\n  backend = \"voicebox\"\n  voice = \"{}\"\n",
            self.voice,
            self.takes.len(),
            self.seconds,
            self.takes.join(", "),
            self.voice
        )
    }
}

/// Clones `name` on the project's Voicebox server from its takes.
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
            Ok(Sample {
                file: format!("{id}.wav"),
                wav: takes.read(id).map_err(|e| e.to_string())?,
                text: (*text).to_string(),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let backends = project.backends();
    let voicebox = backends
        .voicebox("voicebox")
        .ok_or_else(|| "`[backends.voicebox]` in teleprompt.toml does not validate".to_string())?;
    let profile = voicebox
        .clone_voice(name, language, &samples)
        .await
        .map_err(|e| e.to_string())?;
    Ok(CloneReport {
        voice: profile.name,
        id: profile.id,
        takes: usable.iter().map(|(id, _, _)| (*id).to_string()).collect(),
        seconds: usable.iter().map(|(_, ms, _)| *ms).sum::<u64>() as f64 / 1000.0,
    })
}
