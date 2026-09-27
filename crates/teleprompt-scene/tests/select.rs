//! `include=file#fragment` for bodies split by a mark line.

use teleprompt_scene::select_marked;

const BODY: &str = "Type \"a\"\n# mark\nType \"b\"\n# mark\nType \"c\"\n";

#[test]
fn a_number_is_the_part_after_that_many_marks_less_one() {
    assert_eq!(select_marked(BODY, "# mark", "1").unwrap(), "Type \"a\"\n");
    assert_eq!(select_marked(BODY, "# mark", "3").unwrap(), "Type \"c\"\n");
}

#[test]
fn a_range_keeps_the_marks_between_its_parts() {
    assert_eq!(
        select_marked(BODY, "# mark", "2-3").unwrap(),
        "Type \"b\"\n# mark\nType \"c\"\n"
    );
}

#[test]
fn a_fragment_past_the_parts_says_how_many_there_are() {
    for bad in ["4", "0", "3-2", "deploy"] {
        let e = select_marked(BODY, "# mark", bad).unwrap_err();
        assert!(e.contains("3 part(s)"), "{bad}: {e}");
    }
}
