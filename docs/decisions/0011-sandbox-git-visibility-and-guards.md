# ADR-0011: sandbox git-state visibility, build guards, and archiving as recorded state

**Status:** Accepted (visibility shipped; guards and archive planned, by design)
**Date:** 2026-10-01

## Context

The sandbox is isolated on purpose — no host bind mount, no SSH, egress
default-deny ([ADR-0001](0001-container-runtime-choice.md),
[ADR-0010](0010-sandbox-git-transport.md)). A real consequence of that
isolation surfaced in use: **you cannot see the git state of the
workspace you are about to build in.** `moor status` shows only
image/up-down; `moor next` shows the *pipeline* (spec/plan/build) state
but nothing about the *repository* underneath it. So nothing stops a
feature build starting on:

- a branch that is not the trunk (the build commits onto the wrong
  branch),
- a dirty working tree (the build starts on top of uncommitted work),
- a detached HEAD (commits land on no branch at all), or
- a branch behind the remote (building stale).

Separately, specs accumulate. A real project (`altron`) reached four-plus
specs — several approved-but-unshipped, one explicitly rejected mid-build
— all still listed by `moor next --all` and all still feeding the
ship/next selection logic. There was no way to retire a hanging spec, so
it stayed as noise, and (as a bug this surfaced) a stale approved spec
could even hijack `moor ship`.

Two gaps, then: no visibility/guards on git state before a build, and no
way to archive a spec that is going nowhere.

## Decision

A feature set, delivered in order of risk — read-only first, behaviour
changes behind it.

### 1. `moor doctor` — read-only git-state visibility (shipped)

`moor doctor [-p <project>]` reports the sandbox repo's state and changes
nothing (no fetch, no checkout):

- current branch, and whether it is the trunk;
- clean vs. a count of uncommitted changes;
- ahead/behind the tracked upstream;
- detached HEAD.

It reads `git status --porcelain=v2 --branch` once in the sandbox and
parses it with a pure function (`parse_porcelain_v2`), so the parsing is
unit-tested without a container. Each concern a build would care about is
spelled out plainly (e.g. "not on master — a build here commits onto this
branch, not master"; "behind — refresh first: moor pull"). Verified live
on `altron`: `on master · 2 uncommitted change(s) · 4 ahead vs
origin/master`.

This is deliberately the first piece: a lens that is safe to ship on its
own, and the thing the guard below is built on.

### 2. Hard gate before build (planned)

A build (`moor go`'s run step) should **refuse**, not merely warn, when
the git state is unsafe — off-trunk, dirty, or detached — because, per
this repo's house rules, "a rule nobody can violate mechanically is a
rule that will be violated." The gate reuses `doctor`'s `GitState`:
`moor doctor` is the advisory lens, the gate is the enforcing version of
the same check. The exact conditions, and the escape hatch for a
deliberate off-trunk build, are to be settled when the gate is built —
this ADR records that it is a *hard* gate, not an advisory one.

### 3. Archive a spec as recorded state (planned)

Retiring a hanging spec is modelled as a **recorded state transition**,
the way approvals already are (`.keel/specs/<slug>/approvals.jsonl`,
schema `keel.approval/1`), rather than by deleting or moving the spec's
folder. Important boundary: `approvals.jsonl` is *keel's* schema, written
by `keel approve` — moor must not forge keel records. So archiving is a
**moor-owned recorded fact**: moor records the archive (who, when, why) in
its own trail, and moor's guided layer (`next`/`ship`/`status`) skips
archived specs when presenting state. keel's own view is untouched; the
spec's artefacts and history stay in place and the archive is reversible.
This keeps the keel/moor boundary that the whole project rests on: keel
owns the pipeline truth, moor owns what it shows and guards.

## What this deliberately does not do

- **`moor doctor` does not change git state.** No fetch, no checkout, no
  stash — it is a lens only. Refreshing is `moor pull`
  ([ADR-0010](0010-sandbox-git-transport.md)); it is deliberately a
  separate, explicit act.
- **Archiving does not delete or rewrite history, and does not write
  keel's records.** A moor-owned archive marker, reversible, with keel's
  artefacts left in place.
- **The guard is not shipped with the lens.** Enforcement changes
  behaviour (it can block a build), so it is separated from the read-only
  visibility that is safe to ship immediately.

## Consequences

- Before starting a feature, `moor doctor` answers "what state is this
  project actually in?" — the question the sandbox's isolation otherwise
  made unanswerable from outside.
- Once the hard gate lands, a build cannot silently start on a dirty tree
  or the wrong branch — closing the class of "two specs' changes
  tangled" problem at its root rather than after the fact.
- Once archive lands, `moor next --all` and the ship/next logic see only
  live specs; hanging ones become recorded, reversible history instead of
  permanent noise.
- `doctor` shares its `GitState` type and parser with the future gate, so
  the lens and the enforcement can never disagree about what "unsafe"
  means.
