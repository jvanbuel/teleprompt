use std::path::{Path, PathBuf};

use crate::{Heard, Recognizer, TimedWord};
use sherpa_onnx::{OnlineRecognizer, OnlineRecognizerConfig, OnlineStream, RecognizerResult};

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

    fn reset(&mut self) {
        self.stream = self.recognizer.create_stream();
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

/// Every word in a recording, with when it was said: the whole of
/// `samples` (mono, [`SAMPLE_RATE`]) run through the model in `dir`.
pub fn transcribe(dir: &Path, samples: &[f32]) -> Result<Vec<TimedWord>, String> {
    let SherpaRecognizer { recognizer, stream } = SherpaRecognizer::new(dir)?;
    let mut words = Vec::new();
    // Where the utterance under way began, in samples, for a model that
    // does not say: timestamps count from the last reset.
    let mut offset = 0;
    let mut fed = 0;
    // Silence after the end, so the model hears the last word out.
    let tail = vec![0.0; SAMPLE_RATE as usize];
    for chunk in samples.chunks(SAMPLE_RATE as usize / 10).chain([&tail[..]]) {
        stream.accept_waveform(SAMPLE_RATE as i32, chunk);
        fed += chunk.len();
        while recognizer.is_ready(&stream) {
            recognizer.decode(&stream);
        }
        if recognizer.is_endpoint(&stream) {
            if let Some(r) = recognizer.get_result(&stream) {
                words.extend(timed(&r, offset));
            }
            recognizer.reset(&stream);
            offset = fed;
        }
    }
    stream.input_finished();
    while recognizer.is_ready(&stream) {
        recognizer.decode(&stream);
    }
    if let Some(r) = recognizer.get_result(&stream) {
        words.extend(timed(&r, offset));
    }
    Ok(words)
}

/// How long a word's last token is taken to last: a token is stamped where
/// the model emitted it, near its start.
const LAST_TOKEN_MS: u64 = 300;

/// Tokens into words: a token starting with a space starts a word, which
/// ends where its last token does, or where the next word starts.
fn timed(r: &RecognizerResult, offset: usize) -> Vec<TimedWord> {
    let Some(stamps) = &r.timestamps else {
        return Vec::new();
    };
    // The model's own start for the utterance: where it had decoded to at
    // the reset, which trails what had been fed.
    let base = r
        .start_time
        .map_or(offset as u64 * 1000 / u64::from(SAMPLE_RATE), |s| {
            (f64::from(s) * 1000.0).round() as u64
        });
    let mut words: Vec<TimedWord> = Vec::new();
    for (token, &at) in r.tokens.iter().zip(stamps) {
        let ms = base + (f64::from(at) * 1000.0).round() as u64;
        let piece = token.trim_start_matches([' ', '\u{2581}']);
        let starts = token.starts_with([' ', '\u{2581}']) || words.is_empty();
        match words.last_mut() {
            Some(word) if !starts => {
                word.text.push_str(piece);
                word.end_ms = ms + LAST_TOKEN_MS;
            }
            _ => {
                if let Some(prev) = words.last_mut() {
                    prev.end_ms = prev.end_ms.min(ms);
                }
                words.push(TimedWord {
                    text: piece.to_string(),
                    start_ms: ms,
                    end_ms: ms + LAST_TOKEN_MS,
                });
            }
        }
    }
    words.retain(|w| !w.text.is_empty());
    words
}
