#!/usr/bin/env bash

set -euo pipefail

WARD_REPO="${WARD_REPO:-milchinskiy/ward}"
WARD_BIN="${WARD_BIN:-ward}"
WARD_VERSION="${WARD_VERSION:-latest}"   # "latest" or e.g. "v0.1.0"
WARD_FLAVOR="${WARD_FLAVOR:-auto}"       # auto|gnu|musl

INSTALL_DIR="${INSTALL_DIR:-/usr/local/bin}"

need() { command -v "$1" >/dev/null 2>&1; }

os="$(uname -s)"
arch="$(uname -m)"

if [[ "$os" != "Linux" || "$arch" != "x86_64" ]]; then
  echo "Unsupported platform: ${os} ${arch}. This installer supports Linux x86_64 only." >&2
  exit 1
fi

detect_flavor() {
  if [[ "$WARD_FLAVOR" == "gnu" || "$WARD_FLAVOR" == "musl" ]]; then
    echo "$WARD_FLAVOR"
    return
  fi

  # Alpine/musl often has ldd output containing "musl"
  if need ldd; then
    if ldd --version 2>&1 | grep -qi musl; then
      echo "musl"
      return
    fi
  fi

  echo "gnu"
}

flavor="$(detect_flavor)"

case "$flavor" in
  gnu)  target="x86_64-unknown-linux-gnu" ;;
  musl) target="x86_64-unknown-linux-musl" ;;
  *) echo "Invalid WARD_FLAVOR=$flavor (expected auto|gnu|musl)"; exit 1 ;;
esac

if [[ "$WARD_VERSION" == "latest" ]]; then
  base="https://github.com/${WARD_REPO}/releases/latest/download"
  tag="latest"
else
  base="https://github.com/${WARD_REPO}/releases/download/${WARD_VERSION}"
  tag="$WARD_VERSION"
fi

asset="${WARD_BIN}-${tag}-${target}.tar.gz"
url="${base}/${asset}"
sum_url="${url}.sha256"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

fetch() {
  local u="$1" out="$2"
  if need curl; then
    curl -fsSLo "$out" "$u"
  elif need wget; then
    wget -qO "$out" "$u"
  else
    echo "Need curl or wget" >&2
    exit 1
  fi
}

echo "Downloading: $url"
fetch "$url" "$tmp/$asset"

# Best-effort checksum verification
if need sha256sum; then
  if fetch "$sum_url" "$tmp/$asset.sha256" 2>/dev/null; then
    (cd "$tmp" && sha256sum -c "$asset.sha256")
  else
    echo "Warning: checksum not found; skipping verification."
  fi
fi

tar -xzf "$tmp/$asset" -C "$tmp"

stage="$tmp/${WARD_BIN}-${tag}-${target}"
src="$stage/$WARD_BIN"

if [[ ! -f "$src" ]]; then
  echo "Archive did not contain expected binary: $src" >&2
  exit 1
fi

# Install: prefer /usr/local/bin, but fall back to ~/.local/bin if not writable
dst="${INSTALL_DIR}/${WARD_BIN}"
if install -m 0755 "$src" "$dst" 2>/dev/null; then
  :
else
  if [[ "$INSTALL_DIR" == "/usr/local/bin" ]]; then
    echo "No permission to write ${INSTALL_DIR}; installing to ~/.local/bin instead."
    INSTALL_DIR="${HOME}/.local/bin"
    mkdir -p "$INSTALL_DIR"
    dst="${INSTALL_DIR}/${WARD_BIN}"
    install -m 0755 "$src" "$dst"
    echo "Ensure ${INSTALL_DIR} is in PATH."
  else
    echo "Failed to install to ${INSTALL_DIR}. Try running with sudo or change INSTALL_DIR." >&2
    exit 1
  fi
fi

echo "Installed: $dst"
"$dst" --help >/dev/null 2>&1 || true

