//! The tools session mode offers, from `teleprompt record --tools`.

use teleprompt_gtk::tools::{parse, pick};

const LISTED: &str = r#"[
  {"adapter": "asciinema", "in_terminal": true, "unavailable": null},
  {"adapter": "vhs", "in_terminal": true, "unavailable": "vhs is not on PATH"},
  {"adapter": "playwright", "in_terminal": false, "unavailable": null}
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
        pick(&tools, Some("playwright")).unwrap().adapter,
        "playwright"
    );
    // Not installed: the first that is.
    assert_eq!(pick(&tools, Some("vhs")).unwrap().adapter, "asciinema");
    assert_eq!(pick(&tools, None).unwrap().adapter, "asciinema");
}
