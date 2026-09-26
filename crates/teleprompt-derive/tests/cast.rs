use teleprompt_derive::read_cast;

/// asciicast v2: absolute times, input as "i" events (recorded with
/// `asciinema rec --stdin`).
#[test]
fn a_v2_cast_gives_input_and_output_times() {
    let cast = r#"{"version": 2, "width": 80, "height": 24}
[0.5, "o", "$ "]
[1.25, "i", "l"]
[1.3, "i", "s"]
[1.4, "i", "\r"]
[1.6, "o", "a.txt\r\n"]
[2.0, "m", "marker"]
"#;
    let t = read_cast(cast).unwrap();
    assert_eq!(
        t.input,
        vec![
            (1250, "l".to_string()),
            (1300, "s".to_string()),
            (1400, "\r".to_string())
        ]
    );
    assert_eq!(t.output, vec![500, 1600]);
}

/// asciicast v3 (asciinema 3): each event's time is the interval since the
/// one before.
#[test]
fn a_v3_cast_accumulates_intervals() {
    let cast = r#"{"version": 3, "term": {"cols": 80, "rows": 24}}
[0.5, "o", "$ "]
[0.75, "i", "l"]
[0.05, "i", "s"]
# a comment line
[0.1, "i", "\r"]
"#;
    let t = read_cast(cast).unwrap();
    assert_eq!(
        t.input,
        vec![
            (1250, "l".to_string()),
            (1300, "s".to_string()),
            (1400, "\r".to_string())
        ]
    );
    assert_eq!(t.output, vec![500]);
}

#[test]
fn a_cast_without_input_says_how_to_record_it() {
    let e = read_cast("{\"version\": 2}\n[0.5, \"o\", \"$ \"]\n").unwrap_err();
    assert!(e.contains("no keystrokes"), "{e}");
    assert!(e.contains("--stdin") || e.contains("capture-input"), "{e}");
}

#[test]
fn a_malformed_cast_names_the_line() {
    let e = read_cast("{\"version\": 2}\n[0.5, \"i\"\n").unwrap_err();
    assert!(e.contains("line 2"), "{e}");
    let e = read_cast("{\"version\": 1}\n").unwrap_err();
    assert!(e.contains("version 1"), "{e}");
}
