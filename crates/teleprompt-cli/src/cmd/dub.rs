use std::path::{Path, PathBuf};

use teleprompt_cache::VoiceCache;
use teleprompt_compile::manifest::{self, AudioInfo, NarrationManifest, MANIFEST_VERSION};
use teleprompt_compile::manifest_diff::{self, ManifestDiff};
use teleprompt_core::Hash;
use teleprompt_voice::VoiceRegistry;

use crate::cmd::check::{cache_root, compile_script_with};
use crate::project::Project;

/// The `AudioInfo` sample rate for a locale that rendered no audio at all.
///
/// Every populated manifest takes its rate from the audio actually
/// produced; this is only what the field says when `segments` is empty and
/// there is nothing to describe. It is not a claim about any backend.
const NO_AUDIO_SAMPLE_RATE: u32 = 48_000;

/// A segment the fallback ladder could not deliver at the tier the script
/// asked for. Computed here rather than in `main.rs` so the policy question
/// — is a downgrade fatal? — is the only thing left for the caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Downgrade {
    pub segment_id: String,
    pub requested: String,
    pub actual: String,
    pub reason: String,
}

pub struct DubOutput {
    pub manifest: NarrationManifest,
    pub written: Vec<PathBuf>,
    pub warnings: Vec<String>,
    /// `Some` only under `--check`. `None` means nothing was compared.
    pub drift: Option<ManifestDiff>,
    /// Every segment whose delivered voice tier is not the requested one,
    /// in manifest order. Always populated; `--strict-voice` decides
    /// whether it is fatal.
    pub downgrades: Vec<Downgrade>,
}

/// Reads the downgrades straight off the published manifest, so what
/// `--strict-voice` fails on is exactly what a consumer would read.
fn downgrades_in(manifest: &NarrationManifest) -> Vec<Downgrade> {
    manifest
        .segments
        .iter()
        .filter(|s| s.voice_source != s.voice_source_actual)
        .map(|s| Downgrade {
            segment_id: s.id.clone(),
            requested: s.voice_source.clone(),
            actual: s.voice_source_actual.clone(),
            reason: s
                .downgrade_reason
                .clone()
                .unwrap_or_else(|| "no reason recorded".to_string()),
        })
        .collect()
}

pub fn render_downgrades(downgrades: &[Downgrade]) -> String {
    let mut s = String::new();
    for d in downgrades {
        s.push_str(&format!(
            "  {} — asked for {}, got {}: {}\n",
            d.segment_id, d.requested, d.actual, d.reason
        ));
    }
    s
}

/// Distinguishes a script that fails validation (exit 2) from a runtime
/// failure such as an unreadable committed manifest or a write error (exit
/// 1). A flat `Vec<String>` cannot carry that distinction, so `run_dub`
/// returns this instead of reusing `compile_script`'s error type directly.
pub enum DubError {
    Validation(Vec<String>),
    Runtime(String),
}

/// Where this locale's self-contained directory lives.
fn locale_dir(out_root: &Path, locale: &str) -> PathBuf {
    out_root.join(locale)
}

pub fn manifest_path(out_root: &Path, locale: &str) -> PathBuf {
    locale_dir(out_root, locale).join("narration.json")
}

/// Read a committed manifest, refusing a version this build does not
/// understand rather than letting serde fill in defaults and produce a
/// confident, wrong diff.
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

/// The duration the manifest will publish for `segment_id`.
///
/// Read off the timeline's narration entry, because that is precisely what
/// `manifest::build` copies into `SegmentEntry::duration_ms`. Deriving it
/// again from the synth result would compare a value against itself and
/// catch nothing.
///
/// `None` means the timeline has no narration entry for this segment, in
/// which case `build` drops it and there is no published duration to
/// disagree with.
fn published_duration_ms(
    timeline: &teleprompt_schedule::Timeline,
    segment_id: &str,
) -> Option<u64> {
    timeline
        .entries
        .iter()
        .filter_map(|e| e.narration.as_ref())
        .find(|n| n.segment == segment_id)
        .map(|n| n.duration_ms)
}

/// `Some(message)` when a rendered segment is not the length the manifest
/// is about to publish for it.
///
/// This is the guard rail for the class of bug where the audio and the
/// number describing it come from two different places. It would have
/// caught `dub` rendering at the default speed while publishing a duration
/// measured at the script's configured speed, and it fires before anything
/// is written, so a mismatch never reaches an output directory.
///
/// The message names both values because "they differ" is not actionable —
/// which one is wrong is the whole question.
fn length_mismatch(segment_id: &str, actual_ms: u64, published_ms: u64) -> Option<String> {
    if actual_ms == published_ms {
        return None;
    }
    Some(format!(
        "segment `{segment_id}`: rendered audio is {actual_ms}ms but the manifest \
         publishes {published_ms}ms; a consumer placing this file at its stated \
         duration would clip or pad it"
    ))
}

pub async fn run_dub(
    project: &Project,
    script: &Path,
    locale: &str,
    out_root: &Path,
    check_only: bool,
) -> Result<DubOutput, DubError> {
    run_dub_with(
        &crate::voice::registry(),
        project,
        script,
        locale,
        out_root,
        check_only,
    )
    .await
}

/// [`run_dub`] against a caller-supplied registry. See
/// [`crate::cmd::check::compile_script_with`] for why the seam exists.
pub async fn run_dub_with(
    registry: &VoiceRegistry,
    project: &Project,
    script: &Path,
    locale: &str,
    out_root: &Path,
    check_only: bool,
) -> Result<DubOutput, DubError> {
    // The backend the script's config resolved to, not one chosen here.
    // `detail.cache_key` was computed from this backend's `id()` and
    // `capabilities().version`, so synthesizing with any other one writes
    // the wrong audio into the content-addressed cache under this one's key
    // — permanently, and reported as `measured` by every later `plan`.
    let (compiled, backend) =
        compile_script_with(registry, project, script, locale).map_err(DubError::Validation)?;

    // The same cache `compile_script` just read from, rooted the same way.
    // `compile` only ever looks a key up; `dub` is the one that fills a
    // miss in, by calling the backend and storing what comes back.
    let cache = VoiceCache::new(cache_root(project));

    // Audio is rendered into memory before anything is written to disk, so
    // a synthesis failure cannot leave a half-populated output directory
    // behind.
    let mut audio: Vec<(String, Vec<u8>)> = Vec::new();

    // Read off the audio actually produced rather than assumed from any one
    // backend. First segment wins, so the value is a function of document
    // order rather than of completion order — the manifest has one
    // `AudioInfo` for the whole locale, so a backend that varied its rate
    // per segment would need a wider manifest, not a different pick here.
    let mut audio_format: Option<(u32, u16)> = None;

    for detail in &compiled.narration {
        let hit = cache
            .lookup(&detail.cache_key)
            .map_err(|e| DubError::Runtime(format!("segment `{}`: {e}", detail.segment_id)))?;

        let (wav_bytes, rendered_ms) = match hit {
            Some(cached) => {
                audio_format.get_or_insert((cached.sample_rate, cached.channels));
                (cached.wav, cached.duration_ms)
            }
            None => {
                // The request `compile` measured, not one rebuilt here.
                // Rebuilding it dropped `voice` and `speed`, so a script
                // with `voice: { speed: 2.0 }` published a duration from
                // the resolved config and a file rendered at the default.
                // One source of truth.
                let synthesized = backend
                    .synthesize(&detail.synth_request)
                    .await
                    .map_err(|e| {
                        DubError::Runtime(format!("segment `{}`: {e}", detail.segment_id))
                    })?;
                let stored = cache
                    .store(
                        &detail.cache_key,
                        &synthesized.pcm,
                        synthesized.word_timings.as_deref(),
                    )
                    .map_err(|e| {
                        DubError::Runtime(format!("segment `{}`: {e}", detail.segment_id))
                    })?;
                audio_format.get_or_insert((stored.sample_rate, stored.channels));
                (stored.wav, stored.duration_ms)
            }
        };

        // The guard rail for the above. The manifest publishes the
        // *timeline's* duration for this segment, so that is what the file
        // has to be — read it from the timeline rather than re-deriving it,
        // or the check would only ever compare a value against itself.
        // Checked before anything is written: an output directory that
        // disagrees with its own manifest is worse than no output at all.
        if let Some(published_ms) = published_duration_ms(&compiled.timeline, &detail.segment_id) {
            length_mismatch(&detail.segment_id, rendered_ms, published_ms)
                .map_or(Ok(()), |m| Err(DubError::Runtime(m)))?;
        }

        audio.push((detail.segment_id.clone(), wav_bytes));
    }

    // Every segment is warm now — the loop above either found it already
    // cached or just stored it. Recompile against that warm cache rather
    // than building the manifest from `compiled` above: that first compile
    // ran before anything was rendered, so on a cold project it published
    // `estimated` durations even though real audio was about to exist a few
    // lines later. Recompiling here is what makes `dub` idempotent — two
    // runs on an unedited script see an equally warm cache and produce
    // byte-identical manifests — and it costs one extra compile and no
    // synthesis, since by this point every lookup is a hit.
    let (compiled, _) =
        compile_script_with(registry, project, script, locale).map_err(DubError::Validation)?;

    let (sample_rate, channels) = audio_format.unwrap_or((NO_AUDIO_SAMPLE_RATE, 1));
    let mut built = manifest::build(
        &compiled.timeline,
        &compiled.chapters,
        &compiled.narration,
        AudioInfo {
            format: "wav".to_string(),
            sample_rate,
            channels,
        },
    );

    // `manifest::build` seeds `audio_hash` with the timeline's value, which
    // is the backend's synthesis *cache key* — it embeds the teleprompt
    // version, so every release changed every segment's hash and `--check`
    // reported "audio changed" on every segment of every consumer's next
    // pull request. Spec §5.1 documents this field as hashing the rendered
    // bytes, so publish the rendered bytes' hash. The `Timeline`'s own
    // `audio_hash` is left alone: a synthesis cache key is the right thing
    // there.
    //
    // Done before the `--check` branch, not only on the write path, so a
    // comparison is always like-for-like.
    for (segment_id, bytes) in &audio {
        if let Some(seg) = built.segments.iter_mut().find(|s| s.id == *segment_id) {
            seg.audio_hash = Hash::of(bytes);
        }
    }

    let downgrades = downgrades_in(&built);

    if check_only {
        let committed =
            read_committed(&manifest_path(out_root, locale)).map_err(DubError::Runtime)?;
        let drift = match committed {
            Some(before) => manifest_diff::diff(&before, &built),
            // No manifest at all is maximal drift: everything is new.
            None => manifest_diff::diff(
                &NarrationManifest {
                    segments: Vec::new(),
                    chapters: Vec::new(),
                    duration_ms: 0,
                    ..built.clone()
                },
                &built,
            ),
        };
        return Ok(DubOutput {
            manifest: built,
            written: Vec::new(),
            warnings: compiled.warnings,
            drift: Some(drift),
            downgrades,
        });
    }

    let dir = locale_dir(out_root, locale);
    let audio_dir = dir.join("audio");
    std::fs::create_dir_all(&audio_dir)
        .map_err(|e| DubError::Runtime(format!("cannot create {}: {e}", audio_dir.display())))?;

    let mut written = Vec::new();
    for (segment_id, bytes) in &audio {
        let path = dir.join(manifest::audio_path(segment_id, "wav"));
        std::fs::write(&path, bytes)
            .map_err(|e| DubError::Runtime(format!("cannot write {}: {e}", path.display())))?;
        written.push(path);
    }

    let path = manifest_path(out_root, locale);
    let json = serde_json::to_string_pretty(&built)
        .map_err(|e| DubError::Runtime(format!("cannot serialize manifest: {e}")))?;
    std::fs::write(&path, format!("{json}\n"))
        .map_err(|e| DubError::Runtime(format!("cannot write {}: {e}", path.display())))?;
    written.push(path);

    Ok(DubOutput {
        manifest: built,
        written,
        warnings: compiled.warnings,
        drift: None,
        downgrades,
    })
}

pub fn render_dub(out: &DubOutput) -> String {
    let mut s = format!(
        "{} ({}) — {} segment(s), {:.1}s\n",
        out.manifest.script,
        out.manifest.locale,
        out.manifest.segments.len(),
        out.manifest.duration_ms as f64 / 1000.0,
    );
    for path in &out.written {
        s.push_str(&format!("  wrote {}\n", path.display()));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use teleprompt_compile::manifest::SegmentEntry;
    use teleprompt_core::Hash;

    #[test]
    fn matching_lengths_pass_the_guard() {
        assert_eq!(length_mismatch("welcome", 3250, 3250), None);
    }

    /// The exact shape of C1: a manifest publishing 3250ms beside a file of
    /// 6500ms. The guard has to name the segment and both numbers, because
    /// which of the two is wrong is the whole diagnosis.
    #[test]
    fn a_mismatch_names_the_segment_and_both_lengths() {
        let msg = length_mismatch("welcome", 6500, 3250).expect("must be caught");
        assert!(msg.contains("welcome"), "{msg}");
        assert!(msg.contains("6500ms"), "{msg}");
        assert!(msg.contains("3250ms"), "{msg}");
    }

    fn segment(id: &str, requested: &str, actual: &str, reason: Option<&str>) -> SegmentEntry {
        SegmentEntry {
            id: id.to_string(),
            text: String::new(),
            chapter: "a".to_string(),
            start_ms: 0,
            duration_ms: 0,
            duration_source: "measured".to_string(),
            audio: String::new(),
            voice_source: requested.to_string(),
            voice_source_actual: actual.to_string(),
            downgrade_reason: reason.map(str::to_string),
            source_hash: Hash::of(b""),
            audio_hash: Hash::of(b""),
            words: None,
        }
    }

    fn manifest_with(segments: Vec<SegmentEntry>) -> NarrationManifest {
        NarrationManifest {
            manifest_version: MANIFEST_VERSION,
            script: "s.md".to_string(),
            locale: "en".to_string(),
            generated_by: "teleprompt test".to_string(),
            duration_ms: 0,
            audio: AudioInfo {
                format: "wav".to_string(),
                sample_rate: 48_000,
                channels: 1,
            },
            chapters: Vec::new(),
            segments,
        }
    }

    #[test]
    fn a_segment_delivered_at_the_requested_tier_is_not_a_downgrade() {
        let m = manifest_with(vec![segment("a", "synthetic", "synthetic", None)]);
        assert!(downgrades_in(&m).is_empty());
    }

    #[test]
    fn a_downgrade_carries_both_tiers_and_the_reason() {
        let m = manifest_with(vec![
            segment("a", "synthetic", "synthetic", None),
            segment("b", "recorded", "synthetic", Some("no takes recorded")),
        ]);
        assert_eq!(
            downgrades_in(&m),
            vec![Downgrade {
                segment_id: "b".to_string(),
                requested: "recorded".to_string(),
                actual: "synthetic".to_string(),
                reason: "no takes recorded".to_string(),
            }]
        );
        let rendered = render_downgrades(&downgrades_in(&m));
        assert!(rendered.contains("b"), "{rendered}");
        assert!(rendered.contains("no takes recorded"), "{rendered}");
    }
}
