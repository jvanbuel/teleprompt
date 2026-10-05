//! The tools session mode offers, from `teleprompt record --tools`.

use teleprompt_gtk::tools::{parse, pick};

const LISTED: &str = r#"[
  {"plugin": "asciinema", "in_terminal": true, "unavailable": null},
  {"plugin": "vhs", "in_terminal": true, "unavailable": "vhs is not on PATH"},
  {"plugin": "playwright", "in_terminal": false, "unavailable": null}
]"#;

#[test]
fn each_tool_reads_as_what_it_records() {
    let tools = parse(LISTED).unwrap();
    let labels: Vec<String> = tools.iter().map(|t| t.label()).collect();
    assert_eq!(
        labels,
        [
            "Terminal · asciinema",
            "Terminal · vhs",
            "Browser · playwright"
        ]
    );
}

#[test]
fn the_chosen_tool_is_used_if_it_can_record() {
    let tools = parse(LISTED).unwrap();
    assert_eq!(
        pick(&tools, Some("playwright")).unwrap().plugin,
        "playwright"
    );
    // Not installed: the first that is.
    assert_eq!(pick(&tools, Some("vhs")).unwrap().plugin, "asciinema");
    assert_eq!(pick(&tools, None).unwrap().plugin, "asciinema");
}

/// Whether drafting is set up is `teleprompt setup`'s to say, not the
/// app's to look for.
#[test]
fn drafting_is_ready_when_setup_says_so() {
    use teleprompt_gtk::tools::drafts_ready;
    let report = |installed: bool| {
        format!(
            r#"{{"uses":[{{"name":"render","installed":false}},{{"name":"drafts","installed":{installed}}}]}}"#
        )
    };
    assert!(drafts_ready(&report(true)));
    assert!(!drafts_ready(&report(false)));
    // Unreadable, or an older teleprompt: `record` says what it lacks.
    assert!(drafts_ready("not json"));
    assert!(drafts_ready(r#"{"uses":[]}"#));
}
