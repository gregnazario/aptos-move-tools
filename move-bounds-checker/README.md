# move-bounds-checker

A static analyzer that checks Move source files against the Aptos VM's bytecode verifier limits. Catches limit violations at the source level — before you compile or deploy — so you get fast feedback on code that would fail on-chain.

## Usage

```bash
# Check a directory (recursively finds .move files)
move-bounds-checker sources/

# Check specific files
move-bounds-checker module.move

# Override default limits
move-bounds-checker --max-loop-depth=3 --max-basic-blocks=512 sources/
```

Exit codes: `0` = no violations, `1` = violations found, `2` = error.

Files are processed in parallel using [rayon](https://docs.rs/rayon).

## Checks

Default limits match the current Aptos mainnet production configuration (with `enable_function_values=true`).

| Check | Default Limit | Flag |
|-------|--------------|------|
| Loop nesting depth | 5 | `--max-loop-depth` |
| Generic instantiation length | 32 | `--max-generic-instantiation-length` |
| Function parameters | 128 | `--max-function-parameters` |
| Basic blocks (heuristic) | 1024 | `--max-basic-blocks` |
| Type nodes per function | 128 | `--max-type-nodes` |
| Function return values | 128 | `--max-function-return-values` |
| Type nesting depth | 20 | `--max-type-depth` |

### Notes

- **Basic blocks** are estimated heuristically from source-level control flow (`if`, `while`, `for`, `loop`, `break`, `continue`). The actual bytecode count may differ, but this catches functions that are clearly over the limit.
- **Type nodes** and **type depth** are counted across the full function scope (parameters, return type, and body).
- **Spec blocks** are skipped since they don't produce bytecode.
