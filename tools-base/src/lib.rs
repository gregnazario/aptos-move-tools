//! Shared base library for aptos-move-tools.
//!
//! Provides common functionality for building Move source analysis and transformation tools:
//! - Tree-sitter Move parser setup
//! - Source location helpers (line/column from byte offset)
//! - Edit application (apply multiple text replacements preserving offsets)
//! - Move file discovery (collect .move files from paths/directories)
//! - Optional parallel file processing with rayon

pub mod files;
pub mod parser;
pub mod source;
pub mod tree;

pub use files::collect_move_files;
pub use parser::new_move_parser;
pub use source::{apply_edits, line_col, Edit, IntoEdit};
pub use tree::{count_named_children, node_text};
