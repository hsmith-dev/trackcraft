#!/usr/bin/env bash
# Build TrackCraft as a Flatpak OCI layout and (with --push) push it to the GitHub Container
# Registry, for Flatpak OCI clients. Called by the shared release workflow's flatpak job.
#
# Usage:
#   packaging/linux/flatpak-oci.sh [--push] [TARBALL]
#
# Environment variables:
#   REGISTRY       Target registry image (default: ghcr.io/hsmith-dev/trackcraft)
#   TAG            Target image tag (default: $VERSION-$ARCH, also tags stable-$ARCH)
#   GITHUB_TOKEN   Token for GHCR authentication when --push is used
set -euo pipefail
# shellcheck source=../env.sh
. "$(dirname "${BASH_SOURCE[0]}")/../env.sh"
HERE="$ROOT/packaging/linux"

PUSH=0
TARBALL=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --push) PUSH=1; shift ;;
    -h | --help) sed -n '2,11p' "$0"; exit 0 ;;
    -*) echo "unknown argument: $1" >&2; exit 2 ;;
    *) TARBALL="$1"; shift ;;
  esac
done

ARCH="$(uname -m)"
case "$ARCH" in
  x86_64) ;;
  aarch64 | arm64) ARCH=aarch64 ;;
  *) echo "unsupported architecture $ARCH" >&2; exit 2 ;;
esac

BASENAME="trackcraft-$VERSION-linux-$ARCH"
OUT_OCI="$DIST/$BASENAME-oci"

echo "==> Building Flatpak OCI layout for $BASENAME"
"$HERE/flatpak-bundle.sh" --oci --no-test ${TARBALL:+"$TARBALL"}

[[ -d "$OUT_OCI" ]] || { echo "error: $OUT_OCI layout not generated" >&2; exit 1; }

REGISTRY="${REGISTRY:-ghcr.io/hsmith-dev/trackcraft}"
TAG="${TAG:-$VERSION-$ARCH}"

echo "==> OCI layout ready at $OUT_OCI (target: $REGISTRY:$TAG)"

if [[ "$PUSH" = 1 ]]; then
  command -v skopeo >/dev/null || { echo "error: skopeo is required to push OCI images (apt install skopeo)" >&2; exit 1; }
  AUTH_ARGS=()
  if [[ -n "${GITHUB_TOKEN:-}" ]]; then
    AUTH_ARGS=(--dest-creds "${GITHUB_ACTOR:-hsmith-dev}:$GITHUB_TOKEN")
  fi
  echo "==> Pushing to $REGISTRY:$TAG"
  skopeo copy --format oci ${AUTH_ARGS[@]+"${AUTH_ARGS[@]}"} "oci:$OUT_OCI" "docker://$REGISTRY:$TAG"
  echo "==> Pushing to $REGISTRY:stable-$ARCH"
  skopeo copy --format oci ${AUTH_ARGS[@]+"${AUTH_ARGS[@]}"} "oci:$OUT_OCI" "docker://$REGISTRY:stable-$ARCH"
  echo "==> Push complete"
fi
