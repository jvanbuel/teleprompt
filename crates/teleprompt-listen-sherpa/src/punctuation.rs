//! sherpa-onnx's punctuation model: capitals and punctuation for a
//! transcript the recognizer wrote without either.

use std::path::Path;

use sherpa_onnx::{OnlinePunctuation, OnlinePunctuationConfig, OnlinePunctuationModelConfig};

/// `text` punctuated by the model unpacked in `dir` (sherpa-onnx's
/// `online-punct-en`, with `model.int8.onnx` or `model.onnx`, and
/// `bpe.vocab`).
pub fn punctuate(dir: &Path, text: &str) -> Result<String, String> {
    let model = ["model.int8.onnx", "model.onnx"]
        .iter()
        .map(|f| dir.join(f))
        .find(|p| p.is_file())
        .ok_or_else(|| format!("no punctuation model (model.onnx) in {}", dir.display()))?;
    let vocab = dir.join("bpe.vocab");
    if !vocab.is_file() {
        return Err(format!("no bpe.vocab in {}", dir.display()));
    }
    let punctuator = OnlinePunctuation::create(&OnlinePunctuationConfig {
        model: OnlinePunctuationModelConfig {
            cnn_bilstm: Some(model.display().to_string()),
            bpe_vocab: Some(vocab.display().to_string()),
            ..Default::default()
        },
    })
    .ok_or_else(|| format!("cannot load the punctuation model in {}", dir.display()))?;
    punctuator
        .add_punctuation(&text.to_lowercase())
        .ok_or_else(|| "the punctuation model failed".to_string())
}
