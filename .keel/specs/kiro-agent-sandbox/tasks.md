---
id: TASKS-0018
slug: kiro-agent-sandbox
schema: keel.tasks/1
---

# Tasks

Each task must name the criteria it satisfies, the files it touches, a line
budget and an exit condition. G1 checks all four, and checks that every
criterion in the spec is covered by at least one task.

Add `- depends_on: T-1` where order matters. Tasks with no dependency on
each other form a wave; `keel tasks` shows them.

### T-1 Kiro composes with the detected language into a kiro image
- criteria: AC-1
- files: scope
- budget: 35
- exit: `cargo test --manifest-path cli/Cargo.toml kiro_composes_with_language` exits 0

### T-2 An already-kiro image is left unchanged by resolution
- criteria: AC-2
- files: scope
- budget: 35
- exit: `cargo test --manifest-path cli/Cargo.toml kiro_image_resolution_is_idempotent` exits 0

### T-3 A kiro image reconciles a manifest whose agent field is stale
- criteria: AC-3
- files: scope
- budget: 35
- exit: `cargo test --manifest-path cli/Cargo.toml kiro_image_reconciles_agent` exits 0

### T-4 The --agent flag help names kiro as a valid value
- criteria: AC-4
- files: scope
- budget: 35
- exit: `cargo run --manifest-path cli/Cargo.toml -- new --help 2>&1 | grep -q kiro` exits 0

### T-5 A kiro-agent project records the Kiro credential env names as secrets
- criteria: AC-5
- files: scope
- budget: 35
- exit: `cargo test --manifest-path cli/Cargo.toml kiro_project_declares_kiro_secrets` exits 0

### T-6 The host Kiro credential is discoverable without manual secret entry
- criteria: AC-6
- files: scope
- budget: 35
- exit: `cargo test --manifest-path cli/Cargo.toml finds_kiro_credential_source` exits 0

### T-7 The Kiro credential is only injected for kiro-agent projects
- criteria: AC-7
- files: scope
- budget: 35
- exit: `cargo test --manifest-path cli/Cargo.toml kiro_credential_only_for_kiro_agent` exits 0

### T-8 The Kiro credential is read fresh each up and never persisted by moor
- criteria: AC-8
- files: scope
- budget: 35
- exit: `cargo test --manifest-path cli/Cargo.toml kiro_credential_not_persisted` exits 0

### T-9 The Kiro credential never passes through a process argument
- criteria: AC-9
- files: scope
- budget: 35
- exit: `cargo test --manifest-path cli/Cargo.toml kiro_credential_not_in_argv` exits 0

### T-10 Absence of a Kiro credential is a no-op, not an error
- criteria: AC-10
- files: scope
- budget: 35
- exit: `cargo test --manifest-path cli/Cargo.toml missing_kiro_credential_is_noop` exits 0

### T-11 The egress allowlist permits Kiro's auth and model endpoints
- criteria: AC-11
- files: scope
- budget: 35
- exit: `grep -Eq 'codewhisperer|sso|oidc' proxy/allowlist.base.txt && echo ok` exits 0

### T-12 The whole CLI and image-compose test suite stays green
- criteria: AC-12
- files: scope
- budget: 35
- exit: `cargo test --manifest-path cli/Cargo.toml` exits 0

