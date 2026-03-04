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
/// Uses a single clamped offset to avoid panics on invalid UTF-8 boundaries
/// and to return correct column for offsets past EOF.
pub fn line_col(source: &str, byte_offset: usize) -> (usize, usize) {
    let bytes = source.as_bytes();
    let offset = byte_offset.min(bytes.len());
    let prefix = &bytes[..offset];
    let line = prefix.iter().filter(|&&b| b == b'\n').count() + 1;
    let col = prefix
        .iter()
        .rposition(|&b| b == b'\n')
        .map(|i| offset - i)
        .unwrap_or(offset + 1);
    (line, col)
}

/// Apply edits back-to-front to preserve byte offsets.
/// Edits are sorted by `start_byte` descending before application.
/// Operates on raw bytes so offsets need not be on UTF-8 char boundaries.
pub fn apply_edits<E: IntoEdit>(source: &str, mut edits: Vec<E>) -> String {
    edits.sort_by_key(|b| std::cmp::Reverse(b.start_byte()));
    let mut result = source.as_bytes().to_vec();
    for e in &edits {
        let start = e.start_byte();
        let end = e.end_byte();
        let replacement = e.replacement().as_bytes();
        result.splice(start..end, replacement.iter().copied());
    }
    String::from_utf8(result).expect("edits produced invalid UTF-8")
}
