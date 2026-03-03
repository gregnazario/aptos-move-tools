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

/// Try to match vector::borrow(&v, i) → &v[i] or vector::borrow_mut(&mut v, i) → &mut v[i].
fn try_vector_borrow_edit(node: tree_sitter::Node, source: &[u8]) -> Option<Edit> {
    let func_text = get_func_text(node, source)?;
    let prefix = match func_text {
        "vector::borrow" => "&",
        "vector::borrow_mut" => "&mut ",
        _ => return None,
    };
    let args = node.child_by_field_name("arguments")?;
    if args.named_child_count() != 2 {
        return None;
    }
    let first_arg = args.named_child(0)?;
    let idx_arg = args.named_child(1)?;

    // Strip borrow from first arg if it's a borrow_expression
    let obj_text = if first_arg.kind() == "borrow_expression" {
        first_arg.named_child(0)?.utf8_text(source).ok()?
    } else {
        first_arg.utf8_text(source).ok()?
    };

    let idx_text = idx_arg.utf8_text(source).ok()?;

    Some(Edit {
        start_byte: node.start_byte(),
        end_byte: node.end_byte(),
        replacement: format!("{prefix}{obj_text}[{idx_text}]"),
        rule: if func_text == "vector::borrow" {
            "vector_borrow"
        } else {
            "vector_borrow_mut"
        },
    })
}

/// Known stdlib functions that support receiver-style (dot) calls.
/// These have a `self`/`&self`/`&mut self` first parameter in their Move 2 definition.
/// Note: vector::borrow, vector::borrow_mut, and vector::empty are handled separately
/// by dedicated transforms (index syntax and vector literal).
fn is_receiver_style_func(name: &str) -> bool {
    matches!(
        name,
        // vector
        "vector::push_back"
            | "vector::pop_back"
            | "vector::length"
            | "vector::is_empty"
            | "vector::contains"
            | "vector::index_of"
            | "vector::append"
            | "vector::reverse"
            | "vector::swap"
            | "vector::remove"
            | "vector::swap_remove"
            | "vector::destroy_empty"
            | "vector::for_each"
            | "vector::for_each_ref"
            | "vector::for_each_mut"
            | "vector::map"
            | "vector::map_ref"
            | "vector::filter"
            | "vector::zip"
            | "vector::fold"
            | "vector::any"
            | "vector::all"
            | "vector::enumerate_ref"
            | "vector::enumerate_mut"
            | "vector::flatten"
            | "vector::trim"
            | "vector::trim_reverse"
            // option
            | "option::is_some"
            | "option::is_none"
            | "option::borrow"
            | "option::borrow_mut"
            | "option::borrow_with_default"
            | "option::get_with_default"
            | "option::extract"
            | "option::fill"
            | "option::swap"
            | "option::swap_or_fill"
            | "option::contains"
            | "option::destroy_some"
            | "option::destroy_none"
            | "option::destroy_with_default"
            // string / string_utils
            | "string::length"
            | "string::bytes"
            | "string::is_empty"
            | "string::sub_string"
            | "string::append"
            | "string::append_utf8"
            | "string::insert"
            // table
            | "table::add"
            | "table::borrow"
            | "table::borrow_mut"
            | "table::borrow_with_default"
            | "table::contains"
            | "table::remove"
            | "table::upsert"
            | "table::destroy"
            // smart_table
            | "smart_table::add"
            | "smart_table::borrow"
            | "smart_table::borrow_mut"
            | "smart_table::borrow_with_default"
            | "smart_table::contains"
            | "smart_table::remove"
            | "smart_table::length"
            | "smart_table::upsert"
            | "smart_table::destroy_empty"
            | "smart_table::destroy"
            // smart_vector
            | "smart_vector::push_back"
            | "smart_vector::pop_back"
            | "smart_vector::length"
            | "smart_vector::is_empty"
            | "smart_vector::borrow"
            | "smart_vector::borrow_mut"
            | "smart_vector::append"
            | "smart_vector::contains"
            | "smart_vector::destroy_empty"
            | "smart_vector::remove"
            | "smart_vector::swap_remove"
            // simple_map
            | "simple_map::borrow"
            | "simple_map::borrow_mut"
            | "simple_map::contains_key"
            | "simple_map::add"
            | "simple_map::remove"
            | "simple_map::length"
            | "simple_map::keys"
            | "simple_map::values"
            | "simple_map::destroy_empty"
            | "simple_map::upsert"
            // coin
            | "coin::value"
            | "coin::merge"
            | "coin::extract"
            | "coin::extract_all"
    )
}

/// Try to convert module::func(first_arg, rest...) → first_arg.func(rest...).
fn try_receiver_style_edit(node: tree_sitter::Node, source: &[u8]) -> Option<Edit> {
    let func_text = get_func_text(node, source)?;

    // Must be a module-qualified call (contains ::)
    let colon_pos = func_text.find("::")?;
    let func_name = &func_text[colon_pos + 2..];

    if !is_receiver_style_func(func_text) {
        return None;
    }

    let args = node.child_by_field_name("arguments")?;
    let arg_count = args.named_child_count() as u32;
    if arg_count == 0 {
        return None;
    }

    let first_arg = args.named_child(0)?;

    // Strip borrow from first arg if it's a borrow_expression (compiler auto-borrows)
    let obj_text = if first_arg.kind() == "borrow_expression" {
        first_arg.named_child(0)?.utf8_text(source).ok()?
    } else {
        first_arg.utf8_text(source).ok()?
    };

    // Extract remaining arguments by byte range (preserves original formatting)
    let rest_args = if arg_count > 1 {
        let second = args.named_child(1)?;
        let last = args.named_child(arg_count - 1)?;
        std::str::from_utf8(&source[second.start_byte()..last.end_byte()]).ok()?
    } else {
        ""
    };

    // In receiver style, type args need :: prefix: v.remove::<u64>(i)
    let type_args_str = if let Some(ta) = node.child_by_field_name("type_arguments") {
        format!("::{}", ta.utf8_text(source).ok()?)
    } else {
        String::new()
    };

    Some(Edit {
        start_byte: node.start_byte(),
        end_byte: node.end_byte(),
        replacement: format!("{obj_text}.{func_name}{type_args_str}({rest_args})"),
        rule: "receiver_style",
    })
}

/// Try to strip redundant parentheses around a cast expression: (x as u64) → x as u64.
/// Only strips when the cast is in an isolated position (function argument, let/assign
/// value, block return). Keeps parens when the cast is an operand of a binary expression,
/// comparison, or other context where removing parens could affect readability or semantics.
fn try_cast_paren_edit(node: tree_sitter::Node, source: &[u8]) -> Option<Edit> {
    if node.named_child_count() != 1 {
        return None;
    }
    let inner = node.named_child(0)?;
    if inner.kind() != "cast_expression" {
        return None;
    }

    // Only strip in contexts where the cast stands alone
    let parent = node.parent()?;
    let safe = match parent.kind() {
        "arg_list" => true,
        "let_expression" => true,
        "assign_expression" => true,
        "block" => true,
        "return_expression" => true,
        _ => false,
    };
    if !safe {
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
    // *vector::borrow(&v, i) → v[i]  (deref cancels the &)
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
                if let Some(edit) = try_vector_borrow_edit(inner, source) {
                    edits.push(Edit {
                        start_byte: node.start_byte(),
                        end_byte: node.end_byte(),
                        replacement: strip_borrow_prefix(&edit.replacement),
                        rule: "deref_vector_borrow",
                    });
                    return;
                }
            }
        }
    }

    // &borrow_global<T>(addr) → &T[addr]  (absorb redundant outer &)
    // &vector::borrow(&v, i) → &v[i]  (absorb redundant outer &)
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
                if let Some(edit) = try_vector_borrow_edit(inner, source) {
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
    // vector::borrow(&v, i) → &v[i]  (or v[i] when followed by .field)
    if node.kind() == "call_expression" {
        if let Some(mut edit) = try_borrow_global_edit(node, source) {
            if should_strip_prefix(node) {
                edit.replacement = strip_borrow_prefix(&edit.replacement);
            }
            edits.push(edit);
            return;
        }
        if let Some(mut edit) = try_vector_borrow_edit(node, source) {
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

    // module::func(obj, ...) → obj.func(...)  (receiver-style stdlib calls)
    if node.kind() == "call_expression" {
        if let Some(edit) = try_receiver_style_edit(node, source) {
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
        let mut source = match fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Error reading {}: {}", path, e);
                process::exit(1);
            }
        };

        let mut file_edits = 0;

        // Multi-pass: nested transforms (e.g., vector::borrow wrapping borrow_global)
        // may leave inner expressions untransformed on the first pass.
        loop {
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
                break;
            }

            for edit in &edits {
                let line = source[..edit.start_byte].matches('\n').count() + 1;
                eprintln!("  {}:{}: [{}]", path, line, edit.rule);
            }

            file_edits += edits.len();
            source = apply_edits(&source, edits);
        }

        if file_edits == 0 {
            continue;
        }

        if let Err(e) = fs::write(path, &source) {
            eprintln!("Error writing {}: {}", path, e);
            process::exit(1);
        }

        total_edits += file_edits;
        files_modified += 1;
    }

    eprintln!("{} edit(s) across {} file(s)", total_edits, files_modified);
}
