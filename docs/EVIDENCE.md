# What evidence is kept, and why

keel writes a lot about every run. Not all of it belongs in a commit.
The short version:

| path | committed? | what it is |
| --- | --- | --- |
| `.keel/specs/` | **yes** | spec, plan, tasks, approvals — the record of what was *decided* |
| `.keel/store/` + projections | **yes** | the knowledge store and the files agents read |
| `.keel/bundles/` | **yes** | a verified, portable bundle per run — the evidence that travels |
| `.keel/runs/` | no | raw run evidence: local working state |
| `.keel/chain.jsonl` | no | append-only hash-linked log; ships inside bundles |

## Why raw runs are not committed

Two measured reasons.

**It grows quadratically.** A run records the working-tree diff. While
`.keel/runs/` is itself tracked, every run's diff therefore contains the
previous runs' evidence — so each one is bigger than the last. Measured
across one afternoon:

| run | `evidence/diff.patch` | `.keel/` paths inside it |
| --- | --- | --- |
| 2026-10-01-004 | 63K | 17 |
| 2026-10-01-006 | 333K | 46 |
| 2026-10-01-009 | 508K | 39 |
| 2026-10-01-00a | 1051K | 54 |
| 2026-10-01-00b | **2150K** | 68 |

That 2.1M file is mostly *other runs' evidence*, not the change it was
recording.

**It was most of the repository's keel footprint.** 39 runs came to 12M
of a 13M `.keel/`, across 464 files.

Compressing the directories to `.tgz` would shrink one copy and leave the
doubling in place. Not committing the thing that feeds it fixes the cause.

## Why the bundle is the thing that is committed

keel already defines this. `keel cover` asks whether the tree is
"covered by a **committed, verified bundle** of a passing run" — the
bundle is the artefact, not the run directory. It is also:

- **smaller**: 232K against the 1.1M raw run it came from;
- **self-verifying**: `keel export --verify <bundle>` reports
  `intact — N member(s), run …, spec …`, so a reviewer needs nothing but
  the file;
- **self-contained**: it carries its own slice of the evidence chain.

Write one with:

```bash
make evidence          # keel export, then keel export --verify on the result
git add .keel/bundles/keel-<run>.tar.gz
```

The target fails rather than leaving an unverified bundle to be committed.

## Why the chain is not committed either

`.keel/chain.jsonl` is append-only and hash-linked: each entry chains to
its predecessor. Two feature branches both append from the same ancestor,
so they **always** conflict there, and resolving the conflict necessarily
discards one side's entries — there is no merge that keeps both and still
verifies. That happened three times in one week of parallel work on this
repository.

Keeping it local and shipping it inside bundles means the chain is
verifiable where it matters (`keel export --verify`, `keel chain verify`
locally) and never merged.

What this gives up, stated plainly: there is no single committed chain
covering the whole repository's history. The per-run bundles each carry
their own verified slice.

## Migration (one time)

Already-tracked runs do not disappear when the ignore rule lands — git
keeps tracking what it already tracks. After this change merges:

```bash
make evidence                                     # bundle the latest run first
git add .keel/bundles/                            # commit the evidence that matters
git rm -r --cached .keel/runs .keel/chain.jsonl   # stop tracking the rest
git commit -m "Stop tracking raw keel run evidence (see docs/EVIDENCE.md)"
```

The files stay on disk; only the tracking stops. Do this *after*
exporting a bundle, so the evidence worth keeping is committed before the
raw runs are untracked.

## The upstream half

The doubling has a cause this repository cannot fix: keel captures the
working-tree diff without excluding `.keel/` itself, so its own evidence
is part of the evidence. Not committing runs removes the compounding, but
a single run's `diff.patch` can still include whatever `.keel/` churn is
in the tree at the time. Excluding `.keel/` from the captured diff
belongs in keel, and is worth raising there rather than worked around
here.
