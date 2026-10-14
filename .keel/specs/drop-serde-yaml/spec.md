---
id: SPEC-0017
slug: drop-serde-yaml
schema: keel.spec/1
status: approved
scope:
  - "cli/Cargo.toml"
  - "cli/Cargo.lock"
  - "cli/src/manifest.rs"
  - "cli/src/compose.rs"
  - "cli/src/session.rs"
  - "cli/src/commands/recipe.rs"
  - "cli/src/commands/spec_cmd.rs"
budget:
  criteria: 7
  lines: 180
verified_at: 2026-10-14
---

# Replace unmaintained serde_yaml with yaml_serde

## Context

`cargo deny` fails on every PR because `serde_yaml 0.9.34+deprecated`
pulls in `unsafe-libyaml 0.2.11`, which carries advisory
**RUSTSEC-2024-0449**: the crate's repository was archived read-only on
2024-03-25 and will receive no further fixes. `serde_yaml` itself is
deprecated and unmaintained. There is no safe *upgrade* of `serde_yaml` —
the fix is to stop depending on it.

This change moves to **`yaml_serde`** (`yaml_serde = "0.10.7"`, maintained
by the YAML organization), chosen for the smallest blast radius: its
`from_str`/`to_string`/`Value` surface matches the code's existing usage,
so the migration is a dependency swap plus mechanical call-site edits
rather than a rewrite.

Two alternatives were rejected. `yaml_serde` looked like the obvious
near-drop-in fork but still depends on `unsafe-libyaml 0.2.11` — the exact
crate the advisory is against — so it does not clear RUSTSEC-2024-0449 at
all (verified with `cargo tree -i unsafe-libyaml`). `serde-saphyr` is pure
safe Rust but has a different API (no `to_string`), which would force a
rewrite beyond this spec's scope.

Honest caveat, recorded deliberately: `yaml_serde`'s backend is
`libyaml-rs`, which — like `unsafe-libyaml` — is libyaml's C transpiled to
Rust by c2rust, just a different crate under the YAML org's control. It
clears *this* advisory (RUSTSEC-2024-0449 names `unsafe-libyaml`
specifically), but it is the same risk class, not pure safe Rust. Moving
to a pure-Rust parser (`serde-saphyr`) is a larger follow-up if that risk
class is to be left behind entirely.

The dependency is used at 14 call sites across 6 files in `cli/`, and
nowhere in `mcp/`. The load-bearing uses are `Manifest` load/save
(`manifest.rs`) and YAML front-matter parsing (`recipe.rs`,
`spec_cmd.rs`); the rest are `serde_yaml::Value` parses in tests. The
behaviour that must not change is manifest round-tripping: a manifest
written and reloaded must be identical, because `moor.yaml` files already
exist on disk for every project and must keep loading.

This is deliberately a standalone change, not folded into any feature
PR: it is pure dependency hygiene, and keeping it separate lets it be
reviewed and reverted on its own.

## Acceptance criteria

### AC-1 serde_yaml is no longer a dependency

WHEN the CLI manifest is inspected THE SYSTEM SHALL declare no dependency
on `serde_yaml`.

oracle: cmd `test -z "$(grep -nE '^serde_yaml\b|\"serde_yaml\"' cli/Cargo.toml)"` exit 0

### AC-2 yaml_serde is the YAML dependency

WHEN the CLI manifest is inspected THE SYSTEM SHALL declare a dependency
on `yaml_serde`.

oracle: cmd `grep -qE 'yaml_serde' cli/Cargo.toml` exit 0

### AC-3 No source references the old crate path

WHEN the CLI source is inspected THE SYSTEM SHALL contain no `serde_yaml::`
path, only the replacement crate's path.

oracle: cmd `test -z "$(grep -rn 'serde_yaml::' cli/src)"` exit 0

### AC-4 The unmaintained advisory is gone from the dependency tree

WHEN the dependency advisories are audited THE SYSTEM SHALL report no
`unsafe-libyaml` / RUSTSEC-2024-0449 finding.

oracle: cmd `cargo deny --manifest-path cli/Cargo.toml check advisories` exit 0

### AC-5 A manifest round-trips unchanged

WHEN a manifest is written and reloaded THE SYSTEM SHALL produce an
identical manifest, so existing `moor.yaml` files keep loading.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml --quiet manifest::tests` exit 0

### AC-6 YAML front matter still parses

WHEN a spec's or recipe's YAML front matter is parsed THE SYSTEM SHALL
read it with the replacement crate, preserving existing behaviour.

oracle: cmd `cargo test --manifest-path cli/Cargo.toml --quiet spec_cmd` exit 0
oracle: cmd `cargo test --manifest-path cli/Cargo.toml --quiet recipe` exit 0

### AC-7 The suites stay green

THE SYSTEM SHALL leave the CLI and MCP test suites passing and the tree
rustfmt-clean.

oracle: cmd `cargo fmt --check --manifest-path cli/Cargo.toml` exit 0
oracle: cmd `cargo test --quiet --manifest-path cli/Cargo.toml && cargo test --quiet --manifest-path mcp/Cargo.toml` exit 0

## Out of scope

- **Any API or behaviour change beyond the crate swap.** Error messages,
  manifest schema, and front-matter format are unchanged; only the parser
  behind them moves.
- **`mcp/`** — it does not use `serde_yaml`, so it is untouched except as
  a green-suite check.
- **Suppressing the advisory via `deny.toml`.** The point is to remove the
  dependency, not to ignore the finding.
