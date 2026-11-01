#!/usr/bin/env bash
# Build and package the headless TrackCraft team server for Linux (the shared release workflow's
# `server` job, `has-server: true`):
#
#   $DIST/trackcraft-server-<version>-linux-<arch>.tar.gz
#       trackcraft-server-<version>-linux-<arch>/{trackcraft-server, trackcraft-server.service,
#       README.md, LICENSE-*, NOTICE, ATTRIBUTION.md}
#
# The tarball also carries the web client (web/, from `trunk build`) when trunk is installed, so
# `trackcraft-server serve --web web` serves the browser app next to the API; without trunk the
# server ships alone (the web zip is a separate release asset either way).
#
# Usage: packaging/server/package.sh [--skip-build]
set -euo pipefail
# shellcheck source=../env.sh
. "$(dirname "${BASH_SOURCE[0]}")/../env.sh"
HERE="$ROOT/packaging/server"

SKIP_BUILD=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --skip-build) SKIP_BUILD=1; shift ;;
    -h | --help) sed -n '2,14p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

ARCH="$(uname -m)"
case "$ARCH" in
  x86_64) ;;
  aarch64 | arm64) ARCH=aarch64 ;;
  *) echo "unsupported architecture $ARCH" >&2; exit 2 ;;
esac
OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
[[ "$OS" = darwin ]] && OS=macos
NAME="trackcraft-server-$VERSION-$OS-$ARCH"

echo "==> TrackCraft server $VERSION for $OS $ARCH"
if [[ "$SKIP_BUILD" = 0 ]]; then
  (cd "$ROOT" && cargo build --release --locked -p trackcraft-server)
fi
WORK="$CARGO_TARGET_DIR/server-package"
STAGE="$WORK/$NAME"
rm -rf "$WORK"
mkdir -p "$STAGE"
install -m 755 "$CARGO_TARGET_DIR/release/trackcraft-server" "$STAGE/trackcraft-server"
strip "$STAGE/trackcraft-server" 2>/dev/null || true
install -m 644 "$HERE/trackcraft-server.service" "$STAGE/trackcraft-server.service"
copy_docs "$STAGE"
if command -v trunk >/dev/null; then
  (cd "$ROOT/apps/trackcraft-web" && trunk build --release)
  cp -R "$ROOT/dist/web" "$STAGE/web"
else
  warn "trunk not installed: $NAME.tar.gz has no web/ client"
fi

tar -C "$WORK" -czf "$DIST/$NAME.tar.gz" "$NAME"
echo "wrote $DIST/$NAME.tar.gz"
"$STAGE/trackcraft-server" --help >/dev/null
echo "==> done"
ls -lh "$DIST/$NAME.tar.gz"
