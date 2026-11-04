---
id: PROD-0001
scope: repo
owner: human
verified_at: 2026-09-17
---

# Product

## Purpose

moor runs an AI coding agent (Claude Code, Copilot or Kiro) inside a locked-down Docker sandbox per project, and records what it does in a tamper-evident audit trail on the host.

## Users and stakes

One developer on a Mac using agents on code they do not fully trust. If moor misbehaves, the sandbox boundary or the audit trail stops meaning what it says, so changes to either need a test that proves the boundary, not one that mocks it.

## Out of scope

moor does not replace the agent, does not defend against a kernel escape (shared `runc` kernel, see ADR-0001), and does not run on Linux or Windows hosts. It is not a spec tool: the spec/plan/build pipeline is keel's, and moor only drives it.
