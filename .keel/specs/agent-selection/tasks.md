---
id: TASKS-0010
slug: agent-selection
schema: keel.tasks/1
---

# Tasks

Each task must name the criteria it satisfies, the files it touches, a line
budget and an exit condition. G1 checks all four, and checks that every
criterion in the spec is covered by at least one task.

Add `- depends_on: T-1` where order matters. Tasks with no dependency on
each other form a wave; `keel tasks` shows them.

### T-1 An --agent flag selects the agent at import and new
- criteria: AC-1
- files: scope
- budget: 40
- exit: `cargo test --manifest-path cli/Cargo.toml agent_flag_sets_manifest_agent` exits 0

### T-2 The agent defaults to claude when the flag is omitted
- criteria: AC-2
- files: scope
- budget: 40
- exit: `cargo test --manifest-path cli/Cargo.toml agent_defaults_to_claude` exits 0

### T-3 Copilot composes with the detected language into the right image
- criteria: AC-3
- files: scope
- budget: 40
- exit: `cargo test --manifest-path cli/Cargo.toml copilot_composes_with_language` exits 0

### T-4 The claude agent leaves image selection unchanged
- criteria: AC-4
- files: scope
- budget: 40
- exit: `cargo test --manifest-path cli/Cargo.toml claude_leaves_image_unchanged` exits 0

### T-5 An unknown agent value is rejected
- criteria: AC-5
- files: scope
- budget: 40
- exit: `cargo test --manifest-path cli/Cargo.toml unknown_agent_is_rejected` exits 0

