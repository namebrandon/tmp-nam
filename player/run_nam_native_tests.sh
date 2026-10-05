#!/bin/bash
# shellcheck source-path=SCRIPTDIR
# Compile and run native Intel/Apple-Silicon unit tests for the NAM adapter.
# This runner intentionally does not use Docker: it exercises host C++17 ABI,
# Player allocation behavior, registry lifetimes, model config, and trampoline
# relocation without claiming to validate the firmware's aarch64 runtime.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$ROOT_DIR"
VENDOR_DIR="${TMP_NAM_VENDOR_DIR:-$SCRIPT_DIR/stubs/vendor}"
NAM_DIR="$VENDOR_DIR/NeuralAmpModelerCore"
PASS="${NAM_BUILD_PASS:-perf}"
BUILD_DIR="$VENDOR_DIR/build/native-tests-${PASS}-native"
CORE_DIR="$BUILD_DIR/core"
CORE_LIB="$CORE_DIR/libnam_core.a"


case "$PASS" in parity) OPT_FLAGS="-O2" ;; perf) OPT_FLAGS="-O3 -DNDEBUG -DEIGEN_NO_DEBUG -funroll-loops" ;; *) echo "NAM_BUILD_PASS must be parity or perf" >&2; exit 2 ;; esac
if [ ! -d "$NAM_DIR/NAM" ] || [ ! -d "$NAM_DIR/Dependencies/eigen/Eigen" ]; then
  echo "Missing NAM/Eigen sources; run player/bootstrap_nam_deps.sh" >&2
  exit 1
fi

bash "$SCRIPT_DIR/check_nam_deps.sh"
NAM_BUILD_TARGET=native NAM_BUILD_PASS="$PASS" \
  bash "$SCRIPT_DIR/build_nam_speex.sh"

CXX="${CXX:-clang++}"
command -v "$CXX" >/dev/null 2>&1 || CXX=g++
command -v "$CXX" >/dev/null 2>&1 || { echo "Missing C++ compiler" >&2; exit 1; }
CC="${CC:-clang}"
command -v "$CC" >/dev/null 2>&1 || CC=gcc
command -v "$CC" >/dev/null 2>&1 || { echo "Missing C compiler" >&2; exit 1; }
mkdir -p "$CORE_DIR"
source "$SCRIPT_DIR/nam_build_common.sh"
ARCHIVER="${AR:-ar}"
command -v "$ARCHIVER" >/dev/null || { echo "Missing archiver: $ARCHIVER" >&2; exit 1; }
source_stamp="$(nam_archive_key "$CXX" "$ARCHIVER" "$0" "$OPT_FLAGS" -std=c++17 -DNAM_SAMPLE_FLOAT)"
if ! nam_archive_current "$CORE_LIB" "$source_stamp"; then
  rm -f "$CORE_DIR"/*.o "$CORE_LIB"

  while IFS= read -r src; do
    obj="$CORE_DIR/$(echo "$src" | sed "s|$NAM_DIR/||; s|/|_|g; s|\.cpp$|.o|")"
    # shellcheck disable=SC2086
    "$CXX" -std=c++17 $OPT_FLAGS -Wall -Wextra -DNAM_SAMPLE_FLOAT \
      -isystem "$NAM_DIR/Dependencies/eigen" -I"$NAM_DIR/Dependencies/nlohmann" -I"$NAM_DIR" \
      -c "$src" -o "$obj"
  done < <(find "$NAM_DIR/NAM" -name '*.cpp' | sort)
  "$ARCHIVER" rcs "$CORE_LIB" "$CORE_DIR"/*.o
  nam_archive_stamp "$CORE_LIB" "$source_stamp"
fi

read -r -a OPT_ARGS <<< "$OPT_FLAGS"
COMMON=(-std=c++17 "${OPT_ARGS[@]}" -Wall -Wextra -DNAM_SAMPLE_FLOAT -DRANDOM_PREFIX=tmp_nam_speex -DOUTSIDE_SPEEX
  -isystem "$NAM_DIR/Dependencies/eigen"
  -I"$NAM_DIR/Dependencies/nlohmann" -I"$NAM_DIR" -I"$SCRIPT_DIR"
  -I"$VENDOR_DIR/SpeexDSP/include")
if [ "$(uname -s)" = Darwin ]; then
  CORE_LINK=("-Wl,-force_load,$CORE_LIB")
else
  CORE_LINK=("-Wl,--whole-archive" "$CORE_LIB" "-Wl,--no-whole-archive")
fi

run_player_test() {
  local source="$1" output="$2"
  rm -f "$output"
  # shellcheck disable=SC2086
    "$CXX" "${COMMON[@]}" -UNDEBUG "$source" "$SCRIPT_DIR/nam_player.cpp" \
    "${CORE_LINK[@]}" \
    "$VENDOR_DIR/build/speex-${PASS}-native/libspeexdsp.a" -pthread -lm -o "$output"
  "$output"
}
run_core_header_test() {
  local source="$1" output="$2"
  rm -f "$output"
  "$CXX" "${COMMON[@]}" -UNDEBUG "$source" -o "$output"
  "$output"
}
run_core_test() {
  local source="$1" output="$2"
  rm -f "$output"
  "$CXX" "${COMMON[@]}" -UNDEBUG -DNAM_DENSE8X8_INTEGRATION_CANDIDATE "$source" \
    "${CORE_LINK[@]}" -pthread -lm -o "$output"
  "$output"
}
build_parity_cli() {
  local output="$BUILD_DIR/nam_parity_test"
  rm -f "$output" "$BUILD_DIR/nam_parity_test.o" "$BUILD_DIR/nam_parity_player.o"
  # Keep the parity CLI on the same C++17 flags, Player translation unit,
  # archive, and Speex symbols as the adapter test.  It is compiled here for
  # reproducible native inspection; execution belongs to the model fixture
  # parity/benchmark commands.
  "$CXX" "${COMMON[@]}" "$SCRIPT_DIR/stubs/nam_parity_test.cpp" \
    -c -o "$BUILD_DIR/nam_parity_test.o"
  "$CXX" "${COMMON[@]}" "$SCRIPT_DIR/nam_player.cpp" \
    -c -o "$BUILD_DIR/nam_parity_player.o"
  "$CXX" "${COMMON[@]}" -o "$output" \
    "$BUILD_DIR/nam_parity_test.o" "$BUILD_DIR/nam_parity_player.o" \
    "${CORE_LINK[@]}" "$VENDOR_DIR/build/speex-${PASS}-native/libspeexdsp.a" \
    -pthread -lm
}
run_simple_test() {
  local source="$1" output="$2"
  rm -f "$output"
  "$CXX" -std=c++17 -O2 -Wall -Wextra -I"$SCRIPT_DIR" "$source" -pthread -o "$output"
  "$output"
}
run_dispatch_test() {
  local trampoline="$BUILD_DIR/test_nam_dispatch_trampoline.o"
  local output="$BUILD_DIR/test_nam_dispatch_load"
  rm -f "$trampoline" "$output"
  "$CC" -O2 -Wall -Wextra -I"$SCRIPT_DIR/stubs" \
    -c "$SCRIPT_DIR/stubs/nam_dispatch_trampoline.c" -o "$trampoline"
  "$CXX" "${COMMON[@]}" -UNDEBUG -DTMP_NAM_DISPATCH_TEST \
    "$SCRIPT_DIR/tests/test_nam_dispatch_load.cpp" "$SCRIPT_DIR/nam_player.cpp" \
    "$trampoline" "${CORE_LINK[@]}" \
    "$VENDOR_DIR/build/speex-${PASS}-native/libspeexdsp.a" \
    -pthread -lm -ldl -o "$output"

  verify_and_run() {
    local fixture="$1" expected="$2" label="$3" actual
    [ -f "$fixture" ] || {
      echo "Missing required NAM fixture: $label ($fixture)" >&2
      return 1
    }
    actual="$(shasum -a 256 "$fixture" | awk '{print $1}')"
    [ "$actual" = "$expected" ] || {
      echo "Unexpected SHA-256 for $label: $actual" >&2
      return 1
    }
    "$output" "$fixture"
  }

  verify_and_run "$NAM_DIR/example_models/lstm.nam" \
    df9f78c49f49c2bb32411df47e3f53746075adb206b92d017e06379d1e56234a \
    "pinned upstream LSTM example"
  verify_and_run "$NAM_DIR/example_models/wavenet_condition_dsp.nam" \
    1af5a5d4eb079b894e095882738c102fd2d9eeced387a16d0d24cd73a07de718 \
    "pinned upstream conditioned WaveNet example"
  verify_and_run "$NAM_DIR/example_models/slimmable_container.nam" \
    78eba4fc17c39bba0fef375ee9cd3865d8ffefb76f53920037a874dcb2d2fbdc \
    "pinned upstream slimmable container example"
  verify_and_run "$NAM_DIR/example_models/slimmable_wavenet.nam" \
    735c1a86e18140b7cfe90c08427ca6a85f62c32d34cc4048997933652aa774b4 \
    "pinned upstream slimmable WaveNet example"
  verify_and_run "$NAM_DIR/example_models/wavenet_a2_max.nam" \
    12384c6640e1126907b366584024c4abb129ac5920b3dc2d31b29e39315e820d \
    "pinned upstream A2 max WaveNet example"
}

for required in test_nam_registry.cpp test_nam_loader.cpp test_nam_profile.cpp test_nam_trampoline.cpp test_nam_activation.cpp test_nam_reset_contract.cpp test_nam_dense8x8.cpp test_nam_player.cpp test_nam_model_config.cpp test_nam_dispatch_load.cpp; do
  [ -f "$SCRIPT_DIR/tests/$required" ] || { echo "Missing required native test: $SCRIPT_DIR/tests/$required" >&2; exit 1; }
done
run_simple_test "$SCRIPT_DIR/tests/test_nam_registry.cpp" "$BUILD_DIR/test_nam_registry"
run_simple_test "$SCRIPT_DIR/tests/test_nam_loader.cpp" "$BUILD_DIR/test_nam_loader"
run_simple_test "$SCRIPT_DIR/tests/test_nam_profile.cpp" "$BUILD_DIR/test_nam_profile"
run_simple_test "$SCRIPT_DIR/tests/test_nam_trampoline.cpp" "$BUILD_DIR/test_nam_trampoline"
run_core_header_test "$SCRIPT_DIR/tests/test_nam_activation.cpp" "$BUILD_DIR/test_nam_activation"
run_core_test "$SCRIPT_DIR/tests/test_nam_reset_contract.cpp" "$BUILD_DIR/test_nam_reset_contract"
run_core_test "$SCRIPT_DIR/tests/test_nam_dense8x8.cpp" "$BUILD_DIR/test_nam_dense8x8"
run_player_test "$SCRIPT_DIR/tests/test_nam_player.cpp" "$BUILD_DIR/test_nam_player"
run_player_test "$SCRIPT_DIR/tests/test_nam_model_config.cpp" "$BUILD_DIR/test_nam_model_config"
run_dispatch_test
build_parity_cli

echo "NAM native tests: PASS (artifacts in $BUILD_DIR)"
