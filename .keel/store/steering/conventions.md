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

## Naming: which "keel" is which

Four different things share the name. Do not run a bare `keel ...` unless you
mean the first one.

- `keel` on this machine's PATH works on THIS repository's own `.keel/`
  (specs, store, `CLAUDE.md`/`AGENTS.md` generation). It is a development tool
  for moor itself. Use `keel store render` after editing `.keel/store/`.
- The keel inside a moor sandbox is a separate copy at a pinned version
  (`KEEL_VERSION` in `images/base/Dockerfile`), working on that project's
  workspace volume, never on this checkout. Reach it only through moor:
  `moor next`, `go`, `approve`, `reject`, `ship`. `moor keel <args>` is the
  hidden escape hatch.
- `moor-keel-mcp` (`mcp/`) is the agent's route to `keel gate` and `keel next`
  inside a sandbox. It cannot approve, by design.
- keel the project is github.com/daneb/keel. moor depends on it, never edits it.

When a user says "next step", "approve" or "run the spec", they mean the
`moor` command, not `keel`.

## Rules

_Add rules as you find yourself repeating them. One line each, imperative mood._
