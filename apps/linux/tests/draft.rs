use teleprompt_gtk::draft::{progress, Progress};

#[test]
fn the_status_file_is_read_as_record_writes_it() {
    let dir = std::env::temp_dir().join(format!("tp-draft-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("status.json");
    assert_eq!(progress(&path), Progress::Starting);
    for (json, want) in [
        (
            r#"{"state":"recording","pid":4242,"trace":"/t"}"#,
            Progress::Recording(4242),
        ),
        (r#"{"state":"drafting"}"#, Progress::Drafting),
        (
            r#"{"state":"done","script":"/p/scripts/s.md","lines":2,"tapes":1}"#,
            Progress::Done("/p/scripts/s.md".into()),
        ),
        (
            r#"{"state":"failed","error":"no speech model at /m"}"#,
            Progress::Failed("no speech model at /m".into()),
        ),
        ("{half", Progress::Starting),
    ] {
        std::fs::write(&path, json).unwrap();
        assert_eq!(progress(&path), want, "{json}");
    }
    std::fs::remove_dir_all(dir).unwrap();
}
