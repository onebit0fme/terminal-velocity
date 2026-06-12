#!/bin/sh
# Terminal Velocity (tv) installer — downloads the prebuilt binary for this
# platform from GitHub Releases and installs it. POSIX sh, no dependencies
# beyond curl + tar.
#
#   curl -fsSL https://raw.githubusercontent.com/onebit0fme/terminal-velocity/main/install.sh | sh
#
# Overrides via env:
#   TV_VERSION      release tag to install (default: latest)
#   TV_INSTALL_DIR  where to put the binary (default: ~/.local/bin)
set -eu

REPO="onebit0fme/terminal-velocity"
BIN="tv"
INSTALL_DIR="${TV_INSTALL_DIR:-$HOME/.local/bin}"

err() { printf 'install.sh: %s\n' "$*" >&2; exit 1; }

# Map uname -> release target triple. musl on Linux = one static binary for any distro.
os="$(uname -s)"
arch="$(uname -m)"
case "$os" in
  Linux)  os_part="unknown-linux-musl" ;;
  Darwin) os_part="apple-darwin" ;;
  *) err "unsupported OS '$os'. On Windows, grab the .zip from https://github.com/$REPO/releases/latest" ;;
esac
case "$arch" in
  x86_64|amd64)  arch_part="x86_64" ;;
  aarch64|arm64) arch_part="aarch64" ;;
  *) err "unsupported architecture '$arch'" ;;
esac
asset="${BIN}-${arch_part}-${os_part}.tar.gz"

# Resolve the version: follow the /releases/latest redirect to read the tag
# (no GitHub API token, no jq).
tag="${TV_VERSION:-latest}"
if [ "$tag" = "latest" ]; then
  tag="$(curl -fsSLI -o /dev/null -w '%{url_effective}' "https://github.com/$REPO/releases/latest" | sed 's#.*/##')"
  [ -n "$tag" ] || err "could not resolve the latest release tag"
fi

url="https://github.com/$REPO/releases/download/$tag/$asset"
printf 'Installing %s %s (%s-%s) to %s\n' "$BIN" "$tag" "$arch_part" "$os_part" "$INSTALL_DIR"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
curl -fsSL "$url" -o "$tmp/$asset" || err "download failed: $url"
tar -xzf "$tmp/$asset" -C "$tmp" || err "could not extract $asset"

mkdir -p "$INSTALL_DIR"
mv "$tmp/$BIN" "$INSTALL_DIR/$BIN"
chmod 0755 "$INSTALL_DIR/$BIN"

printf 'Installed %s -> %s\n' "$tag" "$INSTALL_DIR/$BIN"
case ":$PATH:" in
  *":$INSTALL_DIR:"*) printf 'Run: %s\n' "$BIN" ;;
  *) printf '%s is not on your PATH. Add it:\n  export PATH="%s:$PATH"\n' "$INSTALL_DIR" "$INSTALL_DIR" ;;
esac
