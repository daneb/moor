# Threat model

## What moor is protecting

An LLM coding agent (Claude Code today; others may be added later) runs
inside a container, driven by [keel](https://github.com/daneb/keel). keel
has no sandboxing of its own — it trusts the host completely and will
execute any shell command a repo's `.keel/keel.toml` or the agent itself
asks it to. moor's job is to make that assumption safe by making sure
"the host" the agent sees is never the real Mac Mini.

## Assets

- The host OS, its filesystem, its other repos and credentials, and
  anything else running on the Mac Mini (browser sessions, keychain,
  other containers, the network the Mac is on).
- Secrets a project's sandbox legitimately needs (a GitHub token, an
  Anthropic API key, package registry tokens).
- The integrity of the audit trail itself — a compromised sandbox must not
  be able to rewrite or silence its own audit log.
- Other moor projects — a compromised sandbox for project A must not
  reach project B's volume, network, or secrets. Since `moor ask`/`moor
  studio`, that includes conversational context: a turn against project A
  must not carry any part of project A's session into project B's agent.
- The human checkpoints keel defines. `keel approve` is the decision the
  whole pipeline is built around; an agent that can advance its own work
  past it has made the rest of the model decorative.
- The operator's own terminal. Once moor renders an agent's reply, that
  reply is untrusted bytes being drawn on a real screen.

## Trust boundaries

```
Untrusted  →  cloned repo content, `.keel/keel.toml`, agent output, anything
              the agent writes to disk or emits on stdout, every MCP
              tool-call argument the agent sends to `moor-keel-mcp`
Sandboxed  →  the container: agent CLI, keel, the toolchain, the workspace
              volume, the sandbox's own process tree
Trusted    →  the host: the moor CLI process, ~/.moor/, Docker
              Engine itself, the egress proxy's allow-list source of truth
```

A cloned repository is untrusted input the moment it exists inside the
sandbox — same status as the SECURITY.md warning in keel itself. moor
does not try to make repo content trustworthy; it makes the blast radius
of trusting it equal to "one throwaway container," never "the Mac Mini."

## What counts as a breakout (v1 definitions)

1. **Filesystem breakout** — any process inside the sandbox reads or
   writes a path outside its own container's namespace: the host
   filesystem, another project's volume, or the Docker socket. Prevented
   structurally: no bind mounts, `docker.sock` never mounted, no shared
   volumes between projects, read-only rootfs.
2. **Network breakout** — any outbound connection that does not go through
   the project's egress proxy, or that the proxy allows but shouldn't
   (a domain not on `moor.yaml`'s `egress.allow` list). Prevented by
   the custom bridge network having no other route to the internet, and
   detected by the proxy's own access log plus `moor selftest`.
3. **Secret exfiltration** — a secret value (API key, GitHub token)
   leaving the sandbox by any channel other than its intended use (e.g.
   TLS to `api.anthropic.com`, or `git push` to `github.com`). Detected by
   the canary-token watch in the egress proxy: a decoy secret is planted
   alongside real ones, and the proxy flags any request — allowed
   destination or not — whose payload contains that literal string.
4. **Privilege escalation inside the container** — the agent process
   gaining root or new Linux capabilities it didn't start with. Prevented
   by `cap_drop: [ALL]`, `no-new-privileges`, non-root UID from container
   start, default seccomp profile.
5. **Escaping the agent's own verb surface** — the agent reaching a keel
   verb it was not given, `keel approve` above all. This one needs two
   controls, and only the first is structural:

   - The MCP server moor ships into the sandbox (`mcp/`, see
     [MCP.md](MCP.md)) defines no advancement verb, so `keel approve` is
     not on the agent's tool surface. Each tool's argv is a compiled-in
     verb plus at most a slug that passes `manifest::validate_name` and a
     gate id matched against a fixed table, so a tool argument cannot
     smuggle a second verb into the invocation.
   - **A shell bypasses all of that**, so every role also passes
     `--disallowedTools Bash BashOutput KillShell KillBash Task
     SlashCommand`. This is necessary, not belt-and-braces: `--allowedTools`
     is an auto-approval list, and measured against a live sandbox, Claude
     Code runs `Bash` when granted only `Read Glob Grep`, under every
     permission mode, recording no denial. Before the deny list existed,
     the `build` role reached `keel approve --help` through `Bash` and
     quoted it back.

   The deny list is exactly the kind of control ADR-0003 wanted to avoid —
   a list moor must keep current against Claude Code's tool surface. That
   is a property of Claude Code's permission model, not a choice; moor
   over-denies deliberately, since naming a tool that does not exist costs
   nothing and missing one that does costs the checkpoint. `moor recipe`'s
   agent steps carry the same deny list for the same reason.
6. **Addressing the operator's terminal** — an agent reply containing
   escape sequences that move the cursor, rewrite lines already drawn,
   set the window title, or drive an OSC handler. This is the one path
   the container boundary does not cover, because the bytes are *supposed*
   to leave the sandbox: the sandbox constrains what the agent can
   execute, not what it can say. Prevented at the point of drawing:
   everything `moor studio` renders from a turn passes through
   `studio::render::sanitize`, which strips every ANSI/CSI escape, every
   OSC sequence (including OSC-8 hyperlinks, and both BEL and ST
   terminators), two-character escapes like `ESC c`, and every C0 control
   except newline and tab — `\r` included, since a lone carriage return
   rewrites the line just drawn. `frame()` is a pure function, so what
   the console would draw is asserted on directly in tests rather than
   eyeballed.
7. **Hijacking session continuity** — an agent steering the next turn at
   a session that isn't its own. Prevented by keeping the session id
   host-authoritative: it is read from the turn's parsed `session_id`
   field only, stored at `~/.moor/projects/<name>/session` (a path no
   container has mounted) only after it validates as a UUID, and
   re-validated on the way out. A reply that prints a well-formed UUID in
   its *text* is never consulted, because the text is never parsed for
   one.
8. **Audit tampering** — the sandbox writing to or deleting
   `~/.moor/projects/<name>/audit/*`. Prevented structurally: that
   directory lives on the host and is never mounted into any container;
   everything written there is written by the host-side `moor`
   process itself, not by the container. The audit trail itself
   (`audit/chain.jsonl`) is additionally a SHA-256 hash chain — each entry
   commits to the hash of the entry before it, so even someone with
   direct filesystem access to `~/.moor/` (not the sandbox — the
   *operator's own machine*) can't edit, delete, reorder, or insert an
   entry without `moor audit <name> --verify` detecting exactly where
   the chain breaks. The format is keel's `keel.chain/1`, so keel's own
   `keel chain verify` is a second, independent verifier that shares no
   code with moor. This defends against accidental corruption and
   against a compromised process on the host with file access but not
   the ability to observe every command as it ran; it does not defend
   against someone who controls the `moor` binary itself (see
   "explicitly out of scope" below).

## What a turn leaves behind, and where

Two files, deliberately different in kind:

- `audit/chain.jsonl` gets one `agent-turn` entry per turn — role,
  granted tool set, session id, and SHA-256 hashes of the prompt and
  response. **No conversation text, and denials as a count only.** The
  reason is disclosure, not size: `moor audit --export` bundles this file
  wholesale for review, so anything in it travels wherever that bundle
  goes. The entry is written before the operator is shown anything, so a
  turn cannot be read and then quietly not recorded.
- `transcript.jsonl` gets the full prompt and response, passed through
  `audit::redact` with the project's declared secret names first — the
  same masking `log_exec` applies to argv, with the same stated limits
  (it only catches values the `moor` process itself had resolved at write
  time). An agent can echo a secret it was given back at you; this is
  what keeps that out of a file at rest.

Neither path is mounted into any container.

## Why not a git pre-push hook for detecting code leaving the sandbox

An earlier version of this plan considered baking a `pre-push` git hook
into the base image to log every push. It's not used: a git hook runs
*inside* the sandbox, under the same account that controls its own git
config — the agent (or a cloned repo's `.gitattributes`/`core.hooksPath`
override) can simply reconfigure or bypass it, so it would be a control
that looks like a boundary but isn't one. Instead:

- `moor run <project> -- git push ...` is recorded as `kind: "push"` in
  the audit chain by the CLI itself (host-side, so it's a real signal,
  not something the sandbox can suppress), with the ref and the commit it
  resolved to just before the push. The commit comes from a separate
  `git rev-parse` exec, so it names what the repository held, not what the
  agent claims it pushed. It only sees commands run through `moor run`,
  not ones typed inside an interactive `moor shell` session.
- The **egress gateway's own log**, folded into the audit chain by
  `moor audit`, is the reliable signal: every connection to
  `github.com` is recorded from *outside* the sandbox, and nothing inside
  the sandbox can prevent or edit that record. It can't distinguish a
  push from a fetch/clone (TLS isn't intercepted — see
  proxy/README.md) but it can't be lied to about whether the connection
  happened at all.

## The posture attestation and keel's sink

keel runs inside the sandbox and cannot inspect the box it runs in, so
two things cross the boundary for it, neither through a bind mount:

- **The attestation goes in, read-only.** moor derives it on the host from
  `docker inspect` and writes it as root into `/run/moor`, a tmpfs mounted
  root-owned `0755`, as a `0444` file. The `agent` user can read it and
  cannot replace it (`tests/e2e.sh` tries). Its SHA-256 is in the host
  chain, so a copy keel reports back can be compared against what moor
  actually wrote. It is a record of Docker's configuration for the
  container, not proof about the kernel underneath: a runc escape would
  not show up in `docker inspect`. That is still out of scope, below.
- **keel's payloads come out, as claims.** keel writes to
  `/run/moor-sink`, which the agent can also write to. Everything folded
  from it is marked `source: "sandbox"`: moor records faithfully what the
  sandbox said and when moor saw it, but cannot make it true. An agent
  could append a fake `gate` payload or rewrite lines moor has not folded
  yet. It cannot alter anything already folded, because that is in the
  host chain. moor folds after every exec to keep that window short, and
  a line that doesn't parse is recorded as `sink_malformed`, so breaking a
  line doesn't hide it. The tmpfs empties when the container restarts;
  unfolded lines are lost then, the same as the egress log.

## Explicitly out of scope for v1

- Kernel-level container-escape 0-days (runc/containerd vulnerabilities).
  Mitigated only by staying current on Docker/OrbStack updates; a stronger
  runtime (gVisor) is deferred to Phase 9.
- Supply-chain compromise of the base image's own dependencies (a
  malicious `git`, `curl`, or `cargo install keel-harness` payload at
  image build time). Mitigated by pinning versions and building images on
  the host, not letting a sandbox rebuild its own image.
- Physical access to the Mac Mini.
- A malicious *moor* CLI binary itself — the CLI is trusted; if it's
  compromised, the whole model fails. Treat `cli/` changes with the same
  scrutiny as anything else that runs unsandboxed on the host. The same
  goes for `mcp/`, which runs *inside* the sandbox but defines the verb
  surface the agent is confined to.
- **The agent CLI's own output shape.** `images/base/Dockerfile` installs
  `@anthropic-ai/claude-code` unpinned, and moor's failure detection keys
  on the JSON that `--output-format json` returns (`is_error`). That
  shape can move without warning; pinning it is its own deferred change.
  See [IMAGES.md](IMAGES.md).
- **Serialising a turn against a running recipe.** `moor ask`/`moor
  studio` and `moor recipe` can both write one workspace and nothing
  currently orders them. Deferred, not dismissed — it needs a lock with
  a defined staleness rule.
- **`$EDITOR` itself.** `moor studio`'s artifact edit reads a file out of
  the container, opens it in whatever `$EDITOR` names on the host, and
  writes it back over `docker exec -i` (no bind mount, in either
  direction). The editor runs on the host with the operator's own
  privileges, on content that came from the sandbox — same status as
  opening any untrusted file in your own editor.
- Multi-tenant use (multiple distinct humans sharing one moor
  install). v1 assumes a single operator.

## Why containers, not a VM per project

A full VM per project would be stronger isolation but far more friction
for "ideation to shipped in one sitting." Docker's namespace/cgroup
isolation, combined with no bind mounts, no host network mode, and a
locked-down egress path, is judged sufficient for the threat model above —
an agent that misbehaves inside its container loses at worst that one
container's volume and whatever the egress proxy let it reach, never the
host. If that judgment turns out wrong in practice (a real breakout is
observed), the response is Phase 9's stronger runtime, not abandoning the
container model.
