#!/bin/sh
# Install the latest ngx release (static Linux binary) and its man pages.
#
#   curl -fsSL https://raw.githubusercontent.com/yuukiflow/ngx/main/scripts/install.sh | sudo sh
#
# Options (environment): NGX_VERSION=v0.4.0 to pick a release, PREFIX=/usr/local.
# The download is checked against the release's SHA256SUMS before anything is installed.
set -eu

REPO=yuukiflow/ngx
PREFIX=${PREFIX:-/usr/local}

case "$(uname -s)" in
  Linux) ;;
  *) echo "ngx only runs on Linux" >&2; exit 1 ;;
esac
case "$(uname -m)" in
  x86_64 | amd64) target=x86_64-unknown-linux-musl ;;
  aarch64 | arm64) target=aarch64-unknown-linux-musl ;;
  *) echo "no prebuilt binary for $(uname -m); build from source: cargo install --path ." >&2; exit 1 ;;
esac

fetch() {
  if command -v curl >/dev/null 2>&1; then curl -fsSL "$1" -o "$2"; else wget -qO "$2" "$1"; fi
}

version=${NGX_VERSION:-}
if [ -z "$version" ]; then
  tmpv=$(mktemp)
  fetch "https://api.github.com/repos/$REPO/releases/latest" "$tmpv"
  version=$(sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' "$tmpv" | head -n1)
  rm -f "$tmpv"
  [ -n "$version" ] || { echo "could not find the latest release" >&2; exit 1; }
fi

name="ngx-$version-$target"
dir=$(mktemp -d)
trap 'rm -rf "$dir"' EXIT
echo "downloading $name"
fetch "https://github.com/$REPO/releases/download/$version/$name.tar.gz" "$dir/$name.tar.gz"
fetch "https://github.com/$REPO/releases/download/$version/SHA256SUMS" "$dir/SHA256SUMS"
(cd "$dir" && grep " $name.tar.gz\$" SHA256SUMS | sha256sum -c -) || { echo "checksum mismatch, not installing" >&2; exit 1; }

tar -C "$dir" -xzf "$dir/$name.tar.gz"
install -d "$PREFIX/bin" "$PREFIX/share/man/man1"
install -m 755 "$dir/$name/ngx" "$PREFIX/bin/ngx"
install -m 644 "$dir/$name"/man/*.1 "$PREFIX/share/man/man1/"
if [ -d "$dir/$name/completions" ]; then
  install -Dm 644 "$dir/$name/completions/ngx.bash" "$PREFIX/share/bash-completion/completions/ngx"
  install -Dm 644 "$dir/$name/completions/_ngx" "$PREFIX/share/zsh/site-functions/_ngx"
  install -Dm 644 "$dir/$name/completions/ngx.fish" "$PREFIX/share/fish/vendor_completions.d/ngx.fish"
fi
echo "installed ngx $version to $PREFIX/bin/ngx — run \`ngx doctor\`"
