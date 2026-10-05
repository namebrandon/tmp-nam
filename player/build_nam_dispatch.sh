#!/bin/bash
# shellcheck source-path=SCRIPTDIR
# Build a candidate dispatcher from current pinned sources; never replace delivery pins.
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/nam_build_common.sh"
nam_cross_config
nam_cross_archives
OUT_DIR="$VENDOR_DIR/build/dispatch-${PASS}-a57"
OUT="$OUT_DIR/nam_dispatch.so"
mkdir -p "$OUT_DIR"
"$TMP_CROSS_CC" -fPIC -O2 -Wall -Wextra "${NAM_CPU[@]}" \
  -c "$SCRIPT_DIR/stubs/nam_dispatch_trampoline.c" -o "$OUT_DIR/nam_dispatch_trampoline.o"
"$TMP_CROSS_CXX" "${NAM_COMMON[@]}" -c "$SCRIPT_DIR/stubs/nam_dispatch.cpp" -o "$OUT_DIR/nam_dispatch.o"
"$TMP_CROSS_CXX" "${NAM_COMMON[@]}" -c "$SCRIPT_DIR/nam_player.cpp" -o "$OUT_DIR/nam_player.o"
"$TMP_CROSS_CXX" -shared -fPIC "${NAM_CPU[@]}" -o "$OUT" \
  "$OUT_DIR/nam_dispatch.o" "$OUT_DIR/nam_dispatch_trampoline.o" "$OUT_DIR/nam_player.o" "${NAM_LINK[@]}"
"$TMP_CROSS_STRIP" --strip-unneeded "$OUT"
tmp_cross_check --kind shared --allow libc.so.6 --allow libm.so.6 \
  --allow libdl.so.2 --allow libpthread.so.0 "$OUT"
shasum -a 256 "$OUT"
echo "Built $OUT"
