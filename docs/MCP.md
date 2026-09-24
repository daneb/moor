# The transport: how moor and the agent talk to each other

Two channels, opposite directions, both riding `std::process::Command`.
Neither opens a socket, binds a port, or crosses a network.

```
┌─ host (trusted) ───────────────────────────────────────────────────┐
│  moor ask / moor studio                                            │
│      │                                                             │
│      │  ① outbound: docker exec + `claude --print`                 │
│      ▼                                                             │
└──────┼─────────────────────────────────────────────────────────────┘
       │
┌─ <name>-sandbox (untrusted) ───────────────────────────────────────┐
│      ▼                                                             │
│   claude (Claude Code CLI)                                         │
│      │                                                             │
│      │  ② inbound: MCP, JSON-RPC 2.0 over stdio                    │
│      ▼                                                             │
│   moor-keel-mcp ──► keel gate / keel next                          │
│                     (`keel approve` is not reachable — see below)   │
└────────────────────────────────────────────────────────────────────┘
```

Why this shape at all: `docs/ARCHITECTURE.md` already establishes that
moor "shells out to `docker`, `docker compose` and `gh` via
`std::process::Command`", and `proc::run_with_stdin_file` already pipes
bytes through `docker exec -i`. Both directions of this protocol ride
that same pattern rather than introducing a second one.

## ① Outbound: putting an instruction in front of the agent

`moor ask` (and `moor studio`, which calls the same code) runs one turn:

```
docker exec <name>-sandbox \
  claude --print --output-format json \
         [--resume <session-uuid>] \
         "<the prompt>" \
         --mcp-config /etc/moor/mcp-config.json \
         --allowedTools <tool> <tool> ...
```

That argument order is not cosmetic, and getting it wrong is the single
easiest way to break this. Four details are load-bearing, all four
verified against a live sandbox at Claude Code 2.1.270 rather than
assumed:

- **The prompt must come before *every* variadic flag.** `claude --help`
  marks these with `<thing...>`: `--allowedTools <tools...>`,
  `--disallowedTools <tools...>`, and — the one that bites —
  `--mcp-config <configs...>`. Each greedily absorbs every bare argument
  after it until the next flag, so a prompt placed after one is consumed
  as a tool name or a config path. `session::VARIADIC_FLAGS` lists them
  and `the_prompt_precedes_every_variadic_flag` asserts the ordering
  against that whole list, because this bug has now happened twice: first
  with `--allowedTools` (which `recipe::author_spec` documents), then with
  `--mcp-config`, which reported the prompt itself as a missing file —
  `MCP config file not found: /workspace/say ok` — and left `claude`
  writing nothing at all to stdout.
- **`--resume <session-id>` takes exactly one value**, so it sits with the
  single-value flags *ahead* of the prompt. Checked against `claude
  --help` and against a live resumed session, not inferred from the rule
  above.
- **`--output-format json` returns `session_id`, `result`, `is_error`,
  `num_turns` and `permission_denials`** — among some 25 fields; those
  five are all moor reads.
- **`subtype` is not a success signal.** A failed turn came back as
  `"is_error": true` alongside `"subtype": "success"`. `session::turn_failed`
  therefore keys on `is_error` (and on a non-zero exit status), never on
  `subtype`.

### When a turn produces nothing

`claude` writes its own refusals to **stderr** and nothing to stdout, so a
turn that never started looks, to a stdout-only reader, like an empty
string — and a JSON parser reports that as `EOF while parsing a value at
line 1 column 0`, which says nothing about the cause. `run_turn` therefore
captures the two streams separately (`proc::run_capture_split`) and
`session::unparseable_turn` turns the result into something actionable:

- stderr naming `/etc/moor/mcp-config.json` as not found means the
  container predates moor's MCP server, and the error says so along with
  the fix (`make images && moor up <project>`). Every project whose
  container was created before that image existed hits this until it is
  recreated.
- otherwise, empty stdout is reported as "`claude` produced no output",
  with stderr quoted verbatim.
- output that is present but unparseable keeps the parse error *and*
  appends stderr.

No shell is involved on the way in. The prompt is one `argv` element
handed to `docker exec`, so nothing in it is word-split, glob-expanded or
interpreted — however it is quoted, it arrives as one string.

### Roles are the tool boundary

`cli/src/session.rs` defines two roles. Each is a pair of lists, and the
second one is the one that does the work:

| role | granted (`--allowedTools`) | denied (`--disallowedTools`) |
| --- | --- | --- |
| `brainstorm` | `Read`, `Glob`, `Grep` | shell/subagent + network + `Write`, `Edit`, `MultiEdit`, `NotebookEdit` |
| `build` | + `Edit`, `Write`, `mcp__moor-keel__keel_gate`, `mcp__moor-keel__keel_next` | shell/subagent + network |

where shell/subagent is `Bash`, `BashOutput`, `KillShell`, `KillBash`,
`Task`, `SlashCommand`.

### `--allowedTools` is an auto-approval list, not a restriction

This is worth stating flatly because moor got it wrong first, and the
error was load-bearing. Granting `Read Glob Grep` and nothing else does
**not** withhold the rest. Measured against a live sandbox:

```
$ claude --print --output-format json "run the shell command 'id'…" \
         --allowedTools Read Glob Grep
→ uid=10001(agent) gid=10001(agent) groups=10001(agent)
  permission_denials: []
```

`Bash` ran, was not in the granted list, and was not recorded as a
denial. Under `--print` there is nobody to answer a permission prompt,
and **every** permission mode behaves the same way — `auto`, `manual`,
`dontAsk` and `plan` were each measured and each let it through. Only
`--disallowedTools` denies; with it, the agent reports having no shell
tool at all.

The consequence for channel ② below is direct: without a deny list, the
`build` role reached `keel approve --help` through `Bash` and quoted it
back. The MCP server keeps that verb off the *tool surface*, which is
still worth having, but it is the deny list that keeps it from being
reachable through a shell. Both are needed, and only one of them is
structural — see ADR-0008's correction.

### Session continuity is the host's, not the agent's

The `session_id` from a turn's JSON is stored on the host at
`~/.moor/projects/<name>/session`, and only after it validates as a UUID
(8-4-4-4-12 hex). The next turn resumes from that stored file.

Nothing in the agent's *response text* is ever consulted for this. A
reply that prints a perfectly well-formed UUID cannot redirect the next
turn at somebody else's session, because the text is never parsed for
one — `session::next_session_id` reads the parsed field and nothing else.
The file is re-validated on the way out too, so a hand-edited one is
ignored rather than resumed from. Nothing inside a sandbox has that path
mounted.

### Every turn is chained before you see it

`session::run_turn` appends one `agent-turn` entry to the project's
existing hash chain (`audit/chain.jsonl`) and writes the transcript
*before* it returns, so there is no code path to a response that skips
the record. The chain entry carries:

```json
{
  "project": "my-app",
  "role": "brainstorm",
  "tools": ["Read", "Glob", "Grep"],
  "session_id": "3f2504e0-4f89-11d3-9a0c-0305e82c3301",
  "prompt_sha256": "…",
  "response_sha256": "…",
  "result": "ok",
  "num_turns": 3,
  "permission_denials": 0
}
```

**Hashes, not text.** The chain is exported wholesale by
`moor audit --export`, so conversation content in it would leak through
every bundle. `permission_denials` is a count for the same reason: a
denial record carries the argument the agent was denied, which is
content.

The full text goes to `~/.moor/projects/<name>/transcript.jsonl`,
passed through `audit::redact` with the project's declared secret names
first — the same masking `log_exec` applies to argv. An agent can echo a
secret it was given back at you; this is what stops it landing in a file
at rest.

## ② Inbound: how the agent reaches keel

An agent that can build but can't run a gate has to be babysat. An agent
that can run `keel approve` can advance its own work past the human
checkpoints that are the whole point of keel. So the agent gets keel's
*verification* verbs and nothing else.

The first design for this was a scoped shell — `--allowedTools "Bash(keel
gate *)"` plus a moor-maintained allow/deny list of keel verbs. That is
exactly the control [ADR-0003](decisions/0003-claude-code-permissions.md)
had already rejected: a hand-written list that "would need moor to track
Claude Code's tool surface indefinitely and would rot every time that
surface changes." It rotted immediately — checked against the image's own
Claude Code 2.1.270, the real syntax is `Bash(git *)`, with a space, not
the colon form that draft assumed.

So moor ships its own MCP server into the sandbox instead.

### What it is

`mcp/` builds `moor-keel-mcp`, installed at
`/usr/local/bin/moor-keel-mcp`. Hand-rolled JSON-RPC 2.0 over
newline-delimited stdio, `serde_json` as its only dependency — no SDK, no
socket, nothing listening. `claude` spawns it as a child process and
talks to it over pipes; moor points `claude` at it with
`--mcp-config /etc/moor/mcp-config.json`:

```json
{
  "mcpServers": {
    "moor-keel": {
      "command": "/usr/local/bin/moor-keel-mcp",
      "args": [],
      "env": {}
    }
  }
}
```

That file is root-owned, mode 644, outside `/workspace` and outside every
mounted volume, on a read-only rootfs. The agent can read it and cannot
point it anywhere else, so the tool surface it gets is the one moor
shipped.

Three methods are implemented — `initialize`, `tools/list`, `tools/call`
(plus `ping`); anything else returns JSON-RPC error `-32601`. Protocol
revision `2024-11-05`. Notifications (no `id`) are handled by handling
nothing and answering nothing.

### The tool surface

```
$ printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}' \
  | docker exec -i my-app-sandbox /usr/local/bin/moor-keel-mcp
```

Two tools, and that is the whole list:

| tool | argv it can produce |
| --- | --- |
| `keel_gate` | `keel gate <g0\|g1\|g2\|g4> <slug>` |
| `keel_next` | `keel next --json <slug>` |

**`keel approve` is not a tool this server defines.** There is no
allowlist entry to maintain and no denial to enforce — the verb does not
exist in the agent's world. That is a structural control rather than a
configurable one, which is the distinction the README's Security section
draws. MCP itself is a protocol Anthropic maintains, rather than one moor
has to track.

### What can reach a child process's argv

Each tool's argv is a compiled-in `&'static [&'static str]` verb plus, at
most, two things:

- **A spec slug**, which must pass the same rule
  `manifest::validate_name` applies to project names — lowercase
  letters, digits and dashes, starting with a letter, 63 characters max.
  That rejects `spec; keel approve spec`, `--json`, `../../etc/passwd`,
  `spec$(id)` and anything with whitespace or a newline in it.
- **A gate id**, which is *matched* against a fixed table
  (`["g0", "g1", "g2", "g4"]`). On a match, the string pushed onto the
  argv is the `'static` one from that table — the caller's own bytes
  never reach the child at all.

An unexpected argument key is a hard error, not something ignored, so a
call cannot smuggle an extra argument alongside a valid slug and have it
quietly survive. There is no shell anywhere in this path either; the
argv goes straight to `std::process::Command`.

A rejected call comes back as tool *content* with `isError: true`, not as
a JSON-RPC transport error — the agent should read why and correct
itself, the way it reads a failing gate.

### Verifying it yourself

```bash
printf '%s\n%s\n' \
 '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}' \
 '{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}' \
 | docker exec -i <name>-sandbox /usr/local/bin/moor-keel-mcp
```

`tools/list` should return exactly `keel_gate` and `keel_next`, and the
word `approve` should not appear anywhere in the response.

The unit tests assert the same properties three ways — the tool names,
the compiled-in argv heads, and every argv that any accepted call can
produce:

```bash
cargo test --manifest-path mcp/Cargo.toml
```

## What this does *not* do

- **No streaming.** Turns are request/response via `--output-format
  json`. Incremental delivery (`--input-format stream-json`) is a
  distinct concern with a distinct failure mode.
- **No mutual exclusion against a running recipe.** A turn and a
  `moor recipe` can both write one workspace, and nothing currently
  serialises them. Deferred, not dismissed: it needs a lock with a
  defined staleness rule.
- **The Claude Code version is not pinned.** `images/base/Dockerfile`
  installs `@anthropic-ai/claude-code` unpinned, so the JSON shape
  `turn_failed` keys on can move without warning. See
  [IMAGES.md](IMAGES.md).

See [decisions/0008-agent-session-protocol.md](decisions/0008-agent-session-protocol.md)
for the decision record, and [THREAT-MODEL.md](THREAT-MODEL.md) for where
these two channels sit relative to everything else.
