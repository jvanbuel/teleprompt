use teleprompt_plugin::capture::WorkDir;

#[test]
fn a_work_dir_is_removed_with_everything_in_it_when_dropped() {
    let parent = tempfile::tempdir().unwrap();
    let work = WorkDir::create(parent.path(), "test").unwrap();
    std::fs::write(work.join("recording.mp4"), b"half a recording").unwrap();
    assert!(work.is_dir());

    drop(work);
    assert_eq!(std::fs::read_dir(parent.path()).unwrap().count(), 0);
}
