# move-bounds-checker-native

A static analyzer that checks Move source files against the Aptos VM's bytecode verifier limits — the same as [move-bounds-checker](../move-bounds-checker/), but using the Move compiler's own parser (`legacy-move-compiler`) instead of tree-sitter. This gives it access to the full compiler AST for higher accuracy on complex patterns.

## Usage

```bash
# Check a directory (recursively finds .move files)
move-bounds-checker-native sources/

# Check specific files
move-bounds-checker-native module.move

# Override default limits
move-bounds-checker-native --max-loop-depth=3 --max-basic-blocks=512 sources/

# Address identification mode
move-bounds-checker-native --identify sources/
move-bounds-checker-native --identify --all sources/
move-bounds-checker-native --identify --explorer-local=../explorer sources/
```

Exit codes: `0` = no violations, `1` = violations found, `2` = error.

Files are processed in parallel using [rayon](https://docs.rs/rayon) (with 64 MB thread stacks for deeply nested ASTs).

## Checks

Default limits match the current Aptos mainnet production configuration.

| Check | Default Limit | Flag |
|-------|--------------|------|
| Loop nesting depth | 5 | `--max-loop-depth` |
| Generic instantiation length | 32 | `--max-generic-instantiation-length` |
| Function parameters | 128 | `--max-function-parameters` |
| Basic blocks (heuristic) | 1024 | `--max-basic-blocks` |
| Type nodes per function | 128 | `--max-type-nodes` |
| Function return values | 128 | `--max-function-return-values` |
| Type nesting depth | 20 | `--max-type-depth` |
| Struct/enum definitions per module | 200 | `--max-struct-definitions` |
| Enum variants | 64 | `--max-struct-variants` |
| Fields per struct/variant | 64 | `--max-fields-in-struct` |
| Function definitions per module | 1000 | `--max-function-definitions` |
| Identifier length | 255 | `--max-identifier-length` |
| Local variables per function | 255 | `--max-locals` |
| Type parameters | 255 | `--max-type-parameter-count` |

### Notes

- **Basic blocks** are estimated heuristically from source-level control flow. The actual bytecode count may differ, but this catches functions that are clearly over the limit.
- **Type nodes** and **type depth** are counted across the full function scope (parameters, return type, and body).
- **Spec blocks** are skipped since they don't produce bytecode.
- **Native functions** are skipped for body-level checks.

## Address Identification

With `--identify`, violations are grouped by Aptos account address (extracted from file paths) and cross-referenced against the [Aptos Explorer](https://github.com/aptos-labs/explorer) known-address list.

The report shows:
- **Scam-flagged addresses** — accounts marked as scams in the Explorer data
- **Labeled addresses** — known accounts with a human-readable name, sorted by violation count
- **Unlabeled addresses** — unknown accounts (top 20 by default, `--all` for full list)

Labels are fetched from GitHub by default. Use `--explorer-local=<PATH>` to point at a local checkout of the Explorer repo instead.

## vs. move-bounds-checker (tree-sitter)

| | tree-sitter | native |
|---|---|---|
| Parser | tree-sitter grammar | Move compiler AST |
| Accuracy | Good for most patterns | Higher (full type info) |
| Speed | Faster (no compiler overhead) | Slightly slower |
| Dependencies | Standalone | Requires `aptos-core` checkout |

Both tools share the same CLI interface, checks, default limits, and output format. Use the tree-sitter version for quick checks and CI; use the native version when you need the compiler's full understanding of the code.

## Building

Requires `aptos-core` as a sibling directory:

```bash
git clone https://github.com/aptos-labs/aptos-core.git ../aptos-core
cargo build --release
```