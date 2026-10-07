use std::sync::Mutex;

use base64::Engine;
use serde_json::{json, Value};
use teleprompt_core::sync::lock;
use teleprompt_voice::{with_causes, Pcm, VoiceError, WordTiming};

use super::config::ElevenLabsConfig;

/// The rate of the raw PCM asked for: 16-bit mono.
const RATE: u32 = 24_000;

/// A voice: its id, which the API takes, and the name it is shown by.
#[derive(Debug, Clone)]
pub struct Voice {
    pub id: String,
    pub name: String,
}

pub struct Client {
    http: reqwest::Client,
    cfg: ElevenLabsConfig,
    /// The account's voices, once listed, to find one by its name.
    voices: Mutex<Option<Vec<Voice>>>,
}

impl Client {
    pub fn new(cfg: ElevenLabsConfig) -> Result<Self, String> {
        let http = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_millis(cfg.timeout_ms))
            .build()
            .map_err(|e| format!("cannot build HTTP client: {e}"))?;
        Ok(Self {
            http,
            cfg,
            voices: Mutex::new(None),
        })
    }

    pub fn config(&self) -> &ElevenLabsConfig {
        &self.cfg
    }

    pub fn fail(&self, what: &str) -> VoiceError {
        VoiceError::Other(format!("elevenlabs ({}): {what}", self.cfg.model))
    }

    fn key(&self) -> Result<String, VoiceError> {
        let var = &self.cfg.api_key_env;
        std::env::var(var)
            .ok()
            .filter(|k| !k.is_empty())
            .ok_or_else(|| {
                self.fail(&format!(
                    "needs an API key in {var}: make one at elevenlabs.io/app/settings/api-keys"
                ))
            })
    }

    /// `text` in the voice `voice_id`, with when each word is said.
    pub async fn speak(
        &self,
        text: &str,
        voice_id: &str,
        speed: f64,
    ) -> Result<(Pcm, Vec<WordTiming>), VoiceError> {
        let mut settings = json!({ "speed": speed });
        for (name, value) in [
            ("stability", self.cfg.stability),
            ("similarity_boost", self.cfg.similarity_boost),
            ("style", self.cfg.style),
        ] {
            if let Some(value) = value {
                settings[name] = json!(value);
            }
        }
        let mut body = json!({
            "text": text,
            "model_id": self.cfg.model,
            "voice_settings": settings,
        });
        if let Some(seed) = self.cfg.seed {
            body["seed"] = json!(seed);
        }
        let url = format!(
            "{}/v1/text-to-speech/{voice_id}/with-timestamps?output_format=pcm_{RATE}",
            self.cfg.base_url
        );
        let answer = self.send(self.http.post(&url).json(&body)).await?;
        let audio = base64::engine::general_purpose::STANDARD
            .decode(answer["audio_base64"].as_str().unwrap_or_default())
            .map_err(|e| self.fail(&format!("its audio was not base64: {e}")))?;
        let pcm = Pcm::from_le_bytes(&audio, RATE, 1).map_err(|e| self.fail(&e))?;
        Ok((pcm, words(&answer["alignment"])))
    }

    /// The account's voices, premade and its own: listed once.
    pub async fn voices(&self) -> Result<Vec<Voice>, VoiceError> {
        if let Some(listed) = lock(&self.voices).clone() {
            return Ok(listed);
        }
        let url = format!("{}/v1/voices", self.cfg.base_url);
        let answer = self.send(self.http.get(&url)).await?;
        let listed: Vec<Voice> = answer["voices"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| {
                Some(Voice {
                    id: v["voice_id"].as_str()?.to_string(),
                    name: v["name"].as_str().unwrap_or_default().to_string(),
                })
            })
            .collect();
        *lock(&self.voices) = Some(listed.clone());
        Ok(listed)
    }

    async fn send(&self, request: reqwest::RequestBuilder) -> Result<Value, VoiceError> {
        let reply = request
            .header("xi-api-key", self.key()?)
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
        Ok(answer)
    }

    fn unsent(&self, e: &reqwest::Error) -> VoiceError {
        if e.is_timeout() {
            self.fail(&format!(
                "no answer within {} ms: raise `timeout_ms` under [backends.elevenlabs]",
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
        // `detail` is a message, or an object holding one.
        let detail = &answer["detail"];
        let why = detail
            .as_str()
            .or_else(|| detail["message"].as_str())
            .unwrap_or("no reason given");
        let help = match status {
            401 => ": the key was not accepted",
            429 => {
                ": too many lines at once for the plan; lower `concurrency` under \
                   [backends.elevenlabs], or wait"
            }
            _ => "",
        };
        self.fail(&format!("refused ({status}): {why}{help}"))
    }
}

/// Characters and when each is said, as words: the runs between spaces
/// that hold a letter or a digit.
fn words(alignment: &Value) -> Vec<WordTiming> {
    let chars = alignment["characters"].as_array();
    let starts = alignment["character_start_times_seconds"].as_array();
    let ends = alignment["character_end_times_seconds"].as_array();
    let (Some(chars), Some(starts), Some(ends)) = (chars, starts, ends) else {
        return Vec::new();
    };
    let ms = |v: &Value| v.as_f64().map(|s| (s * 1000.0).round().max(0.0) as u64);
    let mut out = Vec::new();
    let mut current: Option<WordTiming> = None;
    for ((c, start), end) in chars.iter().zip(starts).zip(ends) {
        let c = c.as_str().unwrap_or(" ");
        if c.trim().is_empty() {
            out.extend(current.take());
            continue;
        }
        let (Some(start), Some(end)) = (ms(start), ms(end)) else {
            continue;
        };
        match current.as_mut() {
            Some(w) => {
                w.word.push_str(c);
                w.end_ms = end;
            }
            None => {
                current = Some(WordTiming {
                    word: c.to_string(),
                    start_ms: start,
                    end_ms: end,
                })
            }
        }
    }
    out.extend(current);
    out.retain(|w| w.word.chars().any(char::is_alphanumeric));
    out
}
