use std::path::{Path, PathBuf};

use sherpa_onnx::{OnlineRecognizer, OnlineRecognizerConfig, OnlineStream};
use teleprompt_listen::{Heard, Recognizer};

/// The rate the streaming zipformer models are trained at; audio is
/// resampled to it before it gets here.
pub const SAMPLE_RATE: u32 = 16_000;

/// A streaming transducer model (a sherpa-onnx "streaming zipformer")
/// listening to one reader.
pub struct SherpaRecognizer {
    recognizer: OnlineRecognizer,
    stream: OnlineStream,
}

impl SherpaRecognizer {
    /// Loads the model in `dir`: its encoder, decoder and joiner (the int8
    /// variants where present) and `tokens.txt`.
    pub fn new(dir: &Path) -> Result<Self, String> {
        let mut config = OnlineRecognizerConfig::default();
        let model = &mut config.model_config;
        model.transducer.encoder = Some(model_file(dir, "encoder")?);
        model.transducer.decoder = Some(model_file(dir, "decoder")?);
        model.transducer.joiner = Some(model_file(dir, "joiner")?);
        model.tokens = Some(existing(dir.join("tokens.txt"))?);
        model.num_threads = 2;
        config.decoding_method = Some("greedy_search".to_string());
        // An utterance ends at a pause, so the follower can commit it.
        config.enable_endpoint = true;
        config.rule1_min_trailing_silence = 2.4;
        config.rule2_min_trailing_silence = 1.2;
        config.rule3_min_utterance_length = 20.0;
        let recognizer = OnlineRecognizer::create(&config)
            .ok_or_else(|| format!("the model in {} did not load", dir.display()))?;
        let stream = recognizer.create_stream();
        Ok(Self { recognizer, stream })
    }
}

impl Recognizer for SherpaRecognizer {
    fn listen(&mut self, samples: &[f32]) -> Heard {
        self.stream.accept_waveform(SAMPLE_RATE as i32, samples);
        while self.recognizer.is_ready(&self.stream) {
            self.recognizer.decode(&self.stream);
        }
        let text = self
            .recognizer
            .get_result(&self.stream)
            .map(|r| r.text)
            .unwrap_or_default();
        let is_final = self.recognizer.is_endpoint(&self.stream);
        if is_final {
            self.recognizer.reset(&self.stream);
        }
        Heard { text, is_final }
    }
}

/// The model's `<part>-*.onnx`, preferring the int8 variant: a quarter of
/// the size, and fast enough to follow speech on a laptop CPU.
fn model_file(dir: &Path, part: &str) -> Result<String, String> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| format!("cannot read the model directory {}: {e}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            name.starts_with(part) && name.ends_with(".onnx")
        })
        .collect();
    found.sort_by_key(|p| !p.to_string_lossy().ends_with(".int8.onnx"));
    let path = found
        .into_iter()
        .next()
        .ok_or_else(|| format!("no {part}*.onnx in {}", dir.display()))?;
    Ok(path.to_string_lossy().into_owned())
}

fn existing(path: PathBuf) -> Result<String, String> {
    if path.is_file() {
        Ok(path.to_string_lossy().into_owned())
    } else {
        Err(format!("{} is missing", path.display()))
    }
}
