use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::process;

use tools_base::{collect_move_files, new_move_parser, node_text};

// ─── Types ────────────────────────────────────────────────────

/// Fully-qualified function key → list of parameter names.
type FnMap = BTreeMap<String, Vec<String>>;

// ─── AST helpers ──────────────────────────────────────────────

/// Returns `true` if the function has an `entry_modifier` child.
fn is_entry(func: tree_sitter::Node) -> bool {
    let mut cursor = func.walk();
    func.children(&mut cursor)
        .any(|c| c.kind() == "entry_modifier")
}

/// Returns `true` if the function has a `#[view]` attribute.
fn is_view(func: tree_sitter::Node, source: &[u8]) -> bool {
    let mut cursor = func.walk();
    func.children(&mut cursor).any(|c| {
        if c.kind() == "attributes" {
            let mut ac = c.walk();
            c.named_children(&mut ac)
                .any(|attr| attr.kind() == "attribute" && node_text(attr, source) == "view")
        } else {
            false
        }
    })
}

/// Extract the module address and name from a `module_identity` node.
/// Returns `(address_text, module_name)`.
fn module_identity(node: tree_sitter::Node, source: &[u8]) -> Option<(String, String)> {
    let addr = node.child_by_field_name("address")?;
    let name = node.child_by_field_name("module")?;
    Some((node_text(addr, source), node_text(name, source)))
}

/// Collect parameter names from a `function_parameters` node.
fn param_names(params_node: tree_sitter::Node, source: &[u8]) -> Vec<String> {
    let mut cursor = params_node.walk();
    params_node
        .named_children(&mut cursor)
        .filter(|p| p.kind() == "function_parameter")
        .filter_map(|p| p.child_by_field_name("name"))
        .map(|n| node_text(n, source))
        .collect()
}

// ─── Tree walk ────────────────────────────────────────────────

fn extract_functions(
    node: tree_sitter::Node,
    source: &[u8],
    addr_override: Option<&str>,
    out: &mut FnMap,
) {
    match node.kind() {
        "module_declaration" => {
            let identity = node.child_by_field_name("name");
            let (addr, module_name) = identity
                .and_then(|id| module_identity(id, source))
                .unwrap_or_else(|| ("<unknown>".into(), "<unknown>".into()));

            let effective_addr = addr_override.unwrap_or(&addr);

            // Walk module body for functions.
            if let Some(body) = node.child_by_field_name("body") {
                let mut cursor = body.walk();
                for child in body.children(&mut cursor) {
                    if child.kind() == "function_declaration"
                        && (is_entry(child) || is_view(child, source))
                    {
                        let fn_name = child
                            .child_by_field_name("name")
                            .map(|n| node_text(n, source))
                            .unwrap_or_else(|| "<unknown>".into());

                        let args = child
                            .child_by_field_name("parameters")
                            .map(|p| param_names(p, source))
                            .unwrap_or_default();

                        let key = format!("{effective_addr}::{module_name}::{fn_name}");
                        out.insert(key, args);
                    }
                }
            }
            return; // Don't recurse into module children again.
        }
        "spec_block" => return,
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        extract_functions(child, source, addr_override, out);
    }
}

// ─── TypeScript output ────────────────────────────────────────

fn print_typescript(fns: &FnMap, name: &str, address: &str) {
    // Comment header.
    println!("/**");
    println!(" * {name} package on Aptos mainnet.");
    println!(" * Keys are every `entry` / `view` function; values are the source parameter names.");
    println!(" * Address: {address}");
    println!(" */");

    println!("export const {name}FunctionArgumentNameOverrides = {{",);

    for (key, args) in fns {
        let args_str: Vec<String> = args.iter().map(|a| format!("\"{a}\"")).collect();
        println!("  \"{key}\":");
        println!("    [{}],", args_str.join(", "));
    }

    println!("}} as const satisfies FunctionArgumentNameOverrideMap;");
}

// ─── CLI ──────────────────────────────────────────────────────

fn usage() {
    eprintln!("Usage: move-arg-names [--address <HEX>] [--name <IDENT>] <path>...");
    eprintln!();
    eprintln!("Extracts parameter names from entry/view functions in Move source files");
    eprintln!("and outputs a TypeScript map.");
    eprintln!();
    eprintln!("Options:");
    eprintln!("  --address <HEX>   Override the on-chain address (hex). Falls back to source.");
    eprintln!(
        "  --name <IDENT>    Identifier prefix for the exported const (e.g. 'decibelMainnet')."
    );
    eprintln!("  --help            Show this help.");
}

fn main() {
    let args: Vec<String> = env::args().collect();

    let mut address: Option<String> = None;
    let mut name = String::from("package");
    let mut paths: Vec<String> = Vec::new();

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--help" | "-h" => {
                usage();
                process::exit(0);
            }
            "--address" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("error: --address requires a value");
                    process::exit(2);
                }
                address = Some(args[i].clone());
            }
            "--name" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("error: --name requires a value");
                    process::exit(2);
                }
                name = args[i].clone();
            }
            arg if arg.starts_with("--address=") => {
                address = Some(arg.strip_prefix("--address=").unwrap().to_string());
            }
            arg if arg.starts_with("--name=") => {
                name = arg.strip_prefix("--name=").unwrap().to_string();
            }
            arg if arg.starts_with('-') => {
                eprintln!("error: unknown flag: {arg}");
                usage();
                process::exit(2);
            }
            _ => {
                paths.push(args[i].clone());
            }
        }
        i += 1;
    }

    if paths.is_empty() {
        eprintln!("error: no source paths provided");
        usage();
        process::exit(2);
    }

    let files = collect_move_files(&paths);
    if files.is_empty() {
        eprintln!("error: no .move files found in the given paths");
        process::exit(2);
    }

    let mut parser = new_move_parser();
    let mut fns = FnMap::new();

    for path in &files {
        let source = match fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("warning: cannot read {}: {e}", path.display());
                continue;
            }
        };
        let tree = match parser.parse(&source, None) {
            Some(t) => t,
            None => {
                eprintln!("warning: parse failed for {}", path.display());
                continue;
            }
        };
        extract_functions(
            tree.root_node(),
            source.as_bytes(),
            address.as_deref(),
            &mut fns,
        );
    }

    let effective_addr = address.as_deref().unwrap_or("<address>");
    print_typescript(&fns, &name, effective_addr);

    eprintln!(
        "Extracted {} entry/view functions from {} files.",
        fns.len(),
        files.len()
    );
}
