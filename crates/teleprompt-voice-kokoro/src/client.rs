use base64::Engine;
use teleprompt_voice::{Pcm, VoiceError, WordTiming};

use crate::config::{KokoroConfig, KOKORO_SAMPLE_RATE};

pub struct Client {
    http: reqwest::Client,
    cfg: KokoroConfig,
}

impl Client {
    pub fn new(cfg: KokoroConfig) -> Result<Self, String> {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_millis(cfg.timeout_ms))
            .build()
            .map_err(|e| format!("cannot build HTTP client: {e}"))?;
        Ok(Self { http, cfg })
    }

    pub fn config(&self) -> &KokoroConfig {
        &self.cfg
    }

    /// Every failure here is fatal to the command. Spec §7.1: a broken
    /// synthesizer is not a voice tier, so there is no fallback to silence
    /// anywhere in this file.
    fn fail(&self, what: &str) -> VoiceError {
        VoiceError::Other(format!("kokoro at {}: {what}", self.cfg.base_url))
    }

    pub async fn speech(
        &self,
        text: &str,
        voice: Option<&str>,
        speed: f64,
    ) -> Result<Pcm, VoiceError> {
        let url = format!("{}/v1/audio/speech", self.cfg.base_url);
        let mut body = serde_json::json!({
            "model": self.cfg.model,
            "input": text,
            "response_format": "pcm",
            "speed": speed,
        });
        if let Some(v) = voice {
            body["voice"] = serde_json::Value::String(v.to_string());
        }

        let resp = self.http.post(&url).json(&body).send().await.map_err(|e| {
            if e.is_timeout() {
                self.fail(&format!("no response within {}ms", self.cfg.timeout_ms))
            } else {
                self.fail(&format!("request failed: {e}"))
            }
        })?;

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
                self.fail(&format!("response body incomplete: {e}"))
            }
        })?;

        decode_pcm(&bytes).map_err(|what| self.fail(&what))
    }

    /// Speech and when each word of it is said, from
    /// `/dev/captioned_speech`. Punctuation comes back as words of its own
    /// and is dropped: a cue names words.
    pub async fn captioned(
        &self,
        text: &str,
        voice: Option<&str>,
        speed: f64,
    ) -> Result<(Pcm, Vec<WordTiming>), VoiceError> {
        let url = format!("{}/dev/captioned_speech", self.cfg.base_url);
        let mut body = serde_json::json!({
            "model": self.cfg.model,
            "input": text,
            "response_format": "pcm",
            "speed": speed,
            "stream": false,
            "return_timestamps": true,
        });
        if let Some(v) = voice {
            body["voice"] = serde_json::Value::String(v.to_string());
        }
        let resp = self.http.post(&url).json(&body).send().await.map_err(|e| {
            if e.is_timeout() {
                self.fail(&format!("no response within {}ms", self.cfg.timeout_ms))
            } else {
                self.fail(&format!("request failed: {e}"))
            }
        })?;
        let status = resp.status();
        if status.as_u16() == 404 {
            return Err(self.fail(
                "has no /dev/captioned_speech, so it cannot time words; \
                 set `backends.kokoro.word_timings = false`",
            ));
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
        let pcm = decode_pcm(&bytes).map_err(|what| self.fail(&what))?;
        Ok((pcm, words(&v["timestamps"])))
    }

    pub async fn voices(&self) -> Result<Vec<String>, VoiceError> {
        let url = format!("{}/v1/audio/voices", self.cfg.base_url);
        let resp = self.http.get(&url).send().await.map_err(|e| {
            if e.is_timeout() {
                self.fail(&format!("no response within {}ms", self.cfg.timeout_ms))
            } else {
                self.fail(&format!("cannot list voices: {e}"))
            }
        })?;
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
        Ok(names)
    }
}

/// Word timings from a captioned response, in milliseconds, without the
/// punctuation Kokoro times as words of its own.
pub fn words(timestamps: &serde_json::Value) -> Vec<WordTiming> {
    let ms = |v: &serde_json::Value| (v.as_f64().unwrap_or(0.0) * 1000.0).round() as u64;
    timestamps
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|t| {
                    let word = t["word"].as_str()?;
                    word.chars().any(char::is_alphanumeric).then(|| WordTiming {
                        word: word.to_string(),
                        start_ms: ms(&t["start_time"]),
                        end_ms: ms(&t["end_time"]),
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

/// `response_format: "pcm"` is raw little-endian 16-bit mono at 24 kHz. No
/// mp3 or wav decoder enters the workspace.
///
/// Both rejections below matter: an odd byte count means the body is not
/// what it claims, and an empty body would become a zero-length WAV
/// published as a real line. Neither is recoverable by guessing.
fn decode_pcm(bytes: &[u8]) -> Result<Pcm, String> {
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
        sample_rate: KOKORO_SAMPLE_RATE,
        channels: 1,
        samples,
    })
}
