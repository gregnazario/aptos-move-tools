//! Tree-sitter node helpers.

use tree_sitter::Node;

/// Count the named children of a node.
pub fn count_named_children(node: Node) -> usize {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).count()
}

/// Get the UTF-8 text of a node, or empty string on error.
pub fn node_text(node: Node, source: &[u8]) -> String {
    node.utf8_text(source).unwrap_or("").to_string()
}
