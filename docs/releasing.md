# Releasing TrackCraft

> **Status:** 2026-11-01 · M6.1 — release pipeline, packaging and app icon in place; no release has been made yet.

TrackCraft releases the way its sibling apps (Wordcraft, Gridcraft, Pdfcraft, Photocraft) do:
`.github/workflows/release.yml` calls the suite's shared workflow,
[`storytold/craft-common-actions/.github/workflows/release.yml@main`](https://github.com/storytold/craft-common-actions),
which runs this repository's `packaging/` scripts on each platform and drafts a GitHub Release.

## Steps

1. Bump the version on `main`: `cargo xtask version set 0.2.0` (the one source of truth is
   `[workspace.package] version` in `Cargo.toml`; `cargo xtask version` prints it,
   `trackcraft-cli --version` and `GET /api/health` report it).
2. Merge `main` into `release` and push. The workflow builds:

| Platform | Artifacts | Signing |
|---|---|---|
| macOS | universal `TrackCraft.app` in a `.dmg`; universal `trackcraft-cli` + `trackcraft-server` zip | Developer ID + notarization (`APPLE_*`) |
| Windows | x64, x86 and arm64 `.msi` (per-user by default, no admin rights; `ALLUSERS=1` per machine; registers `.track`) + portable `.zip` of all three `.exe` | Authenticode: `.pfx` (`WINDOWS_CERTIFICATE*`) or Azure Trusted Signing (`AZURE_*`) |
| Linux | x86_64 and aarch64 `.AppImage` (+ `.zsync`), `.flatpak` bundle (also pushed to GHCR as an OCI image), `.deb`, `.rpm`, `.tar.gz`, Arch `.pkg.tar.zst`; Flathub manifest in the repo | checksums |
| Linux riscv64 | `.deb`, `.rpm`, `.tar.gz` (cross-compiled; CLI smoke-tested under QEMU) | checksums |
| FreeBSD | x86_64 and aarch64 `.tar.gz` (`/usr/local` tree with all three binaries) | checksums |
| Server | `trackcraft-server-<version>-linux-{x86_64,aarch64}.tar.gz` (binary + example systemd unit) | checksums |
| Web | `trackcraft-web-<version>.zip` (static site; serve it with `trackcraft-server serve --web`) | none |

3. The run drafts a GitHub Release `TrackCraft v<version>` with `SHA256SUMS.txt`; review and publish it.

Dry run: `gh workflow run release.yml --ref <branch> -f version=0.1.0-rc.1` builds the unsigned
jobs (version, Linux, riscv64, Flatpak, Arch, FreeBSD, server, web) on any branch; the macOS and
Windows jobs use the `release` environment, whose branch rule should allow only `release`, so no
draft release is made from a test branch.

## Signing secrets (owner action required)

The workflow passes these secrets by the names the shared workflow expects. **None are set yet:
the owner must create a `release` environment in the repository settings (deployment branch:
`release` only) and add the values there.** On the `release` branch the shared workflow refuses
to build macOS or Windows without them; on other branches the packaging scripts warn and ship
unsigned builds, so forks still build.

| Secret | Used for |
|---|---|
| `APPLE_CERTIFICATE` | Developer ID Application certificate + key, base64 `.p12` (`packaging/macos/import-cert.sh`) |
| `APPLE_CERTIFICATE_PASSWORD` | Its `.p12` password |
| `KEYCHAIN_PASSWORD` | Password for the temporary CI keychain (any random string) |
| `APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID` | Notarization (`notarytool`): Apple ID, app-specific password, team id |
| `WINDOWS_CERTIFICATE`, `WINDOWS_CERTIFICATE_PASSWORD` | Authenticode with a code-signing `.pfx` (base64) — or the six below |
| `AZURE_TENANT_ID`, `AZURE_CLIENT_ID`, `AZURE_CLIENT_SECRET`, `AZURE_SIGNING_ENDPOINT`, `AZURE_SIGNING_ACCOUNT`, `AZURE_CERT_PROFILE` | Azure Trusted Signing (`packaging/windows/sign.ps1`) |

The Flatpak OCI push to `ghcr.io/hsmith-dev/trackcraft` uses the run's `GITHUB_TOKEN`
(`packages: write`), no extra secret.

## Identity

- **Application id:** `io.github.hsmith_dev.trackcraft` — the reverse-DNS id of the owner's GitHub
  account (the sibling apps use their organization's domain, `ai.storyteller.<app>`; TrackCraft
  has no domain of its own, so it uses the GitHub form Flathub verifies, with `-` written `_` as
  AppStream and Flatpak ids require). It is the macOS bundle id, the Linux desktop entry,
  AppStream and Flatpak id, and the Wayland app id. It is set in `packaging/env.sh`,
  `apps/trackcraft/src/main.rs` and `xtask/src/icons.rs`, and in the file names under
  `packaging/linux` and `assets/app-icon/hicolor`; change all of them together (it is hard to
  change after a release, since installs and software centres key on it).
- **File type:** `.track` is `application/x-trackcraft` (shared-mime-info:
  `packaging/linux/io.github.hsmith_dev.trackcraft.mime.xml`, matched by extension and by the
  `"format": "trackcraft"` envelope), the UTI `io.github.hsmith_dev.trackcraft.workspace` on macOS
  (`Info.plist.in`) and the `TrackCraft.Workspace` ProgId on Windows (`trackcraft.wxs`).
- **Debian maintainer:** `TrackCraft contributors <hsmith-dev@users.noreply.github.com>` unless
  `TRACKCRAFT_MAINTAINER` is set (`packaging/linux/package.sh`).

## The app icon

`assets/app-icon/trackcraft.svg` is the source (original artwork: the in-app mark — a teal tile
with three board columns). `packaging/icons.sh` regenerates every other icon file from it with
`cargo xtask icons`, which rasterizes the SVG itself (the icon keeps to a small subset it draws
exactly: a vertical gradient and rounded rectangles), so no SVG tool is needed; on macOS it also
writes the `.icns` with `iconutil`. The outputs are committed and listed in `ATTRIBUTION.md`.
`apps/trackcraft/build.rs` and `apps/trackcraft-server/build.rs` embed `trackcraft.ico` and
VERSIONINFO into the Windows executables (winresource); the desktop app sets its window icon
from the PNGs.

## Local packaging

All scripts read the version from `Cargo.toml` and write to `dist/release`:

- `packaging/macos/package.sh --arch universal` (ad-hoc signed without `MACOS_SIGN_IDENTITY`)
- `pwsh packaging/windows/package.ps1 -Arch x64` (needs WiX v5)
- `packaging/linux/package.sh` (needs nfpm for deb/rpm), then `packaging/linux/flatpak-bundle.sh`
- `packaging/freebsd/package.sh` (`--dry-run` prints what it would build)
- `packaging/server/package.sh`
- `packaging/web/package.sh` (needs trunk)

`.github/workflows/packaging-lint.yml` (shared `packaging-lint.yml`) checks the scripts
(shellcheck with `.shellcheckrc`, PowerShell parse), the XML, the desktop entry, AppStream
metadata and that the two Flatpak manifests agree.

## Known limits

- macOS: double-clicking a `.track` file in Finder launches TrackCraft but does not open that
  file yet (winit doesn't deliver the Apple "open documents" event; Wordcraft handles it with an
  extra crate). `trackcraft file.track` and File › Open work.
- No DMG background artwork (Finder's default layout), unlike Wordcraft.
- The Windows MSI has no "for everyone" choice in its UI; pass `ALLUSERS=1` to `msiexec`.
- None of the packaging has been run in CI yet; locally only the macOS script (ad-hoc,
  aarch64), the web zip and the FreeBSD dry run were exercised.
