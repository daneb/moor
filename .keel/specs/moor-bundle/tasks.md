---
id: TASKS-0006
slug: moor-bundle
schema: keel.tasks/1
---

# Tasks

### T-1 Binary-safe stdin-to-file process helper
- criteria: AC-3
- files: cli/src/proc.rs
- budget: 50
- exit: cmd `cargo test --manifest-path cli/Cargo.toml proc::tests::stdout_to_file_keeps_binary_bytes -- --exact 2>&1 | grep -q '1 passed'` exit 0

### T-2 moor bundle: export, clean up, verify
- criteria: AC-1, AC-2, AC-4, AC-5
- files: cli/src/commands/bundle_cmd.rs, cli/src/commands/mod.rs, cli/src/main.rs
- budget: 150
- depends_on: T-1
- exit: cmd `cargo test --manifest-path cli/Cargo.toml commands::bundle_cmd::tests::export_runs_in_a_throwaway_container_with_no_network -- --exact 2>&1 | grep -q '1 passed'` exit 0

### T-3 A real run bundled and verified end to end
- criteria: AC-6
- files: tests/e2e.sh
- budget: 90
- depends_on: T-2
- exit: cmd `bash tests/e2e.sh` exit 0

### T-4 Document moor bundle
- criteria: AC-5
- files: docs/ARCHITECTURE.md, README.md
- budget: 40
- depends_on: T-3
- exit: cmd `grep -q 'moor bundle' docs/ARCHITECTURE.md` exit 0
