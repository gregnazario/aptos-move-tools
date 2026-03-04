//! Source code helpers: line/column computation and edit application.

/// A text edit with byte offsets.
#[derive(Debug, Clone)]
pub struct Edit {
    pub start_byte: usize,
    pub end_byte: usize,
    pub replacement: String,
}

impl Edit {
    pub fn new(start_byte: usize, end_byte: usize, replacement: impl Into<String>) -> Self {
        Self {
            start_byte,
            end_byte,
            replacement: replacement.into(),
        }
    }
}

/// Trait for types that can be converted to an Edit (for applying).
pub trait IntoEdit {
    fn start_byte(&self) -> usize;
    fn end_byte(&self) -> usize;
    fn replacement(&self) -> &str;
}

impl IntoEdit for Edit {
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

/// Compute 1-based line and column from a byte offset in source text.
pub fn line_col(source: &str, byte_offset: usize) -> (usize, usize) {
    let prefix = &source[..byte_offset.min(source.len())];
    let line = prefix.matches('\n').count() + 1;
    let col = prefix
        .rfind('\n')
        .map(|i| byte_offset - i)
        .unwrap_or(byte_offset + 1);
    (line, col)
}

/// Apply edits back-to-front to preserve byte offsets.
/// Edits are sorted by `start_byte` descending before application.
pub fn apply_edits<E: IntoEdit>(source: &str, mut edits: Vec<E>) -> String {
    edits.sort_by_key(|b| std::cmp::Reverse(b.start_byte()));
    let mut result = source.to_string();
    for e in &edits {
        result.replace_range(e.start_byte()..e.end_byte(), e.replacement());
    }
    result
}
