use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::process;

use rayon::prelude::*;
use regex::Regex;
use tools_base::{collect_move_files, count_named_children, line_col, new_move_parser};

// ─── Configuration ─────────────────────────────────────────────

struct BoundsConfig {
    max_loop_depth: usize,
    max_generic_instantiation_length: usize,
    max_function_parameters: usize,
    max_basic_blocks: usize,
    max_type_nodes: usize,
    max_function_return_values: usize,
    max_type_depth: usize,
    max_struct_definitions: usize,
    max_struct_variants: usize,
    max_fields_in_struct: usize,
    max_function_definitions: usize,
    max_identifier_length: usize,
    max_locals: usize,
    max_type_parameter_count: usize,
}

impl Default for BoundsConfig {
    fn default() -> Self {
        // Values from aptos-core/aptos-move/aptos-vm-environment/src/prod_configs.rs
        // with enable_function_values=true (current mainnet production config).
        Self {
            max_loop_depth: 5,
            max_generic_instantiation_length: 32,
            max_function_parameters: 128,
            max_basic_blocks: 1024,
            max_type_nodes: 128,
            max_function_return_values: 128,
            max_type_depth: 20,
            max_struct_definitions: 200,
            max_struct_variants: 64,
            max_fields_in_struct: 64,
            max_function_definitions: 1000,
            max_identifier_length: 255,
            max_locals: 255,
            max_type_parameter_count: 255,
        }
    }
}

// ─── Violations ────────────────────────────────────────────────

struct Violation {
    kind: &'static str,
    entity_kind: &'static str,
    entity: String,
    actual: usize,
    limit: usize,
    line: usize,
    col: usize,
}

// ─── Helpers ───────────────────────────────────────────────────

fn is_type_node(kind: &str) -> bool {
    matches!(
        kind,
        "apply_type"
            | "primitive_type"
            | "ref_type"
            | "tuple_type"
            | "function_type"
            // When `>>` is ambiguous (depth-2 nested generics like `extract<B<A>>()`),
            // tree-sitter misparses the call as a binary_expression. The inner type
            // `B<A>` appears as a `generic_name_expression` instead of `apply_type`.
            | "generic_name_expression"
    )
}

// ─── Check 1: Loop Depth ───────────────────────────────────────

fn max_loop_depth(node: tree_sitter::Node, depth: usize) -> usize {
    let is_loop = matches!(
        node.kind(),
        "while_expression" | "loop_expression" | "for_expression"
    );
    let new_depth = if is_loop { depth + 1 } else { depth };

    let mut max = new_depth;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let d = max_loop_depth(child, new_depth);
        if d > max {
            max = d;
        }
    }
    max
}

// ─── Check 2: Generic Instantiation Length ─────────────────────

fn max_type_arguments_length(node: tree_sitter::Node) -> usize {
    let mut max = 0;
    if node.kind() == "type_arguments" {
        let count = count_named_children(node);
        if count > max {
            max = count;
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let d = max_type_arguments_length(child);
        if d > max {
            max = d;
        }
    }
    max
}

// ─── Check 4: Basic Blocks (Heuristic) ────────────────────────

fn estimate_basic_blocks(node: tree_sitter::Node) -> usize {
    let mut count = 1; // entry block
    count_blocks_inner(node, &mut count);
    count
}

fn count_blocks_inner(node: tree_sitter::Node, count: &mut usize) {
    match node.kind() {
        "if_expression" | "while_expression" | "for_expression" => *count += 2,
        "loop_expression" | "break_expression" | "continue_expression" => *count += 1,
        _ => {}
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        count_blocks_inner(child, count);
    }
}

// ─── Check 5: Type Node Count ─────────────────────────────────

fn count_type_nodes(node: tree_sitter::Node) -> usize {
    let kind = node.kind();
    let mut count = if is_type_node(kind) { 1 } else { 0 };

    // In `x is Foo | Bar`, the native compiler counts each variant as a type node.
    // Tree-sitter represents variants as name_access_chain children of is_variant_list.
    if kind == "is_variant_list" {
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            if child.kind() == "name_access_chain" {
                count += 1;
            }
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        count += count_type_nodes(child);
    }
    count
}

// ─── Check 7: Type Depth ──────────────────────────────────────

fn compute_type_depth(node: tree_sitter::Node) -> usize {
    if is_type_node(node.kind()) {
        let mut max_child = 0;
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            let d = compute_type_depth(child);
            if d > max_child {
                max_child = d;
            }
        }
        1 + max_child
    } else {
        // Not a type node — pass through (e.g. type_arguments wrapper)
        let mut max = 0;
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            let d = compute_type_depth(child);
            if d > max {
                max = d;
            }
        }
        max
    }
}

// ─── Check 8/11: Module Declaration Counts ──────────────────────

fn count_module_declarations(node: tree_sitter::Node, structs: &mut usize, functions: &mut usize) {
    if node.kind() == "spec_block" {
        return;
    }
    match node.kind() {
        "struct_declaration" | "enum_declaration" => *structs += 1,
        "function_declaration" => *functions += 1,
        _ => {}
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        count_module_declarations(child, structs, functions);
    }
}

// ─── Check 9: Enum Variant Count ────────────────────────────────

fn count_variants(node: tree_sitter::Node) -> usize {
    let mut count = 0;
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() == "enum_variant" {
            count += 1;
        } else {
            count += count_variants(child);
        }
    }
    count
}

// ─── Check 10: Fields in Struct/Variant ─────────────────────────

fn count_fields(node: tree_sitter::Node) -> usize {
    if node.kind() == "field_declaration" {
        return 1;
    }
    let mut count = 0;
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        count += count_fields(child);
    }
    count
}

fn check_variant_fields(
    node: tree_sitter::Node,
    source: &str,
    config: &BoundsConfig,
    violations: &mut Vec<Violation>,
    enum_name: &str,
) {
    if node.kind() == "enum_variant" {
        let field_count = count_fields(node);
        if field_count > config.max_fields_in_struct {
            let variant_name = node
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(source.as_bytes()).ok())
                .unwrap_or("<unknown>");
            let (line, col) = line_col(source, node.start_byte());
            violations.push(Violation {
                kind: "max_fields_in_struct",
                entity_kind: "variant",
                entity: format!("{}::{}", enum_name, variant_name),
                actual: field_count,
                limit: config.max_fields_in_struct,
                line,
                col,
            });
        }
        return;
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        check_variant_fields(child, source, config, violations, enum_name);
    }
}

// ─── Check 13: Local Variable Count ────────────────────────────

fn count_lets(node: tree_sitter::Node) -> usize {
    let mut count = 0;
    if node.kind() == "let_expression" {
        count += 1;
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        count += count_lets(child);
    }
    count
}

// ─── Main Check Logic ──────────────────────────────────────────

fn check_file(tree: &tree_sitter::Tree, source: &str, config: &BoundsConfig) -> Vec<Violation> {
    let mut violations = Vec::new();
    walk_declarations(tree.root_node(), source, config, &mut violations);
    violations
}

fn walk_declarations(
    node: tree_sitter::Node,
    source: &str,
    config: &BoundsConfig,
    violations: &mut Vec<Violation>,
) {
    match node.kind() {
        "spec_block" => return,
        "module_declaration" => check_module(node, source, config, violations),
        "function_declaration" => check_function(node, source, config, violations),
        "struct_declaration" | "enum_declaration" => check_struct(node, source, config, violations),
        _ => {}
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_declarations(child, source, config, violations);
    }
}

fn check_module(
    node: tree_sitter::Node,
    source: &str,
    config: &BoundsConfig,
    violations: &mut Vec<Violation>,
) {
    let full_name = node
        .child_by_field_name("name")
        .and_then(|n| n.utf8_text(source.as_bytes()).ok())
        .unwrap_or("<unknown>")
        .to_string();
    let (line, col) = line_col(source, node.start_byte());

    let mut struct_count = 0;
    let mut function_count = 0;
    count_module_declarations(node, &mut struct_count, &mut function_count);

    // Check 8: struct/enum definitions per module
    if struct_count > config.max_struct_definitions {
        violations.push(Violation {
            kind: "max_struct_definitions",
            entity_kind: "module",
            entity: full_name.clone(),
            actual: struct_count,
            limit: config.max_struct_definitions,
            line,
            col,
        });
    }

    // Check 11: function definitions per module
    if function_count > config.max_function_definitions {
        violations.push(Violation {
            kind: "max_function_definitions",
            entity_kind: "module",
            entity: full_name.clone(),
            actual: function_count,
            limit: config.max_function_definitions,
            line,
            col,
        });
    }

    // Check 12: module name identifier length
    let ident = full_name.rsplit("::").next().unwrap_or(&full_name);
    if ident.len() > config.max_identifier_length {
        violations.push(Violation {
            kind: "max_identifier_length",
            entity_kind: "module",
            entity: full_name.clone(),
            actual: ident.len(),
            limit: config.max_identifier_length,
            line,
            col,
        });
    }
}

fn check_function(
    node: tree_sitter::Node,
    source: &str,
    config: &BoundsConfig,
    violations: &mut Vec<Violation>,
) {
    let name = node
        .child_by_field_name("name")
        .and_then(|n| n.utf8_text(source.as_bytes()).ok())
        .unwrap_or("<unknown>")
        .to_string();
    let (line, col) = line_col(source, node.start_byte());
    let body = node.child_by_field_name("body");

    // Check 3: function parameters
    if let Some(params) = node.child_by_field_name("parameters") {
        let count = count_named_children(params);
        if count > config.max_function_parameters {
            violations.push(Violation {
                kind: "max_function_parameters",
                entity_kind: "function",
                entity: name.clone(),
                actual: count,
                limit: config.max_function_parameters,
                line,
                col,
            });
        }
    }

    // Check 6: return values
    if let Some(ret) = node.child_by_field_name("return_type")
        && ret.kind() == "tuple_type"
    {
        let count = count_named_children(ret);
        if count > config.max_function_return_values {
            violations.push(Violation {
                kind: "max_function_return_values",
                entity_kind: "function",
                entity: name.clone(),
                actual: count,
                limit: config.max_function_return_values,
                line,
                col,
            });
        }
    }

    // Check 2: generic instantiation length
    // Check both type_parameters on the declaration and type_arguments everywhere
    let max_generic = {
        let mut max = 0;
        if let Some(tp) = node.child_by_field_name("type_parameters") {
            max = count_named_children(tp);
        }
        let ta = max_type_arguments_length(node);
        if ta > max {
            max = ta;
        }
        max
    };
    if max_generic > config.max_generic_instantiation_length {
        violations.push(Violation {
            kind: "max_generic_instantiation_length",
            entity_kind: "function",
            entity: name.clone(),
            actual: max_generic,
            limit: config.max_generic_instantiation_length,
            line,
            col,
        });
    }

    // Body-dependent checks (skip native functions)
    if let Some(body) = body {
        // Check 1: loop depth
        let depth = max_loop_depth(body, 0);
        if depth > config.max_loop_depth {
            violations.push(Violation {
                kind: "max_loop_depth",
                entity_kind: "function",
                entity: name.clone(),
                actual: depth,
                limit: config.max_loop_depth,
                line,
                col,
            });
        }

        // Check 4: basic blocks (heuristic)
        let blocks = estimate_basic_blocks(body);
        if blocks > config.max_basic_blocks {
            violations.push(Violation {
                kind: "max_basic_blocks",
                entity_kind: "function",
                entity: name.clone(),
                actual: blocks,
                limit: config.max_basic_blocks,
                line,
                col,
            });
        }

        // Check 5: type nodes (across entire function scope)
        let mut tn = count_type_nodes(body);
        if let Some(params) = node.child_by_field_name("parameters") {
            tn += count_type_nodes(params);
        }
        if let Some(ret) = node.child_by_field_name("return_type") {
            tn += count_type_nodes(ret);
        }
        if tn > config.max_type_nodes {
            violations.push(Violation {
                kind: "max_type_nodes",
                entity_kind: "function",
                entity: name.clone(),
                actual: tn,
                limit: config.max_type_nodes,
                line,
                col,
            });
        }

        // Check 7: type depth (across entire function scope)
        let mut td = compute_type_depth(body);
        if let Some(params) = node.child_by_field_name("parameters") {
            let d = compute_type_depth(params);
            if d > td {
                td = d;
            }
        }
        if let Some(ret) = node.child_by_field_name("return_type") {
            let d = compute_type_depth(ret);
            if d > td {
                td = d;
            }
        }
        if td > config.max_type_depth {
            violations.push(Violation {
                kind: "max_type_depth",
                entity_kind: "function",
                entity: name.clone(),
                actual: td,
                limit: config.max_type_depth,
                line,
                col,
            });
        }

        // Check 13: locals (let bindings + parameters)
        let param_count = node
            .child_by_field_name("parameters")
            .map(count_named_children)
            .unwrap_or(0);
        let let_count = count_lets(body);
        let total_locals = param_count + let_count;
        if total_locals > config.max_locals {
            violations.push(Violation {
                kind: "max_locals",
                entity_kind: "function",
                entity: name.clone(),
                actual: total_locals,
                limit: config.max_locals,
                line,
                col,
            });
        }
    }

    // Check 12: identifier length
    if name.len() > config.max_identifier_length {
        violations.push(Violation {
            kind: "max_identifier_length",
            entity_kind: "function",
            entity: name.clone(),
            actual: name.len(),
            limit: config.max_identifier_length,
            line,
            col,
        });
    }

    // Check 14: type parameter count
    if let Some(tp) = node.child_by_field_name("type_parameters") {
        let count = count_named_children(tp);
        if count > config.max_type_parameter_count {
            violations.push(Violation {
                kind: "max_type_parameter_count",
                entity_kind: "function",
                entity: name.clone(),
                actual: count,
                limit: config.max_type_parameter_count,
                line,
                col,
            });
        }
    }
}

fn check_struct(
    node: tree_sitter::Node,
    source: &str,
    config: &BoundsConfig,
    violations: &mut Vec<Violation>,
) {
    let kind_label = if node.kind() == "enum_declaration" {
        "enum"
    } else {
        "struct"
    };
    let name = node
        .child_by_field_name("name")
        .and_then(|n| n.utf8_text(source.as_bytes()).ok())
        .unwrap_or("<unknown>")
        .to_string();
    let (line, col) = line_col(source, node.start_byte());

    // Check 2: generic instantiation length on declaration + field types
    let max_generic = {
        let mut max = 0;
        if let Some(tp) = node.child_by_field_name("type_parameters") {
            max = count_named_children(tp);
        }
        let ta = max_type_arguments_length(node);
        if ta > max {
            max = ta;
        }
        max
    };
    if max_generic > config.max_generic_instantiation_length {
        violations.push(Violation {
            kind: "max_generic_instantiation_length",
            entity_kind: kind_label,
            entity: name.clone(),
            actual: max_generic,
            limit: config.max_generic_instantiation_length,
            line,
            col,
        });
    }

    // Check 12: identifier length
    if name.len() > config.max_identifier_length {
        violations.push(Violation {
            kind: "max_identifier_length",
            entity_kind: kind_label,
            entity: name.clone(),
            actual: name.len(),
            limit: config.max_identifier_length,
            line,
            col,
        });
    }

    // Check 14: type parameter count
    if let Some(tp) = node.child_by_field_name("type_parameters") {
        let count = count_named_children(tp);
        if count > config.max_type_parameter_count {
            violations.push(Violation {
                kind: "max_type_parameter_count",
                entity_kind: kind_label,
                entity: name.clone(),
                actual: count,
                limit: config.max_type_parameter_count,
                line,
                col,
            });
        }
    }

    if node.kind() == "enum_declaration" {
        // Check 9: enum variants
        let variant_count = count_variants(node);
        if variant_count > config.max_struct_variants {
            violations.push(Violation {
                kind: "max_struct_variants",
                entity_kind: kind_label,
                entity: name.clone(),
                actual: variant_count,
                limit: config.max_struct_variants,
                line,
                col,
            });
        }

        // Check 10: fields per variant
        check_variant_fields(node, source, config, violations, &name);
    } else {
        // Check 10: fields in struct
        let field_count = count_fields(node);
        if field_count > config.max_fields_in_struct {
            violations.push(Violation {
                kind: "max_fields_in_struct",
                entity_kind: kind_label,
                entity: name.clone(),
                actual: field_count,
                limit: config.max_fields_in_struct,
                line,
                col,
            });
        }
    }
}

// ─── Address Identification ─────────────────────────────────────

const GITHUB_RAW_URL: &str =
    "https://raw.githubusercontent.com/aptos-labs/explorer/main/app/data/mainnet/knownAddresses.ts";

fn parse_labels(content: &str) -> (HashMap<String, String>, HashMap<String, String>) {
    let re = Regex::new(r#""(0x[0-9a-fA-F]+)":\s*\n?\s*"([^"]+)""#).unwrap();
    let mut known = HashMap::new();
    let mut scam = HashMap::new();

    let (known_section, scam_section) = match content.find("ScamAddresses") {
        Some(pos) => (&content[..pos], &content[pos..]),
        None => (content, ""),
    };

    for cap in re.captures_iter(known_section) {
        known.insert(cap[1].to_lowercase(), cap[2].to_string());
    }
    for cap in re.captures_iter(scam_section) {
        scam.insert(cap[1].to_lowercase(), cap[2].to_string());
    }

    (known, scam)
}

fn fetch_labels_from_github() -> (HashMap<String, String>, HashMap<String, String>) {
    eprintln!("Fetching labels from GitHub...");
    let body = match ureq::get(GITHUB_RAW_URL).call() {
        Ok(response) => match response.into_body().read_to_string() {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Failed to read response: {}", e);
                return (HashMap::new(), HashMap::new());
            }
        },
        Err(e) => {
            eprintln!("Error fetching from GitHub: {}", e);
            return (HashMap::new(), HashMap::new());
        }
    };
    parse_labels(&body)
}

fn load_labels_from_local(
    explorer_path: &str,
) -> (HashMap<String, String>, HashMap<String, String>) {
    let label_file = PathBuf::from(explorer_path).join("app/data/mainnet/knownAddresses.ts");

    match fs::read_to_string(&label_file) {
        Ok(content) => parse_labels(&content),
        Err(_) => {
            eprintln!("Warning: label file not found: {}", label_file.display());
            (HashMap::new(), HashMap::new())
        }
    }
}

fn extract_source_file(path: &std::path::Path) -> String {
    let path_str = path.to_string_lossy();
    if let Some(pos) = path_str.find("/sources/") {
        path_str[pos + 9..].to_string()
    } else {
        path.file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_else(|| path_str.to_string())
    }
}

fn print_identify_report(
    results: &[(PathBuf, Vec<Violation>)],
    known: &HashMap<String, String>,
    scam: &HashMap<String, String>,
    show_all: bool,
) {
    let addr_re = Regex::new(r"0x[0-9a-fA-F]{64}").unwrap();

    // Group violations by address
    struct ViolInfo {
        source_file: String,
        line: usize,
        entity_kind: &'static str,
        entity: String,
        violation_kind: &'static str,
        actual: usize,
        limit: usize,
    }

    let mut by_addr: HashMap<String, Vec<ViolInfo>> = HashMap::new();
    for (path, violations) in results {
        let path_str = path.display().to_string();
        if let Some(m) = addr_re.find(&path_str) {
            let addr = m.as_str().to_string();
            let source_file = extract_source_file(path);
            for v in violations {
                by_addr.entry(addr.clone()).or_default().push(ViolInfo {
                    source_file: source_file.clone(),
                    line: v.line,
                    entity_kind: v.entity_kind,
                    entity: v.entity.clone(),
                    violation_kind: v.kind,
                    actual: v.actual,
                    limit: v.limit,
                });
            }
        }
    }

    let total_violations: usize = by_addr.values().map(|vs| vs.len()).sum();
    let total_addrs = by_addr.len();

    let mut labeled_addrs: Vec<(String, &Vec<ViolInfo>)> = Vec::new();
    let mut scam_addrs: Vec<(String, &Vec<ViolInfo>)> = Vec::new();
    let mut unlabeled_addrs: Vec<(String, &Vec<ViolInfo>)> = Vec::new();

    for (addr, vs) in &by_addr {
        let lower = addr.to_lowercase();
        if scam.contains_key(&lower) {
            scam_addrs.push((addr.clone(), vs));
        } else if known.contains_key(&lower) {
            labeled_addrs.push((addr.clone(), vs));
        } else {
            unlabeled_addrs.push((addr.clone(), vs));
        }
    }

    // Sort labeled by violation count descending
    labeled_addrs.sort_by(|a, b| b.1.len().cmp(&a.1.len()));
    scam_addrs.sort_by(|a, b| {
        let la = scam
            .get(&a.0.to_lowercase())
            .map(|s| s.as_str())
            .unwrap_or("");
        let lb = scam
            .get(&b.0.to_lowercase())
            .map(|s| s.as_str())
            .unwrap_or("");
        la.cmp(lb)
    });
    unlabeled_addrs.sort_by(|a, b| b.1.len().cmp(&a.1.len()));

    // ── Header ──
    println!("{}", "=".repeat(72));
    println!("  Move Bounds Checker \u{2014} Address Identification Report");
    println!("{}", "=".repeat(72));
    println!();
    println!("  Total violations : {}", total_violations);
    println!("  Unique addresses : {}", total_addrs);
    println!("  Labeled (known)  : {}", labeled_addrs.len());
    println!("  Flagged (scam)   : {}", scam_addrs.len());
    println!("  Unlabeled        : {}", unlabeled_addrs.len());
    println!();

    // ── Scam Addresses ──
    if !scam_addrs.is_empty() {
        println!("{}", "\u{2500}".repeat(72));
        println!("  \u{26a0} SCAM-FLAGGED ADDRESSES");
        println!("{}", "\u{2500}".repeat(72));
        for (addr, vs) in &scam_addrs {
            let label = scam
                .get(&addr.to_lowercase())
                .map(|s| s.as_str())
                .unwrap_or("Unknown Scam");
            println!();
            println!("  [{}] {}", label, addr);
            println!("  {} violation(s):", vs.len());
            for v in *vs {
                println!(
                    "    {}:{} \u{2014} {} '{}' exceeds {} ({} > {})",
                    v.source_file,
                    v.line,
                    v.entity_kind,
                    v.entity,
                    v.violation_kind,
                    v.actual,
                    v.limit
                );
            }
        }
    }

    // ── Labeled Addresses ──
    if !labeled_addrs.is_empty() {
        println!();
        println!("{}", "\u{2500}".repeat(72));
        println!("  LABELED ADDRESSES WITH VIOLATIONS");
        println!("{}", "\u{2500}".repeat(72));

        for (addr, vs) in &labeled_addrs {
            let label = known
                .get(&addr.to_lowercase())
                .map(|s| s.as_str())
                .unwrap_or("Unknown");
            println!();
            println!("  {}", label);
            println!("  {}", addr);

            let mut kind_counts: HashMap<&str, usize> = HashMap::new();
            for v in *vs {
                *kind_counts.entry(v.violation_kind).or_insert(0) += 1;
            }
            let mut sorted_kinds: Vec<_> = kind_counts.iter().collect();
            sorted_kinds.sort_by_key(|(k, _)| *k);
            let summary: Vec<String> = sorted_kinds
                .iter()
                .map(|(k, c)| format!("{}: {}", k, c))
                .collect();
            println!(
                "  {} violation(s) \u{2014} {}",
                vs.len(),
                summary.join(", ")
            );

            for v in *vs {
                println!(
                    "    {}:{} \u{2014} {} '{}' exceeds {} ({} > {})",
                    v.source_file,
                    v.line,
                    v.entity_kind,
                    v.entity,
                    v.violation_kind,
                    v.actual,
                    v.limit
                );
            }
        }
    }

    // ── Summary Table ──
    if !labeled_addrs.is_empty() {
        println!();
        println!("{}", "\u{2500}".repeat(72));
        println!("  SUMMARY: LABELED ADDRESSES");
        println!("{}", "\u{2500}".repeat(72));
        println!();
        println!(
            "  {:<30} {:>10}  {:<25}",
            "Label", "Violations", "Top Issue"
        );
        println!(
            "  {} {}  {}",
            "\u{2500}".repeat(30),
            "\u{2500}".repeat(10),
            "\u{2500}".repeat(25)
        );

        for (addr, vs) in &labeled_addrs {
            let label = known
                .get(&addr.to_lowercase())
                .map(|s| s.as_str())
                .unwrap_or("Unknown");
            let mut kind_counts: HashMap<&str, usize> = HashMap::new();
            for v in *vs {
                *kind_counts.entry(v.violation_kind).or_insert(0) += 1;
            }
            let top_kind = kind_counts
                .iter()
                .max_by_key(|(_, c)| **c)
                .map(|(k, _)| *k)
                .unwrap_or("");
            println!("  {:<30} {:>10}  {:<25}", label, vs.len(), top_kind);
        }
    }

    // ── Unlabeled Addresses ──
    println!();
    println!("{}", "\u{2500}".repeat(72));
    println!("  UNLABELED ADDRESSES");
    println!("{}", "\u{2500}".repeat(72));
    let unlabeled_total: usize = unlabeled_addrs.iter().map(|(_, vs)| vs.len()).sum();
    println!(
        "  {} address(es) with {} total violation(s)",
        unlabeled_addrs.len(),
        unlabeled_total
    );
    println!();

    if show_all && !unlabeled_addrs.is_empty() {
        for (addr, vs) in &unlabeled_addrs {
            println!("  {}", addr);
            let mut kind_counts: HashMap<&str, usize> = HashMap::new();
            for v in *vs {
                *kind_counts.entry(v.violation_kind).or_insert(0) += 1;
            }
            let mut sorted_kinds: Vec<_> = kind_counts.iter().collect();
            sorted_kinds.sort_by_key(|(k, _)| *k);
            let summary: Vec<String> = sorted_kinds
                .iter()
                .map(|(k, c)| format!("{}: {}", k, c))
                .collect();
            println!(
                "  {} violation(s) \u{2014} {}",
                vs.len(),
                summary.join(", ")
            );
            for v in *vs {
                println!(
                    "    {}:{} \u{2014} {} '{}' exceeds {} ({} > {})",
                    v.source_file,
                    v.line,
                    v.entity_kind,
                    v.entity,
                    v.violation_kind,
                    v.actual,
                    v.limit
                );
            }
            println!();
        }
    } else if !unlabeled_addrs.is_empty() {
        println!("  {:<68} {:>3}", "Address", "#");
        println!("  {} {}", "\u{2500}".repeat(68), "\u{2500}".repeat(3));
        for (addr, vs) in unlabeled_addrs.iter().take(20) {
            println!("  {:<68} {:>3}", addr, vs.len());
        }
        if unlabeled_addrs.len() > 20 {
            println!("  ... and {} more", unlabeled_addrs.len() - 20);
            println!("  (use --all to show every address with full violations)");
        }
    }

    println!();
    println!("{}", "=".repeat(72));
}

// ─── CLI ───────────────────────────────────────────────────────

type ConfigOverride = fn(&mut BoundsConfig, usize);

fn parse_override(arg: &str, config: &mut BoundsConfig) -> bool {
    let overrides: &[(&str, ConfigOverride)] = &[
        ("--max-loop-depth=", |c, v| c.max_loop_depth = v),
        ("--max-generic-instantiation-length=", |c, v| {
            c.max_generic_instantiation_length = v
        }),
        ("--max-function-parameters=", |c, v| {
            c.max_function_parameters = v
        }),
        ("--max-basic-blocks=", |c, v| c.max_basic_blocks = v),
        ("--max-type-nodes=", |c, v| c.max_type_nodes = v),
        ("--max-function-return-values=", |c, v| {
            c.max_function_return_values = v
        }),
        ("--max-type-depth=", |c, v| c.max_type_depth = v),
        ("--max-struct-definitions=", |c, v| {
            c.max_struct_definitions = v
        }),
        ("--max-struct-variants=", |c, v| c.max_struct_variants = v),
        ("--max-fields-in-struct=", |c, v| c.max_fields_in_struct = v),
        ("--max-function-definitions=", |c, v| {
            c.max_function_definitions = v
        }),
        ("--max-identifier-length=", |c, v| {
            c.max_identifier_length = v
        }),
        ("--max-locals=", |c, v| c.max_locals = v),
        ("--max-type-parameter-count=", |c, v| {
            c.max_type_parameter_count = v
        }),
    ];
    for (prefix, setter) in overrides {
        if let Some(val) = arg.strip_prefix(prefix) {
            match val.parse::<usize>() {
                Ok(v) => {
                    setter(config, v);
                    return true;
                }
                Err(_) => {
                    eprintln!(
                        "Invalid value for {}: {}",
                        prefix.trim_end_matches('='),
                        val
                    );
                    process::exit(2);
                }
            }
        }
    }
    false
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let mut config = BoundsConfig::default();
    let mut paths = Vec::new();
    let mut identify = false;
    let mut show_all = false;
    let mut explorer_local: Option<String> = None;

    for arg in &args[1..] {
        if arg == "--identify" {
            identify = true;
        } else if arg == "--all" {
            show_all = true;
        } else if let Some(val) = arg.strip_prefix("--explorer-local=") {
            explorer_local = Some(val.to_string());
        } else if arg.starts_with("--") {
            if !parse_override(arg, &mut config) {
                eprintln!("Unknown option: {}", arg);
                process::exit(2);
            }
        } else {
            paths.push(arg.clone());
        }
    }

    if paths.is_empty() {
        eprintln!(
            "Usage: move-bounds-checker <dir> [--identify] [--all] [--explorer-local=PATH] [--max-loop-depth=N ...]"
        );
        process::exit(2);
    }

    // Collect .move files
    let files: Vec<PathBuf> = collect_move_files(&paths);

    eprintln!("Scanning {} file(s)...", files.len());

    // Process files in parallel, one Parser per thread
    let results: Vec<(PathBuf, Vec<Violation>)> = files
        .par_iter()
        .map_init(new_move_parser, |parser, path| {
            let source = match fs::read_to_string(path) {
                Ok(s) => s,
                Err(_) => return (path.clone(), Vec::new()),
            };
            let tree = match parser.parse(&source, None) {
                Some(t) => t,
                None => return (path.clone(), Vec::new()),
            };
            let violations = check_file(&tree, &source, &config);
            (path.clone(), violations)
        })
        .collect();

    // Output
    let total: usize = results.iter().map(|(_, vs)| vs.len()).sum();

    if identify {
        let (known, scam) = if let Some(ref local) = explorer_local {
            let labels = load_labels_from_local(local);
            eprintln!(
                "Loaded {} known + {} scam labels from {}",
                labels.0.len(),
                labels.1.len(),
                local
            );
            labels
        } else {
            let labels = fetch_labels_from_github();
            eprintln!(
                "Loaded {} known + {} scam labels from GitHub",
                labels.0.len(),
                labels.1.len()
            );
            labels
        };
        print_identify_report(&results, &known, &scam, show_all);
    } else {
        let mut by_kind: HashMap<&str, usize> = HashMap::new();
        for (path, violations) in &results {
            for v in violations {
                println!(
                    "{}:{}:{}: {} '{}' exceeds {} ({} > {})",
                    path.display(),
                    v.line,
                    v.col,
                    v.entity_kind,
                    v.entity,
                    v.kind,
                    v.actual,
                    v.limit,
                );
                *by_kind.entry(v.kind).or_insert(0) += 1;
            }
        }
        if !by_kind.is_empty() {
            let mut sorted: Vec<_> = by_kind.iter().collect();
            sorted.sort_by_key(|(_, count)| std::cmp::Reverse(**count));
            for (kind, count) in sorted {
                eprintln!("  {}: {}", kind, count);
            }
        }
    }

    eprintln!(
        "{} file(s) scanned, {} violation(s) found",
        files.len(),
        total
    );

    process::exit(if total > 0 { 1 } else { 0 });
}
