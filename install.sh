#!/usr/bin/env bash
#
# install.sh — download and install the `prima` binary for your OS/architecture.
#
# The binary is fetched from the GitHub Releases of this repository and verified
# against the published SHA-256 checksum before installation.
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/TickPoints/prima-language/main/install.sh | bash
#   bash install.sh                       # install latest release to ~/.local/bin
#   bash install.sh --version v0.3.0 # pin a version
#   bash install.sh --dir ~/bin           # override the install directory
#
# Overridable via environment variables:
#   PRIMA_VERSION      release tag to install (default: latest)
#   PRIMA_TARGET       target triple, e.g. x86_64-apple-darwin (default: detected)
#   PRIMA_LIBC         "gnu" (default) or "musl" on Linux; Termux/Android auto-selects musl
#   PRIMA_INSTALL_DIR  install directory (default: $HOME/.local/bin, $PREFIX/bin on Termux)
#   PRIMA_REPO         "owner/repo" (default: TickPoints/prima-language)

set -euo pipefail

REPO="${PRIMA_REPO:-TickPoints/prima-language}"
VERSION=""
TARGET=""
# Termux and other Android-on-Linux environments use bionic libc, so glibc/Linux-GNU
# binaries will not load; only static musl builds run there. Detect Termux via its
# `$PREFIX`/`$TERMUX_VERSION` markers and default the libc to musl.
is_termux() {
  [ "$(uname -s)" = "Linux" ] || return 1
  [ -n "${TERMUX_VERSION:-}" ] && return 0
  [ -n "${PREFIX:-}" ] && [ -d "${PREFIX:-}" ]
}
if is_termux; then
  DEFAULT_DIR="${PREFIX:-$HOME}/bin"
else
  DEFAULT_DIR="$HOME/.local/bin"
fi
INSTALL_DIR="${PRIMA_INSTALL_DIR:-$DEFAULT_DIR}"

usage() {
  sed -n '3,19p' "$0" | sed 's/^# \{0,1\}//'
  exit "${1:-0}"
}

while [ $# -gt 0 ]; do
  case "$1" in
    --version) VERSION="${2:?--version requires a value}"; shift 2 ;;
    --target) TARGET="${2:?--target requires a value}"; shift 2 ;;
    --dir) INSTALL_DIR="${2:?--dir requires a value}"; shift 2 ;;
    -h|--help) usage ;;
    *) usage 1 ;;
  esac
done

# --- Architecture / OS detection ---------------------------------------------

detect_os() {
  local uname
  uname="$(uname -s)"
  case "$uname" in
    Linux) echo linux ;;
    Darwin) echo darwin ;;
    MINGW*|MSYS*|CYGWIN*) echo windows ;;
    *) echo "unsupported: unsupported kernel '$uname'" >&2; exit 1 ;;
  esac
}

detect_arch() {
  local mach
  mach="$(uname -m)"
  case "$mach" in
    x86_64|amd64) echo x86_64 ;;
    aarch64|arm64) echo aarch64 ;;
    armv7l|armv7hf|armhf) echo armv7 ;;
    riscv64) echo riscv64 ;;
    ppc64le) echo ppc64le ;;
    s390x) echo s390x ;;
    *) echo "unsupported: machine '$mach'" >&2; exit 1 ;;
  esac
}

map_target() {
  local os="$1" arch="$2"
  case "$os" in
    darwin)
      echo "${arch}-apple-darwin"
      ;;
    windows)
      echo "${arch}-pc-windows-msvc"
      ;;
    linux)
      local libc="${PRIMA_LIBC:-}"
      if [ -z "$libc" ] && is_termux; then
        libc="musl"
      fi
      libc="${libc:-gnu}"
      case "$arch:$libc" in
        x86_64:gnu) echo x86_64-unknown-linux-gnu ;;
        x86_64:musl) echo x86_64-unknown-linux-musl ;;
        aarch64:gnu) echo aarch64-unknown-linux-gnu ;;
        aarch64:musl) echo aarch64-unknown-linux-musl ;;
        armv7:gnu) echo armv7-unknown-linux-gnueabihf ;;
        riscv64:gnu) echo riscv64gc-unknown-linux-gnu ;;
        ppc64le:gnu) echo powerpc64le-unknown-linux-gnu ;;
        s390x:gnu) echo s390x-unknown-linux-gnu ;;
        *) echo "unsupported: linux $arch ($libc) has no release asset" >&2; exit 1 ;;
      esac
      ;;
    *) exit 1 ;;
  esac
}

OS="$(detect_os)"
if [ -z "$TARGET" ]; then
  TARGET="$(map_target "$OS" "$(detect_arch)")"
fi

if [ "$OS" = "windows" ]; then
  EXT=".exe"
else
  EXT=""
fi

# --- Resolve the version ------------------------------------------------------

if [ -z "$VERSION" ]; then
  echo "==> resolving the latest release of $REPO"
  VERSION="$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -n1)"
  if [ -z "$VERSION" ]; then
    echo "error: could not determine the latest release of $REPO" >&2
    exit 1
  fi
fi

ARTIFACT="prima-${VERSION}-${TARGET}${EXT}"
URL="https://github.com/${REPO}/releases/download/${VERSION}/${ARTIFACT}"
SHA_URL="${URL}.sha256"

# --- Download and verify ------------------------------------------------------

TMPDIR="${TMPDIR:-/tmp}"
WORK="$(mktemp -d "$TMPDIR/prima-install.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT

# Download a URL to a file. Show curl's progress bar when stderr is a terminal
# (interactive session) so the user can watch the transfer; otherwise (CI,
# pipes, logs) stay quiet. curl emits the meter on stderr, leaving stdout clean.
fetch() {
  local url="$1" out="$2"
  if [ -t 2 ]; then
    curl -fL --progress-bar "$url" -o "$out"
  else
    curl -fsSL "$url" -o "$out"
  fi
}

echo "==> downloading $ARTIFACT"
if ! fetch "$URL" "$WORK/prima$EXT"; then
  echo "error: download failed — check that release '$VERSION' provides a '$TARGET' asset:" >&2
  echo "  $URL" >&2
  exit 1
fi
if ! curl -fsSL "$SHA_URL" -o "$WORK/prima.sha256"; then
  echo "error: could not fetch the checksum file:" >&2
  echo "  $SHA_URL" >&2
  exit 1
fi

echo "==> verifying SHA-256"
# The release .sha256 file is "<hash>  <asset-name>" (the release asset label, e.g.
# prima-v0.3.0-x86_64-unknown-linux-gnu), not the local filename we downloaded to.
# Compare the reported hash against the actual bytes directly rather than relying on
# `sha256sum -c`, which would look for a file named after the asset and fail.
expected="$(awk '{print $1}' "$WORK/prima.sha256")"
if command -v sha256sum >/dev/null 2>&1; then
  actual="$(sha256sum "$WORK/prima$EXT" | awk '{print $1}')"
elif command -v shasum >/dev/null 2>&1; then
  actual="$(shasum -a 256 "$WORK/prima$EXT" | awk '{print $1}')"
else
  echo "warning: no checksum tool found (need sha256sum or shasum); skipping SHA-256 verification" >&2
  actual=""
fi
if [ -n "$actual" ]; then
  if [ "$expected" != "$actual" ]; then
    echo "error: SHA-256 checksum mismatch — aborting for safety" >&2
    echo "       expected $expected" >&2
    echo "       got      $actual" >&2
    exit 1
  fi
  echo "==> checksum verified"
fi

# --- Install ---------------------------------------------------------------

mkdir -p "$INSTALL_DIR"
if [ ! -w "$INSTALL_DIR" ]; then
  echo "error: install directory is not writable: $INSTALL_DIR" >&2
  echo "       use --dir <path> or PRIMA_INSTALL_DIR to choose another location" >&2
  exit 1
fi

install -m 0755 "$WORK/prima$EXT" "$INSTALL_DIR/prima$EXT"
echo "==> installed to $INSTALL_DIR/prima$EXT"

case ":$PATH:" in
  *":$INSTALL_DIR:"*) ;;
  *)
    echo "==> note: $INSTALL_DIR is not on your PATH"
    echo "    add it, e.g.:"
    echo "      echo 'export PATH=\"$INSTALL_DIR:\$PATH\"' >> ~/.$(basename "$SHELL")rc"
    ;;
esac

echo "==> run \`prima --help\` to get started"
