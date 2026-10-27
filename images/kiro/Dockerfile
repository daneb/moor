# Kiro agent layer.
#
# The agentic `kiro-cli` — AWS's coding agent CLI (descended from the
# Amazon Q / CodeWhisperer line). moor's turn protocol already speaks it:
# `session::profile(Agent::Kiro)` invokes `kiro-cli chat --no-interactive`.
# This layer is what puts that binary into a project's sandbox so the turn
# can actually run; without it a kiro project lands on a base/language
# image with no `kiro-cli`, and every turn fails with "kiro-cli: not
# found".
#
# WHY A SEPARATE LAYER, not part of moor/base: Kiro is a non-default,
# opt-in agent. Baking its binary into moor/base would make EVERY project
# carry it (and its CVE surface, and the egress hosts it reaches) to
# enable an agent most projects never select. Same reasoning as the
# Copilot layer — see images/copilot/Dockerfile. Only a project that opts
# into Kiro (`--agent kiro`) pays the cost, and moor/base stays lean.
#
# No auth material is baked in. Kiro's enterprise auth is IAM Identity
# Center via the device flow: the operator runs `kiro-cli login` ON THE
# HOST, and moor reads the resulting cached SSO token fresh at each `moor
# up`, injecting it as an env var (AWS_BEARER_TOKEN_BEDROCK) — never
# stored in the image. Same rule as Claude Code in the base image; see
# docs/decisions/0002-claude-code-authentication.md.
#
# BASE_IMAGE lets this stack on top of a language layer, so a project can
# have both its toolchain and Kiro, e.g.:
#   docker build -t moor/kiro-node:latest \
#     --build-arg BASE_IMAGE=moor/node:latest ./kiro
ARG BASE_IMAGE=moor/base:latest
FROM ${BASE_IMAGE}

# Installed as the non-root `agent` user, exactly as `kiro-cli` installs
# itself on a developer's machine: the official installer drops the binary
# under the user's home (~/.local/bin). Done here, in the image, so the
# read-only rootfs at runtime never has to be written to. `unzip` is the
# one thing the installer needs that moor/base doesn't already carry.
USER root
RUN apt-get update \
    && apt-get install -y --no-install-recommends unzip \
    && rm -rf /var/lib/apt/lists/*
USER agent
RUN curl -fsSL https://cli.kiro.dev/install | bash

# The installer puts kiro-cli under ~/.local/bin; make sure every later
# `docker exec` (moor runs non-login shells) finds it on PATH.
ENV PATH="/home/agent/.local/bin:${PATH}"

# kiro-cli writes its own state (session bookkeeping, config, its local
# token store) under these dirs. The sandbox's rootfs is read-only, so
# moor mounts a named volume at ~/.aws (where the injected SSO credential
# and kiro-cli's own cache live); the dir must exist, owned by `agent`, IN
# THE IMAGE, or Docker creates the mount point root-owned and kiro-cli
# dies on the read-only rootfs — the same mechanic as /home/agent/.claude
# and /home/agent/.copilot. The others sit on the already-agent-owned home
# and need no volume.
RUN mkdir -p /home/agent/.aws /home/agent/.local/share/kiro-cli /home/agent/.kiro \
    && chown -R agent:agent /home/agent/.aws /home/agent/.local /home/agent/.kiro
