use tree_sitter::Node;

use crate::suggest::{get_args, is_in_consumed, parse_qualified_call, Suggestion};

/// Check if a borrow_expression uses &mut (vs &).
fn is_mut_borrow(node: Node, source: &[u8]) -> bool {
    node.utf8_text(source)
        .unwrap_or("")
        .starts_with("&mut")
}

// ── Simple cases ────────────────────────────────────────────────────────────

fn try_vector_empty<'a>(node: Node<'a>, source: &'a [u8]) -> Option<Suggestion> {
    let func = node.child_by_field_name("function")?;
    let (module, member) = parse_qualified_call(func, source)?;
    if module != "vector" || member != "empty" {
        return None;
    }

    let args = node.child_by_field_name("arguments")?;
    if !get_args(args).is_empty() {
        return None;
    }

    let type_args_text = node
        .child_by_field_name("type_arguments")
        .and_then(|ta| ta.utf8_text(source).ok())
        .unwrap_or("");

    let replacement = format!("vector{}[]", type_args_text);

    Some(Suggestion {
        start_byte: node.start_byte(),
        end_byte: node.end_byte(),
        replacement,
        rule: "vector_empty_literal",
        message: "vector::empty() can be written as vector[]".into(),
    })
}

fn try_vector_singleton<'a>(node: Node<'a>, source: &'a [u8]) -> Option<Suggestion> {
    let func = node.child_by_field_name("function")?;
    let (module, member) = parse_qualified_call(func, source)?;
    if module != "vector" || member != "singleton" {
        return None;
    }

    let args = node.child_by_field_name("arguments")?;
    let arg_nodes = get_args(args);
    if arg_nodes.len() != 1 {
        return None;
    }

    let elem = arg_nodes[0].utf8_text(source).ok()?;
    let replacement = format!("vector[{}]", elem);

    Some(Suggestion {
        start_byte: node.start_byte(),
        end_byte: node.end_byte(),
        replacement: replacement.clone(),
        rule: "vector_singleton_literal",
        message: format!("vector::singleton({}) can be written as {}", elem, replacement),
    })
}

/// Collect simple vector literal suggestions (empty + singleton), skipping consumed ranges.
pub fn collect_simple_vector(
    node: Node,
    source: &[u8],
    suggestions: &mut Vec<Suggestion>,
    consumed: &[(usize, usize)],
) {
    if is_in_consumed(node, consumed) {
        return;
    }
    if node.kind() == "call_expression" {
        if let Some(s) = try_vector_empty(node, source) {
            suggestions.push(s);
            return;
        }
        if let Some(s) = try_vector_singleton(node, source) {
            suggestions.push(s);
            return;
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_simple_vector(child, source, suggestions, consumed);
    }
}

// ── Multi-push pattern ──────────────────────────────────────────────────────

/// Match `let v = vector::empty<T>()` — returns (var_name, optional_type_args_text).
fn match_vector_empty_let<'a>(node: Node<'a>, source: &'a [u8]) -> Option<(String, String)> {
    if node.kind() != "let_expression" {
        return None;
    }
    let binds = node.child_by_field_name("binds")?;
    if binds.kind() != "bind_var" {
        return None;
    }
    let var_name = binds.named_child(0)?.utf8_text(source).ok()?.to_string();

    let value = node.child_by_field_name("value")?;
    if value.kind() != "call_expression" {
        return None;
    }
    let func = value.child_by_field_name("function")?;
    let (module, member) = parse_qualified_call(func, source)?;
    if module != "vector" || member != "empty" {
        return None;
    }
    let args = value.child_by_field_name("arguments")?;
    if !get_args(args).is_empty() {
        return None;
    }

    let type_args = value
        .child_by_field_name("type_arguments")
        .and_then(|ta| ta.utf8_text(source).ok())
        .unwrap_or("")
        .to_string();

    Some((var_name, type_args))
}

/// Match `push_back(&mut var, elem)` or `vector::push_back(&mut var, elem)`.
/// Returns the element text if matched.
fn match_push_back_on_var<'a>(
    node: Node<'a>,
    var_name: &str,
    source: &'a [u8],
) -> Option<String> {
    if node.kind() != "call_expression" {
        return None;
    }
    let func = node.child_by_field_name("function")?;

    // Check function name: either "push_back" or "vector::push_back"
    let mut identifiers = Vec::new();
    let mut cursor = func.walk();
    for child in func.named_children(&mut cursor) {
        if child.kind() == "identifier" {
            identifiers.push(child.utf8_text(source).ok()?);
        }
    }
    let is_push_back = match identifiers.as_slice() {
        ["push_back"] | ["vector", "push_back"] => true,
        _ => false,
    };
    if !is_push_back {
        return None;
    }

    let args_node = node.child_by_field_name("arguments")?;
    let args = get_args(args_node);
    if args.len() != 2 {
        return None;
    }

    // First arg must be &mut {var_name}
    let first = args[0];
    if first.kind() != "borrow_expression" || !is_mut_borrow(first, source) {
        return None;
    }
    let inner = first.named_child(0)?;
    if inner.utf8_text(source).ok()? != var_name {
        return None;
    }

    // Second arg is the element
    let elem = args[1].utf8_text(source).ok()?.to_string();
    Some(elem)
}

/// Scan all blocks in the tree for multi-push patterns.
/// Returns consumed byte ranges so other rules can skip them.
pub fn collect_multi_push(
    node: Node,
    source: &[u8],
    suggestions: &mut Vec<Suggestion>,
    consumed: &mut Vec<(usize, usize)>,
) {
    if node.kind() == "block" {
        scan_block_for_multi_push(node, source, suggestions, consumed);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_multi_push(child, source, suggestions, consumed);
    }
}

fn scan_block_for_multi_push(
    block: Node,
    source: &[u8],
    suggestions: &mut Vec<Suggestion>,
    consumed: &mut Vec<(usize, usize)>,
) {
    let child_count = block.child_count();
    let mut i: usize = 0;

    while i < child_count {
        let child = match block.child(i as u32) {
            Some(c) => c,
            None => {
                i += 1;
                continue;
            }
        };

        if child.kind() == "let_expression" {
            if let Some((var_name, type_args)) = match_vector_empty_let(child, source) {
                let let_start = child.start_byte();
                let mut let_end = child.end_byte();
                let mut elements: Vec<String> = Vec::new();

                // Skip past the let's trailing semicolon
                let mut j = i + 1;
                if j < child_count {
                    if let Some(semi) = block.child(j as u32) {
                        if semi.kind() == ";" {
                            let_end = semi.end_byte();
                            j += 1;
                        }
                    }
                }

                // Scan for consecutive push_back calls
                while j < child_count {
                    let stmt = match block.child(j as u32) {
                        Some(s) => s,
                        None => break,
                    };
                    if stmt.kind() == "call_expression" {
                        if let Some(elem) = match_push_back_on_var(stmt, &var_name, source) {
                            elements.push(elem);
                            // Look for trailing semicolon
                            if j + 1 < child_count {
                                if let Some(semi) = block.child((j + 1) as u32) {
                                    if semi.kind() == ";" {
                                        let_end = semi.end_byte();
                                        j += 2;
                                        continue;
                                    }
                                }
                            }
                            let_end = stmt.end_byte();
                            j += 1;
                            continue;
                        }
                    }
                    break;
                }

                if elements.len() >= 2 {
                    let elems_str = elements.join(", ");
                    let replacement = format!("let {} = vector{}[{}];", var_name, type_args, elems_str);
                    suggestions.push(Suggestion {
                        start_byte: let_start,
                        end_byte: let_end,
                        replacement,
                        rule: "vector_multi_push",
                        message: format!(
                            "vector::empty() followed by {} push_back calls can be written as vector[{}]",
                            elements.len(),
                            elems_str,
                        ),
                    });
                    consumed.push((let_start, let_end));
                    i = j;
                    continue;
                }
            }
        }
        i += 1;
    }
}
