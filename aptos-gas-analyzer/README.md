# aptos-gas-analyzer

Analyzes Aptos transaction gas costs under a simulated multiplier (e.g., 10x) to identify which transactions would fail if gas costs increased. Useful for assessing the impact of fee changes or network upgrades.

## Usage

```bash
# Analyze recent transactions for an account
aptos-gas-analyzer --account 0xADDR

# Analyze transactions in a ledger version range
aptos-gas-analyzer --versions 1000..2000

# Analyze specific transactions by hash
aptos-gas-analyzer --txns HASH1,HASH2

# Continuously poll for transactions that would fail
aptos-gas-analyzer --live
```

Exit codes: `0` = no failures at multiplier, `1` = failures found, `2` = error.

## Modes

Exactly one mode is required:

| Flag | Description |
|------|-------------|
| `--account <ADDRESS>` | Recent transactions for an account |
| `--versions <START>..<END>` | Transactions in a ledger version range |
| `--txns <HASH>[,<HASH>,...]` | Specific transactions by hash |
| `--live` | Poll continuously for new transactions |

## Options

| Flag | Default | Description |
|------|---------|-------------|
| `--multiplier <N>` | `10` | Gas cost multiplier for "what-if" analysis |
| `--network <NET>` | `mainnet` | Network: `mainnet`, `testnet`, or `devnet` |
| `--url <URL>` | — | Custom API endpoint (overrides `--network`) |
| `--limit <N>` | `100` | Max transactions per API request |
| `--interval <SECS>` | `5` | Polling interval for `--live` mode |
| `--json` | — | Output JSON instead of human-readable table |

## How It Works

For each user transaction, the analyzer computes:

```
simulated_gas = gas_used × multiplier
would_fail    = simulated_gas > max_gas_amount
headroom_pct  = ((max_gas_amount - simulated_gas) / max_gas_amount) × 100%
```

A positive headroom means the transaction has budget to spare; negative means it would exceed its gas limit.

## Output

The default human-readable output shows a table of failing transactions with columns for version, hash, sender, gas used, max gas, simulated gas, and headroom percentage. A summary follows with totals, failure rate, and the transaction closest to failing.

With `--json`, the output is a JSON object:

```json
{
  "multiplier": 10,
  "failing_transactions": [
    {
      "version": "123456",
      "hash": "0xabc...",
      "sender": "0x123...",
      "gas_used": 500,
      "max_gas_amount": 2000,
      "gas_unit_price": 100,
      "simulated_gas": 5000,
      "headroom_pct": -150.0,
      "would_fail": true,
      "original_success": true
    }
  ],
  "summary": {
    "total_user_transactions": 100,
    "would_fail_count": 3,
    "would_fail_pct": 3.0,
    "already_failed_count": 1,
    "closest_to_failing": { "version": "789", "headroom_pct": 5.2 }
  }
}
```

## Examples

```bash
# Which of account's recent txns would fail at 10x gas?
aptos-gas-analyzer --account 0x1

# Test a tighter multiplier on testnet
aptos-gas-analyzer --account 0x1 --multiplier 5 --network testnet

# JSON output for scripting
aptos-gas-analyzer --versions 100000..100500 --json

# Monitor mainnet in real time
aptos-gas-analyzer --live --multiplier 10 --interval 10
```
