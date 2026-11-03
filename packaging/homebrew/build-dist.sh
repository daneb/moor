#!/usr/bin/env bash
#
# Package one release binary for the Homebrew cask:
#
#   packaging/homebrew/build-dist.sh <rust-target> <out-dir>
#
# Writes <out-dir>/moor-<version>-<arch>-apple-darwin.tar.gz and its .sha256.
# The archive holds the binary plus the sources `images/build.sh` needs
# (images/, proxy/, policies/, mcp/, .dockerignore) under share/moor/, laid
# out as in the repo because the base image builds from the repo root. A
# bare `cargo install moor` has no images — this is what fixes that.

set -euo pipefail

target="${1:?usage: build-dist.sh <rust-target> <out-dir>}"
out="${2:?usage: build-dist.sh <rust-target> <out-dir>}"

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
version=$(awk -F'"' '/^version = /{ print $2; exit }' "$root/cli/Cargo.toml")
arch="${target%%-*}"
name="moor-${version}-${arch}-apple-darwin"

(cd "$root/cli" && cargo build --release --locked --target "$target")

mkdir -p "$out"
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT

dest="$stage/$name"
share="$dest/share/moor"
mkdir -p "$share"
cp "$root/cli/target/$target/release/moor" "$dest/moor"
cp -R "$root/images" "$root/proxy" "$root/policies" "$share/"
cp "$root/.dockerignore" "$share/"
mkdir -p "$share/mcp"
cp -R "$root/mcp/src" "$root/mcp/Cargo.toml" "$share/mcp/"
[ ! -f "$root/mcp/Cargo.lock" ] || cp "$root/mcp/Cargo.lock" "$share/mcp/"
cp "$root/LICENSE" "$dest/"

tar -C "$stage" -czf "$out/$name.tar.gz" "$name"
(cd "$out" && shasum -a 256 "$name.tar.gz" > "$name.tar.gz.sha256")
echo "$out/$name.tar.gz"
