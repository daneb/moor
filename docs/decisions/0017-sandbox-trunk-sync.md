# ADR-0017: trunk is synced as a host-fetched ref update, never a checkout

**Status:** Accepted
**Date:** 2026-11-10

## Context

A sandbox's keel pipeline state (`.keel/specs/<slug>/`, `.keel/runs/<id>/`)
lives in the same git working tree as the project code, often on a
feature branch with uncommitted in-progress artifacts. `moor pull`
(ADR-0014) only ever refreshes the *currently checked-out branch* with
`git merge --ff-only FETCH_HEAD`. Run mid-feature it either refuses
(dirty tree) or rewrites the very working tree that holds the pipeline's
state.

Separately, `moor doctor`'s "behind" count came from `git status
--porcelain=v2 --branch`, which reflects only the last fetch. Without a
host-side fetch that number is stale — it can read 0 while origin has
moved — and doctor's remedy line pointed at `moor pull`, which disturbs
an active build.

## Decision

`moor sync` keeps trunk current as a *ref update*, never a checkout of
the active branch, extending ADR-0014's rule that every credentialed git
operation runs on the host:

1. The host fetches trunk — discovered via `git ls-remote --symref
   <origin> HEAD`, authenticated with `gh auth token` — and bundles it,
   exactly as `moor pull` does. The fetch is always a plain host `git`
   invocation, never a `docker exec` into the sandbox.
2. The bundle crosses in as a stream (`docker exec -i ... cat >`) and is
   landed with a plain fetch refspec: `<trunk>:refs/heads/<trunk>`. Git
   itself refuses that refspec when `<trunk>` is the currently
   checked-out branch, so a feature branch's HEAD, index, and working
   tree — including every path under `.keel/` — are never touched.
3. If trunk itself is checked out, sync instead fast-forwards it
   (`git merge --ff-only FETCH_HEAD`), and only when `git status
   --porcelain` reports zero entries; a dirty trunk moves no ref and
   names what's in the way.
4. A non-github.com `origin` or a missing host credential stops sync
   before any fetch or ref update runs.
5. `moor doctor` runs the same mechanism best-effort, before reporting,
   but lands the bundle at trunk's *remote-tracking* ref
   (`refs/remotes/origin/<trunk>`) instead of the branch ref — always
   safe regardless of what's checked out, and enough to make the ab line
   `git status` reports truthful. Its "behind" advice now names `moor
   sync`, not `moor pull`.

## What this deliberately does not do

- Does not rebase or merge a feature branch onto freshly synced trunk —
  that stays a separate, explicit, operator-invoked step.
- Does not make the build gate refuse on a stale/behind state.
- Does not change `moor ship`, credential handling, or the egress
  allowlist.

## Consequences

- An active build's working tree is never disturbed by a sync or by
  doctor's refresh — only `update-ref`-equivalent refspec fetches and,
  when trunk is checked out, an already-gated fast-forward ever run.
- `moor sync` depends on `gh auth login` having been run on the host,
  the same dependency `moor pull` and `moor ship` already have.
