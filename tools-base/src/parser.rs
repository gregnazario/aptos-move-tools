//! Tree-sitter Move parser setup.

use tree_sitter::Parser;

/// Creates a new tree-sitter parser configured for Move on Aptos.
///
/// # Panics
///
/// Panics if the Move grammar fails to load.
pub fn new_move_parser() -> Parser {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_move_on_aptos::language())
        .expect("Error loading Move grammar");
    parser
}
