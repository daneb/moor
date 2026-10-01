---
id: TASKS-0012
slug: agent-session-abstraction
schema: keel.tasks/1
---

# Tasks

Each task must name the criteria it satisfies, the files it touches, a line
budget and an exit condition. G1 checks all four, and checks that every
criterion in the spec is covered by at least one task.

Add `- depends_on: T-1` where order matters. Tasks with no dependency on
each other form a wave; `keel tasks` shows them.

### T-1 The agent axis includes kiro
- criteria: AC-1
- files: scope
- budget: 53
- exit: `cargo test --manifest-path cli/Cargo.toml agent_parses_kiro` exits 0

### T-2 A turn's argv is built for the project's agent
- criteria: AC-2
- files: scope
- budget: 53
- exit: `cargo test --manifest-path cli/Cargo.toml turn_argv_is_per_agent` exits 0

### T-3 A restricted role is refused on an agent that cannot deny tools
- criteria: AC-3
- files: scope
- budget: 53
- exit: `cargo test --manifest-path cli/Cargo.toml restricted_role_refused_without_deny_support` exits 0

### T-4 Tool denial is expressed per agent where it exists
- criteria: AC-4
- files: scope
- budget: 53
- exit: `cargo test --manifest-path cli/Cargo.toml deny_flag_is_per_agent` exits 0

### T-5 Turn output is parsed per agent
- criteria: AC-5
- files: scope
- budget: 53
- exit: `cargo test --manifest-path cli/Cargo.toml turn_output_parsed_per_agent` exits 0

### T-6 Session continuity uses the agent's own resume form
- criteria: AC-6
- files: scope
- budget: 53
- exit: `cargo test --manifest-path cli/Cargo.toml resume_form_is_per_agent` exits 0

### T-7 A missing credential names the right fix for that agent
- criteria: AC-7
- files: scope
- budget: 53
- exit: `cargo test --manifest-path cli/Cargo.toml failure_hint_is_per_agent` exits 0

### T-8 The chained turn record names the agent
- criteria: AC-8
- files: scope
- budget: 53
- exit: `cargo test --manifest-path cli/Cargo.toml turn_record_names_the_agent` exits 0

### T-9 Existing CLI tests stay green
- criteria: AC-9
- files: scope
- budget: 53
- exit: `cargo test --manifest-path cli/Cargo.toml` exits 0

