#!/usr/bin/env bash
# Build (and tag) every moor sandbox image locally.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")"

# Built from the repo root, not ./base: the base image compiles moor's
# own MCP server from the mcp/ crate, so the context has to include it.
# See images/base/Dockerfile and /.dockerignore.
echo "==> building moor/base:latest"
# KEEL_VERSION, when set, overrides the Dockerfile's pinned keel.
docker build -t moor/base:latest ${KEEL_VERSION:+--build-arg KEEL_VERSION="$KEEL_VERSION"} -f base/Dockerfile ..

for lang in node rust python; do
  echo "==> building moor/${lang}:latest"
  docker build -t "moor/${lang}:latest" --build-arg BASE_IMAGE=moor/base:latest "./${lang}"
done

# The opt-in GitHub Copilot agent layer, on top of base. Kept out of
# moor/base on purpose — it ships a large compiled binary with vendored,
# not-independently-patchable HIGH CVEs, so only projects that select the
# copilot driver should carry it. See images/copilot/Dockerfile.
echo "==> building moor/copilot:latest"
docker build -t moor/copilot:latest --build-arg BASE_IMAGE=moor/base:latest ./copilot

echo "==> building moor/egress:latest"
docker build -t moor/egress:latest ../proxy

echo "==> done"
docker images 'moor/*'
