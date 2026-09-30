# Architecture

```
Host (Mac Mini, OrbStack/Docker)
 └─ moor (Rust CLI)                     ← the only thing you run on host
     ├─ ~/.moor/projects/<name>/moor.yaml   (manifest: allowlist, limits, secret refs)
     ├─ ~/.moor/projects/<name>/audit/*.jsonl    (host-only-writable, append-only)
     ├─ ~/.moor/projects/<name>/posture.json     (host copy of the attestation keel reads)
     ├─ ~/.moor/projects/<name>/session          (agent session id — host-authoritative)
     ├─ ~/.moor/projects/<name>/transcript.jsonl (turn text, redacted at rest)
     └─ docker compose per project:
         ├─ <name>-sandbox   container   ← keel + agent CLIs + toolchain run here
         │     workspace = named volume <name>-workspace (no host mount)
         │     network   = <name>-net (custom bridge, egress via proxy only)
         │     non-root user, read-only rootfs, cap-drop ALL, no-new-privileges,
         │     seccomp default, pids/mem/cpu limits, no docker.sock
         │     claude ──stdio──► moor-keel-mcp ──► keel gate / keel next
         │     /run/moor       tmpfs, root-owned 0755: posture.json (read-only to agent)
         │     /run/moor-sink  tmpfs, 1777: keel's chain payloads, folded by moor
         └─ <name>-egress    container   ← forward proxy, domain allowlist,
               logs every request, holds the canary token watch
```

## What moor is built from

moor's own commands are all you need day to day. Underneath, it puts
together a handful of tools, and this is where they are named:

| Piece | What it does | Where it runs |
| --- | --- | --- |
| **Docker** (via OrbStack on macOS) | Runs each project's sandbox and egress containers, via `docker compose`. | Host |
| **Claude Code** (`claude`) | The coding agent. | Sandbox |
| **[keel](https://github.com/daneb/keel)** | The spec → plan → build pipeline: specs, the checks on them (G0 for a spec, G1 for a plan, G2/G2.5/G3 after a build), approvals, build runs, and the evidence of all of it. | Sandbox |
| **`moor-keel-mcp`** | The MCP server that is the agent's only route to keel: it can run checks and ask what's next, but cannot approve. See below and [MCP.md](MCP.md). | Sandbox |
| **tinyproxy** | The egress proxy: a domain allowlist, and a log of every request. | Egress container |

How moor's commands map onto keel, for when you need to look underneath:

| moor | keel, inside the sandbox |
| --- | --- |
| `moor spec new <name>` | nothing: writes a starter spec file on the host |
| `moor spec push <file>` | `keel spec new` (first time only, so the sandbox assigns the id), then writes the file to `.keel/specs/<name>/spec.md` and runs `keel gate g0` |
| `moor next` | `keel next --json`, turned into one step and one command |
| `moor go` | whichever of `keel gate g0`, `keel plan`, `keel gate g1` and `keel run` the spec is waiting on; for a rejected or stale approval, the `approval.recheck` command `keel next --json` names (keel 0.11+), run once |
| `moor approve` / `moor reject "why"` | `keel approve <spec> --stage <stage>` (with `--reject --note` for a rejection) |
| `moor go --check` | at the build step, `keel run <spec> --no-driver`: re-checks the work already in the sandbox without the agent |
| `moor view <spec> <artifact>` | reads `.keel/specs/<spec>/{spec,plan,tasks}.md`; `report` runs `keel report <spec>` |
| `moor ship` | no keel command: commits the approved spec's change, its spec folder and its runs on `moor/<spec>` in the sandbox, pushes from the sandbox, opens the pull request from the host with `gh`, and switches the sandbox back to its trunk |
| `moor new` / `moor import` | `keel init` in the new workspace |
| `moor bundle` | `keel export --chain` and `keel bundle verify`, in throwaway containers |
| `moor recipe` | the whole pipeline, as ADR-0005 describes |

After a rejection, `moor next` shows who rejected it and the reason, from
`keel next --json`'s `approval` object. Both come from files the agent can
write, so moor strips control characters before printing them and never
puts them into a command it suggests; the one value it embeds is the
spec's slug, and only once it has a slug's shape.

`moor spec push` sends text one way only, host to sandbox. The one thing
it reads back is the spec's sandbox-assigned `id` (and `schema`), accepted
only in the exact shapes `SPEC-<digits>` and `keel.spec/<digits>`, so a
compromised sandbox has no way to get its own text into a file on the
host that a host-side AI assistant might then read.

For anything the guided commands don't cover, `moor keel <args>` runs
`keel <args>` in the project's sandbox, logged like any `moor run`. It is
left out of `moor --help` on purpose: it's an escape hatch, not the way to
work. keel's own documentation covers its commands and its credential
check (which decides whether `keel run` drives the agent or falls back to
`--no-driver` mode).

## Components

### `moor` (host, trusted)

A Rust CLI (`cli/`) that is the only thing the operator runs directly on
the Mac Mini for project work. It shells out to `docker`, `docker compose`
and `gh` via `std::process::Command` — the same subprocess pattern keel
uses for its drivers (own process group, captured stdout/stderr, no shell
interpolation of untrusted strings). It owns:

- `~/.moor/projects/<name>/moor.yaml` — the per-project manifest:
  egress allow-list, resource limits, referenced (not valued) secret
  names, base image choice.
- `~/.moor/projects/<name>/audit/*.jsonl` — append-only audit log,
  written only by the host process. No container ever has this path
  mounted.

### Sandbox container (untrusted workload)

Built from `images/base` (+ a language layer from `images/<lang>`).
Contains: `keel`, the configured agent CLI (`claude`, later others), git,
common dev tools. Runs as a fixed non-root UID. Its only writable
filesystem is a named Docker volume (`<name>-workspace`) plus `/tmp`
(tmpfs) and a small package-cache overlay — the container root filesystem
itself is read-only. It has **no bind mount to any host path** and never
sees `docker.sock`.

keel runs entirely inside this container and drives the agent CLI exactly
as it does on a bare host today (see keel's `SECURITY.md` — keel assumes
a trusted host and does no sandboxing itself; this container *is* that
trust boundary instead of the Mac Mini).

See [IMAGES.md](IMAGES.md) for what is in each image layer, what is
pinned and what is not, and the three build mechanics (apt upgrade,
`pipefail`, volume mount-point ownership) that are easy to get wrong.

### The agent's route to keel (`moor-keel-mcp`, in the sandbox)

A third binary ships inside the sandbox: `moor-keel-mcp`, built from
this repository's `mcp/` crate. It is an MCP server — hand-rolled
JSON-RPC 2.0 over newline-delimited stdio, `serde_json` only — that
`claude` spawns as a child process and reaches over pipes. It exposes
keel's *verification* verbs as tools (`keel_gate`, `keel_next`) and
nothing else.

`keel approve` is not a tool it defines, so advancing a spec past a human
checkpoint is absent from the agent's tool surface rather than denied by
a configuration file someone has to keep current. Each tool's argv is a
compiled-in verb plus, at most, a slug that passes
`manifest::validate_name` and a gate id matched against a fixed table.

No socket is opened and nothing crosses a network in either direction.
See [MCP.md](MCP.md) for the wire format, the role/tool table, and how to
probe a live sandbox's tool list yourself.

### Egress container (untrusted-facing, moor-controlled)

A forward proxy that is the sandbox's only route to the internet
(`HTTP_PROXY`/`HTTPS_PROXY`, plus network-level enforcement so a process
can't just ignore the env vars). Allows CONNECT/requests only to hostnames
listed in the project's `moor.yaml`. Does not terminate TLS — it
allow-lists by SNI/CONNECT target, so the agent's connection to
`api.anthropic.com` or `github.com` stays end-to-end encrypted. Logs every
request (domain, verdict, size, timestamp) and watches every request body
for the planted canary token.

### Audit pipeline

One hash-chained, append-only file per project:
`~/.moor/projects/<name>/audit/chain.jsonl`, in keel's `keel.chain/1`
format ([keel ADR-0001](https://github.com/daneb/keel/blob/master/docs/decisions/0001-one-chain-runtime-writes.md)):
keel owns the format and the verifier, and moor, as the host process no
sandbox can reach, is the only thing that writes it. Every entry has a
`kind` (`exec`, `push`, `egress`, `tripwire`, `tripwire-check`,
`recipe-event`, `agent-turn`, `studio`, `attest`, `legacy_seal`, and
whatever keel folds in through the sink, such as `gate` and `approval`),
`writer: "moor"`, a timestamp, and a `hash` that commits to the entry's
own contents plus the previous entry's hash: a Merkle-style chain, not
just a log file. The hashing is keel's, byte for byte, so `keel chain
verify` accepts a moor chain with no moor-specific knowledge. A chain
written before the switch is moved to `chain.legacy.jsonl` unchanged, and
the new chain opens with a `legacy_seal` entry that commits to its head.
Six things append to it:

1. **Exec entries** — every `moor run`/`shell` invocation (redacted
   argv, exit code), written directly by the CLI as it happens. A command
   that looks like `git push` is recorded as `kind: "push"` instead of the
   generic `exec`, with the ref it pushed and the commit that ref resolved
   to just before the push, since that's the one channel through which
   code actually leaves the sandbox for real.
2. **Egress entries** — folded in from the egress gateway's own access
   log by `moor audit` / `moor selftest` (idempotent — a
   `.egress-offset` checkpoint tracks how much has already been folded).
   A request to the reserved canary domain folds in as `kind: "tripwire"`
   instead of routine `egress`.
3. **Tripwire-check entries** — a marker written each time `moor
   selftest`'s active breakout battery runs (canary-domain reachability,
   read-only-filesystem write attempt, `docker.sock` presence).
4. **Agent-turn entries** — one per `moor ask`/`moor studio` turn,
   carrying the role, the granted tool set, the session id, and SHA-256
   hashes of the prompt and the response. **Hashes, not text**: the chain
   is exported wholesale by `--export`, so conversation content in it
   would leak through every bundle, and `permission_denials` is recorded
   as a count for the same reason. The entry is appended *before* the
   operator is shown anything, so there is no path to a response that
   skips the record. Full text goes to `transcript.jsonl` instead, passed
   through `audit::redact` first — see [MCP.md](MCP.md).
5. **The attestation** — after `compose up`, moor reads the running
   sandbox with `docker inspect` and derives a `keel.posture/1`
   attestation: read-only root, no bind mounts, non-root user, all
   capabilities dropped, no-new-privileges, not privileged, internal-only
   networks. Anything inspect does not establish is `unproven`, never
   assumed. moor writes it as root into `/run/moor/posture.json`, keeps a
   host copy at `posture.json`, and appends `kind: "attest"` with its
   SHA-256. keel reads it through `KEEL_RUNTIME_ATTESTATION` and blocks a
   run whose required properties are not `proven`.
6. **keel's own evidence** — keel inside the sandbox never writes a chain.
   It appends payloads (gate verdicts, approvals, run start and end) to
   `KEEL_CHAIN_SINK`, a tmpfs at `/run/moor-sink`. After every exec, and on
   `moor audit`, moor pulls new lines with `docker exec cat` (tracked by
   `.sink-offset`, like the egress fold) and appends each under its own
   `kind` with `source: "sandbox"`. A line that isn't a payload is kept as
   `sink_malformed` rather than dropped.

`moor audit <name> --verify` recomputes the whole chain, and a sealed
legacy chain with the hash it was written under, and reports exactly which
entry (if any) has been edited, deleted, reordered, or forged — see
docs/THREAT-MODEL.md's "audit tampering" section. keel's `keel chain
verify` checks the same file independently.
`moor audit <name> --export <dir>` bundles `chain.jsonl` plus keel's
own exported run bundles (`.keel/bundles/keel-<run-id>.tar.gz`, written by
`keel export` and pulled out of the workspace volume via `docker cp` —
that data is keel's job, not moor's, so moor only ever reads it,
never generates it) into one `tar.gz` for review.

`moor bundle [-p <name>] [run] --out <dir>` builds the one keel bundle an
auditor can verify alone. keel inside the sandbox never holds the chain, so
a bundle it exports there has none and verifies as blocked. moor folds
keel's sink, then runs `keel export --chain` in a **throwaway** container
from the project's image: `--rm`, `--network none`, the workspace's named
volume, and the host chain on stdin. It's never the sandbox, because the
agent could hand keel a chain of its own there. The archive streams back
byte for byte. A second throwaway container, with no network and no volume,
runs `keel bundle verify` on it, and `moor bundle` exits with keel's
verdict.

Secret values known to the `moor` process's own environment (per the
manifest's `secrets:` list) are redacted from every chain entry before
it's written — see `audit::redact` and its stated limits in
docs/THREAT-MODEL.md.

### Where secret values actually come from

`moor up`/`run`/`shell` resolve each name in the manifest's `secrets:`
list in this order, before doing anything else:

1. Already set in the operator's own shell (`export ANTHROPIC_API_KEY=...`
   before running the command) — used as-is.
2. Otherwise, the macOS Keychain, under a service name scoped to that one
   project+secret pair (`moor secrets set <project> <name>` writes
   one; `moor secrets status <project>` shows where each currently
   resolves from). This is what makes ongoing use practical — you set a
   project's secrets once, and every future `moor up` just works,
   with nothing to re-export each session.
3. Otherwise, left unset — `docker compose`'s `${VAR:-}` substitution
   leaves it blank in the container, same as today.

Either way, the resolved value is also set in the `moor` process's own
environment for the rest of that invocation, so `audit::redact` can find
and scrub it — this matters specifically because a value that only ever
existed in Keychain (never exported to a shell) would otherwise be
invisible to redaction in a *different* `moor run` invocation than the
one that first resolved it.

### Console (`moor studio`, host)

One local terminal process over every project at once: which sandboxes
are up (one `docker ps` for all of them, not one per project), what stage
each spec is at according to that project's own `keel next --json`, a
conversational turn with any of them, and stage approval behind two
distinct keypresses. It opens no socket, binds no port and runs no
daemon; it drives the same `docker exec` path as everything else and
records through the same chained-audit path, writing no console-only log.

Two properties are load-bearing there rather than incidental:

- **Agent text is untrusted bytes being drawn into the operator's
  terminal.** The sandbox protects the host from the agent's
  *execution*; it does nothing about a reply that contains escape
  sequences which move the cursor, rewrite lines already drawn, or drive
  an OSC handler. Everything the console draws from a turn passes
  through `studio::render::sanitize` first, which strips every ANSI/CSI
  escape, every OSC sequence, and every C0 control except newline and
  tab (`\r` included — a lone carriage return rewrites the line just
  drawn).
- **Approval is two different keys, and can only name what keel named.**
  `a` arms it against the currently selected project and the slug keel
  itself reported; only `y` confirms, and any other key — including a
  second `a` — cancels. The argv run is keel's own `approve` command
  taken verbatim from `keel next --json`, and it is refused outright if
  it does not begin `keel approve`.

## Why no bind mount

The container never has a host path mounted into it — not the project
folder, not `$HOME`, nothing. This is the one place strict isolation was
chosen over convenience: project source lives only in a Docker-managed
volume, and is reached from the host only via `moor shell` (an
interactive `docker exec`) or by `git push` through the egress proxy. See
[THREAT-MODEL.md](THREAT-MODEL.md) for the reasoning.
