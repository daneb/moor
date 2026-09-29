---
id: TASKS-0009
slug: tool-call-ask-verdict
schema: keel.tasks/1
---

# Tasks

Each task must name the criteria it satisfies, the files it touches, a line
budget and an exit condition. G1 checks all four, and checks that every
criterion in the spec is covered by at least one task.

Add `- depends_on: T-1` where order matters. Tasks with no dependency on
each other form a wave; `keel tasks` shows them.

### T-1 An interactive operator can override a flagged command
- criteria: AC-1
- files: scope
- budget: 60
- exit: `cargo test --manifest-path cli/Cargo.toml commands::run_cmd::tests::interactive_yes_allows -- --exact 2>&1 | grep -q '1 passed'` exits 0

### T-2 A decline denies exactly as an unprompted Deny does
- criteria: AC-2
- files: scope
- budget: 60
- exit: `cargo test --manifest-path cli/Cargo.toml commands::run_cmd::tests::interactive_no_denies -- --exact 2>&1 | grep -q '1 passed'` exits 0

### T-3 A non-interactive invocation is never prompted
- criteria: AC-3
- files: scope
- budget: 60
- exit: `cargo test --manifest-path cli/Cargo.toml commands::run_cmd::tests::non_interactive_denies_without_reading_stdin -- --exact 2>&1 | grep -q '1 passed'` exits 0

### T-4 Every prompted decision is attributed in the audit chain
- criteria: AC-4
- files: scope
- budget: 60
- exit: `cargo test --manifest-path cli/Cargo.toml commands::run_cmd::tests::decision_entry_records_resolution_and_operator -- --exact 2>&1 | grep -q '1 passed'` exits 0

### T-5 An Allow verdict is still a no-op
- criteria: AC-5
- files: scope
- budget: 60
- exit: `cargo test --manifest-path cli/Cargo.toml --quiet` exits 0

