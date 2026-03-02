use std::fs;
use std::process::Command;

fn transform(input: &str) -> String {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.move");
    fs::write(&path, input).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_move1-to-move2"))
        .arg(&path)
        .output()
        .expect("failed to run tool");

    assert!(
        output.status.success(),
        "Tool failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    fs::read_to_string(&path).unwrap()
}

#[test]
fn test_borrow_global_simple() {
    let input = "module 0x1::test { fun f() { borrow_global<Coin>(addr); } }";
    let expected = "module 0x1::test { fun f() { &Coin[addr]; } }";
    assert_eq!(transform(input), expected);
}

#[test]
fn test_borrow_global_qualified_type() {
    let input = "module 0x1::test { fun f() { borrow_global<coin::Coin>(addr); } }";
    let expected = "module 0x1::test { fun f() { &coin::Coin[addr]; } }";
    assert_eq!(transform(input), expected);
}

#[test]
fn test_borrow_global_mut_simple() {
    let input = "module 0x1::test { fun f() { borrow_global_mut<Counter>(addr); } }";
    let expected = "module 0x1::test { fun f() { &mut Counter[addr]; } }";
    assert_eq!(transform(input), expected);
}

#[test]
fn test_no_change_when_no_match() {
    let input = "module 0x1::test { fun f() { let x = 1 + 2; } }";
    assert_eq!(transform(input), input);
}

#[test]
fn test_deref_borrow_global() {
    let input = "module 0x1::test { fun f() { let x = *borrow_global<Counter>(addr); } }";
    let expected = "module 0x1::test { fun f() { let x = Counter[addr]; } }";
    assert_eq!(transform(input), expected);
}

#[test]
fn test_deref_borrow_global_mut() {
    let input =
        "module 0x1::test { fun f() { *borrow_global_mut<Counter>(addr) = Counter { i: 0 }; } }";
    let expected = "module 0x1::test { fun f() { Counter[addr] = Counter { i: 0 }; } }";
    assert_eq!(transform(input), expected);
}

#[test]
fn test_strip_acquires_single() {
    let input = "module 0x1::test { fun f() acquires Counter { } }";
    let expected = "module 0x1::test { fun f() { } }";
    assert_eq!(transform(input), expected);
}

#[test]
fn test_strip_acquires_multiple() {
    let input = "module 0x1::test { fun f() acquires Counter, Balance { } }";
    let expected = "module 0x1::test { fun f() { } }";
    assert_eq!(transform(input), expected);
}

#[test]
fn test_strip_acquires_with_return_type() {
    let input = "module 0x1::test { fun f(): u64 acquires Counter { 0 } }";
    let expected = "module 0x1::test { fun f(): u64 { 0 } }";
    assert_eq!(transform(input), expected);
}

#[test]
fn test_full_module_transform() {
    let input = r#"module 0x1::counter {
    struct Counter has key {
        value: u64,
    }

    public fun get_value(addr: address): u64 acquires Counter {
        borrow_global<Counter>(addr).value
    }

    public fun increment(addr: address) acquires Counter {
        let counter = borrow_global_mut<Counter>(addr);
        counter.value = counter.value + 1;
    }

    public fun reset(addr: address) acquires Counter {
        *borrow_global_mut<Counter>(addr) = Counter { value: 0 };
    }

    public fun read_value(addr: address): u64 acquires Counter {
        *borrow_global<Counter>(addr)
    }
}"#;
    let expected = r#"module 0x1::counter {
    struct Counter has key {
        value: u64,
    }

    public fun get_value(addr: address): u64 {
        Counter[addr].value
    }

    public fun increment(addr: address) {
        let counter = &mut Counter[addr];
        counter.value += 1;
    }

    public fun reset(addr: address) {
        Counter[addr] = Counter { value: 0 };
    }

    public fun read_value(addr: address): u64 {
        Counter[addr]
    }
}"#;
    assert_eq!(transform(input), expected);
}

#[test]
fn test_redundant_borrow_global() {
    // &borrow_global<T>(addr) should NOT produce &&T[addr]
    let input = "module 0x1::test { fun f() { &borrow_global<CoinMap>(@aptos_framework); } }";
    let expected = "module 0x1::test { fun f() { &CoinMap[@aptos_framework]; } }";
    assert_eq!(transform(input), expected);
}

#[test]
fn test_redundant_borrow_global_mut() {
    // &mut borrow_global_mut<T>(addr) should NOT produce &mut &mut T[addr]
    let input = "module 0x1::test { fun f() { &mut borrow_global_mut<Counter>(addr); } }";
    let expected = "module 0x1::test { fun f() { &mut Counter[addr]; } }";
    assert_eq!(transform(input), expected);
}

#[test]
fn test_redundant_borrow_with_field_access() {
    // &borrow_global<T>(addr).field — outer & stays, inner & stripped
    let input = "module 0x1::test { fun f() { &borrow_global<CoinMap>(@fw).asset_map; } }";
    let expected = "module 0x1::test { fun f() { &CoinMap[@fw].asset_map; } }";
    assert_eq!(transform(input), expected);
}

#[test]
fn test_borrow_global_with_field_access_no_prefix() {
    // borrow_global<T>(addr).field → T[addr].field (no & prefix when followed by .field)
    let input = "module 0x1::test { fun f() { borrow_global<CapDelegateState<Feature>>(addr).root; } }";
    let expected = "module 0x1::test { fun f() { CapDelegateState<Feature>[addr].root; } }";
    assert_eq!(transform(input), expected);
}

#[test]
fn test_borrow_global_mut_with_field_access_no_prefix() {
    // borrow_global_mut<T>(addr).field → T[addr].field (no &mut prefix)
    let input = "module 0x1::test { fun f() { borrow_global_mut<Counter>(addr).value; } }";
    let expected = "module 0x1::test { fun f() { Counter[addr].value; } }";
    assert_eq!(transform(input), expected);
}

// ── Spec block transforms ────────────────────────────────────────────────────

#[test]
fn test_borrow_global_in_spec_no_prefix() {
    // In spec blocks, borrow_global is a value function — no & prefix
    let input = r#"module 0x1::test {
    spec fun check(addr: address): bool {
        let tomb_stone = borrow_global<TombStone>(addr);
        tomb_stone.deleted
    }
}"#;
    let expected = r#"module 0x1::test {
    spec fun check(addr: address): bool {
        let tomb_stone = TombStone[addr];
        tomb_stone.deleted
    }
}"#;
    assert_eq!(transform(input), expected);
}

#[test]
fn test_borrow_global_mut_in_spec_no_prefix() {
    let input = r#"module 0x1::test {
    spec fun check(addr: address) {
        let counter = borrow_global_mut<Counter>(addr);
    }
}"#;
    let expected = r#"module 0x1::test {
    spec fun check(addr: address) {
        let counter = Counter[addr];
    }
}"#;
    assert_eq!(transform(input), expected);
}

// ── Visibility transforms ────────────────────────────────────────────────────

#[test]
fn test_public_friend_to_friend() {
    let input = "module 0x1::test { public(friend) fun f() {} }";
    let expected = "module 0x1::test { friend fun f() {} }";
    assert_eq!(transform(input), expected);
}

#[test]
fn test_public_package_to_package() {
    let input = "module 0x1::test { public(package) fun f() {} }";
    let expected = "module 0x1::test { package fun f() {} }";
    assert_eq!(transform(input), expected);
}

#[test]
fn test_public_stays_public() {
    let input = "module 0x1::test { public fun f() {} }";
    assert_eq!(transform(input), input);
}

// ── Compound assignment transforms ───────────────────────────────────────────

#[test]
fn test_compound_add() {
    let input = "module 0x1::test { fun f() { x = x + 1; } }";
    let expected = "module 0x1::test { fun f() { x += 1; } }";
    assert_eq!(transform(input), expected);
}

#[test]
fn test_compound_sub() {
    let input = "module 0x1::test { fun f() { x = x - 1; } }";
    let expected = "module 0x1::test { fun f() { x -= 1; } }";
    assert_eq!(transform(input), expected);
}

#[test]
fn test_compound_mul() {
    let input = "module 0x1::test { fun f() { x = x * y; } }";
    let expected = "module 0x1::test { fun f() { x *= y; } }";
    assert_eq!(transform(input), expected);
}

#[test]
fn test_compound_div() {
    let input = "module 0x1::test { fun f() { x = x / y; } }";
    let expected = "module 0x1::test { fun f() { x /= y; } }";
    assert_eq!(transform(input), expected);
}

#[test]
fn test_compound_mod() {
    let input = "module 0x1::test { fun f() { x = x % y; } }";
    let expected = "module 0x1::test { fun f() { x %= y; } }";
    assert_eq!(transform(input), expected);
}

#[test]
fn test_no_compound_when_lhs_differs() {
    // x = y + 1 should NOT become x += 1
    let input = "module 0x1::test { fun f() { x = y + 1; } }";
    assert_eq!(transform(input), input);
}

#[test]
fn test_compound_field_access() {
    let input = "module 0x1::test { fun f() { counter.value = counter.value + 1; } }";
    let expected = "module 0x1::test { fun f() { counter.value += 1; } }";
    assert_eq!(transform(input), expected);
}

// ── vector::empty transforms ─────────────────────────────────────────────────

#[test]
fn test_vector_empty_with_type() {
    let input = "module 0x1::test { fun f() { let v = vector::empty<u64>(); } }";
    let expected = "module 0x1::test { fun f() { let v = vector<u64>[]; } }";
    assert_eq!(transform(input), expected);
}

#[test]
fn test_vector_empty_nested_type() {
    let input = "module 0x1::test { fun f() { let v = vector::empty<vector<u8>>(); } }";
    let expected = "module 0x1::test { fun f() { let v = vector<vector<u8>>[]; } }";
    assert_eq!(transform(input), expected);
}

#[test]
fn test_vector_empty_no_type_args() {
    let input = "module 0x1::test { fun f() { let v = vector::empty(); } }";
    let expected = "module 0x1::test { fun f() { let v = vector[]; } }";
    assert_eq!(transform(input), expected);
}

#[test]
fn test_vector_empty_not_empty_args() {
    // vector::empty is not called with args normally, but if it were, don't transform
    let input = "module 0x1::test { fun f() { let v = vector::empty<u64>(x); } }";
    assert_eq!(transform(input), input);
}
