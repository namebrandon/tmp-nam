#!/bin/bash
# shellcheck source-path=SCRIPTDIR
# Shared Linux/AArch64 cross-tool selection. Source this file; never execute ELF here.
TMP_CROSS_PLAYER_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

tmp_cross_init() {
  local vendor="${TMP_NAM_VENDOR_DIR:-$TMP_CROSS_PLAYER_DIR/stubs/vendor}"
  local toolroot="${TMP_NAM_TOOLCHAIN_DIR:-$vendor/build/toolchains/messense-v1.1.0/aarch64-unknown-linux-gnu}"
  local name
  for name in gcc g++ ar readelf strip; do
    [ -x "$toolroot/bin/aarch64-unknown-linux-gnu-$name" ] || {
      echo "Missing cross tool: $toolroot/bin/aarch64-unknown-linux-gnu-$name" >&2
      echo "Run player/bootstrap_nam_toolchain.sh or set TMP_NAM_TOOLCHAIN_DIR." >&2
      return 1
    }
  done
  TMP_CROSS_CC="$toolroot/bin/aarch64-unknown-linux-gnu-gcc"
  TMP_CROSS_CXX="$toolroot/bin/aarch64-unknown-linux-gnu-g++"
  export TMP_CROSS_AR="$toolroot/bin/aarch64-unknown-linux-gnu-ar"
  TMP_CROSS_READELF="$toolroot/bin/aarch64-unknown-linux-gnu-readelf"
  export TMP_CROSS_STRIP="$toolroot/bin/aarch64-unknown-linux-gnu-strip"
  [ "$("$TMP_CROSS_CC" -dumpmachine)" = aarch64-unknown-linux-gnu ] &&
  [ "$("$TMP_CROSS_CXX" -dumpmachine)" = aarch64-unknown-linux-gnu ] || {
    echo "Expected Linux/AArch64 cross-compilers; host compilers are unsupported." >&2
    return 1
  }
  [ "$("$TMP_CROSS_CC" -dumpfullversion)" = 10.2.0 ] &&
  [ "$("$TMP_CROSS_CXX" -dumpfullversion)" = 10.2.0 ] || {
    echo "Expected pinned GCC 10.2.0; run player/bootstrap_nam_toolchain.sh." >&2
    return 1
  }
  [ -d "$("$TMP_CROSS_CC" -print-sysroot)" ] || {
    echo "Cross-compiler sysroot is missing." >&2; return 1
  }
}

tmp_cross_check() {
  # Per-artifact dependencies are explicit; compiler libraries must stay static.
  python3 "$TMP_CROSS_PLAYER_DIR/check_arm_abi.py" --readelf "$TMP_CROSS_READELF" "$@"
}
