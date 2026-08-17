use teleprompt_voice::{ErrorKind, Pcm, VoiceError};

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

    /// No failure here falls back to silence. Spec §7.1: a broken
    /// synthesizer is not a voice tier. What a failure *does* get is a
    /// classification, so `dub` can tell a server that is briefly busy from
    /// one that will never answer this request.
    ///
    /// The base URL rides in `detail` rather than being prefixed onto every
    /// message by hand: `VoiceError`'s `Display` already writes `kokoro:`,
    /// and a machine may be running several servers, so which one refused is
    /// still worth naming.
    fn fail(&self, kind: ErrorKind, what: &str) -> VoiceError {
        VoiceError::new("kokoro", kind, format!("{} — {what}", self.cfg.base_url))
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
                self.fail(
                    ErrorKind::Transient,
                    &format!("no response within {}ms", self.cfg.timeout_ms),
                )
            } else {
                self.fail(ErrorKind::Transient, &format!("request failed: {e}"))
            }
        })?;

        let status = resp.status();
        if !status.is_success() {
            // Read before `text()` consumes the response.
            let retry_after = retry_after_of(&resp);
            let detail = resp.text().await.unwrap_or_default();
            let detail = detail.trim();
            let tail = if detail.is_empty() {
                String::new()
            } else {
                format!(" — {}", truncate(detail, 200))
            };
            let mut err = self.fail(
                classify(status),
                &format!("returned {}{tail}", status.as_u16()),
            );
            if let Some(after) = retry_after {
                err = err.with_retry_after(after);
            }
            return Err(err);
        }

        let bytes = resp.bytes().await.map_err(|e| {
            if e.is_timeout() {
                self.fail(
                    ErrorKind::Transient,
                    &format!("no response within {}ms", self.cfg.timeout_ms),
                )
            } else {
                self.fail(
                    ErrorKind::Transient,
                    &format!("response body incomplete: {e}"),
                )
            }
        })?;

        // A 2xx whose bytes are not what `response_format: "pcm"` promised.
        decode_pcm(&bytes).map_err(|what| self.fail(ErrorKind::Protocol, &what))
    }

    pub async fn voices(&self) -> Result<Vec<String>, VoiceError> {
        let url = format!("{}/v1/audio/voices", self.cfg.base_url);
        let resp = self.http.get(&url).send().await.map_err(|e| {
            if e.is_timeout() {
                self.fail(
                    ErrorKind::Transient,
                    &format!("no response within {}ms", self.cfg.timeout_ms),
                )
            } else {
                self.fail(ErrorKind::Transient, &format!("cannot list voices: {e}"))
            }
        })?;
        if !resp.status().is_success() {
            let status = resp.status();
            return Err(self.fail(
                classify(status),
                &format!("listing voices returned {}", status.as_u16()),
            ));
        }
        let v: serde_json::Value = resp.json().await.map_err(|e| {
            self.fail(
                ErrorKind::Protocol,
                &format!("voice list was not JSON: {e}"),
            )
        })?;

        // Kokoro-FastAPI answers `{"voices": [...]}`; some OpenAI-compatible
        // servers answer a bare array. Accept both rather than making the
        // author debug a shape mismatch.
        let arr = v
            .get("voices")
            .and_then(|x| x.as_array())
            .or_else(|| v.as_array())
            .ok_or_else(|| self.fail(ErrorKind::Protocol, "voice list had no `voices` array"))?;

        // A non-string entry means the server's shape is not what we
        // parsed above; skipping it silently would hide that a real
        // protocol change happened and hand back a list that looks fine.
        // Same reasoning as `decode_pcm`'s rejection of an odd byte count.
        //
        // Compatibility note for whoever revisits this: as of this writing
        // there is an open, unmerged upstream proposal for Kokoro-FastAPI to
        // return `{id, name}` objects here instead of bare strings, for
        // OpenAI-client compatibility. Today's server returns bare strings,
        // which is what this loop parses and what the error below rejects
        // as "not a string" if it lands. If it does land, `voices()` needs
        // to accept an object with an `id` (or `name`) field alongside the
        // bare-string case — check upstream's release notes before assuming
        // this rejection is still correct.
        let mut names = Vec::with_capacity(arr.len());
        for item in arr {
            match item.as_str() {
                Some(name) => names.push(name.to_string()),
                None => {
                    return Err(self.fail(
                        ErrorKind::Protocol,
                        &format!("voice list contained a non-string entry: {item}"),
                    ));
                }
            }
        }
        Ok(names)
    }
}

/// Kokoro-FastAPI is a local model server: it has no credentials and no
/// billing, so `Auth` and `Quota` are unreachable here. Anything that is
/// neither a rate limit nor a server fault is the request's own problem.
///
/// The wildcard is deliberately the non-retryable answer. A status this
/// function has never seen is not something to hammer a server with.
fn classify(status: reqwest::StatusCode) -> ErrorKind {
    match status.as_u16() {
        429 => ErrorKind::RateLimited,
        500..=599 => ErrorKind::Transient,
        _ => ErrorKind::InvalidRequest,
    }
}

/// `Retry-After` in its delta-seconds form. The HTTP-date form is legal too
/// and is ignored: without a clock-skew story a date is worse than the
/// caller's own backoff, which is what `None` falls back to.
fn retry_after_of(resp: &reqwest::Response) -> Option<std::time::Duration> {
    resp.headers()
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
        .map(std::time::Duration::from_secs)
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
