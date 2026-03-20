# named-address-recover

Recovers named address mappings for Move packages by cross-referencing `Move.toml`, source files, and compiled bytecode. Supports both local packages and on-chain accounts (Aptos mainnet).

## Usage

```bash
# Local package
named-address-recover <path>

# On-chain account
named-address-recover --address 0xHEX

# On-chain + local sources
named-address-recover --address 0xHEX --source ./sources

# Output as Move.toml format
named-address-recover <path> --toml

# Show resolution sources
named-address-recover <path> --verbose
```

Exit codes: `0` = addresses found, `1` = no addresses found, `2` = error.

## Options

| Flag | Description |
|------|-------------|
| `<path>` | Path to a local Move package root (has `Move.toml`) |
| `--address <HEX>` | Fetch modules from Aptos mainnet at this address |
| `--source <path>` | Supplement on-chain data with local `.move` files (use with `--address`) |
| `--toml` | Output as a `[addresses]` TOML section for `Move.toml` |
| `--verbose` | Show where each mapping was resolved from |

## How It Resolves Addresses

The tool uses a 3-tier resolution system (highest priority first):

### 1. Move.toml direct mappings

Entries in `[addresses]` take priority. Underscore (`"_"`) placeholders are skipped.

### 2. Module-name matching

Cross-references source and bytecode: if source uses `my_addr::cool_module` and bytecode shows `cool_module` lives at `0xABCD`, then `my_addr = 0xABCD`.

### 3. Well-known Aptos addresses

Applied only when the name appears in source code:

| Name | Address |
|------|---------|
| `std` | `0x1` |
| `aptos_std` | `0x1` |
| `aptos_framework` | `0x1` |
| `aptos_token` | `0x3` |
| `aptos_token_objects` | `0x4` |

## Output Formats

**Default:**
```
my_addr = 0xABCD
std = 0x1
```

**`--toml`:**
```toml
[addresses]
my_addr = "0xABCD"
std = "0x1"
```

**`--verbose`:**
```
my_addr = 0xABCD  (bytecode-match)
std = 0x1  (well-known)
deployer = 0x5678  (Move.toml)
```

## On-Chain Mode

With `--address`, the tool fetches from Aptos mainnet:

- **Bytecode**: paginated module list from `/accounts/{addr}/modules`
- **Source code**: decompressed from the `PackageRegistry` resource

Use `--source` to provide local `.move` files when on-chain sources are incomplete.

## Building

Requires `aptos-core` as a sibling directory (for `move-binary-format` and `move-core-types`):

```bash
git clone https://github.com/aptos-labs/aptos-core.git ../aptos-core
cargo build --release
```