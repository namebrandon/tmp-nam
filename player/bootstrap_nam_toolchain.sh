#!/bin/bash
# shellcheck source-path=SCRIPTDIR
# Install the pinned macOS cross-toolchain after verifying the pinned archive.
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
VENDOR_DIR="${TMP_NAM_VENDOR_DIR:-$SCRIPT_DIR/stubs/vendor}"
MANIFEST="$SCRIPT_DIR/toolchains/messense-v1.1.0.json"
[ "$(uname -s)" = Darwin ] && [ "$(uname -m)" = arm64 ] || {
  echo "The pinned toolchain requires macOS on Apple Silicon." >&2; exit 2;
}
get_field() { python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))[sys.argv[2]])' "$MANIFEST" "$1"; }
DEST="$VENDOR_DIR/build/toolchains/messense-v1.1.0"
ARCHIVE="$DEST/toolchain.zip"
case "$#" in
  0) ;;
  2) [ "$1" = --archive ] || { echo "Usage: $0 [--archive PATH]" >&2; exit 2; }; ARCHIVE="$2" ;;
  *) echo "Usage: $0 [--archive PATH]" >&2; exit 2 ;;
esac
mkdir -p "$DEST"
if [ ! -f "$ARCHIVE" ]; then
  [ "$#" -eq 0 ] || { echo "Missing archive: $ARCHIVE" >&2; exit 1; }
  curl --fail --location --proto '=https' --tlsv1.2 "$(get_field archive_url)" -o "$ARCHIVE.partial"
  mv "$ARCHIVE.partial" "$ARCHIVE"
fi
ACTUAL="$(shasum -a 256 "$ARCHIVE" | awk '{print $1}')"
[ "$ACTUAL" = "$(get_field archive_sha256)" ] || {
  echo "Cross-toolchain archive SHA-256 mismatch: $ACTUAL" >&2; exit 1;
}
TARGET="$DEST/aarch64-unknown-linux-gnu"
if [ -e "$TARGET" ]; then
  echo "Toolchain already exists: $TARGET (leave it unchanged; use another TMP_NAM_VENDOR_DIR to reinstall)."
else
  STAGE="$(mktemp -d "$DEST/.extract.XXXXXX")"
  trap 'rm -rf "$STAGE"' EXIT
  ditto -x -k "$ARCHIVE" "$STAGE"
  [ -x "$STAGE/aarch64-unknown-linux-gnu/bin/aarch64-unknown-linux-gnu-gcc" ]
  mv "$STAGE/aarch64-unknown-linux-gnu" "$TARGET"
fi
# Validate host execution, target, compiler version, and bundled sysroot.
source "$SCRIPT_DIR/cross_toolchain.sh"
TMP_NAM_TOOLCHAIN_DIR="$TARGET" tmp_cross_init
cp "$MANIFEST" "$DEST/manifest.json"
echo "Ready: $TARGET"
