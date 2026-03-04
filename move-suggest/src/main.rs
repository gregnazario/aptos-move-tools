use std::env;
use std::fs;
use std::io::IsTerminal;
use std::process;

mod receiver_style;
mod suggest;
mod vector_literal;

use suggest::{apply_suggestions, line_col, Suggestion};
use tools_base::new_move_parser;

// ANSI color codes
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const YELLOW: &str = "\x1b[33m";
const GREEN: &str = "\x1b[32m";
const MAGENTA: &str = "\x1b[35m";
const RESET: &str = "\x1b[0m";

fn format_suggestion(path: &str, line: usize, col: usize, s: &Suggestion, color: bool) -> String {
    if !color {
        return format!("{}:{}:{} [{}] {}", path, line, col, s.rule, s.message);
    }

    // Split message on " can be written as " to highlight the suggestion part
    let (before, after) = match s.message.split_once(" can be written as ") {
        Some((b, a)) => (b, Some(a)),
        None => (s.message.as_str(), None),
    };

    match after {
        Some(replacement) => format!(
            "{BOLD}{path}{RESET}{DIM}:{line}:{col}{RESET} {YELLOW}[{rule}]{RESET} {before} {DIM}can be written as{RESET} {GREEN}{BOLD}{replacement}{RESET}",
            rule = s.rule,
        ),
        None => format!(
            "{BOLD}{path}{RESET}{DIM}:{line}:{col}{RESET} {YELLOW}[{rule}]{RESET} {before}",
            rule = s.rule,
        ),
    }
}

fn main() {
    let args: Vec<String> = env::args().collect();

    let fix_mode = args.iter().any(|a| a == "--fix");
    let files: Vec<&String> = args[1..].iter().filter(|a| !a.starts_with("--")).collect();

    if files.is_empty() {
        eprintln!("Usage: move-suggest [--fix] <file.move> [file2.move ...]");
        process::exit(2);
    }

    let color = std::io::stdout().is_terminal();

    let mut parser = new_move_parser();
    let mut total_suggestions = 0;

    for path in &files {
        let source = match fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Error reading {}: {}", path, e);
                process::exit(2);
            }
        };

        let tree = match parser.parse(&source, None) {
            Some(t) => t,
            None => {
                eprintln!("Error parsing {}", path);
                process::exit(2);
            }
        };

        let root = tree.root_node();
        let bytes = source.as_bytes();

        // Phase 1: multi-push patterns (returns consumed byte ranges)
        let mut suggestions: Vec<Suggestion> = Vec::new();
        let mut consumed: Vec<(usize, usize)> = Vec::new();
        vector_literal::collect_multi_push(root, bytes, &mut suggestions, &mut consumed);

        // Phase 2: single-call patterns, skipping consumed ranges
        receiver_style::collect_receiver_style(root, bytes, &mut suggestions, &consumed);
        vector_literal::collect_simple_vector(root, bytes, &mut suggestions, &consumed);

        if suggestions.is_empty() {
            continue;
        }

        // Sort by position for display
        suggestions.sort_by_key(|s| s.start_byte);

        for s in &suggestions {
            let (line, col) = line_col(&source, s.start_byte);
            println!("{}", format_suggestion(path, line, col, s, color));
        }

        total_suggestions += suggestions.len();

        if fix_mode {
            let result = apply_suggestions(&source, suggestions);
            if let Err(e) = fs::write(path, &result) {
                eprintln!("Error writing {}: {}", path, e);
                process::exit(2);
            }
        }
    }

    if color {
        eprintln!(
            "{MAGENTA}{BOLD}{}{RESET} suggestion(s) found",
            total_suggestions
        );
    } else {
        eprintln!("{} suggestion(s) found", total_suggestions);
    }
    process::exit(if total_suggestions > 0 { 1 } else { 0 });
}
