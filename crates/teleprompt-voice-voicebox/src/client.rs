use std::collections::BTreeMap;
use std::sync::Mutex;
use teleprompt_voice::with_causes;

use serde::Deserialize;
use teleprompt_voice::{Pcm, VoiceError, VoiceSample as Sample};

use crate::config::VoiceboxConfig;

/// A voice on the server.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Profile {
    pub id: String,
    pub name: String,
}

pub struct Client {
    http: reqwest::Client,
    cfg: VoiceboxConfig,
    /// Names and ids to ids, once listed: the profiles do not change
    /// during a dub.
    ids: Mutex<Option<BTreeMap<String, String>>>,
}

impl Client {
    pub fn new(cfg: VoiceboxConfig) -> Result<Self, String> {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_millis(cfg.timeout_ms))
            .build()
            .map_err(|e| format!("cannot build HTTP client: {e}"))?;
        Ok(Self {
            http,
            cfg,
            ids: Mutex::new(None),
        })
    }

    pub fn config(&self) -> &VoiceboxConfig {
        &self.cfg
    }

    /// Fatal to the command, as every backend failure is
    /// (docs/design.md#backend-failure).
    pub fn fail(&self, what: &str) -> VoiceError {
        VoiceError::Other(format!("voicebox at {}: {what}", self.cfg.base_url))
    }

    async fn send(
        &self,
        request: reqwest::RequestBuilder,
        what: &str,
    ) -> Result<reqwest::Response, VoiceError> {
        let resp = request.send().await.map_err(|e| {
            if e.is_timeout() {
                self.fail(&format!(
                    "no answer to {what} within {}ms",
                    self.cfg.timeout_ms
                ))
            } else {
                self.fail(&format!("{what} failed: {}", with_causes(&e)))
            }
        })?;
        let status = resp.status();
        if status.is_success() {
            return Ok(resp);
        }
        let detail = resp.text().await.unwrap_or_default();
        let detail: String = detail.trim().chars().take(200).collect();
        let tail = if detail.is_empty() {
            String::new()
        } else {
            format!(" — {detail}")
        };
        Err(self.fail(&format!("{what} returned {}{tail}", status.as_u16())))
    }

    pub async fn profiles(&self) -> Result<Vec<Profile>, VoiceError> {
        let resp = self
            .send(
                self.http.get(format!("{}/profiles", self.cfg.base_url)),
                "listing voices",
            )
            .await?;
        resp.json()
            .await
            .map_err(|e| self.fail(&format!("its voices were not the JSON expected: {e}")))
    }

    /// The id of the profile `voice` names, by id or by name, ignoring case.
    pub async fn profile_id(&self, voice: &str) -> Result<String, VoiceError> {
        if self.ids.lock().expect("ids lock").is_none() {
            let listed = self.profiles().await?;
            let mut ids = BTreeMap::new();
            for p in listed {
                ids.insert(p.name.to_lowercase(), p.id.clone());
                ids.insert(p.id.to_lowercase(), p.id);
            }
            *self.ids.lock().expect("ids lock") = Some(ids);
        }
        let found = {
            let ids = self.ids.lock().expect("ids lock");
            ids.as_ref()
                .and_then(|ids| ids.get(&voice.to_lowercase()).cloned())
        };
        if let Some(id) = found {
            return Ok(id);
        }
        let names: Vec<String> = self.profiles().await?.into_iter().map(|p| p.name).collect();
        Err(self.fail(&format!(
            "has no voice `{voice}`; it has {}",
            if names.is_empty() {
                "none".to_string()
            } else {
                names.join(", ")
            }
        )))
    }

    /// A cloned voice named `name`, from `samples`: a profile, then each
    /// sample with the text it says, which cloning needs to be faithful.
    /// A name already taken is refused before anything is made.
    pub async fn clone_voice(
        &self,
        name: &str,
        language: &str,
        samples: &[Sample],
    ) -> Result<Profile, VoiceError> {
        let existing = self.profiles().await?;
        if let Some(p) = existing.iter().find(|p| p.name.eq_ignore_ascii_case(name)) {
            return Err(self.fail(&format!(
                "already has a voice named `{}`: choose another name",
                p.name
            )));
        }
        let body = serde_json::json!({
            "name": name,
            "language": language,
            "voice_type": "cloned",
            "default_engine": self.cfg.engine,
        });
        let url = format!("{}/profiles", self.cfg.base_url);
        let profile: Profile = self
            .send(self.http.post(url).json(&body), "making a voice")
            .await?
            .json()
            .await
            .map_err(|e| self.fail(&format!("the new voice was not the JSON expected: {e}")))?;
        for sample in samples {
            let part = reqwest::multipart::Part::bytes(sample.wav.clone())
                .file_name(sample.file.clone())
                .mime_str("audio/wav")
                .map_err(|e| self.fail(&e.to_string()))?;
            let form = reqwest::multipart::Form::new()
                .part("file", part)
                .text("reference_text", sample.text.clone());
            let url = format!("{}/profiles/{}/samples", self.cfg.base_url, profile.id);
            self.send(
                self.http.post(url).multipart(form),
                &format!("adding {}", sample.file),
            )
            .await?;
        }
        *self.ids.lock().expect("ids lock") = None;
        Ok(profile)
    }

    /// One line, in the profile `profile_id`, as WAV, from the route that
    /// answers with the audio rather than a job to poll.
    pub async fn speak(
        &self,
        profile_id: &str,
        text: &str,
        language: &str,
        instruct: Option<&str>,
    ) -> Result<Pcm, VoiceError> {
        let mut body = serde_json::json!({
            "profile_id": profile_id,
            "text": text,
            "language": language,
            "engine": self.cfg.engine,
            "seed": self.cfg.seed,
            // Voicebox can rewrite a line "in character" first; the words
            // are the script's, so never.
            "personality": false,
        });
        if let Some(i) = instruct {
            body["instruct"] = serde_json::Value::String(i.to_string());
        }
        if let Some(size) = &self.cfg.model_size {
            body["model_size"] = serde_json::Value::String(size.clone());
        }
        let url = format!("{}/generate/stream", self.cfg.base_url);
        let resp = self
            .send(self.http.post(url).json(&body), "speaking a line")
            .await?;
        let bytes = resp.bytes().await.map_err(|e| {
            self.fail(&format!(
                "the audio came back incomplete: {}",
                with_causes(&e)
            ))
        })?;
        teleprompt_voice::wav::decode(&bytes).map_err(|e| self.fail(&format!("its audio: {e}")))
    }
}
