---
id: SPEC-0023
slug: sandbox-trunk-sync
schema: keel.spec/1
status: approved
scope:
- cli/src/commands/sync_cmd.rs
- cli/src/commands/pull_cmd.rs
- cli/src/commands/doctor_cmd.rs
- cli/src/commands/mod.rs
- cli/src/main.rs
- docs/decisions/0017-*.md
budget:
  criteria: 10
  lines: 560
verified_at: 2026-11-10
---

# Keep the sandbox trunk in sync with origin without disturbing the active build

## Context

In moor the keel pipeline's state (`/workspace/.keel/specs/<slug>/` and
`/workspace/.keel/runs/<id>/`) lives in the same git working tree as the
project code, often on a feature branch with uncommitted in-progress
artifacts. `moor pull` (`cli/src/commands/pull_cmd.rs`) only ever refreshes
the *currently checked-out branch* with `git merge --ff-only FETCH_HEAD`.
Run mid-feature it either refuses (dirty tree) or rewrites the working
tree that holds the pipeline's state — the "loses its place" drift the
operator hit.

Separately, `moor doctor` (`cli/src/commands/doctor_cmd.rs`) reports a
"behind" count read from `git status --porcelain=v2 --branch`, which
reflects only the last fetch. Without a host-side fetch that number is
stale: it can read 0 while origin has moved, so drift goes unseen until it
is painful, and doctor's remedy line points at the very `moor pull` that
disturbs an active build.

The fix is a new `moor sync` that keeps trunk current as a *ref update*
(never a checkout of the active branch), plus a truthful behind-count in
doctor driven by a host-side fetch. Credentialed fetch stays on the host
and crosses in only as a `git bundle` stream (ADR-0014); no credential or
`ssh` ever enters the sandbox.

## Acceptance criteria

### AC-1 sync fetches trunk on the host, never in the sandbox

WHEN `moor sync` fetches trunk from the GitHub remote THE SYSTEM SHALL run
the `git fetch` as a host process and never as a `docker exec` into the
sandbox.

oracle: test sync_cmd::tests::fetch_runs_on_host

### AC-2 trunk is updated as a ref, leaving the active branch untouched

WHILE the sandbox has a branch other than trunk checked out THE SYSTEM
SHALL advance the local trunk ref to the fetched origin trunk commit
without changing HEAD, the index, or any working-tree file.

oracle: test sync_cmd::tests::updates_trunk_ref_without_touching_worktree

### AC-3 the pipeline's working-tree state is preserved across a sync

WHEN `moor sync` completes with a feature branch checked out THE SYSTEM
SHALL leave every tracked and untracked path under `.keel/specs/` and
`.keel/runs/` byte-for-byte unchanged from before the sync.

oracle: human A reviewer confirms the sync path only calls `git
update-ref`/`git fetch` into the trunk ref and issues no `checkout`,
`switch`, `merge`, `reset`, `restore`, or `stash` against the working
tree.

### AC-4 syncing the checked-out trunk fast-forwards only with 0 changes

IF trunk itself is the checked-out branch and `git status --porcelain`
reports 0 entries THEN THE SYSTEM SHALL fast-forward it to origin trunk
with `--ff-only`, and IF `git status --porcelain` reports 1 or more
entries THEN THE SYSTEM SHALL change no git ref and exit non-zero naming
those entries.

oracle: test sync_cmd::tests::dirty_trunk_refuses_fast_forward

### AC-5 sync refuses non-github remotes rather than rewriting them

IF the sandbox `origin` is not a github.com remote THEN THE SYSTEM SHALL
exit non-zero and change no git ref.

oracle: test sync_cmd::tests::refuses_non_github_origin

### AC-6 a missing host credential stops sync before any ref changes

IF no host GitHub credential resolves THEN THE SYSTEM SHALL exit non-zero
with the text `gh auth login` before any fetch or ref update runs.

oracle: test sync_cmd::tests::missing_credential_blocks_before_fetch

### AC-7 doctor fetches before reporting a behind count

WHEN `moor doctor` reports commits behind trunk THE SYSTEM SHALL first
update the local trunk tracking ref from origin on the host, so the count
reflects origin's current tip and not only the last fetch.

oracle: test doctor_cmd::tests::behind_count_reflects_a_host_fetch

### AC-8 doctor recommends sync, not pull, when behind

WHEN `moor doctor` reports the sandbox is 1 or more commits behind trunk
THE SYSTEM SHALL print a next-step line naming `moor sync` and SHALL NOT
name `moor pull`.

oracle: test doctor_cmd::tests::behind_advice_names_sync_not_pull

### AC-9 sync reports the trunk ref movement it made

WHEN `moor sync` advances the trunk ref THE SYSTEM SHALL print the trunk
name and the number of commits it moved, and WHEN trunk is already current
THE SYSTEM SHALL print that it is up to date and move no ref.

oracle: test sync_cmd::tests::reports_commits_advanced

### AC-10 the sync behaviour is recorded as a decision

THE SYSTEM SHALL add an ADR under `docs/decisions/0017-*.md` stating that
trunk is synced as a host-fetched ref update that never touches the active
branch's working tree, extending ADR-0014.

oracle: cmd `ls docs/decisions/0017-*.md` exit 0

## Out of scope

- Rebasing or merging a feature branch onto freshly synced trunk — that
  stays a separate, explicit, operator-invoked step (the `.keel/`-stashed
  catch-up), not part of `moor sync`.
- Making the build gate (G2) *refuse* on a stale/behind state; AC-7/AC-8
  make the count truthful and the advice safe, but the hard refusal
  remains the separately planned gate change.
- Any change to `moor ship`, credential handling, or the egress allowlist.
- Auto-running sync on a timer or on `moor up`.
