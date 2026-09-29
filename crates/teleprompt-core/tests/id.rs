//! A shot id names its block and its place in it.

use teleprompt_core::{BlockId, ShotId};

#[test]
fn a_shot_id_splits_into_its_block_and_index() {
    let block = BlockId::from("the-loop-a2");
    let shot = ShotId::of(&block, 3);
    assert_eq!(shot.block(), "the-loop-a2");
    assert_eq!(shot.index(), Some(3));
}

#[test]
fn an_id_without_an_index_is_all_block() {
    let shot = ShotId::from("welcome-a");
    assert_eq!(shot.block(), "welcome-a");
    assert_eq!(shot.index(), None);
}
