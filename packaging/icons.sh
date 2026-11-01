#!/usr/bin/env bash
# Regenerate every app icon from assets/app-icon/trackcraft.svg.
#
# `cargo xtask icons` rasterizes the SVG itself (xtask/src/icons.rs: the icon keeps to a small
# SVG subset it draws exactly), so no SVG tool is needed. On macOS, iconutil also writes the
# .icns. The outputs are committed, so builds and packaging never run this.
#
#   packaging/icons.sh
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIR="$ROOT/assets/app-icon"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

SET="$TMP/trackcraft.iconset"
# 1024/512 PNGs, Linux hicolor 16-512 + scalable, Windows .ico, and the macOS iconset.
(cd "$ROOT" && cargo run -q -p xtask -- icons --iconset "$SET")

# macOS .icns (icons on Apple's 824/1024 grid, with a transparent margin).
if command -v iconutil >/dev/null; then
  iconutil -c icns -o "$DIR/trackcraft.icns" "$SET"
else
  echo "warning: iconutil not found (macOS only); trackcraft.icns not regenerated" >&2
fi
echo "icons written to $DIR"
