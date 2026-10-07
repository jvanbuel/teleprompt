//! Where core, scene, voice and schedule meet (docs/design.md#crates):
//! walks a resolved [`Program`], has the scene plugins validate and split action
//! blocks, takes each line's duration from the voice cache or a
//! [`WpmEstimator`], pairs lines with the shots that follow them, and
//! schedules the items into a [`Timeline`]. It never reaches a voice
//! backend; see [`VoiceContext`].

use std::collections::BTreeMap;
use std::path::Path;

use crate::schedule::{schedule, Timeline};
use teleprompt_core::{Diagnostics, LineId, ShotId};
use teleprompt_scene::{Measured, ScenePlugins};
use teleprompt_script::config::{Config, OutputConfig, SceneConfig};
use teleprompt_script::program::{ChapterInfo, Program};
use teleprompt_voice::cache::{CacheKey, VoiceCache};
use teleprompt_voice::takes::{TakeMeta, Takes};
use teleprompt_voice::WpmEstimator;
use teleprompt_voice::{SynthRequest, WordTiming};

mod capture_key;
mod cue;
pub mod length;
pub mod manifest;
mod retime;
mod walker;

use capture_key::chain_capture_keys;
pub use capture_key::CAPTURE_RECIPE;
pub use cue::word_offset_ms;
use retime::retime_stretched_shots;
use walker::Walker;

/// Everything `compile` needs about voice, and deliberately no backend, so
/// the inner loop cannot synthesize (docs/design.md#async-boundary).
pub struct VoiceContext<'a> {
    pub backend_id: &'a str,
    pub backend_version: &'a str,
    /// The version of each other backend a line may pick, by id, for a
    /// cast whose speakers use several.
    pub other_backends: BTreeMap<String, String>,
    pub cache: &'a VoiceCache,
    pub estimator: &'a WpmEstimator,
    /// Recorded takes; a current one stands in for synthesis.
    pub takes: &'a Takes,
}

/// Per-line detail the manifest needs and the [`Timeline`], a review
/// surface, deliberately leaves out: the prose and word timings.
#[derive(Debug, Clone)]
pub struct NarrationDetail {
    pub line_id: LineId,
    pub text: String,
    /// The chapter's slug, for display. Two chapters with the same title
    /// share one, so it is not a join key.
    pub chapter: String,
    /// The chapter's position in [`CompileOutput::chapters`]: the join key.
    pub chapter_index: usize,
    /// The request the timeline's duration came from. `dub` synthesizes
    /// from this rather than building its own, so the published duration
    /// and the audio file cannot disagree.
    pub synth_request: SynthRequest,
    /// The key `synth_request` was looked up under, and that `dub` stores
    /// its audio under.
    pub cache_key: CacheKey,
    pub word_timings: Option<Vec<WordTiming>>,
    /// The recorded take the line is spoken from, when it has a current
    /// one; `synth_request` and `cache_key` then go unused.
    pub take: Option<TakeMeta>,
    /// The voice backend that speaks it, by id.
    pub backend: String,
    /// Who says it, from the cast; the narrator when `None`.
    pub speaker: Option<String>,
    /// Who says it, as the screen names them: their voice's `name`, or a
    /// speaker's key title-cased; `None` for an unnamed narrator.
    pub name: Option<String>,
}

impl VoiceContext<'_> {
    /// The version of backend `id`, if this compile has it.
    pub(crate) fn version_of(&self, id: &str) -> Option<&str> {
        if id == self.backend_id {
            Some(self.backend_version)
        } else {
            self.other_backends.get(id).map(String::as_str)
        }
    }
}

/// One action shot's source, as the plugin split it (and re-timed it), for
/// whatever draws the scene. Not in the manifest: it is scene plugin-native code.
#[derive(Debug, Clone)]
pub struct ShotSource {
    pub scene: String,
    pub plugin: String,
    pub source: String,
    /// How long its plugin says it lasts.
    pub length: Measured,
}

#[derive(Debug)]
pub struct CompileOutput {
    pub timeline: Timeline,
    pub warnings: Vec<String>,
    /// One entry per narration line, in document order.
    pub narration: Vec<NarrationDetail>,
    /// Every action shot's source, by its id.
    pub shots: BTreeMap<ShotId, ShotSource>,
    // `chapters`, `output` and `scenes` are carried because `compile` drops
    // the `Program` and callers have no other route to them. `output` and
    // `scenes` stay out of the manifest: they are not timing facts.
    /// The script's chapters, in document order.
    pub chapters: Vec<ChapterInfo>,
    /// The frame the script asked for, resolved through every layer.
    pub output: OutputConfig,
    /// The scenes as configured, resolved through every layer.
    pub scenes: BTreeMap<String, SceneConfig>,
}

/// The scene name a pause wears, which is not a scene and has no picture.
const PAUSE_SCENE: &str = "pause";

/// Compiles `program` into a scheduled [`Timeline`].
///
/// Deterministic: the same inputs produce byte-identical timeline JSON.
/// Durations come from the voice cache on a hit and the estimator on a
/// miss (docs/design.md#estimated-and-measured).
pub fn compile(
    program: &Program,
    scenes: &ScenePlugins,
    voice_ctx: &VoiceContext,
    base_dir: &Path,
    version: &str,
) -> Result<CompileOutput, Diagnostics> {
    let walker = Walker::walk(program, scenes, voice_ctx, base_dir);

    let d = Diagnostics(walker.diags);
    if d.has_errors() {
        return Err(d);
    }

    let mut shot_sources = walker.shots;
    let (mut timeline, scheduling_warnings) = schedule(
        &walker.items,
        &program.script_name,
        &program.locale,
        version,
    );
    let cut_warnings = retime_stretched_shots(&mut timeline, &mut shot_sources, scenes);
    chain_capture_keys(&mut timeline, &program.config, scenes, &shot_sources);
    // Cache warnings first: they explain the scheduler's numbers.
    let mut warnings = walker.warnings;
    warnings.extend(scheduling_warnings);
    warnings.extend(cut_warnings);
    if let Some(length) = program.config.timing.length_ms {
        warnings.extend(length::over_length(
            &timeline,
            &walker.narration,
            &program.chapters,
            length.ms(),
        ));
    }
    let scenes = scenes_shown(&timeline, &program.config);
    Ok(CompileOutput {
        timeline,
        warnings,
        narration: walker.narration,
        shots: shot_sources,
        scenes,
        chapters: program.chapters.clone(),
        output: program.config.output.clone(),
    })
}

/// The scenes as configured, and every scene a block names that none
/// configures, with its plugin's defaults: what capture opens.
fn scenes_shown(timeline: &Timeline, config: &Config) -> BTreeMap<String, SceneConfig> {
    let mut scenes = config.scenes.clone();
    for action in timeline.entries.iter().filter_map(|e| e.action.as_ref()) {
        if action.scene != PAUSE_SCENE {
            scenes
                .entry(action.scene.clone())
                .or_insert_with(|| SceneConfig {
                    plugin: action.plugin.clone(),
                    settings: BTreeMap::new(),
                    root: config.root.clone(),
                });
        }
    }
    scenes
}
