//! How teleprompt reads and writes YAML, held still so the library behind
//! `teleprompt_core::yaml` can be changed without a script or a project
//! file reading differently.

use serde_json::{json, Value};
use teleprompt_core::yaml::{from_str, to_string};

fn read(text: &str) -> Value {
    from_str(text).unwrap_or_else(|e| panic!("{text:?}: {e}"))
}

#[test]
fn scalars_are_the_types_they_look_like() {
    assert_eq!(read("a: 1"), json!({"a": 1}));
    assert_eq!(read("a: -12"), json!({"a": -12}));
    assert_eq!(read("a: 1.5"), json!({"a": 1.5}));
    assert_eq!(read("a: 1e3"), json!({"a": 1000.0}));
    assert_eq!(read("a: true"), json!({"a": true}));
    assert_eq!(read("a: false"), json!({"a": false}));
    assert_eq!(read("a: hello"), json!({"a": "hello"}));
    assert_eq!(read("a: \"1\""), json!({"a": "1"}));
    assert_eq!(read("a: '1'"), json!({"a": "1"}));
    assert_eq!(read("a: ~"), json!({"a": null}));
    assert_eq!(read("a: null"), json!({"a": null}));
    assert_eq!(read("a:"), json!({"a": null}));
}

/// An author's `no` or `on` is a word, not a boolean: only `true` and
/// `false` are.
#[test]
fn only_true_and_false_are_booleans() {
    for word in ["yes", "no", "on", "off", "y", "n", "Yes", "NO", "On"] {
        assert_eq!(read(&format!("a: {word}")), json!({ "a": word }), "{word}");
    }
}

#[test]
fn a_date_or_a_time_stays_text() {
    assert_eq!(read("a: 2024-01-31"), json!({"a": "2024-01-31"}));
    assert_eq!(read("a: 12:30"), json!({"a": "12:30"}));
}

/// `0777` is not read as octal 511. Whether a settings value comes back as
/// the text or as the number 777 depends on the YAML library; either way
/// it is not 511.
#[test]
fn a_leading_zero_is_not_octal() {
    let v = &read("a: 0777")["a"];
    assert!(v.as_f64() != Some(511.0) && v.as_i64() != Some(511), "{v}");
}

#[test]
fn collections_read_in_flow_and_block_style() {
    assert_eq!(read("a: [1, 2, 3]"), json!({"a": [1, 2, 3]}));
    assert_eq!(read("a: {b: 1, c: x}"), json!({"a": {"b": 1, "c": "x"}}));
    assert_eq!(
        read("a:\n  - x\n  - y: 1\n    z: 2\n"),
        json!({"a": ["x", {"y": 1, "z": 2}]})
    );
    assert_eq!(
        read("a:\n  b:\n    c: d\n"),
        json!({"a": {"b": {"c": "d"}}})
    );
}

#[test]
fn block_scalars_keep_their_lines() {
    assert_eq!(read("a: |\n  one\n  two\n"), json!({"a": "one\ntwo\n"}));
    assert_eq!(read("a: >\n  one\n  two\n"), json!({"a": "one two\n"}));
    assert_eq!(read("a: |-\n  one\n"), json!({"a": "one"}));
}

#[test]
fn quoted_strings_unescape() {
    assert_eq!(read(r#"a: "tab\there""#), json!({"a": "tab\there"}));
    assert_eq!(read(r#"a: "say \"hi\"""#), json!({"a": "say \"hi\""}));
    assert_eq!(read("a: 'it''s'"), json!({"a": "it's"}));
    assert_eq!(read(r#"a: "é ü 日本""#), json!({"a": "é ü 日本"}));
}

#[test]
fn comments_are_ignored() {
    assert_eq!(read("# top\na: 1 # end\n# last\n"), json!({"a": 1}));
}

#[test]
fn anchors_and_aliases_are_followed() {
    assert_eq!(
        read("a: &x {b: 1}\nc: *x\n"),
        json!({"a": {"b": 1}, "c": {"b": 1}})
    );
}

#[test]
fn an_empty_document_is_null() {
    assert_eq!(read(""), Value::Null);
    assert_eq!(read("# nothing\n"), Value::Null);
}

#[test]
fn a_document_that_is_a_list_or_a_scalar_reads_as_one() {
    assert_eq!(read("- 1\n- 2\n"), json!([1, 2]));
    assert_eq!(read("hello"), json!("hello"));
}

/// A key written twice is a mistake the author should hear about, not one
/// that silently keeps the last.
#[test]
fn a_repeated_key_is_an_error_for_a_typed_target() {
    #[derive(serde::Deserialize, Debug)]
    #[allow(dead_code)]
    struct One {
        a: u32,
    }
    assert!(from_str::<One>("a: 1\na: 2\n").is_err());
}

#[test]
fn bad_yaml_is_an_error_that_says_something() {
    for bad in [
        "a: [1, 2",
        "a: {b: 1",
        "a:\n\t- tab",
        "a: \"unterminated",
        ": x",
    ] {
        let e = from_str::<Value>(bad).unwrap_err().to_string();
        assert!(!e.trim().is_empty(), "{bad:?}");
    }
}

/// Typed targets read the way the project file's own structs need them to.
#[test]
fn typed_targets_read_numbers_strings_and_options() {
    #[derive(serde::Deserialize, Debug, PartialEq)]
    struct Settings {
        count: u32,
        ratio: f64,
        name: String,
        flag: bool,
        maybe: Option<String>,
        list: Vec<u8>,
    }
    let got: Settings =
        from_str("count: 3\nratio: 2\nname: x\nflag: true\nlist: [1, 2]\n").unwrap();
    assert_eq!(
        got,
        Settings {
            count: 3,
            ratio: 2.0,
            name: "x".into(),
            flag: true,
            maybe: None,
            list: vec![1, 2],
        }
    );
    assert!(from_str::<Settings>("count: x\nratio: 1\nname: a\nflag: true\nlist: []\n").is_err());
}

#[test]
fn a_string_field_takes_a_word_that_looks_like_something_else() {
    #[derive(serde::Deserialize)]
    struct Word {
        w: String,
    }
    for word in ["no", "on", "null", "~"] {
        // Quoted, so the author said it is text.
        let got: Word = from_str(&format!("w: \"{word}\"")).unwrap();
        assert_eq!(got.w, word);
    }
}

/// What the translation sidecar and other one-line scalars rely on: the
/// text comes back as it went in, on one line.
#[test]
fn a_string_written_as_yaml_reads_back_the_same() {
    let hard = [
        "plain",
        "",
        " leading space",
        "trailing space ",
        "true",
        "false",
        "null",
        "~",
        "yes",
        "no",
        "on",
        "1",
        "1.5",
        "0x1f",
        "1e3",
        "-",
        "- dash",
        "? question",
        "a: b",
        "a #b",
        "# hash",
        "{brace",
        "[bracket",
        "*star",
        "&amp",
        "!bang",
        "%percent",
        "@at",
        "`tick",
        "'single'",
        "it's",
        "say \"hi\"",
        "back\\slash",
        "tab\there",
        "line\nbreak",
        "trailing\n",
        "é ü 日本 🙂",
        "2024-01-31",
        "12:30",
        ":",
        "a, b",
    ];
    for s in hard {
        let yaml = to_string(&s).unwrap();
        let back: String = from_str(&yaml).unwrap_or_else(|e| panic!("{s:?} -> {yaml:?}: {e}"));
        assert_eq!(back, s, "{s:?} written as {yaml:?}");
    }
}

#[test]
fn a_simple_string_is_one_unquoted_line() {
    assert_eq!(to_string(&"hello").unwrap().trim_end(), "hello");
    assert_eq!(
        to_string(&"Hello, world.").unwrap().trim_end(),
        "Hello, world."
    );
}

#[test]
fn a_value_written_as_yaml_reads_back_the_same() {
    let v = json!({
        "name": "x",
        "n": 3,
        "f": 1.5,
        "t": true,
        "none": null,
        "list": [1, "two", {"three": 3}],
        "nested": {"a": {"b": "c"}},
    });
    let back: Value = from_str(&to_string(&v).unwrap()).unwrap();
    assert_eq!(back, v);
}
