# Egress gateway

A default-deny forward proxy (tinyproxy) that is the only route out of a
project sandbox. `entrypoint.sh` renders `/etc/tinyproxy/filter` from
`allowlist.base.txt` plus the project's `moor.yaml` `egress.allow`
entries (passed in as `EXTRA_ALLOW_DOMAINS`) before starting tinyproxy.

## What this does and doesn't see

TLS is **not** intercepted — the proxy allow-lists by hostname (the
`CONNECT` target for HTTPS, the `Host` header for plain HTTP) and then
passes the encrypted stream through untouched. That means:

- **Domain-level allow/deny is fully enforced and logged**, for HTTP and
  HTTPS alike. This is the primary control.
- **There is no payload inspection.** Nothing scans request bodies for
  the planted canary token (`MOOR_CANARY_TOKEN`): tinyproxy has no such
  feature, and nothing else is in the path. Even if one were added, it
  could only see plain HTTP — almost everything a sandbox legitimately
  talks to (github.com, api.anthropic.com, package registries) is HTTPS.
  A credential leaving over an allowed HTTPS destination is not caught.
  The defenses against secret exfiltration are
  (a) the allow-list itself (a secret can only leave via a destination
  that's already permitted) and (b) never granting the sandbox a secret
  it doesn't need in the first place.
- **The canary *domain*** (a reserved hostname never added to any
  project's allow-list) works fully as designed regardless of TLS,
  because the CONNECT/Host lookup itself is visible to the proxy before
  any encryption happens.

If payload-level inspection over HTTPS is ever needed, it requires a
TLS-terminating MITM proxy with an injected CA trusted inside the sandbox
— a real tradeoff (breaks certificate pinning in some tools, adds a new
thing to secure: the proxy's own CA key) deliberately deferred rather than
taken silently. Revisit only if the domain-allow-list control proves
insufficient in practice.
