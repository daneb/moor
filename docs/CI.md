# CI: security and quality scanning

Run any of this locally with `make` — `make help` lists every target,
`make security` runs the scans, `make ci` runs everything below
(including the real `tests/e2e.sh`) in one shot. The `Makefile` targets
and this workflow are meant to stay in lockstep: if you add a check to
one, add it to the other.

`.github/workflows/ci.yml` runs on every push/PR to `master`. Every job
below was actually run — locally, via [`act`](https://github.com/nektos/act)
executing the real workflow file against real Docker — before being
wired in, and every finding it turned up became either a real fix or a
documented, narrow suppression. Nothing here is a rubber stamp.

| Job | Tool | What it checks |
|---|---|---|
| `rust` | `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test` | Formatting, lint-level correctness issues, the unit tests |
| `rust-security` | [`cargo-audit`](https://github.com/RustSec/rustsec) (via `rustsec/audit-check`), [`cargo-deny`](https://github.com/EmbarkStudios/cargo-deny) | Known-vulnerable dependencies (RustSec advisory DB), license compliance, banned/duplicate/untrusted dependency sources |
| `shell-lint` | [`shellcheck`](https://www.shellcheck.net/) | Every `.sh` script |
| `docker-lint` | [`hadolint`](https://github.com/hadolint/hadolint) | Every Dockerfile (base, node, rust, python, egress) |
| `secret-scan` | [`gitleaks`](https://github.com/gitleaks/gitleaks) | Full git history for leaked credentials |
| `image-scan` | [`trivy`](https://github.com/aquasecurity/trivy) | Every built image, for HIGH/CRITICAL CVEs with an available fix |
| `e2e` | `tests/e2e.sh` | The real thing: live containers, `moor selftest`'s full hardening + breakout battery, the audit chain (fold, verify, a live tamper attempt), `moor import` |

## What actually got fixed (not just gated on)

Running these for real, not just writing the YAML, found genuine bugs:

- **`cli/` was never run through `cargo fmt`** — 53 files reformatted (whitespace only, verified behavior-identical: 59/59 tests still pass).
- **5 real clippy lints** — a manual `split_once`, two `println!` literal-format-string warnings, a `map_or` that should be `is_some_and`, and `log_exec`/`fold_egress_log` defined after the test module in `audit.rs`.
- **`curl | bash` and `curl | sh` installs had no `pipefail`** (`images/base/Dockerfile`, `images/rust/Dockerfile`) — a failed download would have silently produced a broken image instead of failing the build, since dash (the default `RUN` shell) doesn't have `pipefail` and only the last command in the pipe was checked.
- **Node 20 is past end-of-life** (Node 20 LTS ended April 2026) — bumped to Node 22, which also required pinning `npm@11` instead of `npm@latest` (npm's newest major now requires Node ≥22.22/24.15).
- **Debian packages were installed without `apt-get upgrade`** — `debian:bookworm-slim`'s own base layer can lag Debian's current security snapshot; `libpcre2-8-0`'s two HIGH CVEs (already fixed upstream, just not pulled) are what this actually caught.
- **npm's own vendored dependencies** (`tar`, `minimatch`, `glob`, `cross-spawn`, `brace-expansion`, `pacote`, `sigstore`) carried multiple HIGH/CRITICAL CVEs, all with fixes already published — the Node 22 + npm 11 bump above resolved every one (verified: zero fixable HIGH/CRITICAL findings across all five images after, via `trivy --ignore-unfixed`).
- **`tests/e2e.sh`'s own cleanup trap never deleted the `claude-state` volume** (added earlier for the Claude Code auth fix) or matched the real export-bundle filename pattern — both fixed; four `$? `-after-the-fact exit-code checks refactored to capture-then-check (shellcheck `SC2181`); the bundle-discovery `ls -t | head -1` replaced with a pure-bash glob (shellcheck's suggested `find` doesn't work reliably in every shell environment — verified directly, see the commit history around this point).

## What's deliberately ignored, and why

Real problems got fixed above; these are narrow, documented, non-default suppressions — see `.hadolint.yaml` and `.gitleaks.toml` for the exact list and reasoning inline:

- **hadolint**: unpinned `apt-get`/`npm`/`pip` package versions (pinning would freeze known-vulnerable versions instead of tracking upstream fixes — `trivy`'s CVE scan is the actual gate on that), non-numeric `USER` (named users, not a cross-image UID-mapping scenario this project has), and `DL3006` on `FROM ${ARG}` (hadolint can't statically verify a tag through an ARG default, even though `images/build.sh` always passes one explicitly).
- **gitleaks**: three fake, deliberately-secret-shaped strings — two are `cli/src/audit.rs` unit-test fixtures for `redact()`/`redact_argv()` themselves, one is the illustrative `canary_token` in `policies/moor.manifest.example.yaml`.
- **trivy**: run with `--ignore-unfixed` — the vast majority of a `debian:bookworm-slim` image's CVE surface has no fix published yet (Debian's security tracker marks it `affected`, `fix_deferred`, or `will_not_fix`); failing CI on those would be permanent, unactionable red and would just teach people to ignore it. The gate is "is there a fix available that we haven't applied" — which, after the fixes above, is currently zero across all five images.
- **cargo-deny `multiple-versions`**: left at `warn` (`syn` v2 and v3 both appear, from `windows-*` crates vs. `clap_derive`/`serde_derive`/`wasm-bindgen` — an ordinary transitive-dependency-graph reality, not something pinning would fix without vendoring patches).

## Known local-verification gaps

Two things could not be fully verified via `act` (GitHub's real `ubuntu-latest` runners don't have these limitations — both are documented `act` limitations, not gaps in the workflow):

- `secret-scan`'s SARIF-artifact upload step fails under `act` with a path-resolution error specific to how `act` implements `actions/upload-artifact` locally. The actual gitleaks scan logic — the part that matters — ran correctly and reported "no leaks found" against the real `.gitleaks.toml` config.
- Nothing else — `rust`, `rust-security`, `shell-lint`, `docker-lint`, `image-scan`, and `e2e` all ran to completion and passed under `act`, including real `docker build`/`docker compose`/`docker exec` against the host Docker daemon (`act` passes through the host's Docker socket rather than sandboxing it).

`image-scan` and `e2e` each run `./images/build.sh` independently (parallel jobs, not shared state) — a deliberate simplicity-over-speed tradeoff rather than plumbing image artifacts between jobs.

## Verified against real GitHub Actions, not just `act`

After the `act` pass above, this workflow was pushed and watched run for
real (`gh run watch`) — all 11 jobs passed, including `secret-scan`,
which is the one job `act` couldn't fully verify locally. The only
leftover issue was a cosmetic "Node.js 20 is deprecated" annotation on
several third-party actions; `actions/checkout` was bumped to `v5`
(confirmed via its `action.yml`: `v4` still declares `node20`, `v5`+
declares `node24`) to clear it. `rustsec/audit-check@v2` still declares
`node20` with no newer major published — nothing to bump to; it's a
non-blocking warning, not a failure, until RustSec cuts one.
`gitleaks/gitleaks-action` has a `v3` that does declare `node24`, but
its `action.yml` also newly carries commercial EULA/license-agreement
language that `v2` doesn't — deliberately staying on `v2` (already
verified working, and confirmed free for individual use in the actual
run's own log output) rather than adopt an ambiguous licensing change
for a cosmetic warning.
