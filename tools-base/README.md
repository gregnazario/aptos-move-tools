# tools-base

Shared base library for aptos-move-tools. Reduces boilerplate when building new Move analysis and transformation tools.

## Features

- **Parser**: `new_move_parser()` — creates a tree-sitter parser configured for Move on Aptos
- **Source**: `line_col()`, `apply_edits()`, `Edit` — location helpers and text edit application
- **Files**: `collect_move_files()` — discover `.move` files from paths (files or directories)
- **Tree**: `count_named_children()`, `node_text()` — tree-sitter node utilities

## Usage

```rust
use tools_base::{new_move_parser, collect_move_files, line_col, apply_edits, Edit};

// Parse Move source
let mut parser = new_move_parser();
let tree = parser.parse(source, None).unwrap();

// Find line/column for a byte offset
let (line, col) = line_col(source, 42);

// Apply edits (e.g., from a linter or transformer)
let edits = vec![
    Edit::new(0, 5, "hello"),
];
let result = apply_edits(source, edits);

// Collect .move files from paths
let files = collect_move_files(&["src/", "other/file.move"]);
```

## Dependencies

Uses the same `tree-sitter` and `tree-sitter-move-on-aptos` versions as the tools (per AGENTS.md).
