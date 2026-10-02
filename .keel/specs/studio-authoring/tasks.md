---
id: TASKS-0014
slug: studio-authoring
schema: keel.tasks/1
---

# Tasks

Each task must name the criteria it satisfies, the files it touches, a line
budget and an exit condition. G1 checks all four, and checks that every
criterion in the spec is covered by at least one task.

Add `- depends_on: T-1` where order matters. Tasks with no dependency on
each other form a wave; `keel tasks` shows them.

### T-1 A key turns the current answer into a spec draft
- criteria: AC-1
- files: scope
- budget: 50
- exit: `cargo test --manifest-path cli/Cargo.toml author_key_drafts_a_spec_from_the_answer` exits 0

### T-2 Nothing is written when there is no answer
- criteria: AC-2
- files: scope
- budget: 50
- exit: `cargo test --manifest-path cli/Cargo.toml author_without_an_answer_writes_nothing` exits 0

### T-3 A draft that does not parse is kept as a draft, not installed
- criteria: AC-3
- files: scope
- budget: 50
- exit: `cargo test --manifest-path cli/Cargo.toml unparseable_answer_stays_a_draft` exits 0

### T-4 A drafted spec is reported with the command that checks it
- criteria: AC-4
- files: scope
- budget: 50
- exit: `cargo test --manifest-path cli/Cargo.toml drafted_spec_names_the_next_command` exits 0

### T-5 The answer is sanitized before it becomes an artefact
- criteria: AC-5
- files: scope
- budget: 50
- exit: `cargo test --manifest-path cli/Cargo.toml authored_draft_is_sanitized` exits 0

### T-6 Authoring is recorded like any other sandbox action
- criteria: AC-6
- files: scope
- budget: 50
- exit: `cargo test --manifest-path cli/Cargo.toml authoring_is_recorded` exits 0

### T-7 The host writes the draft; the sandbox is not asked to author it
- criteria: AC-7
- files: scope
- budget: 50
- exit: `cargo test --manifest-path cli/Cargo.toml draft_is_written_by_the_host` exits 0

### T-8 Existing CLI tests stay green
- criteria: AC-8
- files: scope
- budget: 50
- exit: `cargo test --manifest-path cli/Cargo.toml` exits 0

