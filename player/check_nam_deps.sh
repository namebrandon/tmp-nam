#!/bin/bash
# Verify immutable dependency checkouts and apply only the reviewed Core patch.
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
VENDOR_DIR="${TMP_NAM_VENDOR_DIR:-$SCRIPT_DIR/stubs/vendor}"
PATCH="$SCRIPT_DIR/patches/model.cpp.patch"
NAM_DIR="$VENDOR_DIR/NeuralAmpModelerCore"
for item in NeuralAmpModelerCore eigen SpeexDSP; do
  case "$item" in
    eigen) dep="$NAM_DIR/Dependencies/eigen" ;;
    *) dep="$VENDOR_DIR/$item" ;;
  esac
  expected="$(sed -n "s/^$item=//p" "$SCRIPT_DIR/stubs/vendor/VERSION")"
  [ -n "$expected" ] && [ "$(git -C "$dep" rev-parse HEAD)" = "$expected" ] || {
    echo "Unpinned dependency: $dep; run bootstrap_nam_deps.sh" >&2; exit 1;
  }
  [ -z "$(git -C "$dep" ls-files --others --exclude-standard)" ] || {
    echo "Untracked dependency sources: $dep" >&2; exit 1;
  }
  if [ "$item" != NeuralAmpModelerCore ]; then
    git -C "$dep" diff --quiet HEAD || { echo "Modified dependency: $dep" >&2; exit 1; }
  fi
done
if git -C "$NAM_DIR" diff --quiet --ignore-submodules=all HEAD; then
  git -C "$NAM_DIR" apply "$PATCH"
fi
# Compare the complete diff, including staged edits, with the canonical patch.
# A reverse-apply check alone would also accept unrelated edits in the files.
cmp -s "$PATCH" <(git -C "$NAM_DIR" -c core.abbrev=7 diff --no-ext-diff --no-color --binary --ignore-submodules=all HEAD | sed 's/^ $//') || {
  echo "Core source differs from the approved compatibility patch" >&2; exit 1;
}
