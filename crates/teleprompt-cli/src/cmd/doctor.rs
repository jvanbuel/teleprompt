use serde::Serialize;
use teleprompt_compile::manifest::MANIFEST_VERSION;
use teleprompt_scene::SceneRegistry;

#[derive(Debug, Serialize)]
pub struct DoctorReport {
    pub ok: bool,
    pub adapters: Vec<String>,
    pub voice_backends: Vec<String>,
    pub manifest_version: u32,
    pub notes: Vec<String>,
}

pub fn doctor_report(registry: &SceneRegistry) -> DoctorReport {
    DoctorReport {
        ok: true,
        adapters: registry.available().iter().map(|s| s.to_string()).collect(),
        voice_backends: vec!["null".to_string()],
        manifest_version: MANIFEST_VERSION,
        notes: vec![
            "M0 builds no video, so ffmpeg is not required yet.".to_string(),
            "M0 ships no external runtime, so Node and Playwright are not required yet."
                .to_string(),
            "dub writes 16-bit PCM WAV at 48 kHz; the null backend renders silence \
             of the estimated duration."
                .to_string(),
        ],
    }
}

impl DoctorReport {
    pub fn render(&self) -> String {
        let mut out = String::from("teleprompt doctor\n");
        out.push_str(&format!(
            "  scene adapters   {}\n",
            self.adapters.join(", ")
        ));
        out.push_str(&format!(
            "  voice backends   {}\n",
            self.voice_backends.join(", ")
        ));
        out.push_str(&format!(
            "  manifest         narration v{}\n",
            self.manifest_version
        ));
        for n in &self.notes {
            out.push_str(&format!("  note             {n}\n"));
        }
        out
    }
}
