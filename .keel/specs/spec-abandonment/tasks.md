---
id: TASKS-0019
slug: spec-abandonment
schema: keel.tasks/1
---

# Tasks

Each task must name the criteria it satisfies, the files it touches, a line
budget and an exit condition. G1 checks all four, and checks that every
criterion in the spec is covered by at least one task.

Add `- depends_on: T-1` where order matters. Tasks with no dependency on
each other form a wave; `keel tasks` shows them.

### T-1 A drop verb records abandonment with a reason
- criteria: AC-1
- files: cli/src/commands/flow_cmd.rs, cli/src/main.rs, cli/src/commands/mod.rs
- budget: 40
- exit: `cargo test --manifest-path cli/Cargo.toml drop_records_abandonment_with_a_reason` exits 0

### T-2 An empty reason is refused
- criteria: AC-2
- files: cli/src/commands/flow_cmd.rs
- budget: 15
- depends_on: T-1
- exit: `cargo test --manifest-path cli/Cargo.toml drop_refuses_an_empty_reason` exits 0

### T-3 Drop is distinct from reject at the boundary
- criteria: AC-3
- files: cli/src/commands/flow_cmd.rs
- budget: 20
- depends_on: T-2
- exit: `cargo test --manifest-path cli/Cargo.toml drop_and_reject_send_distinct_keel_commands` exits 0

### T-4 An abandoned spec is not shown as active
- criteria: AC-4
- files: cli/src/guide.rs
- budget: 20
- depends_on: T-3
- exit: `cargo test --manifest-path cli/Cargo.toml an_abandoned_spec_is_never_the_active_spec` exits 0

### T-5 An abandoned spec reports a terminal step label
- criteria: AC-5
- files: cli/src/guide.rs
- budget: 15
- depends_on: T-4
- exit: `cargo test --manifest-path cli/Cargo.toml step_label_of_an_abandoned_spec_is_abandoned` exits 0

### T-6 Dropping at any pre-merge stage leaves no guided dead-end
- criteria: AC-6
- files: cli/src/commands/flow_cmd.rs
- budget: 20
- depends_on: T-5
- exit: `cargo test --manifest-path cli/Cargo.toml drop_is_accepted_at_every_pre_merge_stage` exits 0

### T-7 keel exposes an abandoned terminal state
- criteria: AC-7
- files: cli/src/guide.rs
- budget: 5
- depends_on: T-6
- exit: a reviewer runs `keel next --json` against a keel build with the companion change and confirms an abandoned spec is not in the open set

### T-8 Abandonment is recorded with who and why
- criteria: AC-8
- files: cli/src/guide.rs
- budget: 5
- depends_on: T-7
- exit: a reviewer confirms an abandon record carries `by` and `note` in the spec's audit trail beside approvals
