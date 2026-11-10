---
id: TASKS-0023
slug: sandbox-trunk-sync
schema: keel.tasks/1
---

# Tasks

Each task must name the criteria it satisfies, the files it touches, a line
budget and an exit condition. G1 checks all four, and checks that every
criterion in the spec is covered by at least one task.

Add `- depends_on: T-1` where order matters. Tasks with no dependency on
each other form a wave; `keel tasks` shows them.

### T-1 promote the reusable host-fetch helpers in pull_cmd
- criteria: AC-1
- files: cli/src/commands/pull_cmd.rs
- budget: 20
- exit: `cargo build --manifest-path cli/Cargo.toml` succeeds with `https_github_url`, `fetch_argv` and the host-fetch helper reachable as `pub(crate)` from `sync_cmd`; `moor pull`'s own tests still pass

### T-2 scaffold sync_cmd and wire Command::Sync
- criteria: AC-1
- files: cli/src/commands/sync_cmd.rs, cli/src/commands/mod.rs, cli/src/main.rs
- budget: 70
- exit: test sync_cmd::tests::fetch_runs_on_host
- depends_on: T-1

### T-3 update trunk as a ref without touching the active worktree
- criteria: AC-2, AC-3
- files: cli/src/commands/sync_cmd.rs
- budget: 60
- exit: test sync_cmd::tests::updates_trunk_ref_without_touching_worktree passes, AND `git grep -nE 'checkout|switch|merge|reset|restore|stash' cli/src/commands/sync_cmd.rs` prints no line outside the trunk-is-HEAD fast-forward branch (the AC-3 reviewer check)
- depends_on: T-2

### T-4 fast-forward the checked-out trunk only with 0 changes
- criteria: AC-4
- files: cli/src/commands/sync_cmd.rs
- budget: 45
- exit: test sync_cmd::tests::dirty_trunk_refuses_fast_forward
- depends_on: T-3

### T-5 refuse non-github origins and absent host credential
- criteria: AC-5, AC-6
- files: cli/src/commands/sync_cmd.rs
- budget: 40
- exit: tests sync_cmd::tests::refuses_non_github_origin and sync_cmd::tests::missing_credential_blocks_before_fetch both pass
- depends_on: T-4

### T-6 report the trunk ref movement sync made
- criteria: AC-9
- files: cli/src/commands/sync_cmd.rs
- budget: 35
- exit: test sync_cmd::tests::reports_commits_advanced
- depends_on: T-5

### T-7 doctor fetches the trunk ref on the host before reporting behind
- criteria: AC-7
- files: cli/src/commands/doctor_cmd.rs, cli/src/commands/mod.rs
- budget: 50
- exit: test doctor_cmd::tests::behind_count_reflects_a_host_fetch
- depends_on: T-2

### T-8 doctor recommends sync, not pull, when behind
- criteria: AC-8
- files: cli/src/commands/doctor_cmd.rs
- budget: 20
- exit: test doctor_cmd::tests::behind_advice_names_sync_not_pull
- depends_on: T-7

### T-9 record the decision as ADR-0017
- criteria: AC-10
- files: docs/decisions/0017-sandbox-trunk-sync.md
- budget: 60
- exit: `ls docs/decisions/0017-*.md` exits 0
- depends_on: T-3

