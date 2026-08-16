use teleprompt_voice::{Pcm, VoiceError};

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

    pub async fn voices(&self) -> Result<Vec<String>, VoiceError> {
        let url = format!("{}/v1/audio/voices", self.cfg.base_url);
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| self.fail(&format!("cannot list voices: {e}")))?;
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

        // A non-string entry means the server's shape is not what we
        // parsed above; skipping it silently would hide that a real
        // protocol change happened and hand back a list that looks fine.
        // Same reasoning as `decode_pcm`'s rejection of an odd byte count.
        let mut names = Vec::with_capacity(arr.len());
        for item in arr {
            match item.as_str() {
                Some(name) => names.push(name.to_string()),
                None => {
                    return Err(
                        self.fail(&format!("voice list contained a non-string entry: {item}"))
                    );
                }
            }
        }
        Ok(names)
    }
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
/// published as a real segment. Neither is recoverable by guessing.
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
