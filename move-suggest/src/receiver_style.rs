use tree_sitter::Node;

use crate::suggest::{Suggestion, get_args, is_in_consumed, parse_qualified_call};

/// How the first argument (receiver) is passed.
#[derive(Clone, Copy)]
enum ReceiverArg {
    BorrowRef,    // &v
    BorrowMutRef, // &mut v
    Owned,        // v
}

/// Special replacement forms (index notation instead of method call).
#[derive(Clone, Copy)]
enum SpecialForm {
    IndexRef,    // &v[i]
    IndexMutRef, // &mut v[i]
}

struct ReceiverRule {
    module: &'static str,
    function: &'static str,
    receiver_arg: ReceiverArg,
    extra_args: usize,
    special: Option<SpecialForm>,
}

const RECEIVER_RULES: &[ReceiverRule] = &[
    // vector methods
    ReceiverRule {
        module: "vector",
        function: "push_back",
        receiver_arg: ReceiverArg::BorrowMutRef,
        extra_args: 1,
        special: None,
    },
    ReceiverRule {
        module: "vector",
        function: "pop_back",
        receiver_arg: ReceiverArg::BorrowMutRef,
        extra_args: 0,
        special: None,
    },
    ReceiverRule {
        module: "vector",
        function: "length",
        receiver_arg: ReceiverArg::BorrowRef,
        extra_args: 0,
        special: None,
    },
    ReceiverRule {
        module: "vector",
        function: "is_empty",
        receiver_arg: ReceiverArg::BorrowRef,
        extra_args: 0,
        special: None,
    },
    ReceiverRule {
        module: "vector",
        function: "borrow",
        receiver_arg: ReceiverArg::BorrowRef,
        extra_args: 1,
        special: Some(SpecialForm::IndexRef),
    },
    ReceiverRule {
        module: "vector",
        function: "borrow_mut",
        receiver_arg: ReceiverArg::BorrowMutRef,
        extra_args: 1,
        special: Some(SpecialForm::IndexMutRef),
    },
    ReceiverRule {
        module: "vector",
        function: "contains",
        receiver_arg: ReceiverArg::BorrowRef,
        extra_args: 1,
        special: None,
    },
    ReceiverRule {
        module: "vector",
        function: "index_of",
        receiver_arg: ReceiverArg::BorrowRef,
        extra_args: 1,
        special: None,
    },
    ReceiverRule {
        module: "vector",
        function: "append",
        receiver_arg: ReceiverArg::BorrowMutRef,
        extra_args: 1,
        special: None,
    },
    ReceiverRule {
        module: "vector",
        function: "reverse",
        receiver_arg: ReceiverArg::BorrowMutRef,
        extra_args: 0,
        special: None,
    },
    ReceiverRule {
        module: "vector",
        function: "swap",
        receiver_arg: ReceiverArg::BorrowMutRef,
        extra_args: 2,
        special: None,
    },
    // string methods
    ReceiverRule {
        module: "string",
        function: "length",
        receiver_arg: ReceiverArg::BorrowRef,
        extra_args: 0,
        special: None,
    },
    ReceiverRule {
        module: "string",
        function: "bytes",
        receiver_arg: ReceiverArg::BorrowRef,
        extra_args: 0,
        special: None,
    },
    // option methods
    ReceiverRule {
        module: "option",
        function: "is_some",
        receiver_arg: ReceiverArg::BorrowRef,
        extra_args: 0,
        special: None,
    },
    ReceiverRule {
        module: "option",
        function: "is_none",
        receiver_arg: ReceiverArg::BorrowRef,
        extra_args: 0,
        special: None,
    },
    ReceiverRule {
        module: "option",
        function: "borrow",
        receiver_arg: ReceiverArg::BorrowRef,
        extra_args: 0,
        special: None,
    },
    ReceiverRule {
        module: "option",
        function: "borrow_mut",
        receiver_arg: ReceiverArg::BorrowMutRef,
        extra_args: 0,
        special: None,
    },
    ReceiverRule {
        module: "option",
        function: "extract",
        receiver_arg: ReceiverArg::BorrowMutRef,
        extra_args: 0,
        special: None,
    },
    ReceiverRule {
        module: "option",
        function: "contains",
        receiver_arg: ReceiverArg::BorrowRef,
        extra_args: 1,
        special: None,
    },
    ReceiverRule {
        module: "option",
        function: "swap",
        receiver_arg: ReceiverArg::BorrowMutRef,
        extra_args: 1,
        special: None,
    },
    ReceiverRule {
        module: "option",
        function: "destroy_some",
        receiver_arg: ReceiverArg::Owned,
        extra_args: 0,
        special: None,
    },
    ReceiverRule {
        module: "option",
        function: "destroy_none",
        receiver_arg: ReceiverArg::Owned,
        extra_args: 0,
        special: None,
    },
];

/// Extract the inner expression text from a borrow_expression (&v or &mut v → v).
fn strip_borrow<'a>(node: Node<'a>, source: &'a [u8]) -> Option<&'a str> {
    if node.kind() != "borrow_expression" {
        return None;
    }
    let inner = node.named_child(0)?;
    inner.utf8_text(source).ok()
}

/// Check if a borrow_expression uses &mut (vs &).
fn is_mut_borrow(node: Node, source: &[u8]) -> bool {
    node.utf8_text(source).unwrap_or("").starts_with("&mut")
}

fn try_receiver_style<'a>(node: Node<'a>, source: &'a [u8]) -> Option<Suggestion> {
    let func = node.child_by_field_name("function")?;
    let (module, member) = parse_qualified_call(func, source)?;

    let rule = RECEIVER_RULES
        .iter()
        .find(|r| r.module == module && r.function == member)?;

    let args_node = node.child_by_field_name("arguments")?;
    let args = get_args(args_node);

    if args.len() != 1 + rule.extra_args {
        return None;
    }

    let first_arg = args[0];

    // Extract receiver text based on expected argument form
    let receiver_text = match rule.receiver_arg {
        ReceiverArg::BorrowRef => {
            if first_arg.kind() != "borrow_expression" {
                return None;
            }
            if is_mut_borrow(first_arg, source) {
                return None;
            }
            strip_borrow(first_arg, source)?
        }
        ReceiverArg::BorrowMutRef => {
            if first_arg.kind() != "borrow_expression" {
                return None;
            }
            if !is_mut_borrow(first_arg, source) {
                return None;
            }
            strip_borrow(first_arg, source)?
        }
        ReceiverArg::Owned => first_arg.utf8_text(source).ok()?,
    };

    // Collect remaining argument texts
    let remaining: Vec<&str> = args[1..]
        .iter()
        .filter_map(|a| a.utf8_text(source).ok())
        .collect();

    // Build replacement
    let (replacement, message) = match rule.special {
        Some(SpecialForm::IndexRef) => {
            let idx = remaining.first()?;
            (
                format!("&{}[{}]", receiver_text, idx),
                format!(
                    "{}::{}(&{}, {}) can be written as &{}[{}]",
                    module, member, receiver_text, idx, receiver_text, idx
                ),
            )
        }
        Some(SpecialForm::IndexMutRef) => {
            let idx = remaining.first()?;
            (
                format!("&mut {}[{}]", receiver_text, idx),
                format!(
                    "{}::{}(&mut {}, {}) can be written as &mut {}[{}]",
                    module, member, receiver_text, idx, receiver_text, idx
                ),
            )
        }
        None => {
            let args_str = remaining.join(", ");
            let call = if args_str.is_empty() {
                format!("{}.{}()", receiver_text, member)
            } else {
                format!("{}.{}({})", receiver_text, member, args_str)
            };

            let orig_first = match rule.receiver_arg {
                ReceiverArg::BorrowRef => format!("&{}", receiver_text),
                ReceiverArg::BorrowMutRef => format!("&mut {}", receiver_text),
                ReceiverArg::Owned => receiver_text.to_string(),
            };
            let orig_args = if remaining.is_empty() {
                orig_first
            } else {
                format!("{}, {}", orig_first, remaining.join(", "))
            };

            (
                call.clone(),
                format!(
                    "{}::{}({}) can be written as {}",
                    module, member, orig_args, call
                ),
            )
        }
    };

    Some(Suggestion {
        start_byte: node.start_byte(),
        end_byte: node.end_byte(),
        replacement,
        rule: "receiver_style",
        message,
    })
}

pub fn collect_receiver_style(
    node: Node,
    source: &[u8],
    suggestions: &mut Vec<Suggestion>,
    consumed: &[(usize, usize)],
) {
    if is_in_consumed(node, consumed) {
        return;
    }
    if node.kind() == "call_expression"
        && let Some(s) = try_receiver_style(node, source)
    {
        suggestions.push(s);
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_receiver_style(child, source, suggestions, consumed);
    }
}
