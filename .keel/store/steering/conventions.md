---
id: CONV-0001
scope: repo
owner: human
verified_at: 2026-09-17
---

# Conventions

House rules that apply to every change in this repository. Keep this list short
and specific: a rule nobody can violate mechanically is a rule that will be
violated. Where a rule can be checked by a command, say so — in Phase 3 those
become gate checks and stop costing context.

## Working agreement

- Match the surrounding code. Naming, comment density and idiom are local
  conventions, not global ones.
- Change the smallest surface that solves the problem. If a fix needs a wider
  blast radius, say so before making it, not after.
- A test that mocks away the behaviour under test is worse than no test.

## Naming: keel is internal

moor's users never type `keel`; everything is a `moor` command. keel
(github.com/daneb/keel) is the pipeline engine moor installs inside each
sandbox at a pinned version (`KEEL_VERSION` in `images/base/Dockerfile`).

- To drive a project's pipeline, use `moor next`, `go`, `approve`, `reject`,
  `ship`. `moor pipeline <args>` is the hidden escape hatch to the sandbox's
  keel. When a user says "next step" or "approve", they mean the `moor` command.
- `moor-mcp` (`mcp/`) is the agent's route to checks inside a sandbox
  (`check`, `next`). It cannot approve, by design.
- A bare `keel` on this machine's PATH works on THIS repo's `.keel/` (specs,
  store, generating `CLAUDE.md`/`AGENTS.md`): development tooling only. Run
  `keel store render` after editing `.keel/store/`. It is a different copy
  from the sandbox's.

## Rules

_Add rules as you find yourself repeating them. One line each, imperative mood._
