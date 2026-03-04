//! Integration tests for tools-base.

use tools_base::{apply_edits, collect_move_files, line_col, new_move_parser, Edit};

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
fn test_apply_edits() {
    let source = "hello world";
    let edits = vec![Edit::new(0, 5, "hi"), Edit::new(6, 11, "there")];
    let result = apply_edits(source, edits);
    assert_eq!(result, "hi there");
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
fn test_collect_move_files_from_tools_base() {
    // Collect .move files from the tools-base directory (contains no .move files)
    let files = collect_move_files(&["."]);
    // Should return a vec (possibly empty if no .move files in cwd)
    let _ = files;
}

#[test]
fn test_collect_move_files_single_file_path() {
    // Collect with a non-existent path - should return empty for non-.move files
    let files = collect_move_files(&["Cargo.toml"]);
    assert!(files.is_empty());
}
