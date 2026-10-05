#!/bin/bash
# shellcheck source-path=SCRIPTDIR
# Build pinned NAM Core as a provenance-checked Linux/AArch64 static archive.
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/nam_build_common.sh"
nam_cross_config
bash "$SCRIPT_DIR/check_nam_deps.sh"
BUILD_DIR="$VENDOR_DIR/build/core-${PASS}-a57"
OUT="$BUILD_DIR/libnam_core.a"
KEY="$(nam_archive_key "$TMP_CROSS_CXX" "$TMP_CROSS_AR" "$0" "${NAM_COMMON[@]}")"
if nam_archive_current "$OUT" "$KEY"; then
  tmp_cross_check --kind archive "$OUT"
  echo "Current: $OUT"
  exit 0
fi
mkdir -p "$BUILD_DIR"
rm -f "$BUILD_DIR"/*.o "$OUT" "$OUT.provenance" "$OUT.sha256"
while IFS= read -r src; do
  relative="${src#"$NAM_DIR"/}"
  obj="$BUILD_DIR/${relative//\//_}"
  obj="${obj%.cpp}.o"
  "$TMP_CROSS_CXX" "${NAM_COMMON[@]}" -c "$src" -o "$obj"
done < <(find "$NAM_DIR/NAM" -name '*.cpp' | sort)
"$TMP_CROSS_AR" rcs "$OUT" "$BUILD_DIR"/*.o
tmp_cross_check --kind archive "$OUT"
nam_archive_stamp "$OUT" "$KEY"
echo "Built $OUT"
