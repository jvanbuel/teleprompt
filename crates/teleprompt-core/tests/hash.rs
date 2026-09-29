//! A file hashed as it is read is the hash of its contents.

use teleprompt_core::Hash;

#[test]
fn a_file_hashes_as_its_contents_do() {
    let dir = std::env::temp_dir().join(format!("teleprompt-hash-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("clip.bin");
    // Larger than any one read, so the reads are stitched together.
    let bytes: Vec<u8> = (0..3_000_000u32).map(|i| (i % 251) as u8).collect();
    std::fs::write(&path, &bytes).unwrap();
    assert_eq!(Hash::of_file(&path).unwrap(), Hash::of(&bytes));
    assert!(Hash::of_file(&dir.join("missing")).is_err());
    std::fs::remove_dir_all(&dir).unwrap();
}
