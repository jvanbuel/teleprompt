use base64::Engine;
use teleprompt_plugin::voice::with_causes;
use teleprompt_plugin::voice::{Pcm, VoiceError, WordTiming};

use crate::config::{OpenAiConfig, Preset};

pub struct Client {
    http: reqwest::Client,
    cfg: OpenAiConfig,
}

impl Client {
    pub fn new(cfg: OpenAiConfig) -> Result<Self, String> {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_millis(cfg.timeout_ms))
            .build()
            .map_err(|e| format!("cannot build HTTP client: {e}"))?;
        Ok(Self { http, cfg })
    }

    pub fn config(&self) -> &OpenAiConfig {
        &self.cfg
    }

    /// Every failure here is fatal to the command
    /// (docs/design.md#backend-failure): a broken synthesizer is not a voice
    /// tier, so there is no fallback to silence anywhere in this file.
    fn fail(&self, what: &str) -> VoiceError {
        VoiceError::Other(format!("{} at {}: {what}", self.cfg.id, self.cfg.base_url))
    }

    /// The key, read when it is needed and not before: `check` and `plan`
    /// never ask for it, and a project that names the variable works on a
    /// machine that has not set it until it speaks.
    fn key(&self) -> Result<Option<String>, VoiceError> {
        let Some(var) = &self.cfg.api_key_env else {
            return Ok(None);
        };
        match std::env::var(var) {
            Ok(k) if !k.trim().is_empty() => Ok(Some(k.trim().to_string())),
            _ => Err(self.fail(&format!(
                "needs an API key in the environment variable `{var}`, which is not set"
            ))),
        }
    }

    async fn send(
        &self,
        req: reqwest::RequestBuilder,
        what: &str,
    ) -> Result<reqwest::Response, VoiceError> {
        let req = match self.key()? {
            Some(k) => req.bearer_auth(k),
            None => req,
        };
        req.send().await.map_err(|e| {
            if e.is_timeout() {
                self.fail(&format!("no response within {}ms", self.cfg.timeout_ms))
            } else {
                self.fail(&format!("{what}: {}", with_causes(&e)))
            }
        })
    }

    /// The request body every speech call shares.
    fn body(
        &self,
        text: &str,
        voice: Option<&str>,
        speed: f64,
        instruct: Option<&str>,
    ) -> serde_json::Value {
        let mut body = serde_json::json!({
            "model": self.cfg.model,
            "input": text,
            "response_format": "pcm",
            "speed": speed,
        });
        if let Some(v) = voice.or(self.cfg.voice.as_deref()) {
            body["voice"] = serde_json::Value::String(v.to_string());
        }
        if let Some(i) = instruct {
            body["instructions"] = serde_json::Value::String(i.to_string());
        }
        body
    }

    pub async fn speech(
        &self,
        text: &str,
        voice: Option<&str>,
        speed: f64,
        instruct: Option<&str>,
    ) -> Result<Pcm, VoiceError> {
        let url = format!("{}/audio/speech", self.cfg.api_root());
        let body = self.body(text, voice, speed, instruct);
        let resp = self
            .send(self.http.post(&url).json(&body), "request failed")
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let detail = resp.text().await.unwrap_or_default();
            let detail = detail.trim();
            let tail = if detail.is_empty() {
                String::new()
            } else {
                format!(" — {}", truncate(detail, 200))
            };
            return Err(self.fail(&format!("returned {}{tail}", status.as_u16())));
        }

        let bytes = resp.bytes().await.map_err(|e| {
            if e.is_timeout() {
                self.fail(&format!("no response within {}ms", self.cfg.timeout_ms))
            } else {
                self.fail(&format!("response body incomplete: {}", with_causes(&e)))
            }
        })?;

        self.decode_pcm(&bytes).map_err(|what| self.fail(&what))
    }

    /// Speech and when each word of it is said, from
    /// `/dev/captioned_speech`. Punctuation comes back as words of its own
    /// and is dropped: a cue names words.
    pub async fn captioned(
        &self,
        text: &str,
        voice: Option<&str>,
        speed: f64,
        instruct: Option<&str>,
    ) -> Result<(Pcm, Vec<WordTiming>), VoiceError> {
        let url = format!("{}/dev/captioned_speech", self.cfg.server_root());
        let mut body = self.body(text, voice, speed, instruct);
        body["stream"] = false.into();
        body["return_timestamps"] = true.into();
        let resp = self
            .send(self.http.post(&url).json(&body), "request failed")
            .await?;
        let status = resp.status();
        if status.as_u16() == 404 {
            return Err(self.fail(&format!(
                "has no /dev/captioned_speech, so it cannot time words; \
                 set `backends.{}.word_timings = false`",
                self.cfg.id
            )));
        }
        if !status.is_success() {
            return Err(self.fail(&format!("captioned speech returned {}", status.as_u16())));
        }
        let v: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| self.fail(&format!("captioned speech was not JSON: {e}")))?;
        let audio = v["audio"]
            .as_str()
            .ok_or_else(|| self.fail("captioned speech had no `audio`"))?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(audio)
            .map_err(|e| self.fail(&format!("captioned audio was not base64: {e}")))?;
        let pcm = self.decode_pcm(&bytes).map_err(|what| self.fail(&what))?;
        Ok((pcm, words(&v["timestamps"])))
    }

    /// The voices the server lists, or `None` where it lists none: OpenAI's
    /// API has no such call, and another server need not have Kokoro's.
    pub async fn voices(&self) -> Result<Option<Vec<String>>, VoiceError> {
        if self.cfg.preset == Preset::OpenAi {
            return Ok(None);
        }
        let url = format!("{}/audio/voices", self.cfg.api_root());
        let resp = self.send(self.http.get(&url), "cannot list voices").await?;
        if resp.status().as_u16() == 404 && self.cfg.preset == Preset::Endpoint {
            return Ok(None);
        }
        if !resp.status().is_success() {
            return Err(self.fail(&format!(
                "listing voices returned {}",
                resp.status().as_u16()
            )));
        }
        let v: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| self.fail(&format!("voice list was not JSON: {e}")))?;

        // Kokoro-FastAPI answers `{"voices": [...]}`; some OpenAI-compatible
        // servers answer a bare array. Accept both rather than making the
        // author debug a shape mismatch.
        let arr = v
            .get("voices")
            .and_then(|x| x.as_array())
            .or_else(|| v.as_array())
            .ok_or_else(|| self.fail("voice list had no `voices` array"))?;

        // Two entry shapes are voices: a bare string, which Kokoro-FastAPI
        // answered through 0.2.x, and an object with an `id`, which it
        // answers since the list became OpenAI-compatible —
        // `{"id": "af_heart", "name": "af_heart", "overall_grade": …}`.
        // `id` is what `/v1/audio/speech` takes as `voice`; `name` is read
        // only where there is no `id`.
        //
        // Anything else is rejected rather than skipped: it means the
        // server's shape changed again, and silently dropping it would hand
        // back a list that looks fine. Same reasoning as `decode_pcm`'s
        // rejection of an odd byte count.
        let mut names = Vec::with_capacity(arr.len());
        for item in arr {
            let name = item.as_str().or_else(|| {
                item.get("id")
                    .or_else(|| item.get("name"))
                    .and_then(|v| v.as_str())
            });
            match name {
                Some(name) => names.push(name.to_string()),
                None => {
                    return Err(self.fail(&format!(
                        "voice list contained an entry that is neither a name nor an \
                         object with an `id`: {item}"
                    )));
                }
            }
        }
        Ok(Some(names))
    }

    /// Whether the server answers, and accepts the key: its models, which
    /// every server of the API lists, where it cannot list voices.
    pub async fn reachable(&self) -> Result<(), VoiceError> {
        let url = format!("{}/models", self.cfg.api_root());
        let resp = self.send(self.http.get(&url), "cannot reach it").await?;
        match resp.status().as_u16() {
            401 | 403 => Err(self.fail(&format!("refused the key ({})", resp.status().as_u16()))),
            // Any other answer is a server that is up; one without a model
            // list is still one that can speak.
            _ => Ok(()),
        }
    }

    /// `response_format: "pcm"` is raw little-endian 16-bit mono, at 24 kHz
    /// unless `sample_rate` says otherwise. No mp3 or wav decoder enters the
    /// workspace.
    ///
    /// Both rejections below matter: an odd byte count means the body is
    /// not what it claims, and an empty body would become a zero-length WAV
    /// published as a real line. Neither is recoverable by guessing.
    fn decode_pcm(&self, bytes: &[u8]) -> Result<Pcm, String> {
        if bytes.is_empty() {
            return Err("returned an empty audio body".to_string());
        }
        if bytes.len() % 2 != 0 {
            return Err(format!(
                "returned {} bytes, an odd count for 16-bit samples",
                bytes.len()
            ));
        }
        let samples = bytes
            .chunks_exact(2)
            .map(|p| i16::from_le_bytes([p[0], p[1]]))
            .collect();
        Ok(Pcm {
            sample_rate: self.cfg.sample_rate,
            channels: 1,
            samples,
        })
    }
}

/// Word timings from a captioned response, in milliseconds, without the
/// punctuation Kokoro times as words of its own, and without any word it
/// gave no times for: an untimed word is interpolated by the caller.
pub fn words(timestamps: &serde_json::Value) -> Vec<WordTiming> {
    let ms = |v: &serde_json::Value| v.as_f64().map(|s| (s * 1000.0).round() as u64);
    timestamps
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|t| {
                    let word = t["word"].as_str()?;
                    word.chars().any(char::is_alphanumeric).then_some(())?;
                    Some(WordTiming {
                        word: word.to_string(),
                        start_ms: ms(&t["start_time"])?,
                        end_ms: ms(&t["end_time"])?,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.to_string();
    }
    s.chars().take(n).collect::<String>() + "…"
}
