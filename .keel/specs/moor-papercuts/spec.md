---
id: SPEC-0007
slug: moor-papercuts
schema: keel.spec/1
status: approved
scope:
- cli/src/compose.rs
- cli/templates/project.compose.yml.tmpl
- cli/src/commands/mod.rs
- images/base/Dockerfile
- images/build.sh
- tests/e2e.sh
- docs/**
budget:
  criteria: 6
  lines: 250
verified_at: 2026-09-25
---

# Moor papercuts: the approver's identity, and keel pinned in the image

## Context

Two problems found while taking keel runs through Moor end to end:

- **In a new project, approvals record `unknown` as the approver.**
  `sync_git_identity` runs `git config user.name <host value>` inside the
  sandbox. With no `--global`, git writes to the repository in the current
  directory, and a new project's `/workspace` isn't a repository yet, so it
  fails with "not in a git directory". `--global` fails too, because the home
  directory is on the read-only root filesystem. Both errors are discarded.
  keel then finds no `user.name`, and every approval in the chain says
  `unknown`.
- **Rebuilding the image can keep an old keel.** The Dockerfile's `cargo
  install keel-harness --locked` layer has no version in it, so Docker reuses
  the cached layer after a keel release. Twice, a rebuild quietly shipped the
  previous keel.

git reads configuration from `GIT_CONFIG_COUNT`, `GIT_CONFIG_KEY_n` and
`GIT_CONFIG_VALUE_n` environment variables without writing any file, and every
git command honours them in any directory. That's the fix for the first
problem. For the second, a pinned `KEEL_VERSION` build argument makes the keel
version part of the layer's cache key.

## Acceptance criteria

### AC-1 The host's identity reaches the sandbox as environment

WHEN Moor renders a project's compose file and the host's git config sets
`user.name` and `user.email` THE SYSTEM SHALL give the sandbox service
`GIT_CONFIG_COUNT`, `GIT_CONFIG_KEY_n` and `GIT_CONFIG_VALUE_n` entries that
set both, quoted for compose, with any `$` escaped.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml compose::tests::host_git_identity_is_rendered_as_git_config_env -- --exact 2>&1 | grep -q '1 passed'` exit 0

### AC-2 No identity, no entries

IF the host's git config sets neither `user.name` nor `user.email` THEN THE
SYSTEM SHALL render no `GIT_CONFIG_*` entries.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml compose::tests::no_host_identity_renders_no_git_config_env -- --exact 2>&1 | grep -q '1 passed'` exit 0

### AC-3 An approval in a new project names the approver

WHEN the end-to-end suite approves a spec inside a newly created project's
sandbox THE SYSTEM SHALL record the host's `user.name` as the approver, not
`unknown`.

oracle: cmd `bash tests/e2e.sh` exit 0

### AC-4 keel's version is pinned in the image build

THE SYSTEM SHALL install keel in the base image with `cargo install
keel-harness --version "$KEEL_VERSION" --locked`, where `KEEL_VERSION` is a
build argument whose default names one exact version.

oracle: cmd `grep -Eq '^ARG KEEL_VERSION=[0-9]+\.[0-9]+\.[0-9]+$' images/base/Dockerfile && grep -q 'cargo install keel-harness --version "\$KEEL_VERSION" --locked' images/base/Dockerfile` exit 0

## Out of scope

- gVisor. That's a separate spike: `runsc` in OrbStack, and a
  `kernel.isolated` posture property.
- Updating keel driver scripts already copied into repositories.
