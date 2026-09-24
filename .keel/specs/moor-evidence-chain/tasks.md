---
id: TASKS-0005
slug: moor-evidence-chain
schema: keel.tasks/1
---

# Tasks

### T-1 keel.chain/1 entries with keel's hashing
- criteria: AC-1
- files: cli/src/audit.rs
- budget: 110
- exit: cmd `cargo test --manifest-path cli/Cargo.toml audit::tests::matches_a_keel_written_golden_entry -- --exact 2>&1 | grep -q '1 passed'` exit 0

### T-2 Seal a legacy chain into the new one
- criteria: AC-2
- files: cli/src/audit.rs, cli/src/paths.rs, cli/src/commands/audit_cmd.rs
- budget: 110
- depends_on: T-1
- exit: cmd `cargo test --manifest-path cli/Cargo.toml audit::tests::legacy_chain_is_sealed_into_the_new_one -- --exact 2>&1 | grep -q '1 passed'` exit 0

### T-3 Fold keel's sink into the host chain
- criteria: AC-6
- files: cli/src/audit.rs, cli/src/paths.rs, cli/src/commands/audit_cmd.rs
- budget: 110
- depends_on: T-2
- exit: cmd `cargo test --manifest-path cli/Cargo.toml audit::tests::sink_lines_fold_in_order_and_malformed_ones_are_kept -- --exact 2>&1 | grep -q '1 passed'` exit 0

### T-4 Posture attestation from docker inspect
- criteria: AC-3, AC-4
- files: cli/src/posture.rs, cli/src/main.rs, cli/src/commands/mod.rs, cli/src/paths.rs
- budget: 150
- depends_on: T-3
- exit: cmd `cargo test --manifest-path cli/Cargo.toml posture::tests::missing_inspect_fields_are_unproven -- --exact 2>&1 | grep -q '1 passed'` exit 0

### T-5 Attestation and sink mounts in compose
- criteria: AC-5
- files: cli/templates/project.compose.yml.tmpl, cli/src/compose.rs
- budget: 50
- exit: cmd `cargo test --manifest-path cli/Cargo.toml compose::tests::sandbox_gets_attestation_and_sink_without_bind_mounts -- --exact 2>&1 | grep -q '1 passed'` exit 0

### T-6 Push entries carry ref and commit
- criteria: AC-7
- files: cli/src/commands/run_cmd.rs, cli/src/audit.rs
- budget: 90
- depends_on: T-4
- exit: cmd `cargo test --manifest-path cli/Cargo.toml commands::run_cmd::tests::push_entry_names_ref_and_commit -- --exact 2>&1 | grep -q '1 passed'` exit 0

### T-7 End-to-end chain check
- criteria: AC-8
- files: tests/e2e.sh
- budget: 60
- depends_on: T-5, T-6
- exit: cmd `bash tests/e2e.sh` exit 0
