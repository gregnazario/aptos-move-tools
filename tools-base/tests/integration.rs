//! Integration tests for tools-base.

use std::path::PathBuf;

use tools_base::{Edit, apply_edits, collect_move_files, line_col, new_move_parser};

#[test]
fn test_line_col() {
    let source = "line1\nline2\nline3";
    assert_eq!(line_col(source, 0), (1, 1));
    assert_eq!(line_col(source, 5), (1, 6));
    assert_eq!(line_col(source, 6), (2, 1));
    assert_eq!(line_col(source, 11), (2, 6)); // at \n ending line2
    assert_eq!(line_col(source, 12), (3, 1)); // at 'l' of line3
}

#[test]
fn test_line_col_past_eof() {
    let source = "ab\nc";
    // Offset past EOF: should clamp and return (2, 2) not a huge column
    assert_eq!(line_col(source, 100), (2, 2));
}

#[test]
fn test_apply_edits() {
    let source = "hello world";
    let edits = vec![Edit::new(0, 5, "hi"), Edit::new(6, 11, "there")];
    let result = apply_edits(source, edits);
    assert_eq!(result, "hi there");
}

#[test]
fn test_apply_edits_unicode() {
    // Byte-based application handles non-ASCII (avoids replace_range char-boundary panic)
    let source = "café = 1"; // "café" is 5 bytes in UTF-8 (é is 2 bytes)
    let edits = vec![Edit::new(0, 5, "tea")];
    let result = apply_edits(source, edits);
    assert_eq!(result, "tea = 1");
}

#[test]
fn test_apply_edits_order_independent() {
    let source = "abc def ghi";
    let edits = vec![
        Edit::new(8, 11, "XXX"),
        Edit::new(0, 3, "AAA"),
        Edit::new(4, 7, "BBB"),
    ];
    let result = apply_edits(source, edits);
    assert_eq!(result, "AAA BBB XXX");
}

#[test]
fn test_new_move_parser() {
    let mut parser = new_move_parser();
    let source = "module 0x1::m { }";
    let tree = parser.parse(source, None).unwrap();
    assert!(tree.root_node().child_count() > 0);
}

#[test]
fn test_collect_move_files_from_temp_dir() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();

    // Create known layout: nested dirs with .move and non-.move files
    std::fs::create_dir(root.join("sources")).unwrap();
    std::fs::create_dir(root.join("sources/nested")).unwrap();
    std::fs::write(root.join("sources/a.move"), "").unwrap();
    std::fs::write(root.join("sources/b.move"), "").unwrap();
    std::fs::write(root.join("sources/nested/c.move"), "").unwrap();
    std::fs::write(root.join("sources/other.rs"), "").unwrap();
    std::fs::write(root.join("sources/script.sh"), "").unwrap();

    let files = collect_move_files(&[root]);
    let mut names: Vec<String> = files
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
        .collect();
    names.sort();

    assert_eq!(names, ["a.move", "b.move", "c.move"]);
    assert_eq!(files.len(), 3);
}

#[test]
fn test_collect_move_files_single_file() {
    let temp = tempfile::tempdir().unwrap();
    let move_file = temp.path().join("foo.move");
    std::fs::write(&move_file, "module 0x1::m {}").unwrap();

    let files = collect_move_files(&[&move_file]);
    assert_eq!(files.len(), 1);
    assert!(files[0].ends_with("foo.move"));
}

#[test]
fn test_collect_move_files_non_move_extension() {
    let files = collect_move_files(&[PathBuf::from("Cargo.toml")]);
    assert!(files.is_empty());
}
