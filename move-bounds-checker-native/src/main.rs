use std::collections::{BTreeSet, HashMap};
use std::env;
use std::fs;
use std::path::PathBuf;
use std::process;

use legacy_move_compiler::{
    parser::{
        ast::{
            Definition, Exp, Exp_, Function, FunctionBody_, ModuleDefinition, ModuleMember,
            SequenceItem_, StructDefinition, StructLayout, Type, Type_,
        },
        syntax::parse_file_string,
    },
    shared::{CompilationEnv, Flags, LanguageVersion},
};
use move_command_line_common::files::FileHash;
use rayon::prelude::*;
use regex::Regex;
use walkdir::WalkDir;

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

fn line_col(source: &str, byte_offset: u32) -> (usize, usize) {
    let offset = byte_offset as usize;
    if offset > source.len() {
        return (1, 1);
    }
    let prefix = &source[..offset];
    let line = prefix.matches('\n').count() + 1;
    let col = prefix.rfind('\n').map(|i| offset - i).unwrap_or(offset + 1);
    (line, col)
}

// ─── Check 1: Loop Depth ───────────────────────────────────────

fn max_loop_depth_exp(exp: &Exp, depth: usize) -> usize {
    max_loop_depth_exp_inner(&exp.value, depth)
}

fn max_loop_depth_exp_inner(exp: &Exp_, depth: usize) -> usize {
    match exp {
        Exp_::While(_, cond, body) => {
            let new_depth = depth + 1;
            let d1 = max_loop_depth_exp(cond, depth);
            let d2 = max_loop_depth_exp(body, new_depth);
            d1.max(d2).max(new_depth)
        }
        Exp_::Loop(_, body) => {
            let new_depth = depth + 1;
            max_loop_depth_exp(body, new_depth).max(new_depth)
        }
        _ => {
            let mut max = depth;
            visit_child_exps(exp, &mut |child| {
                let d = max_loop_depth_exp(child, depth);
                if d > max {
                    max = d;
                }
            });
            max
        }
    }
}

// ─── Check 2: Generic Instantiation Length ─────────────────────

fn max_type_args_in_type(ty: &Type) -> usize {
    match &ty.value {
        Type_::Apply(_, args) => {
            let mut max = args.len();
            for arg in args {
                max = max.max(max_type_args_in_type(arg));
            }
            max
        }
        Type_::Ref(_, inner) => max_type_args_in_type(inner),
        Type_::Fun(params, ret, _) => {
            let mut max = 0;
            for p in params {
                max = max.max(max_type_args_in_type(p));
            }
            max = max.max(max_type_args_in_type(ret));
            max
        }
        Type_::Multiple(types) => {
            let mut max = 0;
            for t in types {
                max = max.max(max_type_args_in_type(t));
            }
            max
        }
        Type_::Unit => 0,
    }
}

fn max_type_args_in_exp(exp: &Exp) -> usize {
    let mut max = 0;
    visit_all_exps_recursive(exp, &mut |e| {
        // Check type arguments in call expressions, etc.
        // The expression-level type arguments show up in Call and Pack variants
        visit_types_in_exp_shallow(&e.value, &mut |ty| {
            let ta = max_type_args_in_type(ty);
            if ta > max {
                max = ta;
            }
        });
    });
    max
}

// ─── Check 4: Basic Blocks (Heuristic) ────────────────────────

fn count_blocks_in_exp(exp: &Exp, count: &mut usize) {
    match &exp.value {
        Exp_::IfElse(_, _, _) => *count += 2,
        Exp_::While(_, _, _) => *count += 2,
        Exp_::Loop(_, _) => *count += 1,
        Exp_::Break(_) => *count += 1,
        Exp_::Continue(_) => *count += 1,
        _ => {}
    }
    visit_child_exps(&exp.value, &mut |child| {
        count_blocks_in_exp(child, count);
    });
}

// ─── Check 5: Type Node Count ─────────────────────────────────

fn count_type_nodes_in_type(ty: &Type) -> usize {
    match &ty.value {
        Type_::Apply(_, args) => {
            let mut count = 1;
            for arg in args {
                count += count_type_nodes_in_type(arg);
            }
            count
        }
        Type_::Ref(_, inner) => 1 + count_type_nodes_in_type(inner),
        Type_::Fun(params, ret, _) => {
            let mut count = 1;
            for p in params {
                count += count_type_nodes_in_type(p);
            }
            count += count_type_nodes_in_type(ret);
            count
        }
        Type_::Multiple(types) => {
            let mut count = 0;
            for t in types {
                count += count_type_nodes_in_type(t);
            }
            count
        }
        Type_::Unit => 0,
    }
}

fn count_type_nodes_in_exp(exp: &Exp) -> usize {
    let mut count = 0;
    visit_all_exps_recursive(exp, &mut |e| {
        visit_types_in_exp_shallow(&e.value, &mut |ty| {
            count += count_type_nodes_in_type(ty);
        });
    });
    count
}

// ─── Check 7: Type Depth ──────────────────────────────────────

fn compute_type_depth(ty: &Type) -> usize {
    match &ty.value {
        Type_::Apply(_, args) => {
            let mut max_child = 0;
            for arg in args {
                max_child = max_child.max(compute_type_depth(arg));
            }
            1 + max_child
        }
        Type_::Ref(_, inner) => 1 + compute_type_depth(inner),
        Type_::Fun(params, ret, _) => {
            let mut max_child = 0;
            for p in params {
                max_child = max_child.max(compute_type_depth(p));
            }
            max_child = max_child.max(compute_type_depth(ret));
            1 + max_child
        }
        Type_::Multiple(types) => {
            let mut max = 0;
            for t in types {
                max = max.max(compute_type_depth(t));
            }
            max
        }
        Type_::Unit => 0,
    }
}

fn max_type_depth_in_exp(exp: &Exp) -> usize {
    let mut max = 0;
    visit_all_exps_recursive(exp, &mut |e| {
        visit_types_in_exp_shallow(&e.value, &mut |ty| {
            let d = compute_type_depth(ty);
            if d > max {
                max = d;
            }
        });
    });
    max
}

// ─── Check 13: Local Variable Count ────────────────────────────

fn count_lets_recursive(exp: &Exp, count: &mut usize) {
    visit_all_exps_recursive(exp, &mut |e| {
        if let Exp_::Block(seq) = &e.value {
            for item in &seq.1 {
                match &item.value {
                    SequenceItem_::Bind(_, _, _) | SequenceItem_::Declare(_, _) => {
                        *count += 1;
                    }
                    _ => {}
                }
            }
        }
    });
}

// ─── AST Visitor Helpers ───────────────────────────────────────

/// Visit direct child expressions of an Exp_ node (non-recursive).
fn visit_child_exps<F: FnMut(&Exp)>(exp: &Exp_, f: &mut F) {
    match exp {
        Exp_::Value(_)
        | Exp_::Move(_)
        | Exp_::Copy(_)
        | Exp_::Break(_)
        | Exp_::Continue(_)
        | Exp_::Unit
        | Exp_::UnresolvedError
        | Exp_::Spec(_) => {}
        Exp_::Name(_, tys_opt) => {
            // No child expressions, type args only
            let _ = tys_opt;
        }
        Exp_::Call(_, _, tys_opt, args) => {
            let _ = tys_opt;
            for arg in &args.value {
                f(arg);
            }
        }
        Exp_::ExpCall(callee, args) => {
            f(callee);
            for arg in &args.value {
                f(arg);
            }
        }
        Exp_::Pack(_, tys_opt, fields) => {
            let _ = tys_opt;
            for (_, e) in fields {
                f(e);
            }
        }
        Exp_::Vector(_, tys_opt, args) => {
            let _ = tys_opt;
            for arg in &args.value {
                f(arg);
            }
        }
        Exp_::IfElse(cond, then_e, else_opt) => {
            f(cond);
            f(then_e);
            if let Some(else_e) = else_opt {
                f(else_e);
            }
        }
        Exp_::While(_, cond, body) => {
            f(cond);
            f(body);
        }
        Exp_::Loop(_, body) => {
            f(body);
        }
        Exp_::Match(subject, arms) => {
            f(subject);
            for arm in arms {
                let (_, guard, rhs) = &arm.value;
                if let Some(guard) = guard {
                    f(guard);
                }
                f(rhs);
            }
        }
        Exp_::Block(seq) => {
            visit_sequence_exps(seq, f);
        }
        Exp_::Lambda(_, body, _, _) => {
            f(body);
        }
        Exp_::Quant(_, binds, triggers, where_opt, body) => {
            for bind in &binds.value {
                f(&bind.value.1);
            }
            for trigger_group in triggers {
                for trigger in trigger_group {
                    f(trigger);
                }
            }
            if let Some(w) = where_opt {
                f(w);
            }
            f(body);
        }
        Exp_::ExpList(exps) => {
            for e in exps {
                f(e);
            }
        }
        Exp_::Assign(lhs, _, rhs) => {
            f(lhs);
            f(rhs);
        }
        Exp_::Return(Some(e))
        | Exp_::Abort(e)
        | Exp_::Dereference(e)
        | Exp_::UnaryExp(_, e)
        | Exp_::Borrow(_, e)
        | Exp_::Dot(e, _)
        | Exp_::Cast(e, _)
        | Exp_::Annotate(e, _) => {
            f(e);
        }
        Exp_::Return(None) => {}
        Exp_::BinopExp(lhs, _, rhs) => {
            f(lhs);
            f(rhs);
        }
        Exp_::Index(base, idx) => {
            f(base);
            f(idx);
        }
        Exp_::Test(e, _) => {
            f(e);
        }
        Exp_::Behavior(_, _, _, _, args, _) => {
            for arg in &args.value {
                f(arg);
            }
        }
    }
}

fn visit_sequence_exps<F: FnMut(&Exp)>(
    seq: &legacy_move_compiler::parser::ast::Sequence,
    f: &mut F,
) {
    for item in &seq.1 {
        match &item.value {
            SequenceItem_::Seq(e) => f(e),
            SequenceItem_::Bind(_, _, e) => f(e),
            SequenceItem_::Declare(_, _) => {}
        }
    }
    if let Some(trailing) = &*seq.3 {
        f(trailing);
    }
}

/// Recursively visit all expressions in a tree.
fn visit_all_exps_recursive<F: FnMut(&Exp)>(exp: &Exp, f: &mut F) {
    f(exp);
    visit_child_exps(&exp.value, &mut |child| {
        visit_all_exps_recursive(child, f);
    });
}

/// Visit types that appear directly in an expression (non-recursive into sub-expressions).
fn visit_types_in_exp_shallow<F: FnMut(&Type)>(exp: &Exp_, f: &mut F) {
    match exp {
        Exp_::Call(_, _, Some(tys), _) => {
            for ty in tys {
                f(ty);
            }
        }
        Exp_::Pack(_, Some(tys), _) => {
            for ty in tys {
                f(ty);
            }
        }
        Exp_::Vector(_, Some(tys), _) => {
            for ty in tys {
                f(ty);
            }
        }
        Exp_::Name(_, Some(tys)) => {
            for ty in tys {
                f(ty);
            }
        }
        Exp_::Cast(_, ty) | Exp_::Annotate(_, ty) => {
            f(ty);
        }
        Exp_::Test(_, tys) => {
            for ty in tys {
                f(ty);
            }
        }
        Exp_::Behavior(_, _, _, Some(tys), _, _) => {
            for ty in tys {
                f(ty);
            }
        }
        _ => {}
    }
}

// ─── Main Check Logic ──────────────────────────────────────────

fn check_definitions(defs: &[Definition], source: &str, config: &BoundsConfig) -> Vec<Violation> {
    let mut violations = Vec::new();
    for def in defs {
        match def {
            Definition::Module(module) => check_module(module, source, config, &mut violations),
            Definition::Address(addr) => {
                for module in &addr.modules {
                    check_module(module, source, config, &mut violations);
                }
            }
            _ => {}
        }
    }
    violations
}

fn check_module(
    module: &ModuleDefinition,
    source: &str,
    config: &BoundsConfig,
    violations: &mut Vec<Violation>,
) {
    // Skip spec modules
    if module.is_spec_module {
        return;
    }

    let module_name = module.name.0.value.as_str().to_string();
    let (line, col) = line_col(source, module.loc.start());

    // Count struct/enum and function definitions
    let mut struct_count = 0;
    let mut function_count = 0;
    for member in &module.members {
        match member {
            ModuleMember::Struct(_) => struct_count += 1,
            ModuleMember::Function(_) => function_count += 1,
            ModuleMember::Spec(_) => {} // skip specs
            _ => {}
        }
    }

    // Check 8: struct definitions per module
    if struct_count > config.max_struct_definitions {
        violations.push(Violation {
            kind: "max_struct_definitions",
            entity_kind: "module",
            entity: module_name.clone(),
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
            entity: module_name.clone(),
            actual: function_count,
            limit: config.max_function_definitions,
            line,
            col,
        });
    }

    // Check 12: module name identifier length
    if module_name.len() > config.max_identifier_length {
        violations.push(Violation {
            kind: "max_identifier_length",
            entity_kind: "module",
            entity: module_name.clone(),
            actual: module_name.len(),
            limit: config.max_identifier_length,
            line,
            col,
        });
    }

    // Check individual members
    for member in &module.members {
        match member {
            ModuleMember::Function(func) => {
                check_function(func, source, config, violations);
            }
            ModuleMember::Struct(struct_def) => {
                check_struct(struct_def, source, config, violations);
            }
            _ => {}
        }
    }
}

fn check_function(
    func: &Function,
    source: &str,
    config: &BoundsConfig,
    violations: &mut Vec<Violation>,
) {
    let name = func.name.0.value.as_str().to_string();
    let (line, col) = line_col(source, func.loc.start());

    // Check 3: function parameters
    let param_count = func.signature.parameters.len();
    if param_count > config.max_function_parameters {
        violations.push(Violation {
            kind: "max_function_parameters",
            entity_kind: "function",
            entity: name.clone(),
            actual: param_count,
            limit: config.max_function_parameters,
            line,
            col,
        });
    }

    // Check 6: return values
    if let Type_::Multiple(types) = &func.signature.return_type.value {
        if types.len() > config.max_function_return_values {
            violations.push(Violation {
                kind: "max_function_return_values",
                entity_kind: "function",
                entity: name.clone(),
                actual: types.len(),
                limit: config.max_function_return_values,
                line,
                col,
            });
        }
    }

    // Check 2: generic instantiation length (type parameters on declaration + type args in body)
    let mut max_generic = func.signature.type_parameters.len();
    // Check type args in parameter types
    for (_, ty) in &func.signature.parameters {
        max_generic = max_generic.max(max_type_args_in_type(ty));
    }
    max_generic = max_generic.max(max_type_args_in_type(&func.signature.return_type));

    // Check 14: type parameter count
    let tp_count = func.signature.type_parameters.len();
    if tp_count > config.max_type_parameter_count {
        violations.push(Violation {
            kind: "max_type_parameter_count",
            entity_kind: "function",
            entity: name.clone(),
            actual: tp_count,
            limit: config.max_type_parameter_count,
            line,
            col,
        });
    }

    // Body-dependent checks (skip native functions)
    if let FunctionBody_::Defined(seq) = &func.body.value {
        // Work with the sequence directly to avoid cloning the entire AST
        let seq_exps: Vec<&Exp> = seq
            .1
            .iter()
            .filter_map(|item| match &item.value {
                SequenceItem_::Seq(e) => Some(e.as_ref()),
                SequenceItem_::Bind(_, _, e) => Some(e.as_ref()),
                SequenceItem_::Declare(_, _) => None,
            })
            .chain(seq.3.as_ref().as_ref())
            .collect();

        // Check type args in body
        for e in &seq_exps {
            max_generic = max_generic.max(max_type_args_in_exp(e));
        }

        // Check 1: loop depth
        let depth = seq_exps
            .iter()
            .map(|e| max_loop_depth_exp(e, 0))
            .max()
            .unwrap_or(0);
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
        let mut blocks = 1;
        for e in &seq_exps {
            count_blocks_in_exp(e, &mut blocks);
        }
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
        let mut tn: usize = seq_exps.iter().map(|e| count_type_nodes_in_exp(e)).sum();
        for (_, ty) in &func.signature.parameters {
            tn += count_type_nodes_in_type(ty);
        }
        tn += count_type_nodes_in_type(&func.signature.return_type);
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
        let mut td = seq_exps
            .iter()
            .map(|e| max_type_depth_in_exp(e))
            .max()
            .unwrap_or(0);
        for (_, ty) in &func.signature.parameters {
            td = td.max(compute_type_depth(ty));
        }
        td = td.max(compute_type_depth(&func.signature.return_type));
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
        let mut let_count = 0;
        for item in &seq.1 {
            match &item.value {
                SequenceItem_::Bind(_, _, _) | SequenceItem_::Declare(_, _) => {
                    let_count += 1;
                }
                _ => {}
            }
        }
        // Also count lets in sub-expressions
        for e in &seq_exps {
            count_lets_recursive(e, &mut let_count);
        }
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

    // Check 2: generic instantiation length (final)
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
}

fn check_struct(
    struct_def: &StructDefinition,
    source: &str,
    config: &BoundsConfig,
    violations: &mut Vec<Violation>,
) {
    let name = struct_def.name.0.value.as_str().to_string();
    let (line, col) = line_col(source, struct_def.loc.start());

    let is_enum = matches!(struct_def.layout, StructLayout::Variants(_));
    let kind_label = if is_enum { "enum" } else { "struct" };

    // Check 2: generic instantiation length on declaration + field types
    let mut max_generic = struct_def.type_parameters.len();
    match &struct_def.layout {
        StructLayout::Singleton(fields, _) => {
            for (_, ty) in fields {
                max_generic = max_generic.max(max_type_args_in_type(ty));
            }
        }
        StructLayout::Variants(variants) => {
            for variant in variants {
                for (_, ty) in &variant.fields {
                    max_generic = max_generic.max(max_type_args_in_type(ty));
                }
            }
        }
        StructLayout::Native(_) => {}
    }
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
    let tp_count = struct_def.type_parameters.len();
    if tp_count > config.max_type_parameter_count {
        violations.push(Violation {
            kind: "max_type_parameter_count",
            entity_kind: kind_label,
            entity: name.clone(),
            actual: tp_count,
            limit: config.max_type_parameter_count,
            line,
            col,
        });
    }

    match &struct_def.layout {
        StructLayout::Singleton(fields, _) => {
            // Check 10: fields in struct
            if fields.len() > config.max_fields_in_struct {
                violations.push(Violation {
                    kind: "max_fields_in_struct",
                    entity_kind: kind_label,
                    entity: name.clone(),
                    actual: fields.len(),
                    limit: config.max_fields_in_struct,
                    line,
                    col,
                });
            }
        }
        StructLayout::Variants(variants) => {
            // Check 9: enum variants
            if variants.len() > config.max_struct_variants {
                violations.push(Violation {
                    kind: "max_struct_variants",
                    entity_kind: kind_label,
                    entity: name.clone(),
                    actual: variants.len(),
                    limit: config.max_struct_variants,
                    line,
                    col,
                });
            }
            // Check 10: fields per variant
            for variant in variants {
                let variant_name = variant.name.0.value.as_str();
                if variant.fields.len() > config.max_fields_in_struct {
                    violations.push(Violation {
                        kind: "max_fields_in_struct",
                        entity_kind: "variant",
                        entity: format!("{}::{}", name, variant_name),
                        actual: variant.fields.len(),
                        limit: config.max_fields_in_struct,
                        line,
                        col,
                    });
                }
            }
        }
        StructLayout::Native(_) => {}
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

fn parse_override(arg: &str, config: &mut BoundsConfig) -> bool {
    let overrides: &[(&str, fn(&mut BoundsConfig, usize))] = &[
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
    // Some Move files produce deeply nested ASTs that overflow the default 8MB stack.
    rayon::ThreadPoolBuilder::new()
        .stack_size(64 * 1024 * 1024)
        .build_global()
        .ok();

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
        eprintln!("Usage: move-bounds-checker-native <dir> [--identify] [--all] [--explorer-local=PATH] [--max-loop-depth=N ...]");
        process::exit(2);
    }

    // Collect .move files
    let files: Vec<PathBuf> = paths
        .iter()
        .flat_map(|p| {
            let path = PathBuf::from(p);
            if path.is_dir() {
                WalkDir::new(&path)
                    .into_iter()
                    .filter_map(|e| e.ok())
                    .filter(|e| e.path().extension().is_some_and(|ext| ext == "move"))
                    .map(|e| e.path().to_path_buf())
                    .collect::<Vec<_>>()
            } else {
                vec![path]
            }
        })
        .collect();

    eprintln!("Scanning {} file(s)...", files.len());

    // Process files in parallel
    let results: Vec<(PathBuf, Vec<Violation>)> = files
        .par_iter()
        .map(|path| {
            let source = match fs::read_to_string(path) {
                Ok(s) => s,
                Err(_) => return (path.clone(), Vec::new()),
            };

            let file_hash = FileHash::new(&source);
            let flags = Flags::empty().set_language_version(LanguageVersion::V2_5);
            let mut env = CompilationEnv::new(flags, BTreeSet::new());

            let defs = match parse_file_string(&mut env, file_hash, &source) {
                Ok((defs, _comments)) => defs,
                Err(_) => return (path.clone(), Vec::new()),
            };

            let violations = check_definitions(&defs, &source, &config);
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
