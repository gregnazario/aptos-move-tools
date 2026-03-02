# move1-to-move2

An automated code transformer that migrates Move 1 syntax to Move 2. Applies all transformations in place.

## Usage

```bash
move1-to-move2 file.move [file2.move ...]
```

Exit codes: `0` = success, `1` = error.

## Transformations

### `borrow_global` / `borrow_global_mut` → index syntax

Converts global storage access to the Move 2 resource indexing syntax.

```move
# Before
borrow_global<Counter>(addr)
borrow_global_mut<Counter>(addr)
*borrow_global<Counter>(addr)

# After
&Counter[addr]
&mut Counter[addr]
Counter[addr]
```

The tool handles several edge cases automatically:
- **Dereference**: `*borrow_global<T>(addr)` → `T[addr]` (the `*` and `&` cancel)
- **Redundant borrow**: `&borrow_global<T>(addr)` → `&T[addr]` (absorbs the outer `&`)
- **Field access**: `borrow_global<T>(addr).field` → `T[addr].field` (no prefix when followed by `.`)
- **Spec blocks**: `borrow_global` inside spec blocks omits the `&` prefix (spec semantics differ)

### `strip_acquires`

Removes `acquires` clauses, which are no longer needed in Move 2.

```move
# Before
public fun get(addr: address): u64 acquires Counter { ... }

# After
public fun get(addr: address): u64 { ... }
```

### `visibility`

Simplifies visibility modifiers.

```move
# Before
public(friend) fun f() {}
public(package) fun f() {}

# After
friend fun f() {}
package fun f() {}
```

### `compound_assign`

Converts `x = x op y` patterns to compound assignment operators.

```move
# Before
counter.value = counter.value + 1;
x = x * y;

# After
counter.value += 1;
x *= y;
```

Supports `+`, `-`, `*`, `/`, `%`. Only triggers when the left-hand side of the assignment exactly matches the left operand of the binary expression.
