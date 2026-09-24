# Walkthrough: ideation to shipped

This is not a hypothetical — every command below was actually run against
a real GitHub repository (`daneb/isolator-sample-app` at the time, private;
renamed to [daneb/moor-sample-app](https://github.com/daneb/moor-sample-app)
along with this project itself — see
[ADR-0004](decisions/0004-rename-to-moor.md)) as the Phase 8 acceptance
test for moor. Two real bugs surfaced doing this and are called out
below, fixed in the same commit as this doc. The commands below still
show `isolator-sample-app` as the *local* project name, since that
wasn't renamed — only the GitHub repository it points at was.

## 0. Before you start

```bash
./images/build.sh                      # once
cd cli && cargo build --release        # once
export GITHUB_TOKEN=$(gh auth token)   # needed for --github (see step 1)
```

`GITHUB_TOKEN` isn't required for projects that don't use `--github`. When
it is set, it's picked up by `docker compose`'s `${GITHUB_TOKEN:-}`
substitution and lands in the *container's* environment — never as a CLI
argument, so it can't leak into `docker exec`/`ps` output or the audit
log (see the credential-helper note in step 1).

## 1. Ideation → a project

```bash
moor new isolator-sample-app --github --image moor/node:latest
```

This creates a private GitHub repo, a sandbox + egress container pair, and
runs `keel init` inside the sandbox. **Bug found here:** the very first
run of this command failed to clone — a freshly created private repo
needs credentials, and none were available inside the sandbox. Fixed in
`cli/src/commands/new_cmd.rs`: the clone now uses a git credential helper
that reads `$GITHUB_TOKEN` from the *container's own environment* at
clone time —

```
git -c credential.helper='!f() { echo username=x-access-token; echo "password=$GITHUB_TOKEN"; }; f' clone <url> .
```

— so the token is read by git's shell, never passed as an argument to
`docker exec` itself. Export `GITHUB_TOKEN` before `moor new --github`
and this works cleanly end to end.

## 2. Give the agent instructions

Everything from here happens inside the sandbox, via `moor run
<project> -- <cmd>` or an interactive `moor shell <project>`. This
walkthrough spells out `-- keel` in full for clarity, but day to day
`moor keel <args...>` is shorter for the same thing — it figures out
which project you mean on its own (see the README). Set
`[verify]` in `.keel/keel.toml` (the placeholder keel scaffolds is empty
— G2 blocks without it):

```toml
[verify]
build = "npm install --omit=dev --no-audit --no-fund"
test  = "node --test test/"
lint  = "node --check src/greet.js"
```

Then author a spec:

```bash
moor run isolator-sample-app -- keel spec new greet-name --scope 'src/**'
```

Fill in `.keel/specs/greet-name/spec.md` with one real EARS criterion and
a runnable oracle — this is the step where, in normal use, you'd point an
actual coding agent at the spec and let it do this (and the implementation
in step 3) instead of doing it by hand. This walkthrough was run without
`ANTHROPIC_API_KEY` set, so it used keel's `--no-driver` mode instead —
"make the change yourself, then gate it" — which is also exactly how you'd
review an agent's own proposed change before merging it.

```bash
moor run isolator-sample-app -- keel gate g0 greet-name
moor run isolator-sample-app -- keel approve greet-name --stage spec
moor run isolator-sample-app -- keel plan greet-name
# fill in plan.md's approach + rollback, and tasks.md's file list
moor run isolator-sample-app -- keel gate g1 greet-name
moor run isolator-sample-app -- keel approve greet-name --stage plan
```

**Real gate catch, not theater:** G1 failed the first time because the
task touched `test/greet.test.js`, which was outside the spec's declared
scope (`src/**` only) — exactly the kind of scope-creep G1 exists to
catch. Fixed by widening the scope to `src/**, test/**` in the spec and
re-gating, the same way a real reviewer would push back on an
under-scoped spec.

## 3. Build, then gate the real thing

```bash
moor run isolator-sample-app -- keel run greet-name --no-driver
```

This actually ran `npm install`, `node --test test/`, and `node --check
src/greet.js` **inside the sandbox** — not a simulation. **Second real
gate catch:** `npm install` generated `package-lock.json`, which G2's
`blast-radius` check correctly flagged as outside the declared scope.
Widened scope to include it, re-approved, re-ran — G2 and G2.5 passed
clean:

```
G2 pass — 9 passed, 0 failed, 0 blocked
G2.5 pass — 5 passed, 0 failed, 0 blocked
```

G3 needs a recorded human decision:

```bash
moor run isolator-sample-app -- keel approve greet-name --stage merge
```

## 4. Ship it

```bash
moor run isolator-sample-app -- git add -A
moor run isolator-sample-app -- git commit -m "feat: add greet(name)"
moor run isolator-sample-app -- git push -u origin main
```

The push succeeded — `github.com` is on the default egress allow-list, and
the credential helper set up in step 1 authenticated it. This is the one
real "code leaves the sandbox" event, and it shows up in the audit chain
tagged distinctly:

```json
{"schema":"keel.chain/1","kind":"push","writer":"moor","data":{"argv":["git","push","-u","origin","main"],"commit":"3f1c…","exit_code":0,"ref":"main",...},...}
```

## 5. Review the trail

```bash
moor audit isolator-sample-app          # folds in egress log, prints the chain
moor audit isolator-sample-app --verify # chain OK — 51 entries, no tampering detected.
moor audit isolator-sample-app --export /tmp
```

The chain covers the whole session: every `moor run`/`shell` command,
every widened-scope retry, the push, and the egress gateway's own
record of connections to `registry.npmjs.org` and `github.com` — folded
in automatically, not something anyone inside the sandbox could suppress.
**Third bug found here:** the export bundle's "keel evidence" step was
pointed at `.keel/store/evidence/`, which doesn't exist — keel's real
exported-run artifact lives at `.keel/bundles/keel-<run-id>.tar.gz`
(written by `keel export`). Fixed in `cli/src/commands/audit_cmd.rs` to
pull `.keel/bundles/` instead; re-verified the export actually contains
`keel-bundles/keel-2026-09-11-001.tar.gz` afterward.

```bash
moor selftest isolator-sample-app   # 8/8 static + 3/3 active breakout checks, still pass
```

## What this proved

- The full ideation → spec → plan → gate → ship → audit loop works
  end-to-end against a real GitHub repo, with real build/test/lint
  commands executing inside the sandbox.
- keel's gates caught two genuine scope violations during this run (an
  under-scoped test file, then an under-scoped lockfile) — the moor
  wrapper didn't have to add anything for that; it's keel doing its job,
  running where moor put it.
- Three real bugs in moor itself surfaced only by actually running
  this, not by reasoning about the design: the missing clone credential,
  and the wrong evidence-bundle path (found once, in the export step, but
  worth noting the audit chain's `.keel/store/evidence` string appeared
  in two places — the export code and its own printed help text — both
  fixed together). All three are fixed and re-verified in this same
  commit.
- The hash-chained audit trail held up under real, messy, iterative usage
  (not a clean scripted demo) — 51 organically generated entries, still
  verifying clean.
