---
id: SPEC-0005
slug: moor-evidence-chain
schema: keel.spec/1
status: approved
scope:
- cli/src/audit.rs
- cli/src/posture.rs
- cli/src/paths.rs
- cli/src/main.rs
- cli/src/commands/mod.rs
- cli/src/commands/run_cmd.rs
- cli/src/commands/audit_cmd.rs
- cli/src/compose.rs
- cli/templates/project.compose.yml.tmpl
- tests/e2e.sh
- README.md
- docs/**
- .github/workflows/ci.yml
budget:
  criteria: 8
  lines: 1100
verified_at: 2026-09-24
---

# Moor writes the keel evidence chain

## Context

keel ADR-0001 settles one evidence chain: keel owns the `keel.chain/1` format
and verifier, and the runtime's host process is the only writer. keel's side
has shipped: `KEEL_CHAIN_SINK` for payloads, `KEEL_RUNTIME_ATTESTATION` for
posture (keel SPEC-0009, SPEC-0010). Moor still writes its own chain format,
nothing reads keel's sink, and no attestation exists, so an auditor still has
to trust two logs.

This spec makes Moor that host-side writer. It keeps Moor's standing rule of
no host bind mounts, ever. The attestation reaches the sandbox through a
root-owned `tmpfs` that the `agent` user can read but not replace. keel's
payloads leave through a second `tmpfs` that Moor pulls with `docker exec`,
the same way it already folds the egress log. Anything read from the sink is
what the sandbox *said*; Moor records it faithfully and marks it as such, but
cannot make it true.

## Acceptance criteria

### AC-1 Moor writes keel's format

WHEN Moor appends to a project's audit chain THE SYSTEM SHALL write a
`keel.chain/1` entry with `writer` set to `moor`, hashed as keel hashes it,
so that `keel chain verify` accepts the chain.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml audit::tests::matches_a_keel_written_golden_entry -- --exact 2>&1 | grep -q '1 passed'` exit 0

### AC-2 Legacy chains are sealed, not rewritten

WHEN Moor first appends to a project whose chain predates `keel.chain/1` THE
SYSTEM SHALL move that chain to `chain.legacy.jsonl` unchanged and start the
new chain with a `legacy_seal` entry carrying the legacy head hash and entry
count.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml audit::tests::legacy_chain_is_sealed_into_the_new_one -- --exact 2>&1 | grep -q '1 passed'` exit 0

### AC-3 The sandbox is attested from outside

WHEN `moor up` starts a sandbox THE SYSTEM SHALL derive a `keel.posture/1`
attestation from `docker inspect` of the running container, write it to
`/run/moor/posture.json` inside the container as root, and append an
`attest` entry with its SHA-256 to the host chain.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml posture::tests::attestation_is_derived_from_inspect_output -- --exact 2>&1 | grep -q '1 passed'` exit 0

### AC-4 What inspect cannot show is unproven

IF `docker inspect` output does not establish a posture property THEN THE
SYSTEM SHALL mark that property `unproven`, and SHALL mark it `violated` only
when the output shows it does not hold.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml posture::tests::missing_inspect_fields_are_unproven -- --exact 2>&1 | grep -q '1 passed'` exit 0

### AC-5 The sandbox gets both paths, without a bind mount

THE SYSTEM SHALL render the sandbox service with `KEEL_RUNTIME_ATTESTATION`
and `KEEL_CHAIN_SINK` set, a root-owned `tmpfs` at `/run/moor`, a
world-writable `tmpfs` at `/run/moor-sink`, and no host path in any mount.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml compose::tests::sandbox_gets_attestation_and_sink_without_bind_mounts -- --exact 2>&1 | grep -q '1 passed'` exit 0

### AC-6 keel's payloads are folded after every exec

WHEN Moor logs a command it ran in the sandbox THE SYSTEM SHALL first fold
each new sink line into the host chain under its own `kind`, with
`source: "sandbox"` added to its data, and record any line that does not
parse as a `sink_malformed` entry rather than dropping it.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml audit::tests::sink_lines_fold_in_order_and_malformed_ones_are_kept -- --exact 2>&1 | grep -q '1 passed'` exit 0

### AC-7 A push records what was pushed

WHEN `moor run` executes a `git push` THE SYSTEM SHALL append a `push` entry
carrying the pushed ref and the commit SHA it resolved to before the push,
alongside the exit code.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml commands::run_cmd::tests::push_entry_names_ref_and_commit -- --exact 2>&1 | grep -q '1 passed'` exit 0

### AC-8 One chain, end to end

WHEN the end-to-end suite runs a keel gate inside a real sandbox THE SYSTEM
SHALL produce a host chain that holds the `attest`, folded keel `gate` and
`egress` entries, and that `keel chain verify` accepts.

oracle: cmd `bash tests/e2e.sh` exit 0

## Out of scope

- Checking claims beyond what `docker inspect` shows, such as the egress
  proxy's own allow-list contents.
- Linking a push to the keel run that produced it; the bundle does that
  (keel `bundle-v1`).
- Pushes typed inside `moor shell`, which Moor cannot see (THREAT-MODEL).
- Studio and `moor ask` changes. Their chain entries change format through
  `append_chained`, with no edits of their own.
- Signing the chain, and pointing `keel chain verify` at a file outside
  `.keel/`.
