# The images

Five images, built by `images/build.sh` (`make images`):

```
debian:bookworm-slim
      │
      ├── moor/base ──┬── moor/node     (+ pnpm, yarn)
      │               ├── moor/rust     (+ build-essential, rustup stable)
      │               └── moor/python   (+ python3, venv, pip, pipx)
      │
      └── moor/egress    (tinyproxy — the default-deny forward proxy)
```

A project picks one at `moor new --image moor/<name>:latest`. `moor
import` picks one for you by sniffing the source repo (`Cargo.toml`,
`package.json`, `pyproject.toml`/`requirements.txt`).

## What `moor/base` contains, and why each thing is there

Built in two stages (`images/base/Dockerfile`).

**Stage 1 — `keel-build`**, on `rust:1-slim-bookworm`, exists only to
compile two binaries that the runtime stage copies out:

- `keel`, via `cargo install keel-harness --version "$KEEL_VERSION" --locked`
  from crates.io, at the version pinned by `ARG KEEL_VERSION`.
- `moor-keel-mcp`, via `cargo install --locked --path /build/mcp` from
  **this repository's own `mcp/` crate**. Built from source rather than
  fetched, so the tool surface inside the image can never lag the
  protocol the CLI on the host expects. See [MCP.md](MCP.md).

**Stage 2 — `runtime`**, on `debian:bookworm-slim`:

| in the image | why |
| --- | --- |
| `git`, `curl`, `jq`, `ripgrep`, `ca-certificates`, `gnupg` | the minimum an agent needs to work a repo |
| Node 22.x (NodeSource) | the Claude Code CLI is a Node program |
| `npm@11` | NodeSource's bundled npm is routinely behind on CVE fixes for its own vendored deps (tar, minimatch, glob) |
| `@anthropic-ai/claude-code` | the agent |
| `/usr/local/bin/keel` | the conductor; runs *inside* the sandbox, not on the host |
| `/usr/local/bin/moor-keel-mcp` | the agent's only route to keel |
| `/etc/moor/mcp-config.json` | how `claude` finds that server — root-owned, outside every volume, on a read-only rootfs |
| `/home/agent/.claude/settings.json` | pins Claude Code to its own `auto` permission mode ([ADR-0003](decisions/0003-claude-code-permissions.md)) |
| user `agent`, uid/gid 10001 | nothing runs as root from container start |

And deliberately **not** in it: no `docker` CLI, no `sudo`, no SSH
server, no host toolchain, and no auth material of any kind. An API key
or OAuth token is injected as an environment variable at `moor up` and is
never baked into a layer.

## Three build mechanics that are easy to get wrong

**`apt-get upgrade`, not just `install`.** `debian:bookworm-slim`'s base
layer can be older than Debian's current security snapshot, so packages
already patched upstream (libpcre2, for one) sit un-upgraded until
something forces it. CI's Trivy scan fails the build on any fixable
HIGH/CRITICAL CVE, and this is what keeps that check passing without
hand-tracking individual CVEs.

**`SHELL ["/bin/bash", "-o", "pipefail", "-c"]`.** Debian's default `RUN`
shell is dash, which has no `pipefail`. Without it, a failed `curl` in
`curl … | bash -` still lets the downstream command run on empty input
and exit 0 — silently producing a Node-less image instead of a failed
build.

**Mount points must exist in the image, owned by `agent`, before the
volume is ever mounted.** moor mounts named volumes at
`/home/agent/.cache` and `/home/agent/.claude`, and Docker copies a fresh
volume's initial ownership from whatever already exists at that path in
the image. Skip the `mkdir`/`chown` and the mount point is created
root-owned instead — which silently breaks every Bash tool call Claude
Code makes under the read-only, non-root, `cap_drop: ALL` rootfs, because
it needs to write to `/home/agent/.claude`. `claude-settings.json` has to
be `COPY`'d in for the same reason: it must be there *before* the
`claude-state` volume's first mount, or a fresh project simply doesn't
have it.

## The base image's build context is the repository root

```sh
docker build -t moor/base:latest -f base/Dockerfile ..    # from images/
```

Not `./base`, which is what it used to be. The base image compiles the
`mcp/` crate, so the context has to include it. Consequences:

- `COPY` sources in `images/base/Dockerfile` are repo-root-relative:
  `COPY mcp /build/mcp`, `COPY images/base/mcp-config.json …`.
- `/.dockerignore` keeps `.git`, `**/target`, `cli/`, `docs/`, `.keel/`
  and friends out of what gets sent to the daemon.
- The language layers and the proxy are unaffected — they still build
  from `./node`, `./rust`, `./python` and `../proxy`.

The upside of paying that cost: `make images` builds the MCP server that
is actually on disk, so the image and the CLI driving it cannot drift.

## What is pinned, and what is not

Pinned: Debian release (`bookworm-slim`), the Node major (22.x — Node 20
reached upstream end-of-life in April 2026, so it gets no security
patches at all, ever), the npm major (`npm@11`, not `@latest`, which has
already moved to requiring Node ≥22.22/24.15 and would break this build
the next time npm cuts a major), the Rust toolchain channel (`stable`),
`keel-harness` at an exact version (`ARG KEEL_VERSION` in
`images/base/Dockerfile`) with `--locked`, and `moor-keel-mcp` via `--locked`
against `mcp/Cargo.lock`.

**Bumping keel.** Change `ARG KEEL_VERSION`, or run
`KEEL_VERSION=x.y.z ./images/build.sh` to try a version first. The version
is part of the `cargo install` layer's cache key, so the rebuild installs the
new keel. It used to be unpinned, and a cached layer quietly kept the previous
release twice.

**Not pinned: `@anthropic-ai/claude-code`.** Every image build takes
whatever `npm install -g` resolves to. That is a real, named gap, not an
oversight: moor's own turn handling keys on the JSON shape
`--output-format json` returns (`is_error`, `session_id`), and that shape
can move without warning. Pinning it is its own change — an image
concern, not a protocol one.

## Building, scanning, and checking

```bash
make images        # build all five
make images-clean  # remove every moor-tagged image (leaves project volumes alone)
make hadolint      # lint every Dockerfile (needs: brew install hadolint)
make trivy         # CVE scan every built image, fixable HIGH/CRITICAL only
```

`make trivy` runs `--severity HIGH,CRITICAL --ignore-unfixed --exit-code 1`
against each image. `--ignore-unfixed` is deliberate and is also a known
limitation: Debian CVEs with no fix available yet stay in every image
indefinitely and the scan will never turn red for them. See
[CI.md](CI.md) for what each scanner has actually caught, and the README's
"Open concerns" for the ones still open — including that there is no image
signing, provenance or SBOM today.

Images are built on the host, never by a sandbox. A container cannot
rebuild its own image, which is what keeps image-build-time supply chain
exposure (a malicious `git`, `curl`, or `cargo install` payload) inside
the boundary described in [THREAT-MODEL.md](THREAT-MODEL.md).

## Checking what a live container actually got

The image is only half the story — the hardening that matters is applied
at `docker compose` time (read-only rootfs, `cap_drop: ALL`,
`no-new-privileges`, non-root, internal-only network, pids/mem/cpu
limits). `moor selftest <project>` re-checks all of it against the
**live** container rather than the manifest, and fails safe: a missing or
corrupted `docker inspect` field reads as FAIL, never as "all clear".

```bash
moor selftest my-app

# what the image itself carries
docker run --rm moor/base:latest keel --version
docker run --rm moor/base:latest claude --version
docker run --rm moor/base:latest cat /etc/moor/mcp-config.json

# the MCP server speaks only JSON-RPC on stdin — it has no --help
printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}' \
  | docker run -i --rm moor/base:latest /usr/local/bin/moor-keel-mcp
```
