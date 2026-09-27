//! A cast read back as the commands typed in it, and marked between them.

use teleprompt_asciinema::record::read;
use teleprompt_asciinema::scene::{parse, select};

const CAST: &str = r#"{"version": 2, "width": 80, "height": 24}
[0.1, "o", "$ "]
[1.0, "i", "l"]
[1.1, "i", "s"]
[1.2, "i", "\r"]
[1.3, "o", "a.txt\r\n$ "]
[3.0, "i", "pwd\r"]
[3.1, "o", "/tmp\r\n$ "]
[5.0, "i", "exit\r"]
[5.1, "o", "exit\r\n"]
"#;

#[test]
fn each_command_is_a_step_from_its_first_key() {
    let r = read(CAST).unwrap();
    let starts: Vec<u64> = r.steps.iter().map(|s| s.start_ms).collect();
    assert_eq!(starts, [1000, 3000]);
    assert!(r.head.contains("\"$ \""), "{}", r.head);
    assert!(r.steps[0].text.contains("a.txt"));
}

#[test]
fn the_shell_s_closing_command_is_left_out() {
    let r = read(CAST).unwrap();
    let all = r.marked(&[]);
    assert!(!all.contains("exit"), "{all}");
}

#[test]
fn a_cut_is_a_marker_the_scene_splits_at() {
    let r = read(CAST).unwrap();
    let marked = r.marked(&[1]);
    assert!(marked.contains(r#"[3.0,"m",""]"#), "{marked}");
    let cast = parse(&marked).unwrap();
    let second = select(&cast, "2").unwrap();
    let shown: String = second.events.iter().map(|e| e.data.clone()).collect();
    assert!(
        shown.contains("/tmp") && !shown.contains("a.txt"),
        "{shown}"
    );
}

#[test]
fn a_v3_cast_is_rewritten_with_absolute_times() {
    let v3 = r#"{"version": 3, "term": {"cols": 80, "rows": 24}}
[0.5, "o", "$ "]
[0.5, "i", "ls\r"]
[0.25, "o", "a.txt\r\n"]
"#;
    let r = read(v3).unwrap();
    assert_eq!(r.steps[0].start_ms, 1000);
    assert!(r.head.contains(r#""version":2"#), "{}", r.head);
    assert!(parse(&r.marked(&[])).is_ok());
}

#[test]
fn a_cast_without_keystrokes_says_how_to_record_one() {
    let e = read("{\"version\": 2, \"width\": 80, \"height\": 24}\n[0.1, \"o\", \"$ \"]\n")
        .unwrap_err();
    assert!(e.contains("--stdin"), "{e}");
}
