#!/bin/bash
# shellcheck source-path=SCRIPTDIR
# shellcheck disable=SC2034
# Shared configuration and content-addressed archive freshness for NAM builders.
NAM_PLAYER_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
VENDOR_DIR="${TMP_NAM_VENDOR_DIR:-$NAM_PLAYER_DIR/stubs/vendor}"
export TMP_NAM_VENDOR_DIR="$VENDOR_DIR"
NAM_DIR="$VENDOR_DIR/NeuralAmpModelerCore"
PASS="${NAM_BUILD_PASS:-perf}"
case "$PASS" in
  parity) NAM_OPT=(-O2) ;;
  perf) NAM_OPT=(-O3 -DNDEBUG -DEIGEN_NO_DEBUG -funroll-loops) ;;
  *) echo "NAM_BUILD_PASS must be parity or perf (got $PASS)" >&2; return 2 ;;
esac
NAM_CPU=(-march=armv8-a -mtune=cortex-a57)
# Upstream Core's specialized A2 WaveNet (3/8 channels; our patch adds the
# NEON 8-channel layer) and our fused AArch64 A1 WaveNet (16/8, 12/6, 8/4, 4/2).
# NAM_USE_INLINE_GEMM stays off: it measured ~2% slower for A1 standard (16/8)
# on the unit's Cortex-A57.
NAM_FEATURES=(-DNAM_ENABLE_A2_FAST -DNAM_ENABLE_A1_FAST)

nam_cross_config() {
  source "$NAM_PLAYER_DIR/cross_toolchain.sh"
  tmp_cross_init
  NAM_COMMON=(-std=c++17 -fPIC "${NAM_OPT[@]}" -DNAM_SAMPLE_FLOAT "${NAM_FEATURES[@]}"
    -DRANDOM_PREFIX=tmp_nam_speex -DOUTSIDE_SPEEX -Wall -Wextra "${NAM_CPU[@]}"
    -isystem "$NAM_DIR/Dependencies/eigen" -I"$NAM_DIR/Dependencies/nlohmann"
    -I"$NAM_DIR" -I"$NAM_PLAYER_DIR/stubs" -I"$NAM_PLAYER_DIR"
    -I"$VENDOR_DIR/SpeexDSP/include" -include "$NAM_PLAYER_DIR/toolchains/glibc_macro_cleanup.h")
}

nam_archive_key() {
  # check_nam_deps verifies complete source diffs against these immutable pins.
  # Hash the builder, compiler, archiver, flags, patch and all dependency pins.
  local compiler="$1" archiver="$2" builder="$3"
  shift 3
  {
    printf '%s\n' "$PASS" "$(uname -sm)" "$compiler" "$archiver" "$@"
    "$compiler" --version
    shasum -a 256 "$(command -v "$compiler")" "$(command -v "$archiver")" "$builder" \
      "$NAM_PLAYER_DIR/stubs/vendor/VERSION" "$NAM_PLAYER_DIR/patches/model.cpp.patch" \
      "$NAM_PLAYER_DIR/nam_build_common.sh" "$NAM_PLAYER_DIR/cross_toolchain.sh" \
      "$NAM_PLAYER_DIR/toolchains/glibc_macro_cleanup.h" "$NAM_PLAYER_DIR/toolchains/messense-v1.1.0.json"
  } | shasum -a 256 | awk '{print $1}'
}

nam_archive_current() {
  local archive="$1" key="$2"
  [ -f "$archive" ] && [ -f "$archive.provenance" ] && [ -f "$archive.sha256" ] &&
    [ "$(cat "$archive.provenance")" = "$key" ] &&
    [ "$(shasum -a 256 "$archive" | awk '{print $1}')" = "$(cat "$archive.sha256")" ]
}

nam_archive_stamp() {
  local archive="$1" key="$2"
  shasum -a 256 "$archive" | awk '{print $1}' > "$archive.sha256"
  printf '%s\n' "$key" > "$archive.provenance"
}

nam_cross_archives() {
  NAM_BUILD_PASS="$PASS" bash "$NAM_PLAYER_DIR/build_nam_core.sh"
  NAM_BUILD_TARGET=aarch64-linux NAM_BUILD_PASS="$PASS" bash "$NAM_PLAYER_DIR/build_nam_speex.sh"
  NAM_CORE="$VENDOR_DIR/build/core-${PASS}-a57/libnam_core.a"
  NAM_SPEEX="$VENDOR_DIR/build/speex-${PASS}-a57/libspeexdsp.a"
  NAM_LINK=("-Wl,--whole-archive" "$NAM_CORE" "$NAM_SPEEX" "-Wl,--no-whole-archive"
    -static-libstdc++ -static-libgcc -lpthread -lm -ldl)
}

nam_check_executable() {
  tmp_cross_check --kind executable --allow libc.so.6 --allow libm.so.6 \
    --allow libpthread.so.0 --allow libdl.so.2 --allow ld-linux-aarch64.so.1 "$1"
}
