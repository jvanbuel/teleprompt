use std::path::{Path, PathBuf};

use publish::Published;
use teleprompt_core::{Progress, Reporter, Silent, SpanMs};
use teleprompt_manifest::diff::{self as manifest_diff, ManifestDiff};
use teleprompt_manifest::{audio_path, NarrationManifest, MANIFEST_VERSION};
use teleprompt_manifest::{captions, chapters};
use teleprompt_script::config::OutputConfig;

use crate::project::{Compiled, Script};
use crate::voicing::Lines;
use crate::Failure;

pub mod publish;

/// A script dubbed, in state `S`: [`Written`] under an output root, or
/// [`Compared`] with what is there. What only one state has is only in
/// that state, so nothing asks a check for the files it wrote, or
/// captures from a manifest that was never published.
///
/// ```
/// # use teleprompt_project::{capture::Scenes, dub::Dubber, project::Script};
/// # async fn f(script: &Script, scenes: &Scenes<'_>) {
/// let dubbed = Dubber::new(script).dub("out".as_ref()).await.unwrap();
/// scenes.capture(&dubbed, &mut |_| {});
/// # }
/// ```
///
/// A check publishes nothing, so there is nothing to capture from:
///
/// ```compile_fail
/// # use teleprompt_project::{capture::Scenes, dub::Dubber, project::Script};
/// # async fn f(script: &Script, scenes: &Scenes<'_>) {
/// let checked = Dubber::new(script).check("out".as_ref()).await.unwrap();
/// scenes.capture(&checked, &mut |_| {});
/// # }
/// ```
pub struct Dubbed<S> {
    pub manifest: NarrationManifest,
    /// Every shot's source, which a capture backend runs. The manifest
    /// names the shots; only this says what they do.
    pub shots: std::collections::BTreeMap<
        teleprompt_core::ShotId,
        teleprompt_pipeline::compile::ShotSource,
    >,
    /// The scenes as configured, which a capture backend opens.
    pub scenes: std::collections::BTreeMap<String, teleprompt_script::config::SceneConfig>,
    /// The script's `output:` block, which `build` needs and the manifest
    /// does not carry.
    pub output: OutputConfig,
    pub warnings: Vec<String>,
    pub state: S,
}

/// Published: the files written under the output root.
pub struct Written {
    pub files: Vec<PathBuf>,
}

/// Compared with the manifest already under the output root, writing
/// nothing there.
pub struct Compared {
    pub drift: ManifestDiff,
}

impl<S> Dubbed<S> {
    fn new(voiced: Voiced, state: S) -> Self {
        Dubbed {
            manifest: voiced.published.manifest,
            shots: voiced.compiled.shots,
            scenes: voiced.compiled.scenes,
            output: voiced.compiled.output,
            warnings: voiced.warnings,
            state,
        }
    }
}

fn locale_dir(out_root: &Path, locale: &str) -> PathBuf {
    out_root.join(locale)
}

pub fn manifest_path(out_root: &Path, locale: &str) -> PathBuf {
    locale_dir(out_root, locale).join("narration.json")
}

/// Refuses a `manifest_version` this build does not write, rather than
/// letting serde fill in defaults and produce a confident, wrong diff.
fn read_committed(path: &Path) -> Result<Option<NarrationManifest>, String> {
    let raw = match std::fs::read_to_string(path) {
        Ok(r) => r,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
    };
    let probe: serde_json::Value =
        serde_json::from_str(&raw).map_err(|e| format!("cannot parse {}: {e}", path.display()))?;
    let version = probe.get("manifest_version").and_then(|v| v.as_u64());
    if version != Some(MANIFEST_VERSION as u64) {
        return Err(format!(
            "{}: manifest_version {} is not supported (this build writes {})",
            path.display(),
            version
                .map(|v| v.to_string())
                .unwrap_or_else(|| "missing".into()),
            MANIFEST_VERSION,
        ));
    }
    serde_json::from_str(&raw)
        .map(Some)
        .map_err(|e| format!("cannot parse {}: {e}", path.display()))
}

/// Voices a script and publishes it under an output root:
/// `<root>/<locale>/narration.json` and its audio.
pub struct Dubber<'a> {
    script: &'a Script,
    reporter: &'a dyn Reporter,
}

impl<'a> Dubber<'a> {
    /// Reporting to nobody until [`reporting`](Self::reporting) says to whom.
    pub fn new(script: &'a Script) -> Self {
        Self {
            script,
            reporter: &Silent,
        }
    }

    /// Says each line's progress to `reporter`.
    #[must_use]
    pub fn reporting(self, reporter: &'a dyn Reporter) -> Self {
        Self { reporter, ..self }
    }

    /// Voices the script and writes it under `out_root`.
    pub async fn dub(&self, out_root: &Path) -> Result<Dubbed<Written>, Failure> {
        let voiced = voice(self.script, self.reporter).await?;
        let files = write_output(out_root, self.script.locale(), &voiced.published)?;
        Ok(Dubbed::new(voiced, Written { files }))
    }

    /// Voices the script and compares it with the manifest under
    /// `out_root`, writing nothing there.
    pub async fn check(&self, out_root: &Path) -> Result<Dubbed<Compared>, Failure> {
        let voiced = voice(self.script, self.reporter).await?;
        let drift =
            drift_from_committed(out_root, self.script.locale(), &voiced.published.manifest)?;
        Ok(Dubbed::new(voiced, Compared { drift }))
    }
}

/// A script voiced: every line's audio made or read from its take, and
/// the manifest that publishes it.
pub struct Voiced {
    pub compiled: teleprompt_pipeline::compile::CompileOutput,
    pub published: Published,
    pub warnings: Vec<String>,
}

/// Voices the script, what `dub` does short of writing it out, and what
/// the prompter plays.
pub async fn voice(script: &Script, reporter: &dyn Reporter) -> Result<Voiced, Failure> {
    let (project, backends) = (script.project(), script.backends());
    // Synthesize only with the backend the keys were computed from; see
    // `Script::compile`.
    let Compiled {
        output: compiled,
        cast,
        ..
    } = script.compile().map_err(Failure::Validation)?;
    let limit = backends.concurrency(cast.narrator().id());
    let audio = Lines::of(project, cast)?
        .all(&compiled.narration, limit, reporter)
        .await?;
    // The re-render notices come first: they explain why anything below them
    // is being recomputed at all.
    let mut warnings: Vec<String> = audio
        .iter()
        .filter_map(|a| {
            Some(format!(
                "line `{}`: {}",
                a.line_id,
                a.cache_warning.as_ref()?
            ))
        })
        .collect();

    // Recompile against the now-warm cache: the first compile ran before
    // anything was rendered, so on a cold project it holds estimates. This
    // makes `dub` idempotent, and costs no synthesis.
    let compiled = script.compile().map_err(Failure::Validation)?.output;
    let published = publish::publish(&compiled, audio)?;
    warnings.extend(compiled.warnings.iter().cloned());
    Ok(Voiced {
        compiled,
        published,
        warnings,
    })
}

/// How `built` differs from the manifest committed under `out_root`. No
/// manifest at all is maximal drift: everything is new.
fn drift_from_committed(
    out_root: &Path,
    locale: &str,
    built: &NarrationManifest,
) -> Result<ManifestDiff, Failure> {
    let committed = read_committed(&manifest_path(out_root, locale)).map_err(Failure::Runtime)?;
    Ok(match committed {
        Some(before) => manifest_diff::diff(&before, built),
        None => manifest_diff::diff(
            &NarrationManifest {
                lines: Vec::new(),
                chapters: Vec::new(),
                duration_ms: SpanMs::ZERO,
                ..built.clone()
            },
            built,
        ),
    })
}

/// Writes each line's WAV and then the manifest, returning the paths
/// written.
fn write_output(
    out_root: &Path,
    locale: &str,
    published: &publish::Published,
) -> Result<Vec<PathBuf>, Failure> {
    let built = &published.manifest;
    let dir = locale_dir(out_root, locale);
    let audio_dir = dir.join("audio");
    std::fs::create_dir_all(&audio_dir)
        .map_err(|e| Failure::Runtime(format!("cannot create {}: {e}", audio_dir.display())))?;

    let mut written = Vec::new();
    for (line_id, bytes) in &published.lines {
        let path = dir.join(audio_path(line_id, "wav"));
        std::fs::write(&path, bytes)
            .map_err(|e| Failure::Runtime(format!("cannot write {}: {e}", path.display())))?;
        written.push(path);
    }

    // Beside the manifest, so a player serving the audio finds them too,
    // and chapters to paste into a video's description.
    let cues = captions::cues(built);
    for (name, text) in [
        ("captions.srt", captions::srt(&cues)),
        ("captions.vtt", captions::vtt(&cues)),
        ("chapters.txt", chapters::youtube(built).0),
    ] {
        let path = dir.join(name);
        std::fs::write(&path, text)
            .map_err(|e| Failure::Runtime(format!("cannot write {}: {e}", path.display())))?;
        written.push(path);
    }

    let path = manifest_path(out_root, locale);
    let json = serde_json::to_string_pretty(built)
        .map_err(|e| Failure::Runtime(format!("cannot serialize manifest: {e}")))?;
    std::fs::write(&path, format!("{json}\n"))
        .map_err(|e| Failure::Runtime(format!("cannot write {}: {e}", path.display())))?;
    written.push(path);
    Ok(written)
}

pub fn render_dub(out: &Dubbed<Written>) -> String {
    let mut s = format!(
        "{} ({}) — {} line(s), {:.1}s\n",
        out.manifest.script,
        out.manifest.locale,
        out.manifest.lines.len(),
        out.manifest.duration_ms.ms() as f64 / 1000.0,
    );
    for path in &out.state.files {
        s.push_str(&format!("  wrote {}\n", path.display()));
    }
    s
}

/// `dub --clips`, once `script` is dubbed into `out_root`: records the
/// shots not yet captured as `capture` does, then puts every shot's clip in
/// `<locale>/clips/`. A shot nothing here can record is left out, with
/// capture's warning.
pub fn add_clips(
    script: &Script,
    out_root: &Path,
    dubbed: &mut Dubbed<Written>,
    reporter: &dyn Reporter,
) -> Result<(), Failure> {
    let clips = script.project().caches().clips();
    let frame = crate::build::FrameOverride::default().frame(&dubbed.output);
    let scenes = crate::capture::Scenes::new(script.project().registry.scenes, &clips, frame);
    let captured = scenes.capture(dubbed, &mut |p| reporter.progress(captured_shot(p)));
    dubbed.warnings.extend(captured.warnings);
    let into = locale_dir(out_root, script.locale()).join("clips");
    let placed = clips
        .place_into(&dubbed.manifest, &into)
        .map_err(|e| Failure::Runtime(format!("{}: {e}", into.display())))?;
    dubbed.state.files.extend(placed);
    Ok(())
}

/// A shot's capture progress, as a [`Reporter`] takes it.
pub fn captured_shot(p: teleprompt_scene::capture::Progress) -> Progress {
    Progress::Capture {
        done: p.done,
        of: p.of,
        scene: p.scene,
        shot: p.shot,
    }
}
