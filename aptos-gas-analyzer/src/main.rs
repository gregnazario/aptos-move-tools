use std::env;
use std::process;
use std::thread;
use std::time::Duration;

use serde::Deserialize;

// ── ANSI Colors ─────────────────────────────────────────────────

const RED: &str = "\x1b[31m";
const GREEN: &str = "\x1b[32m";
const YELLOW: &str = "\x1b[33m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const RESET: &str = "\x1b[0m";

// ── CLI Configuration ───────────────────────────────────────────

#[derive(Clone)]
enum Mode {
    Account(String),
    Versions(u64, u64),
    Txns(Vec<String>),
    Live,
}

#[derive(Clone)]
struct Config {
    mode: Mode,
    multiplier: u64,
    base_url: String,
    limit: u64,
    interval_secs: u64,
    json_output: bool,
}

fn parse_args() -> Config {
    let args: Vec<String> = env::args().collect();

    let mut mode: Option<Mode> = None;
    let mut multiplier: u64 = 10;
    let mut network = "mainnet".to_string();
    let mut url_override: Option<String> = None;
    let mut limit: u64 = 100;
    let mut interval_secs: u64 = 5;
    let mut json_output = false;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--account" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("Error: --account requires an address argument");
                    process::exit(2);
                }
                mode = Some(Mode::Account(args[i].clone()));
            }
            "--versions" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("Error: --versions requires a START..END argument");
                    process::exit(2);
                }
                let parts: Vec<&str> = args[i].split("..").collect();
                if parts.len() != 2 {
                    eprintln!("Error: --versions expects START..END format");
                    process::exit(2);
                }
                let start = parts[0].parse::<u64>().unwrap_or_else(|_| {
                    eprintln!("Error: invalid start version: {}", parts[0]);
                    process::exit(2);
                });
                let end = parts[1].parse::<u64>().unwrap_or_else(|_| {
                    eprintln!("Error: invalid end version: {}", parts[1]);
                    process::exit(2);
                });
                if start >= end {
                    eprintln!("Error: start version must be less than end version");
                    process::exit(2);
                }
                mode = Some(Mode::Versions(start, end));
            }
            "--txns" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("Error: --txns requires comma-separated hashes");
                    process::exit(2);
                }
                let hashes: Vec<String> =
                    args[i].split(',').map(|s| s.trim().to_string()).collect();
                if hashes.is_empty() {
                    eprintln!("Error: --txns requires at least one hash");
                    process::exit(2);
                }
                mode = Some(Mode::Txns(hashes));
            }
            "--live" => {
                mode = Some(Mode::Live);
            }
            "--multiplier" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("Error: --multiplier requires a number");
                    process::exit(2);
                }
                multiplier = args[i].parse::<u64>().unwrap_or_else(|_| {
                    eprintln!("Error: invalid multiplier: {}", args[i]);
                    process::exit(2);
                });
            }
            "--network" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("Error: --network requires mainnet|testnet|devnet");
                    process::exit(2);
                }
                network = args[i].clone();
            }
            "--url" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("Error: --url requires a URL");
                    process::exit(2);
                }
                url_override = Some(args[i].clone());
            }
            "--limit" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("Error: --limit requires a number");
                    process::exit(2);
                }
                limit = args[i].parse::<u64>().unwrap_or_else(|_| {
                    eprintln!("Error: invalid limit: {}", args[i]);
                    process::exit(2);
                });
            }
            "--interval" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("Error: --interval requires seconds");
                    process::exit(2);
                }
                interval_secs = args[i].parse::<u64>().unwrap_or_else(|_| {
                    eprintln!("Error: invalid interval: {}", args[i]);
                    process::exit(2);
                });
            }
            "--json" => {
                json_output = true;
            }
            other => {
                eprintln!("Error: unknown argument: {}", other);
                print_usage();
                process::exit(2);
            }
        }
        i += 1;
    }

    let mode = match mode {
        Some(m) => m,
        None => {
            eprintln!("Error: no mode specified");
            print_usage();
            process::exit(2);
        }
    };

    let base_url = match url_override {
        Some(url) => url.trim_end_matches('/').to_string(),
        None => network_to_url(&network),
    };

    Config {
        mode,
        multiplier,
        base_url,
        limit,
        interval_secs,
        json_output,
    }
}

fn network_to_url(network: &str) -> String {
    match network {
        "mainnet" => "https://api.mainnet.aptoslabs.com/v1".to_string(),
        "testnet" => "https://api.testnet.aptoslabs.com/v1".to_string(),
        "devnet" => "https://api.devnet.aptoslabs.com/v1".to_string(),
        other => {
            eprintln!(
                "Error: unknown network '{}' (expected mainnet, testnet, or devnet)",
                other
            );
            process::exit(2);
        }
    }
}

fn print_usage() {
    eprintln!(
        "\
Usage: aptos-gas-analyzer [OPTIONS] <MODE>

Modes:
  --account <ADDRESS>          Analyze recent txns for an account
  --versions <START>..<END>    Analyze txns in a ledger version range
  --txns <HASH>[,<HASH>,...]   Analyze specific txns by hash
  --live                       Poll for new txns continuously

Options:
  --multiplier <N>     Gas cost multiplier (default: 10)
  --network <NET>      mainnet|testnet|devnet (default: mainnet)
  --url <URL>          Custom API URL (overrides --network)
  --limit <N>          Max txns to fetch per request (default: 100)
  --interval <SECS>    Polling interval for --live mode (default: 5)
  --json               Output JSON instead of human-readable table"
    );
}

// ── Transaction Types ───────────────────────────────────────────

#[derive(Debug, Deserialize, Clone)]
#[allow(dead_code)]
struct Transaction {
    #[serde(rename = "type", default)]
    tx_type: String,
    #[serde(default)]
    version: String,
    #[serde(default)]
    hash: String,
    #[serde(default)]
    sender: String,
    #[serde(default)]
    gas_used: String,
    #[serde(default)]
    max_gas_amount: String,
    #[serde(default)]
    gas_unit_price: String,
    #[serde(default)]
    success: bool,
    #[serde(default)]
    vm_status: String,
}

// ── API Client ──────────────────────────────────────────────────

fn api_get(url: &str) -> Result<String, String> {
    match ureq::get(url).call() {
        Ok(response) => match response.into_body().read_to_string() {
            Ok(body) => Ok(body),
            Err(e) => Err(format!("Failed to read response body: {}", e)),
        },
        Err(_) => {
            // Retry once after 2s backoff
            thread::sleep(Duration::from_secs(2));
            match ureq::get(url).call() {
                Ok(response) => match response.into_body().read_to_string() {
                    Ok(body) => Ok(body),
                    Err(e) => Err(format!("Failed to read response body on retry: {}", e)),
                },
                Err(e) => Err(format!("API request failed after retry: {}", e)),
            }
        }
    }
}

fn fetch_account_transactions(
    base_url: &str,
    address: &str,
    limit: u64,
) -> Result<Vec<Transaction>, String> {
    let url = format!(
        "{}/accounts/{}/transactions?limit={}",
        base_url, address, limit
    );
    let body = api_get(&url)?;
    serde_json::from_str(&body).map_err(|e| format!("Failed to parse transactions: {}", e))
}

fn fetch_transactions_by_version(
    base_url: &str,
    start: u64,
    limit: u64,
) -> Result<Vec<Transaction>, String> {
    let url = format!("{}/transactions?start={}&limit={}", base_url, start, limit);
    let body = api_get(&url)?;
    serde_json::from_str(&body).map_err(|e| format!("Failed to parse transactions: {}", e))
}

fn fetch_transaction_by_hash(base_url: &str, hash: &str) -> Result<Transaction, String> {
    let url = format!("{}/transactions/by_hash/{}", base_url, hash);
    let body = api_get(&url)?;
    serde_json::from_str(&body).map_err(|e| format!("Failed to parse transaction: {}", e))
}

// ── Gas Analysis ────────────────────────────────────────────────

struct AnalysisResult {
    version: String,
    hash: String,
    sender: String,
    gas_used: u64,
    max_gas_amount: u64,
    gas_unit_price: u64,
    simulated_gas: u64,
    headroom_pct: f64,
    would_fail: bool,
    original_success: bool,
}

struct Summary {
    total_user_transactions: usize,
    would_fail_count: usize,
    would_fail_pct: f64,
    already_failed_count: usize,
    closest_to_failing: Option<(String, f64)>, // (version, headroom_pct)
}

fn analyze_transactions(txns: &[Transaction], multiplier: u64) -> (Vec<AnalysisResult>, Summary) {
    let mut results = Vec::new();
    let mut total_user = 0usize;
    let mut would_fail_count = 0usize;
    let mut already_failed_count = 0usize;
    let mut closest: Option<(String, f64)> = None;

    for tx in txns {
        if tx.tx_type != "user_transaction" {
            continue;
        }
        total_user += 1;

        let gas_used = tx.gas_used.parse::<u64>().unwrap_or(0);
        let max_gas = tx.max_gas_amount.parse::<u64>().unwrap_or(0);
        let gas_unit_price = tx.gas_unit_price.parse::<u64>().unwrap_or(0);

        let simulated = gas_used.saturating_mul(multiplier);
        let headroom_pct = if max_gas > 0 {
            ((max_gas as f64) - (simulated as f64)) / (max_gas as f64) * 100.0
        } else {
            0.0
        };
        let would_fail = simulated > max_gas;

        if !tx.success {
            already_failed_count += 1;
        }

        if would_fail {
            would_fail_count += 1;
        }

        // Track closest to failing: smallest positive headroom
        if headroom_pct > 0.0 {
            match &closest {
                None => closest = Some((tx.version.clone(), headroom_pct)),
                Some((_, prev_pct)) => {
                    if headroom_pct < *prev_pct {
                        closest = Some((tx.version.clone(), headroom_pct));
                    }
                }
            }
        }

        results.push(AnalysisResult {
            version: tx.version.clone(),
            hash: tx.hash.clone(),
            sender: tx.sender.clone(),
            gas_used,
            max_gas_amount: max_gas,
            gas_unit_price,
            simulated_gas: simulated,
            headroom_pct,
            would_fail,
            original_success: tx.success,
        });
    }

    let would_fail_pct = if total_user > 0 {
        (would_fail_count as f64) / (total_user as f64) * 100.0
    } else {
        0.0
    };

    let summary = Summary {
        total_user_transactions: total_user,
        would_fail_count,
        would_fail_pct,
        already_failed_count,
        closest_to_failing: closest,
    };

    (results, summary)
}

// ── Output Formatting ───────────────────────────────────────────

fn shorten(s: &str, prefix: usize, suffix: usize) -> String {
    if s.len() <= prefix + suffix + 2 {
        return s.to_string();
    }
    format!("{}..{}", &s[..prefix], &s[s.len() - suffix..])
}

fn print_table(results: &[AnalysisResult], summary: &Summary, multiplier: u64) {
    let failing: Vec<&AnalysisResult> = results.iter().filter(|r| r.would_fail).collect();

    eprintln!(
        "Analyzing {} user transactions (multiplier: {}x)...",
        summary.total_user_transactions, multiplier
    );
    println!();

    if failing.is_empty() {
        println!("{GREEN}{BOLD}No transactions would fail at {multiplier}x gas cost.{RESET}",);
    } else {
        println!("{RED}{BOLD}WOULD FAIL at {multiplier}x gas cost:{RESET}",);
        println!();

        // Header
        println!(
            "  {BOLD}{:<12} {:<14} {:<16} {:>10} {:>10} {:>12} {:>10}{RESET}",
            "Version", "Hash", "Sender", "Gas Used", "Max Gas", "Simulated", "Headroom"
        );
        println!(
            "  {DIM}{:<12} {:<14} {:<16} {:>10} {:>10} {:>12} {:>10}{RESET}",
            "-------",
            "-----------",
            "--------------",
            "--------",
            "-------",
            "---------",
            "--------"
        );

        for r in &failing {
            let hash_short = shorten(&r.hash, 6, 4);
            let sender_short = shorten(&r.sender, 6, 4);
            let headroom_str = format!("{:.1}%", r.headroom_pct);
            let color = if r.headroom_pct < -50.0 { RED } else { YELLOW };
            println!(
                "  {color}{:<12} {:<14} {:<16} {:>10} {:>10} {:>12} {:>10}{RESET}",
                r.version,
                hash_short,
                sender_short,
                r.gas_used,
                r.max_gas_amount,
                r.simulated_gas,
                headroom_str
            );
        }
    }

    // Summary
    println!();
    println!("{BOLD}Summary:{RESET}");
    println!(
        "  Total user transactions analyzed: {}",
        summary.total_user_transactions
    );

    if summary.would_fail_count > 0 {
        println!(
            "  {RED}Would fail at {multiplier}x: {} ({:.1}%){RESET}",
            summary.would_fail_count, summary.would_fail_pct,
        );
    } else {
        println!("  {GREEN}Would fail at {multiplier}x: 0 (0.0%){RESET}",);
    }

    println!(
        "  Already failed (original): {}",
        summary.already_failed_count
    );

    if let Some((ref version, pct)) = summary.closest_to_failing {
        println!(
            "  {YELLOW}Closest to failing: version {} (headroom: {:.1}%){RESET}",
            version, pct
        );
    }
}

fn print_json(results: &[AnalysisResult], summary: &Summary, multiplier: u64) {
    let failing: Vec<serde_json::Value> = results
        .iter()
        .filter(|r| r.would_fail)
        .map(|r| {
            serde_json::json!({
                "version": r.version,
                "hash": r.hash,
                "sender": r.sender,
                "gas_used": r.gas_used,
                "max_gas_amount": r.max_gas_amount,
                "gas_unit_price": r.gas_unit_price,
                "simulated_gas": r.simulated_gas,
                "headroom_pct": (r.headroom_pct * 10.0).round() / 10.0,
                "would_fail": r.would_fail,
                "original_success": r.original_success,
            })
        })
        .collect();

    let closest = match &summary.closest_to_failing {
        Some((version, pct)) => serde_json::json!({
            "version": version,
            "headroom_pct": (*pct * 10.0).round() / 10.0,
        }),
        None => serde_json::Value::Null,
    };

    let output = serde_json::json!({
        "multiplier": multiplier,
        "failing_transactions": failing,
        "summary": {
            "total_user_transactions": summary.total_user_transactions,
            "would_fail_count": summary.would_fail_count,
            "would_fail_pct": (summary.would_fail_pct * 10.0).round() / 10.0,
            "already_failed_count": summary.already_failed_count,
            "closest_to_failing": closest,
        }
    });

    println!(
        "{}",
        serde_json::to_string_pretty(&output).unwrap_or_else(|_| "{}".to_string())
    );
}

// ── Execution Modes ─────────────────────────────────────────────

fn run(config: &Config) -> Result<(Vec<AnalysisResult>, Summary), String> {
    let txns = match &config.mode {
        Mode::Account(address) => {
            eprintln!("Fetching transactions for account {}...", address);
            fetch_account_transactions(&config.base_url, address, config.limit)?
        }
        Mode::Versions(start, end) => {
            eprintln!("Fetching transactions for versions {}..{}...", start, end);
            let mut all_txns = Vec::new();
            let mut current = *start;
            while current < *end {
                let batch_limit = std::cmp::min(config.limit, *end - current);
                let batch = fetch_transactions_by_version(&config.base_url, current, batch_limit)?;
                if batch.is_empty() {
                    break;
                }
                // Advance past what we received
                let last_version = batch
                    .last()
                    .and_then(|t| t.version.parse::<u64>().ok())
                    .unwrap_or(current + batch_limit);
                all_txns.extend(batch);
                current = last_version + 1;
            }
            all_txns
        }
        Mode::Txns(hashes) => {
            eprintln!("Fetching {} transaction(s) by hash...", hashes.len());
            let mut txns = Vec::new();
            for hash in hashes {
                match fetch_transaction_by_hash(&config.base_url, hash) {
                    Ok(tx) => txns.push(tx),
                    Err(e) => eprintln!("Warning: failed to fetch {}: {}", hash, e),
                }
            }
            txns
        }
        Mode::Live => {
            // Live mode is handled separately
            unreachable!("Live mode should not reach run()")
        }
    };

    Ok(analyze_transactions(&txns, config.multiplier))
}

fn run_live(config: &Config) -> ! {
    eprintln!(
        "Live mode: polling every {}s with {}x multiplier...",
        config.interval_secs, config.multiplier
    );

    let mut last_version: Option<u64> = None;

    loop {
        let start = match last_version {
            Some(v) => v + 1,
            None => {
                // Get the latest transactions to find current version
                match fetch_transactions_by_version(&config.base_url, 0, 1) {
                    Ok(txns) => {
                        if let Some(tx) = txns.first() {
                            // Start from a recent version
                            tx.version.parse::<u64>().unwrap_or(0)
                        } else {
                            0
                        }
                    }
                    Err(e) => {
                        eprintln!("Error fetching latest version: {}", e);
                        thread::sleep(Duration::from_secs(config.interval_secs));
                        continue;
                    }
                }
            }
        };

        match fetch_transactions_by_version(&config.base_url, start, config.limit) {
            Ok(txns) => {
                if txns.is_empty() {
                    eprint!(
                        "\r{DIM}[polling] No new transactions since version {}{RESET}    ",
                        start
                    );
                    thread::sleep(Duration::from_secs(config.interval_secs));
                    continue;
                }

                // Update last_version
                if let Some(last) = txns.last()
                    && let Ok(v) = last.version.parse::<u64>()
                {
                    last_version = Some(v);
                }

                let (results, summary) = analyze_transactions(&txns, config.multiplier);

                if summary.would_fail_count > 0 {
                    eprintln!(); // Clear the status line
                    if config.json_output {
                        print_json(&results, &summary, config.multiplier);
                    } else {
                        print_table(&results, &summary, config.multiplier);
                    }
                } else {
                    eprint!(
                        "\r{DIM}[polling] {} user txns OK at version ~{}{RESET}    ",
                        summary.total_user_transactions,
                        last_version.unwrap_or(start)
                    );
                }
            }
            Err(e) => {
                eprintln!("\rError fetching transactions: {}", e);
            }
        }

        thread::sleep(Duration::from_secs(config.interval_secs));
    }
}

// ── Main ────────────────────────────────────────────────────────

fn main() {
    let config = parse_args();

    if matches!(config.mode, Mode::Live) {
        run_live(&config);
    }

    match run(&config) {
        Ok((results, summary)) => {
            if config.json_output {
                print_json(&results, &summary, config.multiplier);
            } else {
                print_table(&results, &summary, config.multiplier);
            }
            if summary.would_fail_count > 0 {
                process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("Error: {}", e);
            process::exit(2);
        }
    }
}
