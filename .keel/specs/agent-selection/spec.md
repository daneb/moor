---
id: SPEC-0010
slug: agent-selection
schema: keel.spec/1
status: approved
scope:
- cli/src/commands/import_cmd.rs
- cli/src/commands/new_cmd.rs
- cli/src/manifest.rs
- cli/src/main.rs
- images/**
budget:
  criteria: 8
  lines: 200
verified_at: 2026-10-01
---

# Select the agent/harness up front at import/new

## Context

Today a project's image is chosen only by *language*: `import`'s
`detect_image` picks `moor/base|node|rust|python` from markers like
`Cargo.toml`/`package.json`, and the *agent* (Claude vs. GitHub Copilot)
is a separate axis that is not captured at create time at all. Claude ships
in every image; Copilot ships only in the opt-in `moor/copilot` layer
(see `images/copilot/Dockerfile` and ADR-0010-adjacent work). So a project
that should use Copilot (e.g. a Python service) comes in as
`moor/python:latest` and must be retrofitted by hand — build a combined
`moor/copilot-python` image, add Copilot secrets to the manifest, and set
keel's default driver to copilot. That retrofit was done manually for the
`altron` project and is exactly the friction this change removes.

The agent should be declarable **up front**, with moor composing it with
the detected language and preparing everything the agent needs.

## Acceptance criteria

### AC-1 An --agent flag selects the agent at import and new

WHEN the operator runs `moor import` or `moor new` with `--agent copilot`
THE SYSTEM SHALL record `copilot` as the project's agent in its manifest.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml agent_flag_sets_manifest_agent` exit 0

### AC-2 The agent defaults to claude when the flag is omitted

WHEN the operator runs `moor import` or `moor new` without `--agent`
THE SYSTEM SHALL record `claude` as the project's agent.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml agent_defaults_to_claude` exit 0

### AC-3 Copilot composes with the detected language into the right image

WHEN the agent is `copilot` and the detected language image is
`moor/<lang>:latest` THE SYSTEM SHALL resolve the project image to the
corresponding `moor/copilot-<lang>:latest` (or `moor/copilot:latest` when
the language is base).

oracle: cmd `cargo test --manifest-path cli/Cargo.toml copilot_composes_with_language` exit 0

### AC-4 The claude agent leaves image selection unchanged

WHEN the agent is `claude` THE SYSTEM SHALL resolve the project image to
the plain language image (`moor/base|node|rust|python:latest`), unchanged
from today's behaviour.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml claude_leaves_image_unchanged` exit 0

### AC-5 An unknown agent value is rejected

WHEN the operator passes an `--agent` value other than `claude` or
`copilot` THE SYSTEM SHALL reject it rather than silently defaulting.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml unknown_agent_is_rejected` exit 0

## Out of scope

- Building the combined `moor/copilot-<lang>` images in `images/build.sh`
  up front, and the on-demand build/driver-default/secret wiring at
  runtime — those are real but are a follow-on; this spec establishes the
  agent axis (flag, manifest field, image resolution) that they build on.
- Any change to moor's own agent commands (`moor ask`/`studio`), which
  remain Claude-specific.
- Changing how `--image` works when passed explicitly: an explicit
  `--image` still wins over agent/language resolution.
