#!/bin/bash
# shellcheck source-path=SCRIPTDIR
# Build the pinned floating-point Speex resampler for native or Linux/AArch64 use.
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/nam_build_common.sh"
TARGET="${NAM_BUILD_TARGET:-native}"
SPEEX_DIR="$VENDOR_DIR/SpeexDSP"
FLAGS=(-std=gnu99 -include stdint.h -fPIC -O3 -DNDEBUG -DFLOATING_POINT
  -DOUTSIDE_SPEEX -DRANDOM_PREFIX=tmp_nam_speex)
case "$TARGET" in
  aarch64-linux)
    nam_cross_config
    COMPILER="$TMP_CROSS_CC"; ARCHIVER="$TMP_CROSS_AR"; CPU=a57
    FLAGS+=(-DUSE_NEON "${NAM_CPU[@]}") ;;
  native)
    COMPILER="${CC:-cc}"; ARCHIVER="${AR:-ar}"; CPU=native
    command -v "$COMPILER" >/dev/null || { echo "Missing C compiler: $COMPILER" >&2; exit 1; }
    command -v "$ARCHIVER" >/dev/null || { echo "Missing archiver: $ARCHIVER" >&2; exit 1; } ;;
  *) echo "NAM_BUILD_TARGET must be native or aarch64-linux (got $TARGET)" >&2; exit 2 ;;
esac
bash "$SCRIPT_DIR/check_nam_deps.sh"
BUILD_DIR="$VENDOR_DIR/build/speex-${PASS}-${CPU}"
OUT="$BUILD_DIR/libspeexdsp.a"
KEY="$(nam_archive_key "$COMPILER" "$ARCHIVER" "$0" "$TARGET" "${FLAGS[@]}")"
if nam_archive_current "$OUT" "$KEY"; then
  if [ "$TARGET" = aarch64-linux ]; then tmp_cross_check --kind archive "$OUT"; fi
  echo "Current: $OUT"
  exit 0
fi
mkdir -p "$BUILD_DIR"
rm -f "$BUILD_DIR/resample.o" "$OUT" "$OUT.provenance" "$OUT.sha256"
"$COMPILER" "${FLAGS[@]}" -I"$SPEEX_DIR/include" -I"$SPEEX_DIR/include/speex" \
  -I"$SPEEX_DIR/libspeexdsp" -c "$SPEEX_DIR/libspeexdsp/resample.c" -o "$BUILD_DIR/resample.o"
"$ARCHIVER" rcs "$OUT" "$BUILD_DIR/resample.o"
if [ "$TARGET" = aarch64-linux ]; then tmp_cross_check --kind archive "$OUT"; fi
nam_archive_stamp "$OUT" "$KEY"
echo "Built $OUT"
