#!/bin/sh
# Installs jtac: fetches the release archive for this platform, verifies it against the
# SHA256SUMS the release publishes, and copies `jtac` into one directory. Never edits PATH.
#   sh install.sh [--to DIR] [--version vX.Y.Z]
# JTAC_INSTALL_BASE names where archives come from instead of GitHub: a URL or a directory.
set -eu

to="${HOME:-}/.local/bin"
version=latest
while [ $# -gt 0 ]; do
  case "$1" in
    --to) [ $# -ge 2 ] || { echo "install.sh: --to needs a directory; usage: sh install.sh [--to DIR] [--version vX.Y.Z]" >&2; exit 2; }; to="$2"; shift 2 ;;
    --version) [ $# -ge 2 ] || { echo "install.sh: --version needs a tag; usage: sh install.sh [--to DIR] [--version vX.Y.Z]" >&2; exit 2; }; version="$2"; shift 2 ;;
    -h|--help) echo "usage: sh install.sh [--to DIR] [--version vX.Y.Z]"; exit 0 ;;
    *) echo "install.sh: unknown argument $1; usage: sh install.sh [--to DIR] [--version vX.Y.Z]" >&2; exit 2 ;;
  esac
done
case $version in latest) tag=vX.Y.Z ;; *) tag=$version ;; esac

if [ "$to" = /.local/bin ]; then
  echo "install.sh: HOME is unset, so there is no default directory; say where with --to DIR." >&2
  exit 2
fi

os=$(uname -s)
arch=$(uname -m)
case "$os-$arch" in
  Linux-x86_64) target=x86_64-unknown-linux-musl ;;
  Darwin-arm64) target=aarch64-apple-darwin ;;
  *)
    echo "install.sh: no prebuilt jtac for $os $arch." >&2
    echo "Releases ship Linux x86_64 and macOS arm64; Windows takes the zip from the release page." >&2
    echo "Build from source instead, from a release tag (https://github.com/dmriding/jtac/releases): cargo install --locked --git https://github.com/dmriding/jtac --tag $tag jtac" >&2
    exit 2 ;;
esac

base="${JTAC_INSTALL_BASE:-}"
if [ -z "$base" ]; then
  if [ "$version" = latest ]; then
    base="https://github.com/dmriding/jtac/releases/latest/download"
  else
    base="https://github.com/dmriding/jtac/releases/download/$version"
  fi
fi

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
fetch() {
  if [ -d "${base#file://}" ]; then
    cp "${base#file://}/$1" "$2" && return
  elif command -v curl >/dev/null 2>&1; then
    curl -fsSL "$base/$1" -o "$2" && return
  elif command -v wget >/dev/null 2>&1; then
    wget -q "$base/$1" -O "$2" && return
  else
    echo "install.sh: needs curl or wget to fetch $base/$1; nothing installed." >&2
    exit 2
  fi
  echo "install.sh: could not fetch $base/$1; nothing installed." >&2
  echo "Check the network and the release page, then run the installer again; JTAC_INSTALL_BASE names another source." >&2
  exit 2
}

fetch SHA256SUMS "$tmp/SHA256SUMS"
line=$(grep " jtac-.*-$target\.tar\.gz\$" "$tmp/SHA256SUMS" | head -1 || true)
if [ -z "$line" ]; then
  echo "install.sh: SHA256SUMS at $base lists no archive for $target; nothing installed." >&2
  echo "Build from source instead, from a release tag (https://github.com/dmriding/jtac/releases): cargo install --locked --git https://github.com/dmriding/jtac --tag $tag jtac" >&2
  exit 2
fi
archive=${line##* }
want=${line%% *}
fetch "$archive" "$tmp/$archive"
if command -v sha256sum >/dev/null 2>&1; then got=$(sha256sum "$tmp/$archive" | cut -d' ' -f1)
elif command -v shasum >/dev/null 2>&1; then got=$(shasum -a 256 "$tmp/$archive" | cut -d' ' -f1)
else echo "install.sh: needs sha256sum or shasum to verify $archive; nothing installed." >&2; exit 2
fi
if [ "$got" != "$want" ]; then
  echo "install.sh: $archive does not match SHA256SUMS (got $got, want $want); nothing installed." >&2
  echo "A mismatch is a corrupted or substituted download, and a provenance tool is not installed on trust." >&2
  echo "Run the installer again; if it repeats, fetch $base/$archive by hand and compare it with $base/SHA256SUMS." >&2
  exit 1
fi

tar xzf "$tmp/$archive" -C "$tmp"
if ! { mkdir -p "$to" && [ ! -d "$to/jtac" ] && cp "$tmp/${archive%.tar.gz}/jtac" "$to/.jtac.new" && chmod 755 "$to/.jtac.new" && mv -f "$to/.jtac.new" "$to/jtac"; }; then
  rm -f "$to/.jtac.new" 2>/dev/null || true
  echo "install.sh: could not write $to/jtac; nothing installed." >&2
  echo "The archive verified; only the destination refused the write." >&2
  echo "Pick a directory you can write with --to DIR, then run the installer again." >&2
  exit 2
fi
echo "installed $("$to/jtac" --version) at $to/jtac, verified against SHA256SUMS"
case ":$PATH:" in
  *":$to:"*) ;;
  *) echo "note: $to is not on PATH. Add it in your shell's rc file or run $to/jtac by path; install.sh never edits PATH." ;;
esac
echo "next: cd your-repo && jtac init"
