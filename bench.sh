#!/usr/bin/env bash
set -euo pipefail

CORPUS="${1:-$HOME/git/move-modules/mainnet}"
RUNS="${2:-3}"

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
TS_DIR="$SCRIPT_DIR/move-bounds-checker"
NATIVE_DIR="$SCRIPT_DIR/move-bounds-checker-native"

echo "=== Building tree-sitter version (release) ==="
(cd "$TS_DIR" && cargo build --release 2>&1 | tail -1)
TS_BIN="$TS_DIR/target/release/move-bounds-checker"

echo "=== Building native parser version (release) ==="
(cd "$NATIVE_DIR" && cargo build --release 2>&1 | tail -1)
NATIVE_BIN="$NATIVE_DIR/target/release/move-bounds-checker-native"

echo ""
echo "Corpus: $CORPUS"
echo "Runs:   $RUNS"
echo ""

# Count files once
FILE_COUNT=$(find "$CORPUS" -name '*.move' | wc -l | tr -d ' ')
echo "Move files: $FILE_COUNT"
echo ""

# --- Tree-sitter benchmarks ---
echo "=== tree-sitter ==="
TS_TIMES=()
for i in $(seq 1 "$RUNS"); do
    # Use GNU time format for wall clock, user, sys
    TIMEFORMAT='%R %U %S'
    TIME_OUT=$( { time "$TS_BIN" "$CORPUS" > /dev/null 2>&1 || true; } 2>&1 )
    WALL=$(echo "$TIME_OUT" | awk '{print $1}')
    USER=$(echo "$TIME_OUT" | awk '{print $2}')
    SYS=$(echo "$TIME_OUT" | awk '{print $3}')
    printf "  run %d: wall=%ss  user=%ss  sys=%ss\n" "$i" "$WALL" "$USER" "$SYS"
    TS_TIMES+=("$WALL")
done

# --- Native parser benchmarks ---
echo ""
echo "=== native parser ==="
NATIVE_TIMES=()
for i in $(seq 1 "$RUNS"); do
    TIMEFORMAT='%R %U %S'
    TIME_OUT=$( { time "$NATIVE_BIN" "$CORPUS" > /dev/null 2>&1 || true; } 2>&1 )
    WALL=$(echo "$TIME_OUT" | awk '{print $1}')
    USER=$(echo "$TIME_OUT" | awk '{print $2}')
    SYS=$(echo "$TIME_OUT" | awk '{print $3}')
    printf "  run %d: wall=%ss  user=%ss  sys=%ss\n" "$i" "$WALL" "$USER" "$SYS"
    NATIVE_TIMES+=("$WALL")
done

# --- Summary ---
echo ""
echo "=== Summary ==="

ts_avg=$(printf '%s\n' "${TS_TIMES[@]}" | awk '{s+=$1} END {printf "%.3f", s/NR}')
native_avg=$(printf '%s\n' "${NATIVE_TIMES[@]}" | awk '{s+=$1} END {printf "%.3f", s/NR}')

echo "  tree-sitter avg wall: ${ts_avg}s"
echo "  native      avg wall: ${native_avg}s"

if (( $(echo "$native_avg > 0" | bc -l) )); then
    ratio=$(echo "scale=2; $ts_avg / $native_avg" | bc -l)
    echo "  ratio (ts/native):    ${ratio}x"
fi

# --- Verify same violations ---
echo ""
echo "=== Violation count comparison ==="
TS_COUNT=$("$TS_BIN" "$CORPUS" 2>/dev/null | wc -l | tr -d ' ' || echo "0")
NATIVE_COUNT=$("$NATIVE_BIN" "$CORPUS" 2>/dev/null | wc -l | tr -d ' ' || echo "0")
echo "  tree-sitter: $TS_COUNT violations"
echo "  native:      $NATIVE_COUNT violations"
if [ "$TS_COUNT" = "$NATIVE_COUNT" ]; then
    echo "  MATCH"
else
    echo "  MISMATCH (expected — parsers may differ on edge cases)"
fi
