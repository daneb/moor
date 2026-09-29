# moor base sandbox image.
# Hardened, non-root, no host tooling beyond what an agent needs to build/
# lint/test a repo under keel's direction. No docker CLI, no sudo, no SSH
# server, no bind-mount assumptions anywhere.

# --- stage 1: build keel from crates.io -------------------------------
FROM rust:1-slim-bookworm AS keel-build
# Same reason as the runtime stage below: a failed download in a pipe must
# fail the build, not produce an image without the tool.
SHELL ["/bin/bash", "-o", "pipefail", "-c"]
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config libssl-dev ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*
# Pinned, so a keel release changes this layer's cache key: an unpinned
# install was served from cache and kept the previous keel. Bump it here,
# or pass --build-arg KEEL_VERSION=… (images/build.sh forwards $KEEL_VERSION).
ARG KEEL_VERSION=0.9.0
RUN cargo install keel-harness --version "$KEEL_VERSION" --locked

# The scanners a project's gates call for advisories and leaked secrets.
# A gate that needs one reports "blocked" when it's missing, never "clean"
# (we did not look is not the same as we looked), so without these every
# such gate is blocked in the sandbox. Both pinned, like keel.
ARG CARGO_AUDIT_VERSION=0.22.2
RUN cargo install cargo-audit --version "$CARGO_AUDIT_VERSION" --locked

# gitleaks is a Go binary: fetched from its release, verified against the
# release's own checksum file before it is installed.
ARG GITLEAKS_VERSION=8.30.1
ARG TARGETARCH
WORKDIR /tmp/gitleaks
RUN case "$TARGETARCH" in \
        arm64) arch=arm64 ;; \
        amd64) arch=x64 ;; \
        *) echo "no gitleaks build for $TARGETARCH" >&2; exit 1 ;; \
    esac \
    && url="https://github.com/gitleaks/gitleaks/releases/download/v${GITLEAKS_VERSION}" \
    && tarball="gitleaks_${GITLEAKS_VERSION}_linux_${arch}.tar.gz" \
    && curl -fsSLO "$url/$tarball" \
    && curl -fsSLO "$url/gitleaks_${GITLEAKS_VERSION}_checksums.txt" \
    && grep " ${tarball}\$" "gitleaks_${GITLEAKS_VERSION}_checksums.txt" | sha256sum -c - \
    && tar -xzf "$tarball" gitleaks \
    && install -m 0755 gitleaks /usr/local/bin/gitleaks
WORKDIR /

# moor's own MCP server, built from this repo's mcp/ crate — the agent's
# only route to keel (see docs/decisions/0008-agent-session-protocol.md).
# Built from source here, not fetched: it ships with the moor that drives
# it, so the tool surface in the image can never lag the protocol the CLI
# expects. This COPY is why the base image's build context is the repo
# root — see images/build.sh.
COPY mcp /build/mcp
RUN cargo install --locked --path /build/mcp --root /usr/local

# --- stage 2: runtime image ---------------------------------------------
FROM debian:bookworm-slim AS runtime

ARG AGENT_UID=10001
ARG AGENT_GID=10001

# dash (Debian's default RUN shell) has no `pipefail` — without it, a
# failed `curl` in the pipe below would still let `bash -` run on empty
# input and exit 0, silently producing a Node-less image instead of a
# failed build.
SHELL ["/bin/bash", "-o", "pipefail", "-c"]

# `upgrade`, not just `install`: debian:bookworm-slim's own base layer can
# be older than Debian's current security snapshot, so packages already
# patched upstream (e.g. libpcre2) sit un-upgraded until something forces
# it. A Trivy image scan in CI (.github/workflows/ci.yml) fails the build
# on any HIGH/CRITICAL CVE that already has a fix available — this is
# what keeps that check passing without hand-tracking individual CVEs.
# Node 20 reached upstream end-of-life in April 2026 — no more security
# patches at all, from Node itself, ever, regardless of what else this
# image does. 22.x is still within its LTS support window.
RUN apt-get update && apt-get upgrade -y && apt-get install -y --no-install-recommends \
    git curl jq ripgrep ca-certificates gnupg \
    && curl -fsSL https://deb.nodesource.com/setup_22.x | bash - \
    && apt-get install -y --no-install-recommends nodejs \
    && rm -rf /var/lib/apt/lists/*

# npm ships its own vendored dependencies (tar, minimatch, glob, ...) —
# the version nodesource's setup script installs is routinely behind on
# CVE fixes for those. Pinned to the npm 11 line (not `@latest`, which
# has already moved to requiring Node >=22.22/24.15 and would break this
# build again the next time npm cuts a major) for a version compatible
# with this Node line that still carries current vendored-dependency
# fixes.
RUN npm install -g npm@11 && npm cache clean --force

# Claude Code CLI. No auth material baked in — an API key/token is
# injected as an env var at `moor up`, never stored in the image.
RUN npm install -g @anthropic-ai/claude-code && npm cache clean --force

COPY --from=keel-build /usr/local/cargo/bin/keel /usr/local/bin/keel
COPY --from=keel-build /usr/local/bin/moor-keel-mcp /usr/local/bin/moor-keel-mcp
COPY --from=keel-build /usr/local/cargo/bin/cargo-audit /usr/local/bin/cargo-audit
COPY --from=keel-build /usr/local/bin/gitleaks /usr/local/bin/gitleaks

# How `claude` finds that server. Read-only, root-owned, outside any
# volume and outside /workspace: the agent can read this file but has no
# way to point it somewhere else, so the tool surface it gets is the one
# moor shipped. `moor ask` passes it explicitly with --mcp-config.
COPY images/base/mcp-config.json /etc/moor/mcp-config.json

RUN groupadd -g "${AGENT_GID}" agent \
    && useradd -l -m -u "${AGENT_UID}" -g "${AGENT_GID}" -s /bin/bash agent \
    && mkdir -p /workspace /home/agent/.cache /home/agent/.claude \
    && chown -R agent:agent /workspace /home/agent/.cache /home/agent/.claude

# /home/agent/.cache and /home/agent/.claude must exist with agent
# ownership *in the image*, not just at container runtime: moor mounts
# named volumes there (never a host path — see docs/ARCHITECTURE.md), and
# Docker only copies a fresh volume's initial ownership from whatever
# already exists at that path in the image. Skip this and the mount point
# gets created root-owned instead, silently breaking every Bash tool call
# Claude Code makes (it needs to write to /home/agent/.claude) under the
# read-only, non-root, cap-drop-ALL rootfs below.

# Default Claude Code to its own "auto" permission mode instead of
# requiring --dangerously-skip-permissions for non-interactive `moor
# run` invocations. Auto mode has Claude assess each tool call for risky
# actions and prompt injection itself, running what it judges lower-risk
# and refusing the rest (verified directly: it created a file with no
# flag needed, and refused `rm -rf /workspace/*` unprompted) — a second,
# independent layer on top of the sandbox's own structural containment
# (no bind mount, cap_drop: ALL, non-root, default-deny egress), not a
# replacement for it. See docs/decisions/0003-claude-code-permissions.md.
# Same volume-population mechanic as above: this has to exist in the
# image *before* the claude-state volume's first mount, or the settings
# file simply isn't there for a fresh project.
COPY --chown=agent:agent images/base/claude-settings.json /home/agent/.claude/settings.json

WORKDIR /workspace
USER agent

# No ENTRYPOINT/CMD that assumes a shell — moor drives every
# invocation explicitly (`docker exec`), so the container's default
# command just idles.
CMD ["sleep", "infinity"]
