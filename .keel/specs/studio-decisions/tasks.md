---
id: TASKS-0013
slug: studio-decisions
schema: keel.tasks/1
---

# Tasks

Each task must name the criteria it satisfies, the files it touches, a line
budget and an exit condition. G1 checks all four, and checks that every
criterion in the spec is covered by at least one task.

Add `- depends_on: T-1` where order matters. Tasks with no dependency on
each other form a wave; `keel tasks` shows them.

### T-1 A key begins a rejection and takes a reason
- criteria: AC-1
- files: scope
- budget: 65
- exit: `cargo test --manifest-path cli/Cargo.toml reject_key_collects_a_reason` exits 0

### T-2 An empty reason records nothing
- criteria: AC-2
- files: scope
- budget: 65
- exit: `cargo test --manifest-path cli/Cargo.toml empty_reason_records_no_rejection` exits 0

### T-3 Rejection is confirmed, not a single keypress
- criteria: AC-3
- files: scope
- budget: 65
- exit: `cargo test --manifest-path cli/Cargo.toml rejection_needs_a_second_key` exits 0

### T-4 Escape abandons a rejection
- criteria: AC-4
- files: scope
- budget: 65
- exit: `cargo test --manifest-path cli/Cargo.toml escape_abandons_a_rejection` exits 0

### T-5 The recorded rejection carries keel's own command and the reason
- criteria: AC-5
- files: scope
- budget: 65
- exit: `cargo test --manifest-path cli/Cargo.toml rejection_uses_keels_own_command_and_note` exits 0

### T-6 A failing gate's checks are shown with the project
- criteria: AC-6
- files: scope
- budget: 65
- exit: `cargo test --manifest-path cli/Cargo.toml failing_checks_are_shown` exits 0

### T-7 Gate output is sanitized before it is drawn
- criteria: AC-7
- files: scope
- budget: 65
- exit: `cargo test --manifest-path cli/Cargo.toml gate_output_is_sanitized` exits 0

### T-8 Existing CLI tests stay green
- criteria: AC-8
- files: scope
- budget: 65
- exit: `cargo test --manifest-path cli/Cargo.toml` exits 0

