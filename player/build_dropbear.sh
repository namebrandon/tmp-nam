#!/bin/bash
# Build the SD card's SSH tools from pinned sources; do not install on a device.
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
exec python3 "$SCRIPT_DIR/build_dropbear.py" "$@"
