#!/bin/bash
# Repository checks. Usage: scripts/check.sh [quick|all]
#   quick  syntax checks + release-pin integrity (no toolchains beyond cargo/python)
#   all    quick + Rust tests/clippy + desktop typecheck/lint/tests
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
tier="${1:-quick}"
pass=0
fail=0

report() {
  if [ "$1" = 0 ]; then
    echo "  PASS: $2"
    pass=$((pass + 1))
  else
    echo "  FAIL: $2"
    fail=$((fail + 1))
  fi
}

step() {
  local label=$1
  shift
  "$@" >/tmp/tmp-nam-check.log 2>&1
  local rc=$?
  [ "$rc" = 0 ] || tail -20 /tmp/tmp-nam-check.log
  report "$rc" "$label"
}

echo "=== tmp-nam check: $tier ==="
if [ "$tier" != "quick" ] && [ "$tier" != "all" ]; then
  echo "unknown tier: $tier (use quick or all)" >&2
  exit 2
fi

echo "--- Shell syntax ---"
while IFS= read -r f; do
  bash -n "$f" 2>/dev/null
  report $? "bash -n $f"
done < <(git ls-files '*.sh')

echo "--- Python syntax ---"
while IFS= read -r f; do
  python3 -c "import ast, sys; ast.parse(open(sys.argv[1]).read())" "$f" 2>/dev/null
  report $? "python parse $f"
done < <(git ls-files '*.py' | grep -v '^player/stubs/vendor/')

echo "--- Release pins ---"
step "device/ assets + player sources match device/release.json" \
  cargo test -q -p tmp-sdcard --test release

echo "--- Player test sources ---"
# The native/ARM runners need toolchains, so check here that their sources are tracked.
required_tests=$(sed -n 's/^for required in \(.*\); do$/\1/p' player/run_nam_native_tests.sh)
for f in $required_tests; do
  git ls-files --error-unmatch "player/tests/$f" >/dev/null 2>&1
  report $? "tracked player/tests/$f"
done
[ -n "$required_tests" ]
report $? "parsed required tests from run_nam_native_tests.sh"

echo "--- No exploit material ---"
if git grep -I -l -E 'craft_acd|craft_heap|craft_overflow|cve_probe|a1_activate|exploit_trigger|segv_handler' \
     -- '*.py' '*.c' '*.h' '*.rs' ':!scripts/check.sh' >/dev/null 2>&1; then
  report 1 "exploit material check (should be empty)"
else
  report 0 "exploit material check (clean)"
fi

if [ "$tier" = "all" ]; then
  echo "--- Rust ---"
  step "cargo test --workspace" cargo test -q --workspace
  step "cargo clippy (deny warnings)" cargo clippy -q --workspace --all-targets -- -D warnings
  echo "--- Desktop frontend ---"
  if command -v bun >/dev/null 2>&1; then
    step "bun install" bun install --cwd apps/desktop --frozen-lockfile
    step "typecheck" bun run --cwd apps/desktop typecheck
    step "lint" bun run --cwd apps/desktop lint
    step "vitest" bun run --cwd apps/desktop test
  else
    report 1 "bun not found (needed for the desktop checks)"
  fi
fi

echo "=== Results: $pass passed, $fail failed ==="
[ "$fail" -eq 0 ]
