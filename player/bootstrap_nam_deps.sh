#!/bin/bash
# Fetch the pinned NAM Core, Eigen, and SpeexDSP sources into the ignored
# player/stubs/vendor tree.  This script only installs source dependencies;
# architecture-specific archives are built by the build scripts.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
VENDOR_DIR="${TMP_NAM_VENDOR_DIR:-$SCRIPT_DIR/stubs/vendor}"
VERSION_FILE="$SCRIPT_DIR/stubs/vendor/VERSION"

if [ ! -f "$VERSION_FILE" ]; then
  echo "Missing dependency pin file: $VERSION_FILE" >&2
  exit 1
fi

get_pin() {
  local key="$1"
  sed -n "s/^${key}=//p" "$VERSION_FILE" | sed -n '1p'
}

NAM_URL="$(get_pin NeuralAmpModelerCore_URL)"
NAM_SHA="$(get_pin NeuralAmpModelerCore)"
EIGEN_URL="$(get_pin eigen_URL)"
EIGEN_SHA="$(get_pin eigen)"
SPEEX_URL="$(get_pin SpeexDSP_URL)"
SPEEX_SHA="$(get_pin SpeexDSP)"

for value in "$NAM_URL" "$NAM_SHA" "$EIGEN_URL" "$EIGEN_SHA" "$SPEEX_URL" "$SPEEX_SHA"; do
  [ -n "$value" ] || { echo "Malformed dependency pin file: $VERSION_FILE" >&2; exit 1; }
done

clone_at() {
  local url="$1" sha="$2" destination="$3"
  if [ -e "$destination/.git" ]; then
    git -C "$destination" fetch --no-tags origin "$sha"
    current="$(git -C "$destination" rev-parse HEAD)"
    if [ "$current" != "$sha" ] && [ -n "$(git -C "$destination" status --porcelain)" ]; then
      echo "Refusing to replace dirty dependency $destination; clean it or restore the pinned checkout." >&2
      exit 1
    fi
  else
    mkdir -p "$(dirname "$destination")"
    git clone --no-checkout --filter=blob:none --no-tags "$url" "$destination"
    git -C "$destination" fetch --no-tags origin "$sha"
  fi
  git -C "$destination" checkout --detach "$sha"
  actual="$(git -C "$destination" rev-parse HEAD)"
  [ "$actual" = "$sha" ] || {
    echo "Dependency checkout mismatch for $destination: $actual != $sha" >&2
    exit 1
  }
}

mkdir -p "$VENDOR_DIR"
clone_at "$NAM_URL" "$NAM_SHA" "$VENDOR_DIR/NeuralAmpModelerCore"
# NAM's Eigen entry is a gitlink.  Keep it independently pinned so a future
# NAM checkout cannot silently move the numerical dependency.
clone_at "$EIGEN_URL" "$EIGEN_SHA" "$VENDOR_DIR/NeuralAmpModelerCore/Dependencies/eigen"
clone_at "$SPEEX_URL" "$SPEEX_SHA" "$VENDOR_DIR/SpeexDSP"

echo "NAM Core: $NAM_SHA"
echo "Eigen:    $EIGEN_SHA"
echo "SpeexDSP: $SPEEX_SHA"
echo "Sources installed under $VENDOR_DIR (all ignored by git)."
