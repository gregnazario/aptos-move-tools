use tools_base::IntoEdit;
use tree_sitter::Node;

#[derive(Debug)]
pub struct Suggestion {
    pub start_byte: usize,
    pub end_byte: usize,
    pub replacement: String,
    pub rule: &'static str,
    pub message: String,
}

impl IntoEdit for Suggestion {
    fn start_byte(&self) -> usize {
        self.start_byte
    }
    fn end_byte(&self) -> usize {
        self.end_byte
    }
    fn replacement(&self) -> &str {
        &self.replacement
    }
}

/// Apply suggestions back-to-front to preserve byte offsets.
pub fn apply_suggestions(source: &str, suggestions: Vec<Suggestion>) -> String {
    tools_base::apply_edits(source, suggestions)
}

/// Compute 1-based line and column from a byte offset.
pub fn line_col(source: &str, byte_offset: usize) -> (usize, usize) {
    tools_base::line_col(source, byte_offset)
}

/// Parse a `name_access_chain` node into (module, member).
/// Returns None for unqualified names or paths with 3+ segments.
pub fn parse_qualified_call<'a>(node: Node<'a>, source: &'a [u8]) -> Option<(&'a str, &'a str)> {
    let mut identifiers = Vec::new();
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() == "identifier" {
            identifiers.push(child.utf8_text(source).ok()?);
        }
    }
    if identifiers.len() == 2 {
        Some((identifiers[0], identifiers[1]))
    } else {
        None
    }
}

/// Extract the named children (argument expressions) from an arg_list node.
pub fn get_args(arg_list: Node) -> Vec<Node> {
    let mut cursor = arg_list.walk();
    arg_list.named_children(&mut cursor).collect()
}

/// Check if a node falls within any of the consumed byte ranges.
pub fn is_in_consumed(node: Node, consumed: &[(usize, usize)]) -> bool {
    consumed
        .iter()
        .any(|&(start, end)| node.start_byte() >= start && node.end_byte() <= end)
}
