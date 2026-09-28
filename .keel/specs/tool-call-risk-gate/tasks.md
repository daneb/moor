---
id: TASKS-0008
slug: tool-call-risk-gate
schema: keel.tasks/1
---

# Tasks

Each task must name the criteria it satisfies, the files it touches, a line
budget and an exit condition. G1 checks all four, and checks that every
criterion in the spec is covered by at least one task.

Add `- depends_on: T-1` where order matters. Tasks with no dependency on
each other form a wave; `keel tasks` shows them.

### T-1 The ADR names the enforcement point
- criteria: AC-1
- files: scope
- budget: 110
- exit: `test -f docs/adr/0001-tool-call-risk-gating.md && grep -q 'docker exec' docs/adr/0001-tool-call-risk-gating.md && grep -qi 'in-sandbox' docs/adr/0001-tool-call-risk-gating.md` exits 0

### T-2 Credential-path commands are denied
- criteria: AC-2
- files: scope
- budget: 35
- exit: `cargo test --manifest-path cli/Cargo.toml decision::tests::denies_known_credential_paths -- --exact 2>&1 | grep -q '1 passed'` exits 0

### T-3 Docker-socket and privilege-escalation commands are denied
- criteria: AC-3
- files: scope
- budget: 35
- exit: `cargo test --manifest-path cli/Cargo.toml decision::tests::denies_docker_socket_and_sudo -- --exact 2>&1 | grep -q '1 passed'` exits 0

### T-4 Whole-workspace or home-directory wipes are denied
- criteria: AC-4
- files: scope
- budget: 45
- exit: `cargo test --manifest-path cli/Cargo.toml decision::tests::denies_recursive_wipe_of_workspace_or_home -- --exact 2>&1 | grep -q '1 passed'` exits 0

### T-5 A Deny verdict blocks execution before it reaches Docker
- criteria: AC-5
- files: scope
- budget: 100
- exit: `cargo test --manifest-path cli/Cargo.toml commands::run_cmd::tests::deny_verdict_blocks_docker_exec -- --exact 2>&1 | grep -q '1 passed'` exits 0

### T-6 An Allow verdict changes nothing observable
- criteria: AC-6
- files: scope
- budget: 10
- exit: `cargo test --manifest-path cli/Cargo.toml --quiet` exits 0

