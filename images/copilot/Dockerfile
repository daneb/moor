# GitHub Copilot agent layer.
#
# The agentic `copilot` CLI (npm @github/copilot) — NOT `gh copilot
# suggest`/`explain`, which only proposes shell commands and never edits
# files. This is the binary the `copilot` keel driver
# (.keel/drivers/copilot) invokes when a build runs with
# `keel run --driver copilot` inside the sandbox.
#
# WHY A SEPARATE LAYER, not part of moor/base: `@github/copilot` installs
# a ~174MB compiled platform binary (@github/copilot-<platform>) that
# vendors its own copies of npm dependencies — including, at 1.0.89,
# versions of `undici` and `brace-expansion` with fixable HIGH CVEs baked
# into the binary (no npm override/dedupe can reach them; only a newer
# upstream release can). Copilot is a non-default, opt-in agent, so
# baking it into moor/base would make EVERY project carry that binary,
# that CVE surface, and a failing Trivy scan — to enable a driver most
# projects never use. Confining it to this layer means only a project
# that opts into Copilot pays the cost, and moor/base's scan stays green.
#
# No auth material is baked in — a GitHub token is injected as an env var
# at `moor up` (COPILOT_GITHUB_TOKEN > GH_TOKEN > GITHUB_TOKEN), never
# stored in the image. Same rule as Claude Code in the base image; see
# docs/decisions/0002-claude-code-authentication.md.
#
# BASE_IMAGE lets this stack on top of a language layer, so a project can
# have both its toolchain and Copilot, e.g.:
#   docker build -t moor/copilot-node:latest \
#     --build-arg BASE_IMAGE=moor/node:latest ./copilot
ARG BASE_IMAGE=moor/base:latest
FROM ${BASE_IMAGE}

USER root
RUN npm install -g @github/copilot && npm cache clean --force

# The Copilot CLI writes its own state under ~/.copilot (logs, session
# bookkeeping, config). The sandbox's rootfs is read-only, so moor mounts
# a named volume there — and Docker copies a fresh volume's initial
# ownership from whatever exists at that path in the image. This dir must
# therefore exist, owned by `agent`, IN THE IMAGE, or the mount point is
# created root-owned and copilot dies (silently: it can't write logs, so
# it produces no diagnostic either). Exactly the same mechanic as
# /home/agent/.claude in the base image — see its Dockerfile comment and
# docs/IMAGES.md. Verified: without this, `copilot -p` exits non-zero with
# no output on the read-only rootfs.
RUN mkdir -p /home/agent/.copilot \
    && chown -R agent:agent /home/agent/.copilot
USER agent
