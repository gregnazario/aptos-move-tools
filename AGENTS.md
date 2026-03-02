# Agent Guidelines

## Adding a New Tool

When adding a new tool to this repository:

1. **Makefile**: Add the tool's directory name to the `TOOLS` list in the top-level `Makefile`.
2. **Tool README**: Create a `README.md` inside the tool's directory covering:
   - What the tool does
   - Usage and CLI interface
   - All rules/checks/transformations with before/after examples
3. **Top-level README**: Update the tools table in the top-level `README.md` with a row for the new tool.
4. **Dependencies**: Use the same `tree-sitter` and `tree-sitter-move-on-aptos` dependency versions as the existing tools (git dependency, pinned rev).

## Code Standards

- Run `make lint` before committing (clippy with `-D warnings` + format check).
- All tools must have integration tests in a `tests/` directory.
