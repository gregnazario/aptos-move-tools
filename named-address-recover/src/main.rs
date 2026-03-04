use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::env;
use std::fs;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process;

use flate2::read::GzDecoder;
use tools_base::{new_move_parser, node_text};
use walkdir::WalkDir;

// ─── Move.toml Parsing ─────────────────────────────────────────

fn parse_move_toml(path: &Path) -> BTreeMap<String, String> {
    let content = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(_) => return BTreeMap::new(),
    };
    let table: toml::Table = match content.parse() {
        Ok(t) => t,
        Err(_) => return BTreeMap::new(),
    };
    let mut addrs = BTreeMap::new();
    if let Some(toml::Value::Table(addr_table)) = table.get("addresses") {
        for (name, val) in addr_table {
            if let toml::Value::String(hex) = val {
                if hex != "_" {
                    addrs.insert(name.clone(), hex.clone());
                }
            }
        }
    }
    addrs
}

// ─── Source Scanning (tree-sitter) ──────────────────────────────

const KEYWORD_BUILTINS: &[&str] = &[
    "vector",
    "exists",
    "for",
    "match",
    "Self",
    "self",
    "true",
    "false",
    "abort",
    "return",
    "break",
    "continue",
    "if",
    "else",
    "while",
    "loop",
    "let",
    "mut",
    "copy",
    "move",
    "has",
    "fun",
    "struct",
    "module",
    "use",
    "public",
    "friend",
    "native",
    "const",
    "spec",
    "schema",
    "include",
    "apply",
    "pragma",
    "global",
    "local",
    "assert",
    "assume",
    "ensures",
    "requires",
    "invariant",
    "modifies",
    "acquires",
    "entry",
    "inline",
    "enum",
];

/// Returns a map of named_address → set of module names used with it.
fn scan_source_named_addresses(
    source: &str,
    parser: &mut tree_sitter::Parser,
) -> HashMap<String, BTreeSet<String>> {
    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return HashMap::new(),
    };
    let mut result: HashMap<String, BTreeSet<String>> = HashMap::new();
    let source_bytes = source.as_bytes();

    walk_tree(tree.root_node(), source_bytes, &mut result);
    result
}

fn walk_tree(
    node: tree_sitter::Node,
    source: &[u8],
    result: &mut HashMap<String, BTreeSet<String>>,
) {
    match node.kind() {
        // module_identity: address::module (in use statements and module declarations)
        "module_identity" => {
            if let (Some(addr_node), Some(mod_node)) = (
                node.child_by_field_name("address"),
                node.child_by_field_name("module"),
            ) {
                if addr_node.kind() == "identifier" {
                    let addr_name = node_text(addr_node, source);
                    let mod_name = node_text(mod_node, source);
                    if !is_keyword(&addr_name) {
                        result.entry(addr_name).or_default().insert(mod_name);
                    }
                }
            }
        }
        // address_block: address name { module ... }
        "address_block" => {
            if let Some(addr_node) = node.child_by_field_name("address") {
                if addr_node.kind() == "identifier" {
                    let addr_name = node_text(addr_node, source);
                    if !is_keyword(&addr_name) {
                        // Collect module names declared inside
                        collect_modules_in_address_block(node, source, &addr_name, result);
                    }
                }
            }
        }
        // friend_declaration contains a name_access_chain
        // name_access_chain with 2+ segments: first segment may be named address
        "name_access_chain" => {
            handle_name_access_chain(node, source, result);
        }
        // address_literal: @name
        "annotation_expression" | "address_literal" => {
            // @identifier could be a named address literal
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "identifier" {
                    let name = node_text(child, source);
                    if !is_keyword(&name) {
                        // No module name associated, just record the named address
                        result.entry(name).or_default();
                    }
                }
            }
        }
        _ => {}
    }

    // Recurse into children
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_tree(child, source, result);
    }
}

fn handle_name_access_chain(
    node: tree_sitter::Node,
    source: &[u8],
    result: &mut HashMap<String, BTreeSet<String>>,
) {
    // A name_access_chain like `addr::module::item` or `addr::module`
    // The first child is the leading name access, followed by :: and identifiers
    let mut cursor = node.walk();
    let children: Vec<tree_sitter::Node> = node.children(&mut cursor).collect();

    // We need at least 3 children for a qualified path: name :: name
    // Look for pattern: identifier "::" identifier
    if children.len() >= 3 {
        let first = children[0];
        if first.kind() == "identifier"
            && children.len() >= 3
            && node_text(children[1], source) == "::"
        {
            let addr_name = node_text(first, source);
            let mod_name = node_text(children[2], source);
            if !is_keyword(&addr_name) {
                result.entry(addr_name).or_default().insert(mod_name);
            }
        }
    }
}

fn collect_modules_in_address_block(
    node: tree_sitter::Node,
    source: &[u8],
    addr_name: &str,
    result: &mut HashMap<String, BTreeSet<String>>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "module_definition" || child.kind() == "module" {
            // Look for the module name
            if let Some(name_node) = child.child_by_field_name("name") {
                let mod_name = node_text(name_node, source);
                result
                    .entry(addr_name.to_string())
                    .or_default()
                    .insert(mod_name);
            }
        }
        // Recurse to find nested module definitions
        if child.kind() != "module_definition" && child.kind() != "module" {
            collect_modules_in_address_block(child, source, addr_name, result);
        }
    }
}

fn is_keyword(name: &str) -> bool {
    KEYWORD_BUILTINS.contains(&name)
}

// ─── Bytecode Scanning ─────────────────────────────────────────

use move_binary_format::CompiledModule;
use move_binary_format::access::ModuleAccess;
use move_binary_format::internals::ModuleIndex;

/// Returns (hex_address, module_name) pairs from a compiled .mv file.
fn scan_bytecode_addresses(bytes: &[u8]) -> Vec<(String, String)> {
    let module = match CompiledModule::deserialize(bytes) {
        Ok(m) => m,
        Err(_) => return Vec::new(),
    };

    let mut pairs = Vec::new();

    // Self module
    let self_addr = module.address().to_standard_string();
    let self_name = module.name().to_string();
    pairs.push((self_addr, self_name));

    // All module handles (includes imports)
    for handle in module.module_handles() {
        let addr = module.address_identifiers()[handle.address.into_index()].to_standard_string();
        let name = module.identifiers()[handle.name.into_index()].to_string();
        pairs.push((addr, name));
    }

    // Friend declarations (same structure as module handles)
    for handle in module.friend_decls() {
        let addr = module.address_identifiers()[handle.address.into_index()].to_standard_string();
        let name = module.identifiers()[handle.name.into_index()].to_string();
        pairs.push((addr, name));
    }

    pairs.sort();
    pairs.dedup();
    pairs
}

// ─── Cross-reference Resolution ────────────────────────────────

struct ResolvedAddress {
    name: String,
    hex: String,
    source: &'static str,
}

fn resolve_addresses(
    toml_addrs: &BTreeMap<String, String>,
    source_addrs: &HashMap<String, BTreeSet<String>>,
    bytecode_pairs: &[(String, String)],
) -> Vec<ResolvedAddress> {
    let mut resolved: BTreeMap<String, ResolvedAddress> = BTreeMap::new();

    // Priority 1: Move.toml direct mappings
    for (name, hex) in toml_addrs {
        resolved.insert(
            name.clone(),
            ResolvedAddress {
                name: name.clone(),
                hex: hex.clone(),
                source: "Move.toml",
            },
        );
    }

    // Build reverse index: module_name → hex_address (from bytecode)
    let mut module_to_hex: HashMap<String, String> = HashMap::new();
    for (hex, mod_name) in bytecode_pairs {
        module_to_hex
            .entry(mod_name.clone())
            .or_insert_with(|| hex.clone());
    }

    // Priority 2: Module-name matching
    // If source says `use my_addr::cool_module` and bytecode has `cool_module` at `0xABCD`,
    // then `my_addr = 0xABCD`
    for (named_addr, modules) in source_addrs {
        if resolved.contains_key(named_addr) {
            continue;
        }
        // Try each module name associated with this named address
        let mut candidate_hex: Option<String> = None;
        for mod_name in modules {
            if let Some(hex) = module_to_hex.get(mod_name) {
                match &candidate_hex {
                    None => candidate_hex = Some(hex.clone()),
                    Some(prev) if prev == hex => {} // consistent
                    Some(_) => {
                        // Conflicting addresses for different modules under same named address
                        // This shouldn't happen in well-formed packages, skip
                        candidate_hex = None;
                        break;
                    }
                }
            }
        }
        if let Some(hex) = candidate_hex {
            resolved.insert(
                named_addr.clone(),
                ResolvedAddress {
                    name: named_addr.clone(),
                    hex,
                    source: "bytecode-match",
                },
            );
        }
    }

    // Priority 3: Well-known Aptos addresses (only if name was seen in source)
    let well_known: &[(&str, &str)] = &[
        ("std", "0x1"),
        ("aptos_std", "0x1"),
        ("aptos_framework", "0x1"),
        ("aptos_token", "0x3"),
        ("aptos_token_objects", "0x4"),
    ];
    for (name, hex) in well_known {
        if !resolved.contains_key(*name) && source_addrs.contains_key(*name) {
            resolved.insert(
                name.to_string(),
                ResolvedAddress {
                    name: name.to_string(),
                    hex: hex.to_string(),
                    source: "well-known",
                },
            );
        }
    }

    resolved.into_values().collect()
}

// ─── File Discovery ─────────────────────────────────────────────

struct PackageFiles {
    move_toml: Option<PathBuf>,
    source_files: Vec<PathBuf>,
    bytecode_files: Vec<PathBuf>,
}

fn discover_files(root: &Path) -> PackageFiles {
    let mut pkg = PackageFiles {
        move_toml: None,
        source_files: Vec::new(),
        bytecode_files: Vec::new(),
    };

    // Check for Move.toml at root
    let toml_path = root.join("Move.toml");
    if toml_path.exists() {
        pkg.move_toml = Some(toml_path);
    }

    // Walk directory for .move and .mv files
    for entry in WalkDir::new(root)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
    {
        let path = entry.path();
        match path.extension().and_then(|e| e.to_str()) {
            Some("move") => pkg.source_files.push(path.to_path_buf()),
            Some("mv") => pkg.bytecode_files.push(path.to_path_buf()),
            _ => {}
        }
    }

    pkg
}

// ─── Output Formatting ─────────────────────────────────────────

fn print_default(results: &[ResolvedAddress]) {
    for r in results {
        println!("{} = {}", r.name, r.hex);
    }
}

fn print_toml(results: &[ResolvedAddress]) {
    println!("[addresses]");
    for r in results {
        println!("{} = \"{}\"", r.name, r.hex);
    }
}

fn print_verbose(results: &[ResolvedAddress]) {
    for r in results {
        println!("{} = {}  ({})", r.name, r.hex, r.source);
    }
}

// ─── On-Chain Fetching ──────────────────────────────────────────

const MAINNET_API: &str = "https://api.mainnet.aptoslabs.com/v1";

fn hex_decode(hex: &str) -> Option<Vec<u8>> {
    let hex = hex.strip_prefix("0x").unwrap_or(hex);
    if hex.is_empty() {
        return None;
    }
    let mut bytes = Vec::with_capacity(hex.len() / 2);
    for i in (0..hex.len()).step_by(2) {
        let byte = u8::from_str_radix(&hex[i..i + 2], 16).ok()?;
        bytes.push(byte);
    }
    Some(bytes)
}

/// Fetch all compiled module bytecodes for an on-chain account.
/// Handles cursor-based pagination for accounts with many modules.
fn fetch_modules_bytecode(address: &str) -> Vec<Vec<u8>> {
    let mut all_bytecodes = Vec::new();
    let mut cursor: Option<String> = None;

    loop {
        let url = match &cursor {
            None => format!("{}/accounts/{}/modules?limit=1000", MAINNET_API, address),
            Some(c) => format!(
                "{}/accounts/{}/modules?limit=1000&start={}",
                MAINNET_API, address, c
            ),
        };

        eprintln!("Fetching modules from {}...", url);
        let response = match ureq::get(&url).call() {
            Ok(r) => r,
            Err(e) => {
                eprintln!("Error fetching modules: {}", e);
                process::exit(1);
            }
        };

        // Check for pagination cursor before consuming body
        let next_cursor = response
            .headers()
            .get("x-aptos-cursor")
            .and_then(|v| v.to_str().ok())
            .map(|v| v.to_string());

        let body = match response.into_body().read_to_string() {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Error reading response: {}", e);
                process::exit(1);
            }
        };

        let modules: Vec<serde_json::Value> = match serde_json::from_str(&body) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("Error parsing modules JSON: {}", e);
                process::exit(1);
            }
        };

        for module in &modules {
            if let Some(bytecode_hex) = module.get("bytecode").and_then(|v| v.as_str()) {
                if let Some(bytes) = hex_decode(bytecode_hex) {
                    all_bytecodes.push(bytes);
                }
            }
        }

        eprintln!("  fetched {} module(s) in this page", modules.len());

        match next_cursor {
            Some(c) if !modules.is_empty() => cursor = Some(c),
            _ => break,
        }
    }

    all_bytecodes
}

/// Fetch and decompress all source files from the PackageRegistry resource.
fn fetch_package_sources(address: &str) -> Vec<String> {
    let url = format!(
        "{}/accounts/{}/resource/0x1::code::PackageRegistry",
        MAINNET_API, address
    );

    eprintln!("Fetching source from {}...", url);
    let response = match ureq::get(&url).call() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Warning: could not fetch PackageRegistry: {}", e);
            return Vec::new();
        }
    };

    let body = match response.into_body().read_to_string() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Warning: could not read PackageRegistry response: {}", e);
            return Vec::new();
        }
    };

    let json: serde_json::Value = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("Warning: could not parse PackageRegistry JSON: {}", e);
            return Vec::new();
        }
    };

    let mut sources = Vec::new();

    let packages = match json
        .get("data")
        .and_then(|d| d.get("packages"))
        .and_then(|p| p.as_array())
    {
        Some(pkgs) => pkgs,
        None => return sources,
    };

    for pkg in packages {
        let modules = match pkg.get("modules").and_then(|m| m.as_array()) {
            Some(m) => m,
            None => continue,
        };
        for module in modules {
            let source_hex = match module.get("source").and_then(|s| s.as_str()) {
                Some(s) => s,
                None => continue,
            };
            // Skip empty source ("0x" or "")
            let stripped = source_hex.strip_prefix("0x").unwrap_or(source_hex);
            if stripped.is_empty() {
                continue;
            }
            let compressed = match hex_decode(source_hex) {
                Some(b) => b,
                None => continue,
            };
            let mut decoder = GzDecoder::new(&compressed[..]);
            let mut source_text = String::new();
            if decoder.read_to_string(&mut source_text).is_ok() {
                sources.push(source_text);
            }
        }
    }

    eprintln!("  fetched {} source file(s)", sources.len());
    sources
}

// ─── CLI ────────────────────────────────────────────────────────

fn main() {
    let args: Vec<String> = env::args().collect();
    let mut path: Option<String> = None;
    let mut address: Option<String> = None;
    let mut source_path: Option<String> = None;
    let mut toml_output = false;
    let mut verbose = false;

    let mut iter = args[1..].iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--address" => {
                let val = iter.next().unwrap_or_else(|| {
                    eprintln!("Error: --address requires a value");
                    process::exit(2);
                });
                if address.is_some() {
                    eprintln!("Error: --address specified multiple times");
                    process::exit(2);
                }
                address = Some(val.to_string());
            }
            s if s.starts_with("--address=") => {
                if address.is_some() {
                    eprintln!("Error: --address specified multiple times");
                    process::exit(2);
                }
                address = Some(s["--address=".len()..].to_string());
            }
            "--source" => {
                let val = iter.next().unwrap_or_else(|| {
                    eprintln!("Error: --source requires a value");
                    process::exit(2);
                });
                if source_path.is_some() {
                    eprintln!("Error: --source specified multiple times");
                    process::exit(2);
                }
                source_path = Some(val.to_string());
            }
            s if s.starts_with("--source=") => {
                if source_path.is_some() {
                    eprintln!("Error: --source specified multiple times");
                    process::exit(2);
                }
                source_path = Some(s["--source=".len()..].to_string());
            }
            "--toml" => toml_output = true,
            "--verbose" => verbose = true,
            "--help" | "-h" => {
                eprintln!("Usage: named-address-recover <path> [--toml] [--verbose]");
                eprintln!(
                    "       named-address-recover --address 0xHEX [--source <path>] [--toml] [--verbose]"
                );
                eprintln!();
                eprintln!(
                    "Recovers named address mappings from a Move package by cross-referencing"
                );
                eprintln!("Move.toml, source files, and compiled bytecode.");
                eprintln!();
                eprintln!("Options:");
                eprintln!(
                    "  --address 0xHEX  Fetch modules from Aptos mainnet instead of local path"
                );
                eprintln!(
                    "  --source <path>  Supplement with local .move source files (use with --address)"
                );
                eprintln!("  --toml           Output as [addresses] TOML section");
                eprintln!("  --verbose        Show source attribution for each mapping");
                process::exit(0);
            }
            other if other.starts_with('-') => {
                eprintln!("Unknown option: {}", other);
                process::exit(2);
            }
            other => {
                if path.is_some() {
                    eprintln!("Error: multiple paths provided");
                    process::exit(2);
                }
                path = Some(other.to_string());
            }
        }
    }

    if path.is_some() && address.is_some() {
        eprintln!("Error: cannot use both <path> and --address");
        process::exit(2);
    }
    if source_path.is_some() && address.is_none() {
        eprintln!("Error: --source can only be used with --address");
        process::exit(2);
    }

    // Shared state for resolution
    let toml_addrs: BTreeMap<String, String>;
    let mut all_source_addrs: HashMap<String, BTreeSet<String>> = HashMap::new();
    let mut all_bytecode_pairs: Vec<(String, String)> = Vec::new();

    let mut parser = new_move_parser();

    if let Some(addr) = address {
        // ─── On-chain mode ──────────────────────────────────────
        eprintln!("Fetching on-chain data for {}...", addr);

        let bytecodes = fetch_modules_bytecode(&addr);
        eprintln!("Total bytecode modules: {}", bytecodes.len());

        for bytes in &bytecodes {
            all_bytecode_pairs.extend(scan_bytecode_addresses(bytes));
        }
        all_bytecode_pairs.sort();
        all_bytecode_pairs.dedup();

        let sources = fetch_package_sources(&addr);
        for source in &sources {
            let file_addrs = scan_source_named_addresses(source, &mut parser);
            for (name, modules) in file_addrs {
                all_source_addrs.entry(name).or_default().extend(modules);
            }
        }

        // Supplement with local source files if --source provided
        if let Some(ref src_path) = source_path {
            let src_root = PathBuf::from(src_path);
            if !src_root.exists() {
                eprintln!("Error: source path does not exist: {}", src_root.display());
                process::exit(2);
            }
            let mut local_count = 0;
            for entry in WalkDir::new(&src_root)
                .into_iter()
                .filter_map(|e| e.ok())
                .filter(|e| e.file_type().is_file())
            {
                if entry.path().extension().and_then(|e| e.to_str()) == Some("move") {
                    if let Ok(source) = fs::read_to_string(entry.path()) {
                        let file_addrs = scan_source_named_addresses(&source, &mut parser);
                        for (name, modules) in file_addrs {
                            all_source_addrs.entry(name).or_default().extend(modules);
                        }
                        local_count += 1;
                    }
                }
            }
            eprintln!(
                "Scanned {} local source file(s) from {}",
                local_count, src_path
            );
        }

        // No Move.toml in on-chain mode
        toml_addrs = BTreeMap::new();

        if !all_source_addrs.is_empty() {
            eprintln!(
                "Named addresses in source: {}",
                all_source_addrs
                    .keys()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        if !all_bytecode_pairs.is_empty() {
            eprintln!(
                "Bytecode address/module pairs: {}",
                all_bytecode_pairs.len()
            );
        }
    } else {
        // ─── Local mode ─────────────────────────────────────────
        let root = match path {
            Some(p) => PathBuf::from(p),
            None => {
                eprintln!("Usage: named-address-recover <path> [--toml] [--verbose]");
                eprintln!(
                    "       named-address-recover --address 0xHEX [--source <path>] [--toml] [--verbose]"
                );
                process::exit(2);
            }
        };

        if !root.exists() {
            eprintln!("Error: path does not exist: {}", root.display());
            process::exit(2);
        }

        let pkg = discover_files(&root);

        eprintln!(
            "Found: {}Move.toml, {} source file(s), {} bytecode file(s)",
            if pkg.move_toml.is_some() { "" } else { "no " },
            pkg.source_files.len(),
            pkg.bytecode_files.len(),
        );

        toml_addrs = match &pkg.move_toml {
            Some(path) => parse_move_toml(path),
            None => BTreeMap::new(),
        };
        if !toml_addrs.is_empty() {
            eprintln!(
                "Move.toml addresses: {}",
                toml_addrs.keys().cloned().collect::<Vec<_>>().join(", ")
            );
        }

        for source_path in &pkg.source_files {
            let source = match fs::read_to_string(source_path) {
                Ok(s) => s,
                Err(_) => continue,
            };
            let file_addrs = scan_source_named_addresses(&source, &mut parser);
            for (name, modules) in file_addrs {
                all_source_addrs.entry(name).or_default().extend(modules);
            }
        }
        if !all_source_addrs.is_empty() {
            eprintln!(
                "Named addresses in source: {}",
                all_source_addrs
                    .keys()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }

        for bc_path in &pkg.bytecode_files {
            let bytes = match fs::read(bc_path) {
                Ok(b) => b,
                Err(_) => continue,
            };
            all_bytecode_pairs.extend(scan_bytecode_addresses(&bytes));
        }
        all_bytecode_pairs.sort();
        all_bytecode_pairs.dedup();
        if !all_bytecode_pairs.is_empty() {
            eprintln!("Bytecode modules: {}", all_bytecode_pairs.len());
        }
    }

    // ─── Resolve & Output ───────────────────────────────────────
    let results = resolve_addresses(&toml_addrs, &all_source_addrs, &all_bytecode_pairs);

    if results.is_empty() {
        eprintln!("No named addresses found.");
        process::exit(1);
    }

    eprintln!("Resolved {} named address(es).", results.len());
    eprintln!();

    if toml_output {
        print_toml(&results);
    } else if verbose {
        print_verbose(&results);
    } else {
        print_default(&results);
    }
}
