# move1-to-move2

An automated code transformer that migrates Move 1 syntax to Move 2. Applies all transformations in place, running multiple passes until no more edits are found.

## Usage

```bash
move1-to-move2 file.move [file2.move ...]
```

Exit codes: `0` = success, `1` = error.

## Transformations

| Move 1 | Move 2 | Rule |
|--------|--------|------|
| `borrow_global<T>(addr)` | `&T[addr]` | index syntax |
| `borrow_global_mut<T>(addr)` | `&mut T[addr]` | index syntax |
| `*borrow_global<T>(addr)` | `T[addr]` | deref cancels `&` |
| `&borrow_global<T>(addr)` | `&T[addr]` | redundant borrow absorbed |
| `borrow_global<T>(addr).field` | `T[addr].field` | field access strips prefix |
| `borrow_global<T>(addr)` in spec | `T[addr]` | spec block value semantics |
| `vector::borrow(&v, i)` | `&v[i]` | vector index syntax |
| `vector::borrow_mut(&mut v, i)` | `&mut v[i]` | vector index syntax |
| `*vector::borrow(&v, i)` | `v[i]` | deref cancels `&` |
| `vector::empty<T>()` | `vector<T>[]` | vector literal |
| `vector::push_back(&mut v, e)` | `v.push_back(e)` | receiver style |
| `vector::length(&v)` | `v.length()` | receiver style |
| `option::is_some(&o)` | `o.is_some()` | receiver style |
| `string::length(&s)` | `s.length()` | receiver style |
| `table::add(&mut t, k, v)` | `t.add(k, v)` | receiver style |
| `simple_map::contains_key(&m, &k)` | `m.contains_key(&k)` | receiver style |
| `(x as u64)` | `x as u64` | cast paren removal |
| `fun f() acquires T { }` | `fun f() { }` | strip acquires |
| `public(friend) fun f()` | `friend fun f()` | visibility |
| `public(package) fun f()` | `package fun f()` | visibility |
| `x = x + y` | `x += y` | compound assign (`+`,`-`,`*`,`/`,`%`) |
| `let i = 0; while (i < n) { ...; i = i + 1; }` | `for (i in 0..n) { ...; }` | while to for |

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

### `vector::borrow` / `vector::borrow_mut` → index syntax

Converts vector element access to index notation, with the same edge-case handling as `borrow_global`.

```move
# Before
vector::borrow(&v, i)
vector::borrow_mut(&mut v, i)
*vector::borrow(&v, i)

# After
&v[i]
&mut v[i]
v[i]
```

### `vector::empty` → vector literal

```move
# Before
vector::empty<u64>()

# After
vector<u64>[]
```

### `receiver_style` — stdlib dot-call syntax

Converts fully-qualified stdlib calls to receiver-style (dot) syntax. The compiler auto-borrows the first argument, so `&`/`&mut` wrappers are stripped.

```move
# Before
vector::push_back(&mut v, 42);
vector::length(&v);
option::is_some(&o);
string::append(&mut s, other);

# After
v.push_back(42);
v.length();
o.is_some();
s.append(other);
```

Supported modules: `vector`, `option`, `string`, `table`, `smart_table`, `smart_vector`, `simple_map`.

### `cast_parens`

Removes unnecessary parentheses around `as` casts (Move 2 allows casts as top-level expressions).

```move
# Before
let x = (amount as u64);

# After
let x = amount as u64;
```

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

### `while_to_for`

Converts counter-based `while` loops to Move 2 `for` loops with ranges.

```move
# Before
let i = 0;
while (i < len) {
    do_thing(i);
    i = i + 1;
};

# After
for (i in 0..len) {
    do_thing(i);
};
```

Requirements for conversion:
- A `let i = 0;` (or other literal) immediately precedes the `while`
- The condition is `i < bound`
- The last statement in the loop body is `i = i + 1`
- The loop variable `i` is **not used** after the `while` loop (since `for` scopes it to the loop body)
