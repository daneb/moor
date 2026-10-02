---
id: SPEC-0015
slug: keel-evidence-retention
schema: keel.spec/1
status: approved
scope:
- .gitignore
- Makefile
- docs/**
budget:
  criteria: 7
  lines: 200
verified_at: 2026-10-02
---

# Commit verified bundles, not raw run evidence

## Context

`.keel/` is 13M, of which `.keel/runs/` is **12M across 39 runs and 464
files**. The bulk is not the gate records or the trajectories — it is
`evidence/diff.patch`, and it grows **quadratically**:

| run | diff.patch | `.keel/` paths inside it |
| --- | --- | --- |
| 2026-10-01-004 | 63K | 17 |
| 2026-10-01-006 | 333K | 46 |
| 2026-10-01-009 | 508K | 39 |
| 2026-10-01-00a | 1051K | 54 |
| 2026-10-01-00b | **2150K** | 68 |

The cause: each run captures the working-tree diff, and because prior
runs' evidence is itself tracked, every run's diff contains the previous
runs' evidence. Run N records runs 1..N-1. The 2.1M file is mostly other
runs' evidence rather than the change under test.

Compressing the directories to `.tgz` would shrink today's copy and leave
the doubling in place. The durable fix is to stop tracking the thing that
feeds it.

keel already defines the artefact that should be kept. `keel cover` asks
"is this tree covered by a **committed, verified bundle** of a passing
run?", and `keel export` writes one: for run 2026-10-02-003 the bundle is
**232K against the 1.1M raw run**, and `keel export --verify` reports it
`intact — 27 member(s)`. The bundle is self-verifying, portable, and
carries the chain slice for its run.

This repository has that inverted: `.gitignore` ignores `.keel/bundles/`
and commits `.keel/runs/`. Correcting it also removes a second, separate
problem already hit in practice — `.keel/chain.jsonl` is append-only and
hash-linked, so two feature branches always conflict in it and resolving
the conflict necessarily discards one side's entries. Evidence that
travels in bundles does not conflict.

What stays tracked either way: `.keel/specs/` (spec, plan, tasks,
approvals — small, and the human record of what was decided) and
`.keel/store/` with its projections.

The ignore oracles below pass `--no-index`. `git check-ignore` consults
the index by default and reports an already-tracked path as *not*
ignored, whatever the rules say — so without it these criteria could only
pass after the migration, which is deliberately outside this change.
`--no-index` asks the question the criteria are actually about: are the
rules right?

## Acceptance criteria

### AC-1 Raw run evidence is not tracked

WHEN the repository's ignore rules are evaluated THE SYSTEM SHALL ignore
`.keel/runs/`, so no run's evidence enters a commit and no run's diff can
contain another's.

oracle: cmd `git check-ignore -q --no-index .keel/runs` exit 0

### AC-2 Verified bundles are tracked

WHEN the repository's ignore rules are evaluated THE SYSTEM SHALL NOT
ignore `.keel/bundles/`, so an exported bundle is committable.

oracle: cmd `! git check-ignore -q --no-index .keel/bundles` exit 0

### AC-3 The evidence chain is not tracked

WHEN the repository's ignore rules are evaluated THE SYSTEM SHALL ignore
`.keel/chain.jsonl`, because it is append-only and hash-linked and so
cannot be merged between branches without discarding entries; it travels
inside bundles instead.

oracle: cmd `git check-ignore -q --no-index .keel/chain.jsonl` exit 0

### AC-4 A bundle verifies on its own

WHEN a bundle written by `keel export` is verified THE SYSTEM SHALL report
it intact, so a committed bundle stands as the run's evidence with the raw
run absent.

oracle: cmd `keel export --verify "$(ls -1t .keel/bundles/*.tar.gz | head -1)"` exit 0

### AC-5 One command produces the bundle to commit

WHEN the operator runs the documented make target THE SYSTEM SHALL write a
verified bundle for the latest run into `.keel/bundles/`.

oracle: cmd `grep -qE '^evidence:' Makefile` exit 0

### AC-6 The retention model is documented

WHEN a reader looks for what evidence is kept and why THE SYSTEM SHALL
state it in the docs, including that raw runs are local and bundles are
the committed artefact.

oracle: cmd `grep -rqi 'bundle' docs/EVIDENCE.md` exit 0

### AC-7 Existing CLI tests stay green

THE SYSTEM SHALL leave the existing CLI test suite passing.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml` exit 0

## Out of scope

- **The one-time removal of the 464 already-tracked run files.** That is a
  mechanical `git rm -r --cached .keel/runs` whose diff is tens of
  thousands of deleted lines of generated evidence; gating it would
  measure deletion volume rather than this decision. It is documented as
  the migration step this spec enables, and run once after it merges.
- Changing what keel puts *into* a run's evidence. Excluding `.keel/` from
  the captured diff would fix the doubling at its source, but that is
  keel's own behaviour, not moor's — worth raising there, and noted here
  as the upstream half of the problem.
- Pruning or compressing bundles; they are small and few.
- moor's own per-project audit trail under `~/.moor/projects/*/audit/`,
  which is host-side and outside this repository.
