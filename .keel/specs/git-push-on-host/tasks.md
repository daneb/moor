---
id: TASKS-0016
slug: git-push-on-host
schema: keel.tasks/1
---

# Tasks

Each task names the criteria it satisfies, the files it touches, a line
budget and an exit condition. G1 checks all four, and checks that every
criterion is covered by at least one task.

Budgets reflect the measured churn of run 2026-10-14-002, raised from the
plan-time estimates after the real implementation landed.

### T-1 Host-side git helper (`githost`) with fail-loud credential check
- criteria: AC-5, AC-6
- files: cli/src/commands/mod.rs, cli/src/proc.rs, cli/src/main.rs
- budget: 90
- exit: `cargo test --manifest-path cli/Cargo.toml --quiet ship_cmd::tests::push_runs_on_host ship_cmd::tests::absent_host_credential_fails_before_commit` exits 0

### T-2 Bundle-out transport: stream the sandbox branch's commits to the host
- criteria: AC-4
- files: cli/src/commands/ship_cmd.rs, cli/src/proc.rs
- budget: 60
- exit: `cargo test --manifest-path cli/Cargo.toml --quiet ship_cmd::tests::commits_leave_via_bundle_stream` exits 0
- depends_on: T-1

### T-3 Ship: commit stays in-sandbox, push moves to the host helper
- criteria: AC-2, AC-3
- files: cli/src/commands/ship_cmd.rs
- budget: 110
- exit: `cargo test --manifest-path cli/Cargo.toml --quiet ship_cmd::tests::commit_runs_in_sandbox && test -z "$(grep -n 'password=\$GITHUB_TOKEN' cli/src/commands/ship_cmd.rs)"` exits 0
- depends_on: T-2

### T-4 Host-side push re-emits the `push` audit entry (ref + commit)
- criteria: AC-7
- files: cli/src/commands/ship_cmd.rs, cli/src/commands/run_cmd.rs
- budget: 40
- exit: `cargo test --manifest-path cli/Cargo.toml --quiet ship_cmd::tests::host_push_is_recorded_with_ref_and_commit` exits 0
- depends_on: T-3

### T-5 `moor new --github`: clone on the host, stream history in
- criteria: AC-8
- files: cli/src/commands/new_cmd.rs
- budget: 130
- exit: `cargo test --manifest-path cli/Cargo.toml --quiet new_cmd::tests::github_clone_uses_host_credential && test -z "$(grep -n 'password=\$GITHUB_TOKEN' cli/src/commands/new_cmd.rs)"` exits 0
- depends_on: T-1

### T-6 `moor pull`: fetch on the host, apply into the sandbox
- criteria: AC-9
- files: cli/src/commands/pull_cmd.rs
- budget: 125
- exit: `cargo test --manifest-path cli/Cargo.toml --quiet pull_cmd::tests::pull_runs_on_host && test -z "$(grep -n 'password=\$GITHUB_TOKEN' cli/src/commands/pull_cmd.rs)"` exits 0
- depends_on: T-1

### T-7 Strip GITHUB_TOKEN from the sandbox env and the secret manifest
- criteria: AC-1, AC-10
- files: cli/src/compose.rs, cli/src/manifest.rs
- budget: 50
- exit: `cargo test --manifest-path cli/Cargo.toml --quiet compose::tests::sandbox_env_has_no_github_token && test -z "$(grep -rn 'password=\$GITHUB_TOKEN' cli/src)"` exits 0
- depends_on: T-3, T-5, T-6

### T-8 ADR-0014 and a green suite
- criteria: AC-11
- files: docs/decisions/0014-git-on-host.md
- budget: 60
- exit: `test -f docs/decisions/0014-git-on-host.md && cargo test --quiet --manifest-path cli/Cargo.toml && cargo test --quiet --manifest-path mcp/Cargo.toml` exits 0
- depends_on: T-7
