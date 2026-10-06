//! Who speaks when: sherpa-onnx's offline speaker diarization, a pyannote
//! segmentation model and a speaker embedding model, clustered.

use std::path::{Path, PathBuf};

use crate::SpeakerSpan;
use sherpa_onnx::{OfflineSpeakerDiarization, OfflineSpeakerDiarizationConfig};

/// The voices in `samples` (mono, 16 kHz) and when each speaks, by the
/// models in `dir`: a `sherpa-onnx-pyannote-*` directory beside
/// `embedding.onnx`, as `teleprompt setup speaker-model` lays them out.
/// `speakers` is how many there are, where known; otherwise the voices are
/// told apart by how alike they sound.
pub fn diarize(
    dir: &Path,
    samples: &[f32],
    speakers: Option<usize>,
) -> Result<Vec<SpeakerSpan>, String> {
    let mut config = OfflineSpeakerDiarizationConfig::default();
    config.segmentation.pyannote.model = Some(segmentation(dir)?);
    config.segmentation.num_threads = 2;
    config.embedding.model = Some(file(dir.join("embedding.onnx"))?);
    config.embedding.num_threads = 2;
    if let Some(n) = speakers {
        config.clustering.num_clusters = i32::try_from(n).unwrap_or(i32::MAX);
    }
    let diarizer = OfflineSpeakerDiarization::create(&config)
        .ok_or_else(|| format!("the speaker models in {} did not load", dir.display()))?;
    let result = diarizer
        .process(samples)
        .ok_or("the speaker models could not tell the voices apart")?;
    let ms = |s: f32| (f64::from(s) * 1000.0).round().max(0.0) as u64;
    Ok(result
        .sort_by_start_time()
        .into_iter()
        .map(|s| SpeakerSpan {
            start_ms: ms(s.start),
            end_ms: ms(s.end),
            speaker: usize::try_from(s.speaker).unwrap_or(0),
        })
        .collect())
}

/// The segmentation model, its int8 variant where there is one.
fn segmentation(dir: &Path) -> Result<String, String> {
    let model_dir = std::fs::read_dir(dir)
        .map_err(|e| format!("cannot read {}: {e}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .find(|p| {
            p.is_dir()
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.contains("pyannote"))
        })
        .ok_or_else(|| format!("no pyannote segmentation model in {}", dir.display()))?;
    file(model_dir.join("model.int8.onnx")).or_else(|_| file(model_dir.join("model.onnx")))
}

fn file(path: PathBuf) -> Result<String, String> {
    if path.is_file() {
        Ok(path.to_string_lossy().into_owned())
    } else {
        Err(format!("{} is missing", path.display()))
    }
}
