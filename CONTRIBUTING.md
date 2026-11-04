# Contributing

Building, testing and releasing moor.

## Testing

- `cd cli && cargo test` — 129 unit tests: selftest's hardening evaluator
  (fed synthetic `docker inspect` JSON, including fail-safe-on-missing-data
  cases), manifest validation and round-tripping, compose template
  rendering, the tinyproxy access-log parser, the audit hash chain
  (append/verify, plus deliberately editing, deleting, reordering, and
  forging entries to confirm `--verify` catches each one; a golden entry
  written by keel that moor must hash identically; sealing a legacy
  chain; folding keel's sink, malformed lines included), the posture
  attestation derived from `docker inspect` (unproven when a field is
  missing), push ref and commit capture, Keychain
  service-name scoping, `moor import`'s image auto-detection and
  GitHub-URL parsing, `moor keel`/`moor view`'s project-resolution
  precedence (explicit flag, sticky `moor use` default, the one sandbox
  that's up, ambiguity errors), `moor keel`'s argv building, `moor use`'s
  known-project validation, `moor view`'s artifact-name mapping and
  markdown highlighting, the agent session protocol (role tool sets,
  `is_error`-based turn failure, UUID-validated session continuity, the
  content-free `agent-turn` chain entry, transcript redaction, recipe
  emission), and `moor studio` (one container query per refresh, input
  handled while a turn is in flight, escape-sequence stripping, the
  two-distinct-key approval, the `docker exec -i` artifact round-trip,
  per-project session isolation, stage read from `keel next`).
- `cargo test --manifest-path mcp/Cargo.toml` — 8 unit tests over the MCP
  server's tool surface: that it covers `gate` and `next` and that no argv
  any accepted call can produce reaches an advancement verb, that a slug
  must pass the same rule `manifest::validate_name` applies, that a gate
  id is matched against a fixed table rather than passed through, and that
  an unexpected argument key is refused rather than ignored.
- `./tests/e2e.sh` — end-to-end against real Docker containers: creates a
  throwaway project, runs `moor selftest`'s static checks and active
  breakout battery, confirms egress allow/deny against github.com and
  example.com, confirms a `git push` attempt is recorded with its ref,
  confirms the posture attestation is in the chain and that the agent
  cannot replace it, folds keel's sink and the egress log in, checks the
  host chain with `keel chain verify` (`$KEEL`, default `keel`), verifies
  the chain, **live-tampers with the real chain.jsonl file and confirms
  `--verify` detects it**, exports an audit bundle, and imports a
  throwaway local multi-branch repo end-to-end (image auto-detect, `keel
  init`, both branches present, selftest still passes). Requires the
  images to already be built (`./images/build.sh`).
- Both of the above, plus security/quality scanning (Rust lint/audit/deny,
  shellcheck, hadolint, gitleaks, Trivy image CVE scans), run in CI on
  every push/PR — see [docs/CI.md](docs/CI.md).
- `make help` lists every local dev target — `make check` for a fast
  fmt/clippy/test pass, `make security` for the full scan suite,
  `make ci` to run everything CI runs (including the real `tests/e2e.sh`)
  in one shot. Each security target checks its tool is installed and
  says how to get it (`brew install ...`) rather than failing on a bare
  "command not found." Every target here was run for real before being
  documented, not just written.

## Releasing

`master` is protected, so a release is two steps, each checked by
`release.sh` before it changes anything (add `--dry-run` to see what it
would do):

```bash
./release.sh prepare 0.4.0   # opens the release PR: version + CHANGELOG
# merge that PR, then:
git pull --ff-only && ./release.sh publish 0.4.0
```

`publish` runs the same gates CI does, publishes to crates.io with the
token `cargo login` stored, tags `v0.4.0` (pushing only the tag), and
creates the GitHub release from the CHANGELOG. It refuses to publish from
any commit but the one that matches `origin/master`, and a version already
tagged elsewhere; every step checks whether it's already done, so an
interrupted release finishes by running the same command again.

## Development

```bash
make help          # list every target
make check          # fast: fmt-check, clippy, cargo test — no Docker needed
make security        # cargo audit/deny, shellcheck, hadolint, gitleaks, trivy
make ci               # everything above plus the real tests/e2e.sh — the full CI replica
make build            # cargo build (debug)
make release          # cargo build --release
make install          # cargo install the CLI to ~/.cargo/bin
make clean            # cargo clean
```

`make publish-dry-run` runs `cargo publish --dry-run`. This crate was
originally named `isolator` and marked `publish = false` on purpose —
`isolator` is already taken on crates.io by an unrelated package, and
the crate wasn't meant to be published at all at the time. Three things
were needed to actually fix that, not just the first one that looked
like the whole story: renamed to `moor` (available, checked directly
against the crates.io API — see
[ADR-0004](docs/decisions/0004-rename-to-moor.md) for the naming
process and every alternative tried), MIT-licensed
(`LICENSE` + `license = "MIT"` in `cli/Cargo.toml`), and — found only by
actually running `cargo publish --dry-run --allow-dirty` all the way
through, not stopping at the first passing check —
`cli/src/compose.rs`'s `include_str!` moved from a top-level `compose/`
into `cli/templates/`, since a published crate can't embed a file that
lives outside its own package directory. `cargo publish --dry-run` now
packages, compiles from the isolated package tarball, and gets to the
upload step (aborted only because it's a dry run) — verified, not
assumed.

`make publish` runs the real thing. It's still a deliberate manual step
for whoever owns the crates.io account — nothing here runs it for
you — but it exists now: it re-runs `publish-dry-run` first, then
requires typing the literal word `publish` at a prompt (not just
pressing enter) before calling `cargo publish` itself, since a crates.io
version can be yanked afterward but never deleted or reused. Needs
`cargo login` done once beforehand, with a token from
https://crates.io/settings/tokens. One caveat ADR-0004 is explicit about:
`cargo install moor` only brings the compiled binary, not `images/`,
`proxy/`, or `policies/` — without those (from a full
clone, with `./images/build.sh` run once) the installed binary has no
sandbox images to build from.
