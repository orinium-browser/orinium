#!/usr/bin/env bash
# Runs the Acid3 harness and prints only the score line, for quick before/after
# comparison while working on web API support.
set -uo pipefail
cd "$(dirname "$0")/.."
timeout 300 cargo run --quiet --example acid3 2>/dev/null | rg 'ACID3 SCORE' || echo "ACID3 SCORE: (no output)"
