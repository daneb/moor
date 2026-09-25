#!/usr/bin/env bash
# End-to-end verification against real Docker containers: builds the CLI,
# creates a throwaway project, and exercises every control the plan
# promises — hardening (moor selftest), egress allow/deny, the audit
# chain (folding, verification, a live tamper attempt, export), and the
# git-push audit tag. Exits non-zero if anything fails. Requires Docker
# (OrbStack or otherwise) running locally; does not touch GitHub.
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CLI="$ROOT/cli/target/debug/moor"
PROJECT="e2e-$$"
IMPORT_PROJECT="e2e-import-$$"
IMPORT_SRC="$(mktemp -d)/local-repo"
FAILURES=0

pass() { echo "  [PASS] $1"; }
fail() { echo "  [FAIL] $1"; FAILURES=$((FAILURES + 1)); }
step() { echo; echo "== $1 =="; }

indent() {
  local line
  while IFS= read -r line; do
    echo "         $line"
  done <<<"$1"
}

expect_success() {
  local desc="$1"; shift
  if "$@" >/tmp/moor-e2e-out.$$ 2>&1; then
    pass "$desc"
  else
    fail "$desc (expected success, got exit $?)"
    sed 's/^/         /' /tmp/moor-e2e-out.$$
  fi
  cat /tmp/moor-e2e-out.$$
  rm -f /tmp/moor-e2e-out.$$
}

expect_failure() {
  local desc="$1"; shift
  if "$@" >/tmp/moor-e2e-out.$$ 2>&1; then
    fail "$desc (expected failure, but it succeeded)"
  else
    pass "$desc"
  fi
  cat /tmp/moor-e2e-out.$$
  rm -f /tmp/moor-e2e-out.$$
}

assert_contains() {
  local desc="$1" haystack="$2" needle="$3"
  if grep -qF -- "$needle" <<<"$haystack"; then
    pass "$desc"
  else
    fail "$desc (did not find '$needle')"
  fi
}

# shellcheck disable=SC2329 # invoked indirectly via `trap cleanup EXIT` below
cleanup() {
  step "cleanup"
  "$CLI" down "$PROJECT" >/dev/null 2>&1 || true
  rm -rf "$HOME/.moor/projects/$PROJECT"
  docker volume rm "${PROJECT}-workspace" "${PROJECT}-cache" "${PROJECT}-claude-state" >/dev/null 2>&1 || true
  "$CLI" down "$IMPORT_PROJECT" >/dev/null 2>&1 || true
  rm -rf "$HOME/.moor/projects/$IMPORT_PROJECT"
  docker volume rm "${IMPORT_PROJECT}-workspace" "${IMPORT_PROJECT}-cache" "${IMPORT_PROJECT}-claude-state" >/dev/null 2>&1 || true
  rm -rf "$(dirname "$IMPORT_SRC")"
  rm -f /tmp/moor-e2e-out.$$ /tmp/"${PROJECT}"-audit-*.tar.gz
}
trap cleanup EXIT

step "build the CLI"
if (cd "$ROOT/cli" && cargo build -q); then
  pass "cargo build"
else
  fail "cargo build"
  echo "cannot continue without a working binary"
  exit 1
fi

step "build images (skipped if already present)"
for img in base node rust python egress; do
  if ! docker image inspect "moor/${img}:latest" >/dev/null 2>&1; then
    echo "  moor/${img}:latest missing — run ./images/build.sh first"
    exit 1
  fi
done
pass "all moor images present"

step "create project '$PROJECT'"
expect_success "moor new" "$CLI" new "$PROJECT" --image moor/base:latest

step "static hardening + active breakout battery"
SELFTEST_OUT=$("$CLI" selftest "$PROJECT" 2>&1)
SELFTEST_EXIT=$?
if [ "$SELFTEST_EXIT" -eq 0 ]; then pass "moor selftest exits 0"; else fail "moor selftest exits 0"; fi
indent "$SELFTEST_OUT"
assert_contains "selftest reports all checks passed" "$SELFTEST_OUT" "all checks passed"
assert_contains "selftest ran the canary-domain probe" "$SELFTEST_OUT" "canary domain is unreachable"
assert_contains "selftest ran the read-only-fs probe" "$SELFTEST_OUT" "root filesystem rejects writes"
assert_contains "selftest ran the docker.sock probe" "$SELFTEST_OUT" "docker.sock is not present"

step "egress: allow-listed domain succeeds"
expect_success "curl https://github.com" "$CLI" run "$PROJECT" -- curl -sS -o /dev/null --max-time 10 https://github.com

step "egress: non-allow-listed domain is blocked"
expect_failure "curl https://example.com" "$CLI" run "$PROJECT" -- curl -sS -o /dev/null --max-time 10 https://example.com

step "git push is tagged distinctly in the audit chain"
"$CLI" run "$PROJECT" -- git -C /workspace init -q >/dev/null 2>&1 || true
"$CLI" run "$PROJECT" -- git -C /workspace push origin main >/dev/null 2>&1 || true
CHAIN_FILE="$HOME/.moor/projects/$PROJECT/audit/chain.jsonl"
if grep -q '"kind":"push"' "$CHAIN_FILE" 2>/dev/null \
  && grep '"kind":"push"' "$CHAIN_FILE" | grep -q '"ref":"main"'; then
  pass "a git push attempt was recorded as kind=push with its ref"
else
  fail "no kind=push entry naming ref main in $CHAIN_FILE"
fi

step "one evidence chain: attestation, keel's sink, keel's verifier"
if grep -q '"kind":"attest"' "$CHAIN_FILE" 2>/dev/null; then
  pass "moor up attested the sandbox's posture into the host chain"
else
  fail "no kind=attest entry in $CHAIN_FILE"
fi
POSTURE_OUT=$("$CLI" run "$PROJECT" -- cat /run/moor/posture.json 2>&1)
assert_contains "keel can read the attestation inside the sandbox" "$POSTURE_OUT" '"schema": "keel.posture/1"'
expect_failure "the agent cannot replace the attestation" \
  "$CLI" run "$PROJECT" -- sh -c 'echo forged > /run/moor/posture.json'
# keel records G0's verdict through KEEL_CHAIN_SINK; moor folds it on the
# next exec, so the gate lands in the host chain marked as the sandbox's.
"$CLI" run "$PROJECT" -- keel init --yes >/dev/null 2>&1 || true
"$CLI" run "$PROJECT" -- keel spec new e2e-chain >/dev/null 2>&1 || true
"$CLI" run "$PROJECT" -- true >/dev/null 2>&1 || true
if grep '"kind":"gate"' "$CHAIN_FILE" 2>/dev/null | grep -q '"source":"sandbox"'; then
  pass "keel's G0 verdict was folded from the sink into the host chain"
else
  fail "no sandbox-sourced kind=gate entry in $CHAIN_FILE"
fi
if "$CLI" run "$PROJECT" -- test -e /workspace/.keel/chain.jsonl >/dev/null 2>&1; then
  fail "keel wrote its own chain file inside the sandbox despite the sink"
else
  pass "keel never held the pen inside the sandbox"
fi

step "audit: egress log folds into the chain"
AUDIT_OUT=$("$CLI" audit "$PROJECT" 2>&1)
assert_contains "audit output mentions folded entries" "$AUDIT_OUT" "folded in"
assert_contains "audit chain recorded the github.com allow" "$AUDIT_OUT" "github.com"
assert_contains "audit chain recorded the example.com deny" "$AUDIT_OUT" "example.com"

step "audit: keel's own verifier accepts the host chain"
if grep -q '"kind":"egress"' "$CHAIN_FILE" 2>/dev/null; then
  pass "egress verdicts are in the same chain"
else
  fail "no kind=egress entry in $CHAIN_FILE"
fi
KEEL_BIN="${KEEL:-keel}"
if command -v "$KEEL_BIN" >/dev/null 2>&1; then
  VERIFY_DIR="$(mktemp -d)"
  mkdir -p "$VERIFY_DIR/.keel" && cp "$CHAIN_FILE" "$VERIFY_DIR/.keel/chain.jsonl"
  KEEL_VERIFY_OUT=$(cd "$VERIFY_DIR" && "$KEEL_BIN" chain verify 2>&1)
  KEEL_VERIFY_EXIT=$?
  rm -rf "$VERIFY_DIR"
else
  # No keel on this host (CI): use the one in moor/base, the same keel the
  # sandbox runs. The chain goes in on stdin, so nothing is mounted.
  KEEL_VERIFY_OUT=$(docker run --rm -i --network none --entrypoint sh moor/base:latest -c \
    'mkdir -p /tmp/v/.keel && cat > /tmp/v/.keel/chain.jsonl && cd /tmp/v && keel chain verify' \
    <"$CHAIN_FILE" 2>&1)
  KEEL_VERIFY_EXIT=$?
fi
if [ "$KEEL_VERIFY_EXIT" -eq 0 ]; then pass "keel chain verify accepts moor's chain"; else fail "keel chain verify accepts moor's chain"; fi
indent "$KEEL_VERIFY_OUT"

step "audit: chain verifies OK before tampering"
VERIFY_OUT=$("$CLI" audit "$PROJECT" --verify 2>&1)
VERIFY_EXIT=$?
if [ "$VERIFY_EXIT" -eq 0 ]; then pass "verify exits 0 on an untampered chain"; else fail "verify exits 0 on an untampered chain"; fi
assert_contains "verify reports chain OK" "$VERIFY_OUT" "chain OK"

step "audit: a live tamper attempt against the chain file is detected"
cp "$CHAIN_FILE" "${CHAIN_FILE}.bak"
python3 - "$CHAIN_FILE" <<'PY'
import json, sys
path = sys.argv[1]
with open(path) as f:
    lines = f.read().splitlines()
entry = json.loads(lines[0])
entry.setdefault("data", {})["tampered_by_e2e_test"] = True
lines[0] = json.dumps(entry)
with open(path, "w") as f:
    f.write("\n".join(lines) + "\n")
PY
TAMPER_OUT=$("$CLI" audit "$PROJECT" --verify 2>&1)
TAMPER_EXIT=$?
if [ "$TAMPER_EXIT" -ne 0 ]; then pass "verify exits non-zero after tampering"; else fail "verify exits non-zero after tampering (it exited 0!)"; fi
assert_contains "verify reports TAMPERED" "$TAMPER_OUT" "TAMPERED"
mv "${CHAIN_FILE}.bak" "$CHAIN_FILE"
RESTORED_OUT=$("$CLI" audit "$PROJECT" --verify 2>&1)
RESTORED_EXIT=$?
if [ "$RESTORED_EXIT" -eq 0 ]; then pass "restoring the original file verifies OK again"; else fail "restoring the original file verifies OK again"; fi
indent "$RESTORED_OUT"

step "audit: export bundle"
EXPORT_DIR="/tmp"
expect_success "moor audit --export" "$CLI" audit "$PROJECT" --export "$EXPORT_DIR"
shopt -s nullglob
BUNDLE_MATCHES=("$EXPORT_DIR"/"${PROJECT}"-audit-*.tar.gz)
shopt -u nullglob
BUNDLE="${BUNDLE_MATCHES[0]:-}"
if [ -n "${BUNDLE:-}" ] && tar -tzf "$BUNDLE" | grep -q "chain.jsonl"; then
  pass "export bundle exists and contains chain.jsonl"
  rm -f "$BUNDLE"
else
  fail "export bundle missing or does not contain chain.jsonl"
fi

step "moor bundle: a keel bundle carrying the host chain, verified"
# A spec taken through keel's pipeline inside the sandbox: G0, approval,
# plan, G1, and a driverless run whose run_end reaches the host chain
# through the sink. `moor run` passes no stdin, so the spec travels as
# an argument.
# shellcheck disable=SC2016 # the backticks are literal spec text
SPEC_TEXT='---
id: SPEC-0099
slug: e2e-bundle
schema: keel.spec/1
status: draft
scope:
  - "src/**"
budget:
  criteria: 4
  lines: 40
---

# e2e bundle

## Acceptance criteria

### AC-1 The run completes

WHEN keel runs this spec THE SYSTEM SHALL record a run_end entry.

oracle: cmd `true` exit 0
'
# shellcheck disable=SC2016 # $1 and $2 belong to the sandbox's shell
"$CLI" run "$PROJECT" -- sh -c 'mkdir -p "$(dirname "$1")" && printf "%s" "$2" > "$1"' \
  sh .keel/specs/e2e-bundle/spec.md "$SPEC_TEXT" >/dev/null 2>&1 || true
# keel diffs a run against a commit; the workspace repo has none yet.
"$CLI" run "$PROJECT" -- git -C /workspace -c user.name=e2e -c user.email=e2e@moor.invalid \
  commit -q --allow-empty -m base >/dev/null 2>&1 || true
for step_args in "gate g0 e2e-bundle" "approve e2e-bundle --stage spec" "plan e2e-bundle" \
  "gate g1 e2e-bundle" "run e2e-bundle --no-driver"; do
  # shellcheck disable=SC2086 # each entry is a keel argv, split on purpose
  "$CLI" run "$PROJECT" -- keel $step_args >/dev/null 2>&1 || true
done
# The approval above was made in a project created by `moor new`, whose
# workspace was not yet a git repository: the approver must still be the
# host's identity, reaching keel through GIT_CONFIG_* env, not "unknown".
HOST_NAME=$(git config --get user.name || true)
APPROVALS=$("$CLI" run "$PROJECT" -- cat .keel/specs/e2e-bundle/approvals.jsonl 2>&1)
if [ -z "$HOST_NAME" ]; then
  pass "approver check skipped: this host has no git user.name to compare"
elif grep -qF "\"by\":\"$HOST_NAME\"" <<<"$APPROVALS"; then
  pass "the approval names the host's git identity ($HOST_NAME)"
else
  fail "the approval does not name '$HOST_NAME'"
  indent "$APPROVALS"
fi
BUNDLE_DIR="$(mktemp -d)"
BUNDLE_OUT=$("$CLI" bundle -p "$PROJECT" --out "$BUNDLE_DIR" 2>&1)
BUNDLE_EXIT=$?
indent "$BUNDLE_OUT"
if [ "$BUNDLE_EXIT" -eq 0 ]; then pass "moor bundle exits 0 (keel's verdict)"; else fail "moor bundle exits 0 (got $BUNDLE_EXIT)"; fi
for check in chain approvals gate-verdicts trajectory; do
  if grep -qE "pass +$check " <<<"$BUNDLE_OUT"; then
    pass "bundle check '$check' passes"
  else
    fail "bundle check '$check' did not pass"
  fi
done
shopt -s nullglob
BUNDLE_FILES=("$BUNDLE_DIR"/keel-"$PROJECT"-*.tar.gz)
shopt -u nullglob
# Listed first, then searched: `tar | grep -q` under pipefail fails on GNU
# tar, which gets SIGPIPE when grep stops reading at an early match.
BUNDLE_LIST=""
[ "${#BUNDLE_FILES[@]}" -eq 1 ] && BUNDLE_LIST=$(tar -tzf "${BUNDLE_FILES[0]}")
if [ "${#BUNDLE_FILES[@]}" -eq 1 ] && grep -q "chain.jsonl" <<<"$BUNDLE_LIST"; then
  pass "the bundle on the host carries chain.jsonl"
else
  fail "no single bundle with chain.jsonl in $BUNDLE_DIR"
fi
rm -rf "$BUNDLE_DIR"

step "import: an existing local repo (no bind mount, ever)"
mkdir -p "$IMPORT_SRC"
(cd "$IMPORT_SRC" && git init -q && git config user.email t@t.local && git config user.name t \
  && echo '{}' > package.json && echo hi > README.md && git add -A && git commit -q -m init \
  && git checkout -qb a-second-branch && git checkout -q -)
IMPORT_OUT=$("$CLI" import "$IMPORT_PROJECT" --from "$IMPORT_SRC" 2>&1)
IMPORT_EXIT=$?
indent "$IMPORT_OUT"
if [ "$IMPORT_EXIT" -eq 0 ]; then pass "moor import exits 0"; else fail "moor import exits 0"; fi
assert_contains "auto-detected the node image from package.json" "$IMPORT_OUT" "moor/node:latest"
assert_contains "ran keel init (no prior .keel/ in the source)" "$IMPORT_OUT" "keel is initialised"

IMPORT_LOG=$("$CLI" run "$IMPORT_PROJECT" -- git log --oneline 2>&1)
assert_contains "imported commit is present in the sandbox" "$IMPORT_LOG" "init"
IMPORT_BRANCHES=$("$CLI" run "$IMPORT_PROJECT" -- git branch -a 2>&1)
assert_contains "the second branch came across too (--all bundle)" "$IMPORT_BRANCHES" "a-second-branch"

SELFTEST2_OUT=$("$CLI" selftest "$IMPORT_PROJECT" 2>&1)
SELFTEST2_EXIT=$?
if [ "$SELFTEST2_EXIT" -eq 0 ]; then
  pass "imported project passes selftest too"
else
  fail "imported project passes selftest too"
  indent "$SELFTEST2_OUT"
fi

echo
if [ "$FAILURES" -eq 0 ]; then
  echo "e2e: all checks passed."
  exit 0
else
  echo "e2e: $FAILURES check(s) failed."
  exit 1
fi
