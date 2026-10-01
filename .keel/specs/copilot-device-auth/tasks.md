---
id: TASKS-0011
slug: copilot-device-auth
schema: keel.tasks/1
---

# Tasks

Each task must name the criteria it satisfies, the files it touches, a line
budget and an exit condition. G1 checks all four, and checks that every
criterion in the spec is covered by at least one task.

Add `- depends_on: T-1` where order matters. Tasks with no dependency on
each other form a wave; `keel tasks` shows them.

### T-1 The Copilot CLI's device-flow token is discoverable from the host Keychain
- criteria: AC-1
- files: scope
- budget: 25
- exit: `cargo test --manifest-path cli/Cargo.toml finds_copilot_device_token_source` exits 0

### T-2 An explicitly set credential always wins over the discovered token
- criteria: AC-2
- files: scope
- budget: 25
- exit: `cargo test --manifest-path cli/Cargo.toml explicit_credential_wins_over_device_token` exits 0

### T-3 The device-flow token is only used for copilot-agent projects
- criteria: AC-3
- files: scope
- budget: 25
- exit: `cargo test --manifest-path cli/Cargo.toml device_token_only_for_copilot_agent` exits 0

### T-4 The token is read fresh each up, never persisted by moor
- criteria: AC-4
- files: scope
- budget: 25
- exit: `cargo test --manifest-path cli/Cargo.toml device_token_not_persisted_by_moor` exits 0

### T-5 The token never passes through a process argument
- criteria: AC-5
- files: scope
- budget: 25
- exit: `cargo test --manifest-path cli/Cargo.toml device_token_not_in_argv` exits 0

### T-6 Absence of a device-flow token is a no-op, not an error
- criteria: AC-6
- files: scope
- budget: 25
- exit: `cargo test --manifest-path cli/Cargo.toml missing_device_token_is_noop` exits 0

### T-7 The resolved token is scrubbed from the audit trail
- criteria: AC-7
- files: scope
- budget: 25
- exit: `cargo test --manifest-path cli/Cargo.toml device_token_is_redacted` exits 0

### T-8 Existing CLI tests stay green
- criteria: AC-8
- files: scope
- budget: 25
- exit: `cargo test --manifest-path cli/Cargo.toml` exits 0

