//! A Rust plugin served over the protocol: what teleprompt sends, and what
//! `serve` answers for the plugin or voice behind it.

use std::io::Cursor;
use std::sync::Arc;

use base64::Engine;
use serde_json::{json, Value};
use teleprompt_plugin::capture::mock::MockCapture;
use teleprompt_plugin::protocol::serve;
use teleprompt_plugin::scene::MockScene;
use teleprompt_plugin::voice::{
    async_trait, wav, LanguageSupport, Pcm, SynthRequest, Synthesized, VoiceBackend,
    VoiceCapabilities, VoiceError, VoicePlugin,
};
use teleprompt_plugin::ScenePlugin;

/// Each request in turn, and each line answered.
fn exchange(requests: &[Value], serve: impl FnOnce(Cursor<Vec<u8>>, &mut Vec<u8>)) -> Vec<Value> {
    let input: String = requests.iter().map(|r| format!("{r}\n")).collect();
    let mut out = Vec::new();
    serve(Cursor::new(input.into_bytes()), &mut out);
    String::from_utf8(out)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

fn mock() -> ScenePlugin {
    ScenePlugin::new(MockScene, MockCapture::default())
}

fn request(id: u64, method: &str, params: Value) -> Value {
    json!({ "id": id, "method": method, "params": params })
}

#[test]
fn an_plugin_describes_itself_and_what_it_needs() {
    let answers = exchange(
        &[request(1, "describe", json!({ "protocol": 1 }))],
        |i, o| serve::scene_on(&mock(), i, o).unwrap(),
    );
    let d = &answers[0]["result"];
    assert_eq!(d["protocol"], 1);
    assert_eq!(d["kind"], "scene");
    // Its traits sit beside the kind, and no voice's are there.
    assert_eq!(d["retimes"], true);
    assert!(d.get("speed_control").is_none(), "{d}");
    assert_eq!(d["name"], "mock");
    assert_eq!(d["needs"][0]["name"], "ffmpeg");
    assert_eq!(d["needs"][0]["program"], "ffmpeg");
}

#[test]
fn a_bad_line_is_named_by_its_place_in_the_body() {
    let body = json!({ "scene": "mock", "body": "wait 1s\nfly 2s\n" });
    let answers = exchange(&[request(7, "validate", body)], |i, o| {
        serve::scene_on(&mock(), i, o).unwrap()
    });
    assert_eq!(answers[0]["id"], 7);
    let errors = answers[0]["result"]["errors"].as_array().unwrap();
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert_eq!(errors[0]["line"], 1);
    assert!(errors[0]["message"].as_str().unwrap().contains("fly"));
}

#[test]
fn shots_come_split_at_marks_with_their_lengths() {
    let body = json!({ "scene": "mock", "body": "wait 500ms\nmark\nopen\n" });
    let answers = exchange(&[request(1, "shots", body)], |i, o| {
        serve::scene_on(&mock(), i, o).unwrap()
    });
    let shots = answers[0]["result"]["shots"].as_array().unwrap();
    assert_eq!(shots.len(), 2);
    assert_eq!(shots[0]["ms"], 500);
    assert_eq!(shots[0]["exact"], true);
    // A shot that states no length lasts as long as its line.
    assert_eq!(shots[1]["ms"], Value::Null);
}

#[test]
fn an_unknown_method_is_an_error_not_a_hang() {
    let answers = exchange(
        &[
            request(1, "dance", json!({})),
            request(2, "unavailable", json!(null)),
        ],
        |i, o| serve::scene_on(&mock(), i, o).unwrap(),
    );
    assert!(answers[0]["error"].as_str().unwrap().contains("dance"));
    assert!(answers[1].get("result").is_some(), "{:?}", answers[1]);
}

/// A voice that says every line as a second of silence.
struct Hush;

#[async_trait]
impl VoiceBackend for Hush {
    fn id(&self) -> &str {
        "hush"
    }

    fn capabilities(&self) -> VoiceCapabilities {
        VoiceCapabilities {
            languages: LanguageSupport::Any,
            cloning: false,
            cross_lingual: false,
            word_timings: false,
            ssml: false,
            speed_control: false,
            version: "hush 1".into(),
        }
    }

    async fn synthesize(&self, _: &SynthRequest) -> Result<Synthesized, VoiceError> {
        Ok(Synthesized {
            pcm: Pcm {
                sample_rate: 8000,
                channels: 1,
                samples: vec![0; 8000],
            },
            word_timings: None,
        })
    }
}

fn hush() -> VoicePlugin {
    VoicePlugin {
        id: "hush",
        build: |settings| match settings {
            Some(_) => Err("hush takes no settings".into()),
            None => Ok(Arc::new(Hush)),
        },
        needs: &[],
    }
}

#[test]
fn a_voice_answers_each_line_with_its_audio() {
    let answers = exchange(
        &[
            request(1, "describe", json!({ "protocol": 1 })),
            request(
                2,
                "synthesize",
                json!({ "text": "Hello.", "locale": "en", "speed": 1.0, "settings": null }),
            ),
        ],
        |i, o| serve::voice_on(&hush(), i, o).unwrap(),
    );
    assert_eq!(answers[0]["result"]["kind"], "voice");
    assert_eq!(answers[0]["result"]["version"], "hush 1");
    let audio = answers[1]["result"]["audio"].as_str().expect("audio");
    let wav_bytes = base64::engine::general_purpose::STANDARD
        .decode(audio)
        .unwrap();
    assert_eq!(wav::decode(&wav_bytes).unwrap().duration_ms(), 1000);
}

#[test]
fn settings_a_voice_refuses_say_why() {
    let answers = exchange(
        &[request(
            1,
            "synthesize",
            json!({ "text": "Hi.", "locale": "en", "speed": 1.0, "settings": { "loud": true } }),
        )],
        |i, o| serve::voice_on(&hush(), i, o).unwrap(),
    );
    assert_eq!(answers[0]["error"], "hush takes no settings");
}
