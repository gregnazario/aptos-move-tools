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

// ── Cast paren removal transforms ────────────────────────────────────────────

#[test]
fn test_cast_paren_in_let() {
    let input = "module 0x1::test { fun f() { let x = (y as u64); } }";
    let expected = "module 0x1::test { fun f() { let x = y as u64; } }";
    assert_eq!(transform(input), expected);
}

#[test]
fn test_cast_paren_in_call_arg() {
    let input = "module 0x1::test { fun f() { foo((x as u128)); } }";
    let expected = "module 0x1::test { fun f() { foo(x as u128); } }";
    assert_eq!(transform(input), expected);
}

#[test]
fn test_cast_paren_complex_inner() {
    // Inner expression has its own parens that should stay
    let input = "module 0x1::test { fun f() { let x = ((a + b) as u64); } }";
    let expected = "module 0x1::test { fun f() { let x = (a + b) as u64; } }";
    assert_eq!(transform(input), expected);
}

#[test]
fn test_no_cast_paren_removal_non_cast() {
    // Parenthesized non-cast expression should not be touched
    let input = "module 0x1::test { fun f() { let x = (a + b); } }";
    assert_eq!(transform(input), input);
}

#[test]
fn test_cast_paren_u8() {
    let input = "module 0x1::test { fun f() { let x = (amount as u8); } }";
    let expected = "module 0x1::test { fun f() { let x = amount as u8; } }";
    assert_eq!(transform(input), expected);
}

#[test]
fn test_cast_paren_in_assign() {
    let input = "module 0x1::test { fun f() { x = (y as u64); } }";
    let expected = "module 0x1::test { fun f() { x = y as u64; } }";
    assert_eq!(transform(input), expected);
}

#[test]
fn test_cast_paren_kept_in_binary_add() {
    // Parens around cast in binary expression must stay
    let input = "module 0x1::test { fun f() { let x = (y as u64) + 1; } }";
    assert_eq!(transform(input), input);
}

#[test]
fn test_cast_paren_kept_in_binary_mul() {
    let input = "module 0x1::test { fun f() { let x = (y as u64) * 2; } }";
    assert_eq!(transform(input), input);
}

#[test]
fn test_cast_paren_kept_in_comparison() {
    let input = "module 0x1::test { fun f() { if ((x as u64) > 0) {}; } }";
    assert_eq!(transform(input), input);
}

#[test]
fn test_cast_paren_kept_rhs_of_binary() {
    // Cast as right operand of binary expression
    let input = "module 0x1::test { fun f() { let x = a + (b as u64); } }";
    assert_eq!(transform(input), input);
}

// ── Vector index syntax transforms ───────────────────────────────────────────

#[test]
fn test_vector_borrow() {
    let input = "module 0x1::test { fun f() { vector::borrow(&v, 0); } }";
    let expected = "module 0x1::test { fun f() { &v[0]; } }";
    assert_eq!(transform(input), expected);
}

#[test]
fn test_vector_borrow_mut() {
    let input = "module 0x1::test { fun f() { vector::borrow_mut(&mut v, i); } }";
    let expected = "module 0x1::test { fun f() { &mut v[i]; } }";
    assert_eq!(transform(input), expected);
}

#[test]
fn test_deref_vector_borrow() {
    // *vector::borrow(&v, i) → v[i]  (deref cancels the &)
    let input = "module 0x1::test { fun f() { let x = *vector::borrow(&v, 0); } }";
    let expected = "module 0x1::test { fun f() { let x = v[0]; } }";
    assert_eq!(transform(input), expected);
}

#[test]
fn test_deref_vector_borrow_mut() {
    // *vector::borrow_mut(&mut v, i) = val → v[i] = val
    let input = "module 0x1::test { fun f() { *vector::borrow_mut(&mut v, 0) = 42; } }";
    let expected = "module 0x1::test { fun f() { v[0] = 42; } }";
    assert_eq!(transform(input), expected);
}

#[test]
fn test_vector_borrow_with_field_access() {
    // vector::borrow(&v, i).field → v[i].field  (no prefix when followed by .field)
    let input = "module 0x1::test { fun f() { vector::borrow(&items, idx).value; } }";
    let expected = "module 0x1::test { fun f() { items[idx].value; } }";
    assert_eq!(transform(input), expected);
}

#[test]
fn test_vector_borrow_ref_variable() {
    // First arg is already a reference variable (not a borrow_expression)
    let input = "module 0x1::test { fun f() { vector::borrow(v_ref, 0); } }";
    let expected = "module 0x1::test { fun f() { &v_ref[0]; } }";
    assert_eq!(transform(input), expected);
}

#[test]
fn test_vector_borrow_complex_index() {
    let input = "module 0x1::test { fun f() { vector::borrow(&data, i + 1); } }";
    let expected = "module 0x1::test { fun f() { &data[i + 1]; } }";
    assert_eq!(transform(input), expected);
}

// ── Receiver-style transforms ────────────────────────────────────────────────

// -- vector --

#[test]
fn test_receiver_vector_push_back() {
    let input = "module 0x1::test { fun f() { vector::push_back(&mut v, 42); } }";
    let expected = "module 0x1::test { fun f() { v.push_back(42); } }";
    assert_eq!(transform(input), expected);
}

#[test]
fn test_receiver_vector_pop_back() {
    let input = "module 0x1::test { fun f() { vector::pop_back(&mut v); } }";
    let expected = "module 0x1::test { fun f() { v.pop_back(); } }";
    assert_eq!(transform(input), expected);
}

#[test]
fn test_receiver_vector_length() {
    let input = "module 0x1::test { fun f() { vector::length(&v); } }";
    let expected = "module 0x1::test { fun f() { v.length(); } }";
    assert_eq!(transform(input), expected);
}

#[test]
fn test_receiver_vector_is_empty() {
    let input = "module 0x1::test { fun f() { vector::is_empty(&v); } }";
    let expected = "module 0x1::test { fun f() { v.is_empty(); } }";
    assert_eq!(transform(input), expected);
}

#[test]
fn test_receiver_vector_contains() {
    let input = "module 0x1::test { fun f() { vector::contains(&v, &e); } }";
    let expected = "module 0x1::test { fun f() { v.contains(&e); } }";
    assert_eq!(transform(input), expected);
}

#[test]
fn test_receiver_vector_swap() {
    let input = "module 0x1::test { fun f() { vector::swap(&mut v, i, j); } }";
    let expected = "module 0x1::test { fun f() { v.swap(i, j); } }";
    assert_eq!(transform(input), expected);
}

#[test]
fn test_receiver_vector_append() {
    let input = "module 0x1::test { fun f() { vector::append(&mut v1, v2); } }";
    let expected = "module 0x1::test { fun f() { v1.append(v2); } }";
    assert_eq!(transform(input), expected);
}

#[test]
fn test_receiver_vector_reverse() {
    let input = "module 0x1::test { fun f() { vector::reverse(&mut v); } }";
    let expected = "module 0x1::test { fun f() { v.reverse(); } }";
    assert_eq!(transform(input), expected);
}

#[test]
fn test_receiver_vector_remove() {
    let input = "module 0x1::test { fun f() { vector::remove(&mut v, i); } }";
    let expected = "module 0x1::test { fun f() { v.remove(i); } }";
    assert_eq!(transform(input), expected);
}

#[test]
fn test_receiver_vector_destroy_empty() {
    let input = "module 0x1::test { fun f() { vector::destroy_empty(v); } }";
    let expected = "module 0x1::test { fun f() { v.destroy_empty(); } }";
    assert_eq!(transform(input), expected);
}

#[test]
fn test_receiver_vector_with_type_args() {
    // Type args need :: prefix in receiver style
    let input = "module 0x1::test { fun f() { vector::remove<u64>(&mut v, i); } }";
    let expected = "module 0x1::test { fun f() { v.remove::<u64>(i); } }";
    assert_eq!(transform(input), expected);
}

// -- option --

#[test]
fn test_receiver_option_is_some() {
    let input = "module 0x1::test { fun f() { option::is_some(&o); } }";
    let expected = "module 0x1::test { fun f() { o.is_some(); } }";
    assert_eq!(transform(input), expected);
}

#[test]
fn test_receiver_option_is_none() {
    let input = "module 0x1::test { fun f() { option::is_none(&opt); } }";
    let expected = "module 0x1::test { fun f() { opt.is_none(); } }";
    assert_eq!(transform(input), expected);
}

#[test]
fn test_receiver_option_borrow() {
    let input = "module 0x1::test { fun f() { option::borrow(&opt); } }";
    let expected = "module 0x1::test { fun f() { opt.borrow(); } }";
    assert_eq!(transform(input), expected);
}

#[test]
fn test_receiver_option_extract() {
    let input = "module 0x1::test { fun f() { option::extract(&mut opt); } }";
    let expected = "module 0x1::test { fun f() { opt.extract(); } }";
    assert_eq!(transform(input), expected);
}

#[test]
fn test_receiver_option_destroy_some() {
    let input = "module 0x1::test { fun f() { option::destroy_some(opt); } }";
    let expected = "module 0x1::test { fun f() { opt.destroy_some(); } }";
    assert_eq!(transform(input), expected);
}

#[test]
fn test_receiver_option_contains() {
    let input = "module 0x1::test { fun f() { option::contains(&opt, &val); } }";
    let expected = "module 0x1::test { fun f() { opt.contains(&val); } }";
    assert_eq!(transform(input), expected);
}

// -- string --

#[test]
fn test_receiver_string_length() {
    let input = "module 0x1::test { fun f() { string::length(&s); } }";
    let expected = "module 0x1::test { fun f() { s.length(); } }";
    assert_eq!(transform(input), expected);
}

#[test]
fn test_receiver_string_is_empty() {
    let input = "module 0x1::test { fun f() { string::is_empty(&s); } }";
    let expected = "module 0x1::test { fun f() { s.is_empty(); } }";
    assert_eq!(transform(input), expected);
}

#[test]
fn test_receiver_string_append() {
    let input = "module 0x1::test { fun f() { string::append(&mut s, other); } }";
    let expected = "module 0x1::test { fun f() { s.append(other); } }";
    assert_eq!(transform(input), expected);
}

#[test]
fn test_receiver_string_sub_string() {
    let input = "module 0x1::test { fun f() { string::sub_string(&s, 0, 5); } }";
    let expected = "module 0x1::test { fun f() { s.sub_string(0, 5); } }";
    assert_eq!(transform(input), expected);
}

// -- signer --

#[test]
fn test_receiver_signer_address_of() {
    let input = "module 0x1::test { fun f(account: &signer) { signer::address_of(account); } }";
    let expected =
        "module 0x1::test { fun f(account: &signer) { account.address_of(); } }";
    assert_eq!(transform(input), expected);
}

// -- table --

#[test]
fn test_receiver_table_add() {
    let input = "module 0x1::test { fun f() { table::add(&mut t, key, val); } }";
    let expected = "module 0x1::test { fun f() { t.add(key, val); } }";
    assert_eq!(transform(input), expected);
}

#[test]
fn test_receiver_table_contains() {
    let input = "module 0x1::test { fun f() { table::contains(&t, key); } }";
    let expected = "module 0x1::test { fun f() { t.contains(key); } }";
    assert_eq!(transform(input), expected);
}

// -- simple_map --

#[test]
fn test_receiver_simple_map_contains_key() {
    let input = "module 0x1::test { fun f() { simple_map::contains_key(&m, &key); } }";
    let expected = "module 0x1::test { fun f() { m.contains_key(&key); } }";
    assert_eq!(transform(input), expected);
}

#[test]
fn test_receiver_simple_map_add() {
    let input = "module 0x1::test { fun f() { simple_map::add(&mut m, key, val); } }";
    let expected = "module 0x1::test { fun f() { m.add(key, val); } }";
    assert_eq!(transform(input), expected);
}

// -- negative cases --

#[test]
fn test_receiver_no_transform_unknown_func() {
    // Unknown module::func should not be transformed
    let input = "module 0x1::test { fun f() { my_module::do_thing(&obj, x); } }";
    assert_eq!(transform(input), input);
}

#[test]
fn test_receiver_no_transform_unqualified() {
    // Unqualified function call should not be transformed
    let input = "module 0x1::test { fun f() { push_back(&mut v, 42); } }";
    assert_eq!(transform(input), input);
}

// -- integration: receiver style + other transforms combined --

#[test]
fn test_receiver_combined_with_other_transforms() {
    let input = r#"module 0x1::counter {
    struct Counters has key {
        data: vector<u64>,
    }

    public fun add(addr: address) acquires Counters {
        let counters = borrow_global_mut<Counters>(addr);
        vector::push_back(&mut counters.data, 0);
    }

    public fun get(addr: address, i: u64): u64 acquires Counters {
        *vector::borrow(&borrow_global<Counters>(addr).data, i)
    }

    public fun len(addr: address): u64 acquires Counters {
        vector::length(&borrow_global<Counters>(addr).data)
    }
}"#;
    let expected = r#"module 0x1::counter {
    struct Counters has key {
        data: vector<u64>,
    }

    public fun add(addr: address) {
        let counters = &mut Counters[addr];
        counters.data.push_back(0);
    }

    public fun get(addr: address, i: u64): u64 {
        Counters[addr].data[i]
    }

    public fun len(addr: address): u64 {
        Counters[addr].data.length()
    }
}"#;
    assert_eq!(transform(input), expected);
}
