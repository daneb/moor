---
id: TASKS-0020
slug: audit-chain-atomicity-and-agent-image
schema: keel.tasks/1
---

# Tasks

Each task must name the criteria it satisfies, the files it touches, a line
budget and an exit condition. G1 checks all four, and checks that every
criterion in the spec is covered by at least one task.

Add `- depends_on: T-1` where order matters. Tasks with no dependency on
each other form a wave; `keel tasks` shows them.

Exit conditions pipe `cargo test --exact <name>` through `grep -q '1 passed'`
so a missing test fails the oracle: a name filter alone exits 0 when it
matches nothing, which would make the oracle insensitive at the base (G2's
oracle-sensitivity check). `grep '1 passed'` is true only when the named
test actually ran and passed.

### T-1 Append the whole entry as a single write

- criteria: AC-1
- files: cli/src/audit.rs
- budget: 50
- exit: `cargo test --manifest-path cli/Cargo.toml --quiet -- --exact audit::tests::append_writes_entry_as_one_line 2>&1 | grep -q '1 passed'`

### T-2 Serialise concurrent appends with an advisory file lock

- criteria: AC-2
- files: cli/src/audit.rs, cli/Cargo.toml
- budget: 90
- depends_on: T-4
- exit: `cargo test --manifest-path cli/Cargo.toml --quiet -- --exact audit::tests::concurrent_appends_do_not_interleave 2>&1 | grep -q '1 passed'`

### T-3 Read the final valid record off a glued last line

- criteria: AC-3
- files: cli/src/audit.rs
- budget: 70
- depends_on: T-1
- exit: `cargo test --manifest-path cli/Cargo.toml --quiet -- --exact audit::tests::last_entry_reads_final_record_of_a_glued_line 2>&1 | grep -q '1 passed'`

### T-4 A malformed line degrades to the last good entry, never a crash

- criteria: AC-4
- files: cli/src/audit.rs
- budget: 30
- depends_on: T-3
- exit: `cargo test --manifest-path cli/Cargo.toml --quiet -- --exact audit::tests::malformed_line_does_not_crash_read 2>&1 | grep -q '1 passed'`

### T-5 resolve_image refuses to prefix an image already in another agent's layer

- criteria: AC-5
- files: cli/src/manifest.rs
- budget: 40
- exit: `cargo test --manifest-path cli/Cargo.toml --quiet -- --exact manifest::tests::resolve_image_does_not_cross_compose_agents 2>&1 | grep -q '1 passed'`

### T-6 Prove no resolve_image input yields a double-agent prefix

- criteria: AC-6
- files: cli/src/manifest.rs
- budget: 50
- depends_on: T-5
- exit: `cargo test --manifest-path cli/Cargo.toml --quiet -- --exact manifest::tests::resolve_image_never_double_prefixes 2>&1 | grep -q '1 passed'`

### T-7 update_manifest rejects a cross-agent image edit

- criteria: AC-7
- files: cli/src/ui/data.rs
- budget: 60
- depends_on: T-5
- exit: `cargo test --manifest-path cli/Cargo.toml --quiet -- --exact ui::data::tests::update_manifest_rejects_cross_agent_image 2>&1 | grep -q '1 passed'`

### T-8 Record the ADR

- criteria: AC-8
- files: docs/decisions/0016-atomic-audit-chain-and-image-guard.md
- budget: 130
- depends_on: T-1, T-2, T-3, T-4, T-5, T-6, T-7
- exit: `ls docs/decisions/*audit-chain* docs/decisions/*atomic* 2>/dev/null | head -1 | grep -q .`
