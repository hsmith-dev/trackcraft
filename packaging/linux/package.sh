#!/usr/bin/env bash
# Build and package TrackCraft for Linux (<arch> is x86_64 or aarch64, or riscv64 when cross-compiling):
#
#   $DIST/trackcraft-<version>-linux-<arch>.AppImage  any distro with glibc >= the build host's
#   $DIST/trackcraft-<version>-linux-<arch>.AppImage.zsync  delta updates (needs zsyncmake)
#   $DIST/trackcraft-<version>-linux-<arch>.deb       Debian, Ubuntu, Mint, Pop!_OS, ...
#   $DIST/trackcraft-<version>-linux-<arch>.rpm       Fedora, openSUSE, RHEL, ...
#   $DIST/trackcraft-<version>-linux-<arch>.tar.gz    plain FHS-style tree (bin/, share/)
#
# Each holds the desktop app (trackcraft) and the CLI (trackcraft-cli). The team server ships
# separately (packaging/server/package.sh).
#
# Usage: packaging/linux/package.sh [--skip-build] [--formats "appimage deb rpm tar"]
#
# Needs: cargo; nfpm for deb/rpm (https://nfpm.goreleaser.com); appimagetool for the AppImage
# (downloaded into $CARGO_TARGET_DIR if missing). Build on an old distro (CI: Ubuntu 22.04,
# glibc 2.35) so the binaries run on newer ones. Optional: desktop-file-validate, appstreamcli,
# update-mime-database (checks the MIME XML), zsyncmake for the AppImage's .zsync.
set -euo pipefail
# shellcheck source=../env.sh
. "$(dirname "${BASH_SOURCE[0]}")/../env.sh"
HERE="$ROOT/packaging/linux"

SKIP_BUILD=0
FORMATS="appimage deb rpm tar"
while [[ $# -gt 0 ]]; do
  case "$1" in
    --skip-build) SKIP_BUILD=1; shift ;;
    --formats) FORMATS="$2"; shift 2 ;;
    -h | --help) sed -n '2,19p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

ARCH="${CROSS_ARCH:-$(uname -m)}"
case "$ARCH" in
  riscv64) DEB_ARCH=riscv64 ;;
  x86_64) DEB_ARCH=amd64 ;;
  aarch64 | arm64) ARCH=aarch64; DEB_ARCH=arm64 ;;
  *) echo "unsupported architecture $ARCH" >&2; exit 2 ;;
esac
# The deb Maintainer field needs a name and an address; override with TRACKCRAFT_MAINTAINER.
export TRACKCRAFT_MAINTAINER="${TRACKCRAFT_MAINTAINER:-TrackCraft contributors <hsmith-dev@users.noreply.github.com>}"
export GLIBC_FLOOR="${GLIBC_FLOOR:-2.35}"
BASENAME="trackcraft-$VERSION-linux-$ARCH"

echo "==> TrackCraft $VERSION for Linux $ARCH ($FORMATS)"

if [[ "$SKIP_BUILD" = 0 ]]; then
  if [[ -n "${CROSS_TARGET:-}" ]]; then
    (cd "$ROOT" && cargo build --release --locked -p trackcraft -p trackcraft-cli --target "$CROSS_TARGET")
  else
    (cd "$ROOT" && cargo build --release --locked -p trackcraft -p trackcraft-cli)
  fi
fi
BIN="$CARGO_TARGET_DIR/${CROSS_TARGET:+$CROSS_TARGET/}release"
WORK="$CARGO_TARGET_DIR/linux-package"
STAGE="$WORK/root"
rm -rf "$WORK"

# ---- stage an FHS tree (shared by every format) -------------------------------------------------
install -Dm755 "$BIN/trackcraft" "$STAGE/usr/bin/trackcraft"
install -Dm755 "$BIN/trackcraft-cli" "$STAGE/usr/bin/trackcraft-cli"
"${CROSS_COMPILE:-}strip" "$STAGE/usr/bin/trackcraft" "$STAGE/usr/bin/trackcraft-cli" 2>/dev/null || true
install -Dm644 "$HERE/$APP_ID.desktop" "$STAGE/usr/share/applications/$APP_ID.desktop"
install -Dm644 "$HERE/$APP_ID.mime.xml" "$STAGE/usr/share/mime/packages/$APP_ID.xml"
mkdir -p "$STAGE/usr/share/metainfo"
sed -e "s/@VERSION@/$VERSION/g" -e "s/@DATE@/$TRACKCRAFT_BUILD_DATE/g" \
  "$HERE/$APP_ID.metainfo.xml.in" >"$STAGE/usr/share/metainfo/$APP_ID.metainfo.xml"
mkdir -p "$STAGE/usr/share/icons"
cp -R "$ROOT/assets/app-icon/hicolor" "$STAGE/usr/share/icons/"
mkdir -p "$STAGE/usr/share/doc/trackcraft"
copy_docs "$STAGE/usr/share/doc/trackcraft"

if command -v desktop-file-validate >/dev/null; then
  desktop-file-validate "$STAGE/usr/share/applications/$APP_ID.desktop"
fi
if command -v appstreamcli >/dev/null; then
  appstreamcli validate --no-net --explain "$STAGE/usr/share/metainfo/$APP_ID.metainfo.xml"
fi
if command -v update-mime-database >/dev/null; then
  # Compiles the type into a scratch database: fails on a malformed XML.
  mkdir -p "$WORK/mime/packages"
  cp "$HERE/$APP_ID.mime.xml" "$WORK/mime/packages/"
  update-mime-database "$WORK/mime"
  grep -q 'application/x-trackcraft' "$WORK/mime/globs2"
fi

has() { local item="$1"; case " $FORMATS " in *" $item "*) return 0 ;; *) return 1 ;; esac; }

# ---- .tar.gz ------------------------------------------------------------------------------------
if has tar; then
  mkdir -p "$WORK/tar"
  cp -R "$STAGE/usr" "$WORK/tar/$BASENAME"
  tar -C "$WORK/tar" -czf "$DIST/$BASENAME.tar.gz" "$BASENAME"
  echo "wrote $DIST/$BASENAME.tar.gz"
fi

# ---- .deb / .rpm --------------------------------------------------------------------------------
if has deb || has rpm; then
  command -v nfpm >/dev/null || { echo "error: nfpm not found (https://nfpm.goreleaser.com/install/)" >&2; exit 1; }
  export VERSION
  export NFPM_ARCH="$DEB_ARCH"
  # nfpm expands env vars in fields like `version` and `arch`, but not in `contents[].src`.
  sed "s|\${STAGE}|$STAGE|g" "$HERE/nfpm.yaml" >"$WORK/nfpm.yaml"
  for fmt in deb rpm; do
    if has "$fmt"; then (cd "$ROOT" && nfpm package -f "$WORK/nfpm.yaml" -p "$fmt" -t "$DIST/$BASENAME.$fmt"); fi
  done
fi

# ---- AppImage -----------------------------------------------------------------------------------
if has appimage; then
  # Same pinned appimagetool and type2 runtime as the sibling apps (Wordcraft).
  APPIMAGETOOL_VERSION=1.9.1
  RUNTIME_VERSION=20251108
  case "$ARCH" in
    x86_64)
      APPIMAGETOOL_SHA256=ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0
      RUNTIME_SHA256=2fca8b443c92510f1483a883f60061ad09b46b978b2631c807cd873a47ec260d
      ;;
    aarch64)
      APPIMAGETOOL_SHA256=f0837e7448a0c1e4e650a93bb3e85802546e60654ef287576f46c71c126a9158
      RUNTIME_SHA256=00cbdfcf917cc6c0ff6d3347d59e0ca1f7f45a6df1a428a0d6d8a78664d87444
      ;;
    *) APPIMAGETOOL_SHA256='' RUNTIME_SHA256='' ;;
  esac
  if [[ -z "$RUNTIME_SHA256" ]]; then
    echo "no pinned AppImage runtime for $ARCH" >&2
    exit 2
  fi
  # Downloads $1 to $3 once and checks it against SHA-256 $2, the cached copy again on every run.
  fetch_pinned() {
    local url="$1" expected_sha="$2" target="$3"
    if [[ ! -x "$target" ]]; then
      curl --proto '=https' -fsSL -o "$target.download" "$url"
      printf '%s  %s\n' "$expected_sha" "$target.download" | sha256sum -c -
      chmod 755 "$target.download"
      mv "$target.download" "$target"
    fi
    printf '%s  %s\n' "$expected_sha" "$target" | sha256sum -c -
    return 0
  }
  APPDIR="$WORK/TrackCraft.AppDir"
  cp -R "$STAGE" "$APPDIR"
  ln -s usr/bin/trackcraft "$APPDIR/AppRun"
  cp "$HERE/$APP_ID.desktop" "$APPDIR/$APP_ID.desktop"
  cp "$ROOT/assets/app-icon/hicolor/256x256/apps/$APP_ID.png" "$APPDIR/$APP_ID.png"
  ln -s "$APP_ID.png" "$APPDIR/.DirIcon"

  TOOL="${APPIMAGETOOL:-$(command -v appimagetool || true)}"
  if [[ -z "$TOOL" ]]; then
    TOOL="$CARGO_TARGET_DIR/appimagetool-$APPIMAGETOOL_VERSION-$ARCH.AppImage"
    fetch_pinned "https://github.com/AppImage/appimagetool/releases/download/$APPIMAGETOOL_VERSION/appimagetool-$ARCH.AppImage" \
      "$APPIMAGETOOL_SHA256" "$TOOL"
  fi
  RUNTIME="$CARGO_TARGET_DIR/appimage-runtime-$RUNTIME_VERSION-$ARCH"
  fetch_pinned "https://github.com/AppImage/type2-runtime/releases/download/$RUNTIME_VERSION/runtime-$ARCH" \
    "$RUNTIME_SHA256" "$RUNTIME"
  # Absolute, because appimagetool runs in $DIST below.
  OUT="$(cd "$DIST" && pwd)/$BASENAME.AppImage"
  APPDIR="$(cd "$APPDIR" && pwd)"
  TOOL="$(cd "$(dirname "$TOOL")" && pwd)/$(basename "$TOOL")"
  RUNTIME="$(cd "$(dirname "$RUNTIME")" && pwd)/$(basename "$RUNTIME")"
  rm -f "$OUT.zsync"
  # Update information for AppImageUpdate: the .zsync published next to each AppImage on GitHub
  # Releases. A fork's builds point at its own releases through GITHUB_REPOSITORY.
  REPO="${GITHUB_REPOSITORY:-hsmith-dev/trackcraft}"
  UPDATE_INFO="gh-releases-zsync|${REPO%%/*}|${REPO#*/}|latest|trackcraft-*-linux-$ARCH.AppImage.zsync"
  # Extract-and-run: works without FUSE (containers, CI).
  (cd "$DIST" && ARCH="$ARCH" APPIMAGE_EXTRACT_AND_RUN=1 "$TOOL" --no-appstream \
    --runtime-file "$RUNTIME" -u "$UPDATE_INFO" "$APPDIR" "$OUT")
  echo "wrote $OUT"
  if [[ -s "$OUT.zsync" ]]; then
    echo "wrote $OUT.zsync"
  else
    warn "zsyncmake not found, so $OUT.zsync was not written; AppImage delta updates need it"
  fi
fi

if [[ -z "${CROSS_TARGET:-}" ]]; then
  "$STAGE/usr/bin/trackcraft-cli" --version
elif [[ -n "${EMULATOR:-}" ]]; then
  "$EMULATOR" "$STAGE/usr/bin/trackcraft-cli" --version
fi
echo "==> done"
ls -lh "$DIST"
