#!/bin/sh
# Install corntchap-cli from the latest GitHub release.
#   curl -fsSL https://raw.githubusercontent.com/cratestack/corntchap/main/install.sh | sh
# Env: VERSION=v0.1.0 to pin, INSTALL_DIR=~/.local/bin to override the destination.
set -eu
REPO=cratestack/corntchap
os=$(uname -s | tr '[:upper:]' '[:lower:]')
arch=$(uname -m)
case "$os" in linux|darwin) ;; *) echo "unsupported OS: $os" >&2; exit 1;; esac
case "$arch" in
  x86_64|amd64) arch=x86_64 ;;
  aarch64|arm64) if [ "$os" = darwin ]; then arch=arm64; else arch=aarch64; fi ;;
  *) echo "unsupported architecture: $arch" >&2; exit 1 ;;
esac
asset="corntchap-cli-$os-$arch.tar.gz"
if [ -n "${VERSION:-}" ]; then
  url="https://github.com/$REPO/releases/download/$VERSION/$asset"
else
  url="https://github.com/$REPO/releases/latest/download/$asset"
fi
dir=${INSTALL_DIR:-}
if [ -z "$dir" ]; then
  if [ -w /usr/local/bin ]; then dir=/usr/local/bin; else dir=$HOME/.local/bin; fi
fi
mkdir -p "$dir"
tmp=$(mktemp -d); trap 'rm -rf "$tmp"' EXIT
echo "downloading $url"
curl -fsSL "$url" -o "$tmp/$asset"
curl -fsSL "$url.sha256" -o "$tmp/$asset.sha256"
want=$(awk '{print $1}' "$tmp/$asset.sha256")
if command -v sha256sum >/dev/null 2>&1; then got=$(sha256sum "$tmp/$asset" | awk '{print $1}')
elif command -v shasum >/dev/null 2>&1; then got=$(shasum -a 256 "$tmp/$asset" | awk '{print $1}')
else echo "need sha256sum or shasum to verify the download" >&2; exit 1; fi
if [ -z "$want" ] || [ "$want" != "$got" ]; then
  echo "checksum mismatch for $asset (expected ${want:-none}, got $got); not installing" >&2
  exit 1
fi
echo "checksum ok"
tar xzf "$tmp/$asset" -C "$tmp"
install -m 0755 "$tmp/corntchap-cli-$os-$arch" "$dir/corntchap-cli"
echo "installed $dir/corntchap-cli"
case ":$PATH:" in *":$dir:"*) ;; *) echo "note: $dir is not on your PATH" ;; esac
