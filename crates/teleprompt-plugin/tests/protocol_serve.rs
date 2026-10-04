//! A Rust plugin served over the protocol: what teleprompt sends, and what
//! `serve` answers for the plugin behind it.

use std::io::Cursor;

use serde_json::{json, Value};
use teleprompt_plugin::capture::mock::MockCapture;
use teleprompt_plugin::protocol::serve;
use teleprompt_plugin::scene::MockScene;
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
fn a_plugin_describes_itself_and_what_it_needs() {
    let answers = exchange(
        &[request(1, "describe", json!({ "protocol": 1 }))],
        |i, o| serve::scene_on(&mock(), i, o).unwrap(),
    );
    let d = &answers[0]["result"];
    assert_eq!(d["protocol"], 1);
    assert_eq!(d["retimes"], true);
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
