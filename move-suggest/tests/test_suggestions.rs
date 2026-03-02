use std::fs;
use std::process::Command;

/// Run move-suggest in report mode (no --fix) and return (stdout, exit_code).
fn suggest(input: &str) -> (String, i32) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.move");
    fs::write(&path, input).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_move-suggest"))
        .arg(path.to_str().unwrap())
        .output()
        .expect("failed to run tool");

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let code = output.status.code().unwrap_or(-1);
    (stdout, code)
}

/// Run move-suggest with --fix and return the rewritten file contents.
fn fix(input: &str) -> String {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.move");
    fs::write(&path, input).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_move-suggest"))
        .arg("--fix")
        .arg(path.to_str().unwrap())
        .output()
        .expect("failed to run tool");

    assert!(
        output.status.success() || output.status.code() == Some(1),
        "Tool failed unexpectedly: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    fs::read_to_string(&path).unwrap()
}

// ── Exit codes ──────────────────────────────────────────────────────────────

#[test]
fn test_exit_code_zero_when_clean() {
    let input = "module 0x1::test { fun f() { let x = 1 + 2; } }";
    let (stdout, code) = suggest(input);
    assert_eq!(code, 0);
    assert!(stdout.is_empty());
}

#[test]
fn test_exit_code_one_when_suggestions() {
    let input = "module 0x1::test { fun f() { vector::length(&v); } }";
    let (_, code) = suggest(input);
    assert_eq!(code, 1);
}

// ── Receiver-style: vector ──────────────────────────────────────────────────

#[test]
fn test_vector_push_back() {
    let input = "module 0x1::test { fun f() { vector::push_back(&mut v, x); } }";
    let expected = "module 0x1::test { fun f() { v.push_back(x); } }";
    assert_eq!(fix(input), expected);
}

#[test]
fn test_vector_pop_back() {
    let input = "module 0x1::test { fun f() { vector::pop_back(&mut v); } }";
    let expected = "module 0x1::test { fun f() { v.pop_back(); } }";
    assert_eq!(fix(input), expected);
}

#[test]
fn test_vector_length() {
    let input = "module 0x1::test { fun f() { vector::length(&v); } }";
    let expected = "module 0x1::test { fun f() { v.length(); } }";
    assert_eq!(fix(input), expected);
}

#[test]
fn test_vector_is_empty() {
    let input = "module 0x1::test { fun f() { vector::is_empty(&v); } }";
    let expected = "module 0x1::test { fun f() { v.is_empty(); } }";
    assert_eq!(fix(input), expected);
}

#[test]
fn test_vector_borrow_to_index() {
    let input = "module 0x1::test { fun f() { vector::borrow(&v, i); } }";
    let expected = "module 0x1::test { fun f() { &v[i]; } }";
    assert_eq!(fix(input), expected);
}

#[test]
fn test_vector_borrow_mut_to_index() {
    let input = "module 0x1::test { fun f() { vector::borrow_mut(&mut v, i); } }";
    let expected = "module 0x1::test { fun f() { &mut v[i]; } }";
    assert_eq!(fix(input), expected);
}

#[test]
fn test_vector_contains() {
    let input = "module 0x1::test { fun f() { vector::contains(&v, &e); } }";
    let expected = "module 0x1::test { fun f() { v.contains(&e); } }";
    assert_eq!(fix(input), expected);
}

#[test]
fn test_vector_append() {
    let input = "module 0x1::test { fun f() { vector::append(&mut v1, v2); } }";
    let expected = "module 0x1::test { fun f() { v1.append(v2); } }";
    assert_eq!(fix(input), expected);
}

#[test]
fn test_vector_reverse() {
    let input = "module 0x1::test { fun f() { vector::reverse(&mut v); } }";
    let expected = "module 0x1::test { fun f() { v.reverse(); } }";
    assert_eq!(fix(input), expected);
}

#[test]
fn test_vector_swap() {
    let input = "module 0x1::test { fun f() { vector::swap(&mut v, i, j); } }";
    let expected = "module 0x1::test { fun f() { v.swap(i, j); } }";
    assert_eq!(fix(input), expected);
}

// ── Receiver-style: string ──────────────────────────────────────────────────

#[test]
fn test_string_length() {
    let input = "module 0x1::test { fun f() { string::length(&s); } }";
    let expected = "module 0x1::test { fun f() { s.length(); } }";
    assert_eq!(fix(input), expected);
}

#[test]
fn test_string_bytes() {
    let input = "module 0x1::test { fun f() { string::bytes(&s); } }";
    let expected = "module 0x1::test { fun f() { s.bytes(); } }";
    assert_eq!(fix(input), expected);
}

// ── Receiver-style: option ──────────────────────────────────────────────────

#[test]
fn test_option_is_some() {
    let input = "module 0x1::test { fun f() { option::is_some(&o); } }";
    let expected = "module 0x1::test { fun f() { o.is_some(); } }";
    assert_eq!(fix(input), expected);
}

#[test]
fn test_option_is_none() {
    let input = "module 0x1::test { fun f() { option::is_none(&o); } }";
    let expected = "module 0x1::test { fun f() { o.is_none(); } }";
    assert_eq!(fix(input), expected);
}

#[test]
fn test_option_extract() {
    let input = "module 0x1::test { fun f() { option::extract(&mut o); } }";
    let expected = "module 0x1::test { fun f() { o.extract(); } }";
    assert_eq!(fix(input), expected);
}

#[test]
fn test_option_destroy_some() {
    let input = "module 0x1::test { fun f() { option::destroy_some(o); } }";
    let expected = "module 0x1::test { fun f() { o.destroy_some(); } }";
    assert_eq!(fix(input), expected);
}

#[test]
fn test_option_destroy_none() {
    let input = "module 0x1::test { fun f() { option::destroy_none(o); } }";
    let expected = "module 0x1::test { fun f() { o.destroy_none(); } }";
    assert_eq!(fix(input), expected);
}

// ── Negative: no false positives ────────────────────────────────────────────

#[test]
fn test_no_match_unknown_function() {
    let input = "module 0x1::test { fun f() { vector::unknown_func(&v); } }";
    let (_, code) = suggest(input);
    assert_eq!(code, 0);
}

#[test]
fn test_no_match_wrong_arg_count() {
    // push_back needs 2 args, not 1
    let input = "module 0x1::test { fun f() { vector::push_back(&mut v); } }";
    let (_, code) = suggest(input);
    assert_eq!(code, 0);
}

// ── Vector literal: simple ──────────────────────────────────────────────────

#[test]
fn test_vector_empty_literal() {
    let input = "module 0x1::test { fun f() { vector::empty<u64>(); } }";
    let expected = "module 0x1::test { fun f() { vector<u64>[]; } }";
    assert_eq!(fix(input), expected);
}

#[test]
fn test_vector_empty_no_type_args() {
    let input = "module 0x1::test { fun f() { vector::empty(); } }";
    let expected = "module 0x1::test { fun f() { vector[]; } }";
    assert_eq!(fix(input), expected);
}

#[test]
fn test_vector_singleton_literal() {
    let input = "module 0x1::test { fun f() { vector::singleton(42); } }";
    let expected = "module 0x1::test { fun f() { vector[42]; } }";
    assert_eq!(fix(input), expected);
}

// ── Vector literal: multi-push ──────────────────────────────────────────────

#[test]
fn test_multi_push_three_elements() {
    let input = r#"module 0x1::test {
    fun f() {
        let v = vector::empty<u64>();
        vector::push_back(&mut v, 1);
        vector::push_back(&mut v, 2);
        vector::push_back(&mut v, 3);
    }
}"#;
    let expected = r#"module 0x1::test {
    fun f() {
        let v = vector<u64>[1, 2, 3];
    }
}"#;
    assert_eq!(fix(input), expected);
}

#[test]
fn test_multi_push_unqualified() {
    let input = r#"module 0x1::test {
    fun f() {
        let v = vector::empty<u8>();
        push_back(&mut v, 10);
        push_back(&mut v, 20);
    }
}"#;
    let expected = r#"module 0x1::test {
    fun f() {
        let v = vector<u8>[10, 20];
    }
}"#;
    assert_eq!(fix(input), expected);
}

#[test]
fn test_multi_push_not_enough_elements() {
    // Only 1 push_back — should NOT trigger multi-push
    let input = r#"module 0x1::test {
    fun f() {
        let v = vector::empty<u64>();
        vector::push_back(&mut v, 1);
        let x = 5;
    }
}"#;
    let (stdout, _) = suggest(input);
    assert!(!stdout.contains("[vector_multi_push]"));
}

#[test]
fn test_multi_push_broken_by_other_stmt() {
    // push_backs interrupted by another statement — only first pair counts
    let input = r#"module 0x1::test {
    fun f() {
        let v = vector::empty<u64>();
        vector::push_back(&mut v, 1);
        vector::push_back(&mut v, 2);
        let x = 5;
        vector::push_back(&mut v, 3);
    }
}"#;
    let result = fix(input);
    // First two push_backs should be collapsed
    assert!(result.contains("vector<u64>[1, 2]"));
    // The third push_back after `let x = 5` becomes receiver-style
    assert!(result.contains("v.push_back(3)"));
}

// ── Overlap resolution ──────────────────────────────────────────────────────

#[test]
fn test_multi_push_no_duplicate_receiver_style() {
    // When multi-push matches, individual push_backs should NOT also get
    // receiver_style suggestions
    let input = r#"module 0x1::test {
    fun f() {
        let v = vector::empty<u64>();
        vector::push_back(&mut v, 1);
        vector::push_back(&mut v, 2);
    }
}"#;
    let (stdout, _) = suggest(input);
    assert!(stdout.contains("[vector_multi_push]"));
    // Should NOT also contain receiver_style for the same push_backs
    assert!(!stdout.contains("[receiver_style]"));
}

// ── Suggestion output format ────────────────────────────────────────────────

#[test]
fn test_suggestion_output_format() {
    let input = "module 0x1::test { fun f() { vector::length(&v); } }";
    let (stdout, _) = suggest(input);
    // Should contain file:line:col [rule] message
    assert!(stdout.contains("[receiver_style]"));
    assert!(stdout.contains("v.length()"));
}

// ── Full integration ────────────────────────────────────────────────────────

#[test]
fn test_full_module() {
    let input = r#"module 0x1::example {
    use std::vector;
    use std::option;

    fun process(v: &mut vector<u64>, o: &Option<u64>) {
        vector::push_back(&mut v, 1);
        let len = vector::length(&v);
        let first = vector::borrow(&v, 0);
        let has = option::is_some(&o);
    }

    fun build(): vector<u64> {
        let v = vector::empty<u64>();
        vector::push_back(&mut v, 10);
        vector::push_back(&mut v, 20);
        v
    }
}"#;
    let expected = r#"module 0x1::example {
    use std::vector;
    use std::option;

    fun process(v: &mut vector<u64>, o: &Option<u64>) {
        v.push_back(1);
        let len = v.length();
        let first = &v[0];
        let has = o.is_some();
    }

    fun build(): vector<u64> {
        let v = vector<u64>[10, 20];
        v
    }
}"#;
    assert_eq!(fix(input), expected);
}
