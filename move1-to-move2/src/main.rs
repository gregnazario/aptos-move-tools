use std::env;
use std::fs;
use std::process;

#[derive(Debug)]
struct Edit {
    start_byte: usize,
    end_byte: usize,
    replacement: String,
    rule: &'static str,
}

fn apply_edits(source: &str, mut edits: Vec<Edit>) -> String {
    edits.sort_by(|a, b| b.start_byte.cmp(&a.start_byte));
    let mut result = source.to_string();
    for edit in &edits {
        result.replace_range(edit.start_byte..edit.end_byte, &edit.replacement);
    }
    result
}

/// Check if a node is the object of a dot_expression (i.e., followed by .field).
fn is_dot_object(node: tree_sitter::Node) -> bool {
    if let Some(parent) = node.parent() {
        if parent.kind() == "dot_expression" {
            return parent
                .child_by_field_name("object")
                .map(|o| o.id()) == Some(node.id());
        }
    }
    false
}

/// Check if a node is the leftmost descendant of a borrow_expression (&/&mut).
/// Walks up through dot_expression object positions to find an enclosing borrow.
fn has_ancestor_borrow(node: tree_sitter::Node) -> bool {
    let mut current = node;
    while let Some(parent) = current.parent() {
        match parent.kind() {
            "borrow_expression" => return true,
            "dot_expression" => {
                if parent.child_by_field_name("object").map(|o| o.id()) == Some(current.id()) {
                    current = parent;
                    continue;
                }
                return false;
            }
            _ => return false,
        }
    }
    false
}

/// Check if a node is inside a spec block (where borrow_global is a value, not a reference).
fn is_inside_spec_block(node: tree_sitter::Node) -> bool {
    let mut current = node;
    while let Some(parent) = current.parent() {
        if parent.kind() == "spec_block" {
            return true;
        }
        current = parent;
    }
    false
}

/// Check if a borrow_global call should omit its prefix (&/&mut).
/// This happens when: (1) an ancestor borrow_expression already provides it,
/// (2) the call is followed by .field access, or (3) inside a spec block.
fn should_strip_prefix(node: tree_sitter::Node) -> bool {
    has_ancestor_borrow(node) || is_dot_object(node) || is_inside_spec_block(node)
}

/// Try to match a call_expression as borrow_global or borrow_global_mut.
/// Returns an Edit spanning the call_expression node if it matches.
fn try_borrow_global_edit(node: tree_sitter::Node, source: &[u8]) -> Option<Edit> {
    let func_node = node.child_by_field_name("function")?;
    let func_text = func_node.utf8_text(source).ok()?;
    let prefix = match func_text {
        "borrow_global" => "&",
        "borrow_global_mut" => "&mut ",
        _ => return None,
    };
    let type_args = node.child_by_field_name("type_arguments")?;
    let args = node.child_by_field_name("arguments")?;

    let type_text = type_args.utf8_text(source).ok()?;
    let inner_type = &type_text[1..type_text.len() - 1];
    let args_text = args.utf8_text(source).ok()?;
    let inner_args = &args_text[1..args_text.len() - 1];

    Some(Edit {
        start_byte: node.start_byte(),
        end_byte: node.end_byte(),
        replacement: format!("{prefix}{inner_type}[{inner_args}]"),
        rule: if func_text == "borrow_global" {
            "borrow_global"
        } else {
            "borrow_global_mut"
        },
    })
}

/// Try to match x = x op y → x op= y compound assignment patterns.
fn try_compound_assign_edit(node: tree_sitter::Node, source: &[u8]) -> Option<Edit> {
    let op_node = node.child_by_field_name("op")?;
    let op_text = op_node.utf8_text(source).ok()?;
    if op_text != "=" {
        return None; // Already a compound assignment
    }

    let lhs = node.child_by_field_name("lhs")?;
    let rhs = node.child_by_field_name("rhs")?;

    if rhs.kind() != "binary_expression" {
        return None;
    }

    let bin_lhs = rhs.child_by_field_name("lhs")?;
    let bin_op = rhs.child_by_field_name("operator")?;
    let bin_rhs = rhs.child_by_field_name("rhs")?;

    let lhs_text = lhs.utf8_text(source).ok()?;
    let bin_lhs_text = bin_lhs.utf8_text(source).ok()?;

    if lhs_text != bin_lhs_text {
        return None;
    }

    let bin_op_text = bin_op.utf8_text(source).ok()?;
    let compound_op = match bin_op_text {
        "+" => "+=",
        "-" => "-=",
        "*" => "*=",
        "/" => "/=",
        "%" => "%=",
        _ => return None,
    };

    let bin_rhs_text = bin_rhs.utf8_text(source).ok()?;

    Some(Edit {
        start_byte: node.start_byte(),
        end_byte: node.end_byte(),
        replacement: format!("{} {} {}", lhs_text, compound_op, bin_rhs_text),
        rule: "compound_assign",
    })
}

/// Get the full text of a call_expression's function (e.g., "vector::empty" or "push_back").
fn get_func_text<'a>(node: tree_sitter::Node, source: &'a [u8]) -> Option<&'a str> {
    node.child_by_field_name("function")?.utf8_text(source).ok()
}

/// Try to match vector::empty<T>() → vector<T>[].
fn try_vector_empty_edit(node: tree_sitter::Node, source: &[u8]) -> Option<Edit> {
    let func_text = get_func_text(node, source)?;
    if func_text != "vector::empty" {
        return None;
    }
    let args = node.child_by_field_name("arguments")?;
    let args_text = args.utf8_text(source).ok()?;
    if args_text != "()" {
        return None; // Not an empty argument list
    }
    let type_part = if let Some(ta) = node.child_by_field_name("type_arguments") {
        ta.utf8_text(source).ok()?
    } else {
        ""
    };
    Some(Edit {
        start_byte: node.start_byte(),
        end_byte: node.end_byte(),
        replacement: format!("vector{type_part}[]"),
        rule: "vector_empty",
    })
}

/// Try to strip redundant parentheses around a cast expression: (x as u64) → x as u64.
fn try_cast_paren_edit(node: tree_sitter::Node, source: &[u8]) -> Option<Edit> {
    if node.named_child_count() != 1 {
        return None;
    }
    let inner = node.named_child(0)?;
    if inner.kind() != "cast_expression" {
        return None;
    }
    let inner_text = inner.utf8_text(source).ok()?;
    Some(Edit {
        start_byte: node.start_byte(),
        end_byte: node.end_byte(),
        replacement: inner_text.to_string(),
        rule: "cast_parens",
    })
}

/// Strip the & or &mut prefix from a borrow_global replacement string.
fn strip_borrow_prefix(s: &str) -> String {
    if s.starts_with("&mut ") {
        s[5..].to_string()
    } else if s.starts_with('&') {
        s[1..].to_string()
    } else {
        s.to_string()
    }
}

fn collect_edits(node: tree_sitter::Node, source: &[u8], edits: &mut Vec<Edit>) {
    // *borrow_global<T>(addr) → T[addr]  (deref cancels the &)
    if node.kind() == "dereference_expression" {
        if let Some(inner) = node.named_child(0) {
            if inner.kind() == "call_expression" {
                if let Some(edit) = try_borrow_global_edit(inner, source) {
                    edits.push(Edit {
                        start_byte: node.start_byte(),
                        end_byte: node.end_byte(),
                        replacement: strip_borrow_prefix(&edit.replacement),
                        rule: "deref_borrow_global",
                    });
                    return;
                }
            }
        }
    }

    // &borrow_global<T>(addr) → &T[addr]  (absorb redundant outer &)
    if node.kind() == "borrow_expression" {
        if let Some(inner) = node.named_child(0) {
            if inner.kind() == "call_expression" {
                if let Some(edit) = try_borrow_global_edit(inner, source) {
                    edits.push(Edit {
                        start_byte: node.start_byte(),
                        end_byte: node.end_byte(),
                        ..edit
                    });
                    return;
                }
            }
        }
    }

    // borrow_global<T>(addr) → &T[addr]  (or T[addr] when followed by .field)
    if node.kind() == "call_expression" {
        if let Some(mut edit) = try_borrow_global_edit(node, source) {
            if should_strip_prefix(node) {
                edit.replacement = strip_borrow_prefix(&edit.replacement);
            }
            edits.push(edit);
            return;
        }
    }

    // vector::empty<T>() → vector<T>[]
    if node.kind() == "call_expression" {
        if let Some(edit) = try_vector_empty_edit(node, source) {
            edits.push(edit);
            return;
        }
    }

    // (x as u64) → x as u64  (redundant cast parens)
    if node.kind() == "parenthesized_expression" {
        if let Some(edit) = try_cast_paren_edit(node, source) {
            edits.push(edit);
            return;
        }
    }

    // x = x + y → x += y  (and other compound assignment patterns)
    if node.kind() == "assign_expression" {
        if let Some(edit) = try_compound_assign_edit(node, source) {
            edits.push(edit);
            return;
        }
    }

    // Strip acquires annotations
    if node.kind() == "acquires_clause" {
        let start = if let Some(prev) = node.prev_sibling() {
            prev.end_byte()
        } else {
            node.start_byte()
        };
        edits.push(Edit {
            start_byte: start,
            end_byte: node.end_byte(),
            replacement: String::new(),
            rule: "strip_acquires",
        });
        return;
    }

    // public(friend) → friend, public(package) → package
    // These are anonymous tokens in function/struct declarations.
    // We scan children for the pattern: "public" "(" "friend"/"package" ")"
    if node.kind() == "function_declaration" || node.kind() == "struct_declaration" {
        let count = node.child_count() as u32;
        for i in 0..count.saturating_sub(3) {
            let c0 = node.child(i).unwrap();
            let c1 = node.child(i + 1).unwrap();
            let c2 = node.child(i + 2).unwrap();
            let c3 = node.child(i + 3).unwrap();
            if c0.kind() == "public" && c1.kind() == "(" && c3.kind() == ")" {
                let inner = c2.utf8_text(source).unwrap_or("");
                if inner == "friend" || inner == "package" {
                    edits.push(Edit {
                        start_byte: c0.start_byte(),
                        end_byte: c3.end_byte(),
                        replacement: inner.to_string(),
                        rule: "visibility",
                    });
                }
            }
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_edits(child, source, edits);
    }
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: move1-to-move2 <file.move> [file2.move ...]");
        process::exit(1);
    }

    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_move_on_aptos::language())
        .expect("Error loading Move grammar");

    let mut total_edits = 0;
    let mut files_modified = 0;

    for path in &args[1..] {
        let source = match fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Error reading {}: {}", path, e);
                process::exit(1);
            }
        };

        let tree = match parser.parse(&source, None) {
            Some(t) => t,
            None => {
                eprintln!("Error parsing {}", path);
                process::exit(1);
            }
        };

        let mut edits = Vec::new();
        collect_edits(tree.root_node(), source.as_bytes(), &mut edits);

        if edits.is_empty() {
            continue;
        }

        let num_edits = edits.len();
        for edit in &edits {
            let line = source[..edit.start_byte].matches('\n').count() + 1;
            eprintln!("  {}:{}: [{}]", path, line, edit.rule);
        }

        let result = apply_edits(&source, edits);
        if let Err(e) = fs::write(path, &result) {
            eprintln!("Error writing {}: {}", path, e);
            process::exit(1);
        }

        total_edits += num_edits;
        files_modified += 1;
    }

    eprintln!("{} edit(s) across {} file(s)", total_edits, files_modified);
}
