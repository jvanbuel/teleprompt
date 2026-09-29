use base64::Engine;
use serde_json::{json, Value};
use teleprompt_voice::{with_causes, Pcm, VoiceError};

use crate::config::GeminiConfig;

/// A unary request's audio is a WAV at this rate; raw PCM that states no
/// rate is at it too.
const RATE: u32 = 24_000;

pub struct Client {
    http: reqwest::Client,
    cfg: GeminiConfig,
}

impl Client {
    pub fn new(cfg: GeminiConfig) -> Result<Self, String> {
        let http = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_millis(cfg.timeout_ms))
            .build()
            .map_err(|e| format!("cannot build HTTP client: {e}"))?;
        Ok(Self { http, cfg })
    }

    pub fn config(&self) -> &GeminiConfig {
        &self.cfg
    }

    pub fn fail(&self, what: &str) -> VoiceError {
        VoiceError::Other(format!("gemini ({}): {what}", self.cfg.model))
    }

    fn key(&self) -> Result<String, VoiceError> {
        let var = &self.cfg.api_key_env;
        std::env::var(var)
            .ok()
            .filter(|k| !k.is_empty())
            .ok_or_else(|| {
                self.fail(&format!(
                    "needs an API key in {var}: make one at aistudio.google.com/apikey"
                ))
            })
    }

    /// `text` in `voice`, delivered as `style` says. The text is spoken
    /// verbatim, so direction goes beside it, never in it.
    pub async fn speak(
        &self,
        text: &str,
        voice: &str,
        style: Option<&str>,
    ) -> Result<Pcm, VoiceError> {
        let mut item = json!({ "type": "text", "text": text });
        if let Some(style) = style {
            item["annotations"] = json!([{ "type": "speech_metadata", "style": style }]);
        }
        let mut generation = json!({ "speech_config": [{ "voice": voice }] });
        if let Some(seed) = self.cfg.seed {
            generation["seed"] = json!(seed);
        }
        let body = json!({
            "model": self.cfg.model,
            "input": [item],
            "response_format": { "type": "audio" },
            "generation_config": generation,
        });
        let url = format!("{}/v1beta/interactions", self.cfg.base_url);
        let reply = self
            .http
            .post(&url)
            .header("x-goog-api-key", self.key()?)
            .json(&body)
            .send()
            .await
            .map_err(|e| self.unsent(&e))?;
        let status = reply.status();
        let answer: Value = reply.json().await.map_err(|e| {
            if e.is_timeout() {
                self.unsent(&e)
            } else {
                self.fail(&format!("answered {status} with no JSON: {e}"))
            }
        })?;
        if !status.is_success() {
            return Err(self.refused(status.as_u16(), &answer));
        }
        self.audio(&answer)
    }

    fn unsent(&self, e: &reqwest::Error) -> VoiceError {
        if e.is_timeout() {
            self.fail(&format!(
                "no answer within {} ms: raise `timeout_ms` under [backends.gemini]",
                self.cfg.timeout_ms
            ))
        } else {
            self.fail(&format!(
                "cannot reach {}: {}",
                self.cfg.base_url,
                with_causes(e)
            ))
        }
    }

    fn refused(&self, status: u16, answer: &Value) -> VoiceError {
        let why = answer["error"]["message"]
            .as_str()
            .unwrap_or("no reason given");
        let help = if status == 429 {
            ": too many requests for the key's quota; lower `concurrency` under \
             [backends.gemini], or wait"
        } else {
            ""
        };
        self.fail(&format!("refused ({status}): {why}{help}"))
    }

    /// The audio in the interaction's last model output, as the SDKs find it.
    fn audio(&self, answer: &Value) -> Result<Pcm, VoiceError> {
        let steps = answer["steps"].as_array().map_or(&[][..], Vec::as_slice);
        let outputs = steps
            .iter()
            .rev()
            .take_while(|s| s["type"] != "user_input")
            .filter(|s| s["type"] == "model_output");
        let contents: Vec<&Value> = outputs
            .flat_map(|s| s["content"].as_array().into_iter().flatten().rev())
            .collect();
        let Some(audio) = contents
            .iter()
            .find(|c| c["type"] == "audio")
            .copied()
            .or_else(|| Some(&answer["output_audio"]).filter(|a| a.is_object()))
        else {
            let said: Vec<&str> = contents.iter().filter_map(|c| c["text"].as_str()).collect();
            return Err(self.fail(&format!("answered with no audio: {}", said.join(" "))));
        };
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(audio["data"].as_str().unwrap_or_default())
            .map_err(|e| self.fail(&format!("its audio was not base64: {e}")))?;
        decode(&bytes, audio).map_err(|e| self.fail(&e))
    }
}

/// A WAV as it is, or raw 16-bit PCM at the rate the answer states.
/// Gemini's raw PCM is little-endian, whatever `audio/l16` says elsewhere.
fn decode(bytes: &[u8], audio: &Value) -> Result<Pcm, String> {
    if bytes.starts_with(b"RIFF") {
        return teleprompt_voice::wav::decode(bytes).map_err(|e| format!("its audio: {e}"));
    }
    let kind = audio["mime_type"].as_str().unwrap_or("audio/l16");
    if !kind.starts_with("audio/l16") && !kind.starts_with("audio/pcm") {
        return Err(format!("its audio is {kind}, not WAV or PCM"));
    }
    let number = |key: &str, default: u32| {
        audio[key]
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .unwrap_or(default)
    };
    Ok(Pcm {
        sample_rate: number("sample_rate", RATE),
        channels: u16::try_from(number("channels", 1)).unwrap_or(1),
        samples: bytes
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]))
            .collect(),
    })
}
