---
id: TASKS-0017
slug: drop-serde-yaml
schema: keel.tasks/1
---

# Tasks

Each task names the criteria it satisfies, the files it touches, a line
budget and an exit condition. G1 checks all four, and checks that every
criterion is covered by at least one task.

The dep swap and the call-site renames are one atomic unit — the tree does
not compile with only one done — so T-1 and T-2 are a single wave by
intent, split only so each criterion traces to a task.

### T-1 Swap the dependency: serde_yaml out, yaml_serde in
- criteria: AC-1, AC-2, AC-4
- files: cli/Cargo.toml, cli/Cargo.lock
- budget: 10
- exit: `test -z "$(grep -nE '^serde_yaml\b' cli/Cargo.toml)" && grep -qE 'yaml_serde' cli/Cargo.toml && cargo deny --manifest-path cli/Cargo.toml check advisories` exits 0

### T-2 Rename every serde_yaml:: path to yaml_serde:: (14 sites)
- criteria: AC-3
- files: cli/src/manifest.rs, cli/src/compose.rs, cli/src/session.rs, cli/src/commands/recipe.rs, cli/src/commands/spec_cmd.rs
- budget: 30
- exit: `test -z "$(grep -rn 'serde_yaml::' cli/src)"` exits 0
- depends_on: T-1

### T-3 Prove parity: manifest round-trip and front-matter parsing unchanged
- criteria: AC-5, AC-6
- files: cli/src/manifest.rs, cli/src/commands/spec_cmd.rs, cli/src/commands/recipe.rs
- budget: 10
- exit: `cargo test --manifest-path cli/Cargo.toml --quiet manifest::tests spec_cmd::tests recipe::tests` exits 0
- depends_on: T-2

### T-4 Green, fmt-clean suites across cli and mcp
- criteria: AC-7
- files: cli/src/compose.rs, cli/src/session.rs
- budget: 10
- exit: `cargo fmt --check --manifest-path cli/Cargo.toml && cargo test --quiet --manifest-path cli/Cargo.toml && cargo test --quiet --manifest-path mcp/Cargo.toml` exits 0
- depends_on: T-3
