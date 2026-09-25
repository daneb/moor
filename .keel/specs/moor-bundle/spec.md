---
id: SPEC-0006
slug: moor-bundle
schema: keel.spec/1
status: approved
scope:
- cli/src/commands/bundle_cmd.rs
- cli/src/commands/mod.rs
- cli/src/main.rs
- cli/src/proc.rs
- tests/e2e.sh
- docs/**
- README.md
budget:
  criteria: 8
  lines: 400
verified_at: 2026-09-25
---

# moor bundle: a keel bundle carrying the host chain

## Context

keel's `bundle verify` (keel SPEC-0011) checks every link in a bundle against
the evidence chain it carries. Under Moor, keel never has that chain: Moor
writes it on the host (SPEC-0005). So a bundle keel exports inside the sandbox
has no chain and verifies as blocked. keel added `export --chain` for exactly
this case, and nothing on Moor's side uses it yet.

`moor bundle` builds the bundle where both halves are available. That isn't
the sandbox, where the agent could swap in its own chain. It's a throwaway
container from the project's image: no network, the workspace's named volume,
and the host chain on stdin. Moor then checks the result with the same image's
`keel bundle verify` and exits with its verdict.

## Acceptance criteria

### AC-1 The bundle is built in a throwaway container, not the sandbox

THE SYSTEM SHALL run the export in a new container from the project's image
with `--rm`, `--network none`, the project's workspace named volume, and no
host path mounted.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml commands::bundle_cmd::tests::export_runs_in_a_throwaway_container_with_no_network -- --exact 2>&1 | grep -q '1 passed'` exit 0

### AC-2 keel is handed the host chain

WHEN `moor bundle` runs THE SYSTEM SHALL fold keel's sink into the host
chain, then stream that chain on stdin to `keel export --chain`, passing any
run id given and otherwise letting keel pick the latest run.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml commands::bundle_cmd::tests::export_hands_keel_the_host_chain -- --exact 2>&1 | grep -q '1 passed'` exit 0

### AC-3 The archive comes back byte for byte

WHEN the export succeeds THE SYSTEM SHALL write the container's stdout
unaltered to `<out>/keel-<project>-<timestamp>.tar.gz` and print that path.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml proc::tests::stdout_to_file_keeps_binary_bytes -- --exact 2>&1 | grep -q '1 passed'` exit 0

### AC-4 A failed export leaves nothing behind

IF the export exits non-zero THEN THE SYSTEM SHALL delete the partial archive,
show keel's error, and exit non-zero.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml commands::bundle_cmd::tests::a_failed_export_removes_the_partial_archive -- --exact 2>&1 | grep -q '1 passed'` exit 0

### AC-5 Every bundle is verified before Moor reports success

WHEN the archive is written THE SYSTEM SHALL run `keel bundle verify` on it
in a second throwaway container with no network, print keel's report, and
exit with keel's verdict.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml commands::bundle_cmd::tests::verify_runs_in_a_throwaway_container_with_no_network -- --exact 2>&1 | grep -q '1 passed'` exit 0

### AC-6 A real run under Moor verifies end to end

WHEN the end-to-end suite has taken a spec through `keel run` inside a real
sandbox THE SYSTEM SHALL produce, via `moor bundle`, a bundle whose `chain`,
`approvals`, `gate-verdicts` and `trajectory` checks all pass.

oracle: cmd `bash tests/e2e.sh` exit 0

## Out of scope

- Changing `moor audit --export`. It keeps bundling the raw chain and
  whatever keel bundles sit in the workspace.
- Signing the bundle.
- Checking that the shipped chain is a prefix of the host chain. The
  throwaway container is built from what Moor pipes in, and the agent never
  runs anything inside it.
