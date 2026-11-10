#!/bin/sh
# Installs jtac: fetches the release archive for this platform, verifies it against the release
# attestation the CI key signs, and copies `jtac` into one directory. Never edits PATH.
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
source_build="Build from source instead, from a release tag (https://github.com/dmriding/jtac/releases): cargo install --locked --git https://github.com/dmriding/jtac --tag $tag jtac"
report="This is a fence, not a retry: do not fall back to SHA256SUMS, which sits on the same page. Report it privately: https://github.com/dmriding/jtac/security/advisories/new"
openssh="(apt install openssh-client, or brew install openssh), then run the installer again."

if [ "$to" = /.local/bin ]; then
  echo "install.sh: HOME is unset, so there is no default directory; nothing installed." >&2
  echo "The default is ~/.local/bin, and the installer never guesses another." >&2
  echo "Say where: sh install.sh --to DIR" >&2
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
    echo "$source_build" >&2
    exit 2 ;;
esac
if ! command -v ssh-keygen >/dev/null 2>&1; then
  echo "install.sh: needs ssh-keygen to verify the release attestation; nothing installed." >&2
  echo "jtac signs and verifies with ssh-keygen too, so it cannot run without it." >&2
  echo "Install OpenSSH $openssh" >&2
  exit 2
fi

base="${JTAC_INSTALL_BASE:-}"
if [ -z "$base" ]; then
  if [ "$version" = latest ]; then
    base="https://github.com/dmriding/jtac/releases/latest/download"
  else
    base="https://github.com/dmriding/jtac/releases/download/$version"
  fi
fi
# A URL is never read as a directory: the default one would name a path under the working directory.
case $base in http://*|https://*) dir= ;; *) dir=${base#file://} ;; esac

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
fetch() {
  if [ -n "$dir" ]; then
    cp "$dir/$1" "$2" && return
  elif command -v curl >/dev/null 2>&1; then
    curl -fsSL "$base/$1" -o "$2" && return
  elif command -v wget >/dev/null 2>&1; then
    wget -q "$base/$1" -O "$2" && return
  else
    echo "install.sh: needs curl or wget to fetch $base/$1; nothing installed." >&2
    echo "The release is fetched over HTTPS, and sh has no client of its own." >&2
    echo "Install one (apt install curl), then run the installer again; JTAC_INSTALL_BASE=DIR installs from a directory instead." >&2
    exit 2
  fi
  echo "install.sh: could not fetch $base/$1; nothing installed." >&2
  case $1 in
    jtac-release.*) echo "The installer installs only what a signed release attestation names, and v0.2.7 and older have none. $source_build" >&2 ;;
    *) echo "The installer installs only an archive it fetched whole and verified." >&2 ;;
  esac
  echo "Check the network and the release page, then run the installer again; JTAC_INSTALL_BASE names another source." >&2
  exit 2
}

# The `jtac-release-v1` lines of .jtac/signers, held equal by tests/install.rs. Never the whole
# file: a line with no namespaces option verifies in every namespace, so a developer's key would attest.
cat > "$tmp/signers" <<'SIGNERS'
jtac-ci@users.noreply.github.com namespaces="jtac-ci-root-v1,jtac-release-v1" ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIGuboJpHI1jqXzTmbzYWAuBPUNAxPlR+XfuP4NjWMrGR
SIGNERS
read -r principal _ < "$tmp/signers"
fetch jtac-release.json "$tmp/release.json"
fetch jtac-release.sig "$tmp/release.sig"
# ssh-keygen exits 255 for every signature that fails to verify; other codes mean it could not check.
verified=0
ssh-keygen -Y verify -f "$tmp/signers" -I "$principal" -n jtac-release-v1 -s "$tmp/release.sig" < "$tmp/release.json" >/dev/null || verified=$?
case $verified in
  0) ;;
  255)
    echo "install.sh: the release attestation at $base is not signed by jtac's release key; nothing installed." >&2
    echo "Only the CI key attests a release, so any other signature is a substituted release, and a provenance tool is not installed on trust." >&2
    echo "$report" >&2
    exit 1 ;;
  *)
    echo "install.sh: ssh-keygen could not check the release attestation (exit $verified); nothing installed." >&2
    echo "Checking it takes ssh-keygen -Y verify, which OpenSSH has had since 8.1." >&2
    echo "Upgrade OpenSSH $openssh" >&2
    exit 2 ;;
esac

# The document is one line of sorted keys: each artifact is {"name":...,"sha256":...}.
pair=$(tr '{' '\n' < "$tmp/release.json" | grep -E "^\"name\":\"jtac-v[^\"/]*-$target\\.tar\\.gz\",\"sha256\":\"sha256:[0-9a-f]{64}\"}" || true)
case $pair in
  ""|*"
"*)
    echo "install.sh: the release attestation at $base does not name exactly one archive for $target; nothing installed." >&2
    echo "Only an archive the signed document names installs, and it names no single one for this platform." >&2
    echo "$source_build" >&2
    exit 2 ;;
esac
archive=${pair#'"name":"'}
archive=${archive%%'"'*}
want=${pair#*'"sha256":"sha256:'}
want=${want%%'"'*}
if [ "$version" != latest ] && [ "$archive" != "jtac-$version-$target.tar.gz" ]; then
  echo "install.sh: the release attestation at $base is for $archive, not $version; nothing installed." >&2
  echo "Each release's page carries its own attestation, so another version's is a substituted one." >&2
  echo "$report" >&2
  exit 1
fi
fetch "$archive" "$tmp/$archive"
if command -v sha256sum >/dev/null 2>&1; then got=$(sha256sum "$tmp/$archive" | cut -d' ' -f1)
elif command -v shasum >/dev/null 2>&1; then got=$(shasum -a 256 "$tmp/$archive" | cut -d' ' -f1)
else
  echo "install.sh: needs sha256sum or shasum to verify $archive; nothing installed." >&2
  echo "The archive is held to the digest the attestation signs before anything is copied." >&2
  echo "Install coreutils (apt install coreutils), then run the installer again." >&2
  exit 2
fi
if [ "$got" != "$want" ]; then
  echo "install.sh: $archive does not match the signed release attestation (got $got, want $want); nothing installed." >&2
  echo "A mismatch is a corrupted or substituted download, and a provenance tool is not installed on trust." >&2
  echo "Run the installer again; if it repeats: $report" >&2
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
echo "installed $("$to/jtac" --version) at $to/jtac, verified against the release attestation $principal signed"
case ":$PATH:" in
  *":$to:"*) ;;
  *) echo "note: $to is not on PATH. Add it in your shell's rc file or run $to/jtac by path; install.sh never edits PATH." ;;
esac
echo "next: cd your-repo && jtac init"
