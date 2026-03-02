# move-suggest

A linter that suggests idiomatic Move 2 style improvements. It can report suggestions or auto-fix them in place.

## Usage

```bash
# Report suggestions
move-suggest file.move

# Auto-fix files in place
move-suggest --fix file.move [file2.move ...]
```

Exit codes: `0` = no suggestions, `1` = suggestions found, `2` = error.

## Rules

### `receiver_style`

Converts qualified function calls to receiver (method) syntax for `vector`, `string`, and `option` modules.

```move
# Before
vector::push_back(&mut v, x);
vector::length(&v);
option::is_some(&o);
string::bytes(&s);

# After
v.push_back(x);
v.length();
o.is_some();
s.bytes();
```

Supported methods: `push_back`, `pop_back`, `length`, `is_empty`, `contains`, `index_of`, `append`, `reverse`, `swap` (vector); `length`, `bytes` (string); `is_some`, `is_none`, `borrow`, `borrow_mut`, `extract`, `contains`, `swap`, `destroy_some`, `destroy_none` (option).

### `vector_borrow_index` (via `receiver_style`)

Converts `vector::borrow` / `vector::borrow_mut` to index notation.

```move
# Before
vector::borrow(&v, i);
vector::borrow_mut(&mut v, i);

# After
&v[i];
&mut v[i];
```

### `vector_empty_literal` / `vector_singleton_literal`

Converts `vector::empty()` and `vector::singleton()` to vector literal syntax.

```move
# Before
vector::empty<u64>();
vector::singleton(42);

# After
vector<u64>[];
vector[42];
```

### `vector_multi_push`

Detects `vector::empty()` followed by consecutive `push_back` calls and collapses them into a single vector literal.

```move
# Before
let v = vector::empty<u64>();
vector::push_back(&mut v, 1);
vector::push_back(&mut v, 2);
vector::push_back(&mut v, 3);

# After
let v = vector<u64>[1, 2, 3];
```

Requires at least 2 consecutive `push_back` calls to trigger.
