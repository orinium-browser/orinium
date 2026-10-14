#!/usr/bin/env bash
# Enforces the crate's dependency law: browser -> platform -> engine.
#
#   browser  may reference  engine, platform
#   platform may reference  engine
#   engine   may reference  neither browser nor platform
#
# References are detected anywhere in the source text (code, strings and doc
# comments alike), so even a documentation link counts as a dependency.
set -uo pipefail
cd "$(dirname "$0")/.."

status=0
check() {
  local dir="$1" pattern="$2" label="$3"
  local hits
  hits=$(rg -n --no-heading "$pattern" "$dir" || true)
  if [ -n "$hits" ]; then
    echo "Layering violation: $label"
    echo "$hits"
    echo
    status=1
  fi
}

check src/engine 'crate::platform' 'engine must not reference platform'
check src/engine 'crate::browser' 'engine must not reference browser'
check src/platform 'crate::browser' 'platform must not reference browser'

if [ "$status" -eq 0 ]; then
  echo "Layering OK (browser -> platform -> engine)"
fi
exit "$status"
