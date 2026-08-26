#!/bin/sh
# herdr build hook: fetch a prebuilt lasso binary from GitHub Releases for
# this platform (verified by sha256), falling back to a local cargo build.
set -eu

REPO="skellleks/lasso"
VERSION=$(grep '^version' herdr-plugin.toml | head -1 | cut -d'"' -f2)

case "$(uname -s):$(uname -m)" in
    Darwin:x86_64) TARGET=x86_64-apple-darwin ;;
    Darwin:arm64) TARGET=aarch64-apple-darwin ;;
    Linux:x86_64 | Linux:amd64) TARGET=x86_64-unknown-linux-musl ;;
    Linux:aarch64 | Linux:arm64) TARGET=aarch64-unknown-linux-musl ;;
    *) TARGET="" ;;
esac

mkdir -p bin

fetch() {
    [ -n "$TARGET" ] || return 1
    command -v curl >/dev/null 2>&1 || return 1
    url="https://github.com/${REPO}/releases/download/v${VERSION}/lasso-${TARGET}.tar.gz"
    tmp=$(mktemp -d)
    trap 'rm -rf "$tmp"' EXIT
    echo "fetching ${url}" >&2
    curl -fsSL -o "$tmp/lasso.tar.gz" "$url" || return 1
    curl -fsSL -o "$tmp/lasso.tar.gz.sha256" "${url}.sha256" || return 1
    (
        cd "$tmp"
        if command -v sha256sum >/dev/null 2>&1; then
            sha256sum -c lasso.tar.gz.sha256
        else
            shasum -a 256 -c lasso.tar.gz.sha256
        fi
    ) || return 1
    tar -xzf "$tmp/lasso.tar.gz" -C bin lasso
    chmod +x bin/lasso
    echo "installed prebuilt lasso ${VERSION} (${TARGET})" >&2
}

if fetch; then
    exit 0
fi

echo "no prebuilt binary for $(uname -s)/$(uname -m) — building from source" >&2
command -v cargo >/dev/null 2>&1 || {
    echo "cargo not found; install a Rust toolchain (https://rustup.rs) and reinstall the plugin" >&2
    exit 1
}
cargo build --locked --release
cp target/release/lasso bin/lasso
echo "built lasso from source" >&2
