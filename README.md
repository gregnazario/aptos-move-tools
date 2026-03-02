# aptos-move-tools

A collection of lightweight static analysis and transformation tools for [Move on Aptos](https://aptos.dev/en/build/smart-contracts).

Most tools parse Move source using the `tree-sitter-move-on-aptos` grammar — no full compiler required. They're fast, composable, and work on individual files or entire directory trees. The native bounds checker variant uses the Move compiler's own parser for higher accuracy.

## Tools

| Tool | Purpose |
|------|---------|
| [**move-suggest**](move-suggest/) | Linter that suggests idiomatic Move 2 style (receiver syntax, vector literals, index notation) |
| [**move-bounds-checker**](move-bounds-checker/) | Static checker that catches Aptos VM limit violations before deployment (tree-sitter) |
| [**move-bounds-checker-native**](move-bounds-checker-native/) | Same bounds checking using the native Move compiler parser for higher accuracy |
| [**move1-to-move2**](move1-to-move2/) | Automated transformer that migrates Move 1 code to Move 2 syntax |

## Building

Each tool is a standalone Rust binary. From the tool's directory:

```bash
cargo build --release
```

Or build all tools from the repo root:

```bash
make release
```

## How They Work

The tree-sitter-based tools (`move-suggest`, `move-bounds-checker`, `move1-to-move2`) share the same approach:

1. Parse `.move` files into a concrete syntax tree using tree-sitter
2. Walk the tree looking for specific patterns
3. Report findings or apply transformations

This tree-sitter-based approach means the tools are fast (no compilation needed), error-tolerant (partial parses still work), and independent of the Move compiler toolchain.

`move-bounds-checker-native` takes a different approach — it uses the Move compiler's own parser (`legacy-move-compiler`) to build a full AST, giving it access to richer type information at the cost of requiring the `aptos-core` source tree.
