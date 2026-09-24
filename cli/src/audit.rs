use crate::{manifest::Manifest, paths};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::{Path, PathBuf};

const GENESIS_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// keel's evidence-chain format (keel ADR-0001). keel owns the format and
/// its verifier; moor, as the host process no sandbox can reach, holds the
/// pen — so `keel chain verify` can check a moor chain without trusting
/// anything moor-specific.
pub const CHAIN_SCHEMA: &str = "keel.chain/1";
pub const WRITER: &str = "moor";

/// Where keel inside the sandbox leaves chain payloads for moor to fold
/// (`KEEL_CHAIN_SINK`). A tmpfs, not a bind mount: see the compose template.
pub const SINK_PATH: &str = "/run/moor-sink/keel.jsonl";

/// One entry in a project's audit hash chain. `hash` commits to every
/// other field including `prev_hash` — the hash of the entry before it —
/// so the file is a Merkle-style chain: editing, deleting, or reordering
/// any entry breaks the hash of every entry after it. Nothing inside a
/// sandbox has this path mounted, so the only way to produce a line that
/// verifies is to have generated it honestly at the time; see
/// `verify_chain` and `docs/THREAT-MODEL.md`'s "audit tampering" section.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainEntry {
    pub schema: String,
    pub seq: u64,
    pub ts: String,
    pub kind: String,
    pub writer: String,
    pub data: Value,
    pub prev_hash: String,
    pub hash: String,
}

/// keel's `SetHasher` framing: name, NUL, length as u64 LE, bytes, NUL.
/// Length-framing keeps one field from bleeding into the next; it must
/// match keel byte for byte, which the golden-entry test pins.
fn frame(h: &mut Sha256, name: &str, content: &[u8]) {
    h.update(name.as_bytes());
    h.update([0u8]);
    h.update((content.len() as u64).to_le_bytes());
    h.update(content);
    h.update([0u8]);
}

fn compute_hash(
    prev_hash: &str,
    seq: u64,
    ts: &str,
    kind: &str,
    writer: &str,
    data: &Value,
) -> String {
    let mut h = Sha256::new();
    frame(&mut h, "prev_hash", prev_hash.as_bytes());
    frame(&mut h, "seq", seq.to_string().as_bytes());
    frame(&mut h, "ts", ts.as_bytes());
    frame(&mut h, "kind", kind.as_bytes());
    frame(&mut h, "writer", writer.as_bytes());
    frame(&mut h, "data", data.to_string().as_bytes());
    hex::encode(h.finalize())
}

/// The hash moor used before `keel.chain/1`. Kept only to verify a sealed
/// legacy chain; nothing new is ever written with it.
fn legacy_hash(prev_hash: &str, seq: u64, ts: &str, kind: &str, data: &Value) -> String {
    let mut hasher = Sha256::new();
    hasher.update(prev_hash.as_bytes());
    hasher.update(seq.to_le_bytes());
    hasher.update(ts.as_bytes());
    hasher.update(kind.as_bytes());
    hasher.update(data.to_string().as_bytes());
    hex::encode(hasher.finalize())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LegacyEntry {
    seq: u64,
    ts: String,
    kind: String,
    data: Value,
    prev_hash: String,
    hash: String,
}

fn last_entry(path: &Path) -> Result<Option<ChainEntry>> {
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(path)?;
    match text.lines().rev().find(|l| !l.trim().is_empty()) {
        Some(line) => {
            Ok(Some(serde_json::from_str(line).with_context(|| {
                format!("parsing last line of {}", path.display())
            })?))
        }
        None => Ok(None),
    }
}

/// `chain.jsonl` → `chain.legacy.jsonl`, beside it.
pub fn legacy_path(path: &Path) -> PathBuf {
    path.with_extension("legacy.jsonl")
}

/// Append one entry to a project's hash-chained audit log. This is the
/// only writer of that file — no container ever has this path mounted.
pub fn append_chained(path: &Path, kind: &str, data: Value) -> Result<ChainEntry> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    seal_legacy(path)?;
    append_entry(path, kind, data)
}

fn append_entry(path: &Path, kind: &str, data: Value) -> Result<ChainEntry> {
    let (seq, prev_hash) = match last_entry(path)? {
        Some(e) => (e.seq + 1, e.hash),
        None => (1, GENESIS_HASH.to_string()),
    };
    let ts = chrono::Utc::now().to_rfc3339();
    let hash = compute_hash(&prev_hash, seq, &ts, kind, WRITER, &data);
    let entry = ChainEntry {
        schema: CHAIN_SCHEMA.to_string(),
        seq,
        ts,
        kind: kind.to_string(),
        writer: WRITER.to_string(),
        data,
        prev_hash,
        hash,
    };

    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("opening audit log {}", path.display()))?;
    writeln!(f, "{}", serde_json::to_string(&entry)?)?;
    Ok(entry)
}

/// A chain written before `keel.chain/1` is never rewritten: it moves aside
/// unchanged, and the new chain opens by committing to its head. History
/// stays verifiable with the hash it was written under, and the new chain
/// is verifiable by keel from its first entry.
fn seal_legacy(path: &Path) -> Result<()> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Ok(());
    };
    let Some(first) = text.lines().find(|l| !l.trim().is_empty()) else {
        return Ok(());
    };
    let is_current = serde_json::from_str::<Value>(first)
        .map(|v| v.get("schema").is_some())
        .unwrap_or(false);
    if is_current {
        return Ok(());
    }
    let legacy = legacy_path(path);
    if legacy.exists() {
        anyhow::bail!(
            "{} predates keel.chain/1 but {} already exists — refusing to overwrite a sealed chain",
            path.display(),
            legacy.display()
        );
    }
    let outcome = verify_legacy(path)?;
    let head = text
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .and_then(|l| serde_json::from_str::<Value>(l).ok())
        .and_then(|v| v.get("hash").and_then(Value::as_str).map(String::from));
    std::fs::rename(path, &legacy)
        .with_context(|| format!("moving {} to {}", path.display(), legacy.display()))?;
    let (verified, entries, broken_at) = match outcome {
        VerifyOutcome::Ok { entries } => (true, entries, None),
        VerifyOutcome::Tampered { at_seq, .. } => (false, 0, Some(at_seq)),
    };
    append_entry(
        path,
        "legacy_seal",
        json!({
            "legacy_file": legacy.file_name().map(|n| n.to_string_lossy().into_owned()),
            "legacy_head": head,
            "legacy_entries": entries,
            "legacy_verified": verified,
            "legacy_broken_at": broken_at,
        }),
    )?;
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
pub enum VerifyOutcome {
    Ok { entries: usize },
    Tampered { at_seq: u64, reason: String },
}

/// Walk the whole chain and recompute every hash. Any edit, deletion,
/// reorder, or truncation-from-the-middle of the file will show up here
/// as a mismatch at the first affected entry.
pub fn verify_chain(path: &Path) -> Result<VerifyOutcome> {
    verify_with(path, 1, |line| {
        let e: ChainEntry = serde_json::from_str(line)?;
        let recomputed = compute_hash(&e.prev_hash, e.seq, &e.ts, &e.kind, &e.writer, &e.data);
        Ok((e.seq, e.prev_hash, e.hash, recomputed))
    })
}

/// The same walk over a sealed pre-`keel.chain/1` chain, with the hash it
/// was written under.
pub fn verify_legacy(path: &Path) -> Result<VerifyOutcome> {
    verify_with(path, 0, |line| {
        let e: LegacyEntry = serde_json::from_str(line)?;
        let recomputed = legacy_hash(&e.prev_hash, e.seq, &e.ts, &e.kind, &e.data);
        Ok((e.seq, e.prev_hash, e.hash, recomputed))
    })
}

/// `parse` yields (seq, prev_hash, hash, recomputed hash) for one line.
fn verify_with(
    path: &Path,
    first_seq: u64,
    parse: impl Fn(&str) -> Result<(u64, String, String, String)>,
) -> Result<VerifyOutcome> {
    if !path.exists() {
        return Ok(VerifyOutcome::Ok { entries: 0 });
    }
    let text = std::fs::read_to_string(path)?;
    let mut prev_hash = GENESIS_HASH.to_string();
    let mut expected_seq = first_seq;
    let mut count = 0;

    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let (seq, entry_prev, hash, recomputed) = match parse(line) {
            Ok(parts) => parts,
            Err(e) => {
                return Ok(VerifyOutcome::Tampered {
                    at_seq: expected_seq,
                    reason: format!("entry at seq {expected_seq} is not a valid entry: {e}"),
                })
            }
        };
        if seq != expected_seq {
            return Ok(VerifyOutcome::Tampered {
                at_seq: seq,
                reason: format!(
                    "expected seq {expected_seq}, found {seq} (an entry was deleted or reordered)"
                ),
            });
        }
        if entry_prev != prev_hash {
            return Ok(VerifyOutcome::Tampered {
                at_seq: seq,
                reason: "prev_hash does not match the preceding entry's hash".to_string(),
            });
        }
        if recomputed != hash {
            return Ok(VerifyOutcome::Tampered {
                at_seq: seq,
                reason: "hash does not match entry contents (the entry itself was edited)"
                    .to_string(),
            });
        }
        prev_hash = hash;
        expected_seq += 1;
        count += 1;
    }

    Ok(VerifyOutcome::Ok { entries: count })
}

/// Best-effort redaction: replace any literal occurrence of a named
/// secret's *value* (as currently set in this process's own environment)
/// with a placeholder. This only catches secrets the moor CLI itself
/// had access to at logging time — see docs/THREAT-MODEL.md and
/// proxy/README.md for what this control does and doesn't cover.
pub fn redact(text: &str, secret_names: &[String]) -> String {
    let mut out = text.to_string();
    for name in secret_names {
        if let Ok(val) = std::env::var(name) {
            if val.len() >= 4 {
                out = out.replace(&val, &format!("***REDACTED:{name}***"));
            }
        }
    }
    out
}

fn redact_argv(argv: &[String], secret_names: &[String]) -> Vec<String> {
    argv.iter().map(|a| redact(a, secret_names)).collect()
}

/// Log one `moor run`/`shell`/`new`-driven command. `kind` lets
/// callers flag a command as something more specific than a routine
/// exec — e.g. `run_cmd` tags anything that looks like `git push` as
/// "git-push" instead of "exec", since that's the one channel through
/// which code actually leaves the sandbox (see docs/THREAT-MODEL.md,
/// "why not a pre-push hook").
pub fn log_exec(
    project: &str,
    m: &Manifest,
    kind: &str,
    argv: &[String],
    exit_code: Option<i32>,
) -> Result<()> {
    log_exec_with(project, m, kind, argv, exit_code, json!({}))
}

/// `log_exec`, with extra fields merged into the entry's data.
///
/// keel's sink is folded first: whatever keel recorded while this command
/// ran lands in the chain before the entry that says the command finished.
pub fn log_exec_with(
    project: &str,
    m: &Manifest,
    kind: &str,
    argv: &[String],
    exit_code: Option<i32>,
    extra: Value,
) -> Result<()> {
    paths::ensure_project_dirs(project)?;
    if let Err(e) = fold_sink(project, m) {
        eprintln!("moor: could not fold keel's chain sink: {e:#}");
    }
    let path = paths::chain_log_path(project)?;
    let mut data = json!({
        "project": project,
        "argv": redact_argv(argv, &m.secrets),
        "exit_code": exit_code,
    });
    if let (Some(d), Value::Object(extra)) = (data.as_object_mut(), extra) {
        d.extend(extra);
    }
    append_chained(&path, kind, data)?;
    Ok(())
}

/// Fold new lines from keel's chain sink inside the sandbox into the host
/// chain, tracking progress in `audit/.sink-offset` the way the egress fold
/// does — including resetting when the sink's tmpfs was cleared by a
/// restart.
pub fn fold_sink(project: &str, m: &Manifest) -> Result<usize> {
    let (status, out) = crate::proc::run_capture(
        "docker",
        &["exec", &m.sandbox_container(), "cat", SINK_PATH],
    )?;
    if !status.success() {
        // No sandbox, or keel has not written anything yet.
        return Ok(0);
    }
    let offset_path = paths::sink_offset_path(project)?;
    let prev_offset: usize = std::fs::read_to_string(&offset_path)
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0);
    let lines: Vec<&str> = out.lines().collect();
    let start = if prev_offset > lines.len() {
        0
    } else {
        prev_offset
    };

    let folded = fold_sink_lines(
        &paths::chain_log_path(project)?,
        &lines[start..],
        start + 1,
        &m.secrets,
    )?;
    std::fs::write(&offset_path, lines.len().to_string())?;
    Ok(folded)
}

/// Append each sink line under its own `kind`, marked `source: "sandbox"`:
/// it is what the sandbox *said*, recorded faithfully, not something moor
/// observed. A line that is not a payload is kept as `sink_malformed` —
/// dropping it would let the sandbox hide a line by breaking it.
pub fn fold_sink_lines(
    chain: &Path,
    lines: &[&str],
    first_line_no: usize,
    secret_names: &[String],
) -> Result<usize> {
    let mut folded = 0;
    for (i, raw) in lines.iter().enumerate() {
        if raw.trim().is_empty() {
            continue;
        }
        let line = redact(raw, secret_names);
        let payload = serde_json::from_str::<Value>(&line).ok().and_then(|v| {
            let kind = v.get("kind")?.as_str()?.to_string();
            let data = v.get("data")?.as_object()?.clone();
            Some((kind, data))
        });
        match payload {
            Some((kind, mut data)) => {
                data.insert("source".into(), json!("sandbox"));
                append_chained(chain, &kind, Value::Object(data))?;
            }
            None => {
                append_chained(
                    chain,
                    "sink_malformed",
                    json!({ "line_no": first_line_no + i, "line": line, "source": "sandbox" }),
                )?;
            }
        }
        folded += 1;
    }
    Ok(folded)
}

/// Fold any new lines from the egress gateway's own access log into the
/// project's audit chain. Tracks how many raw lines have already been
/// folded in `audit/.egress-offset` so re-running `moor audit` is
/// idempotent. If the egress container was restarted (its log lives on
/// tmpfs and resets), the offset is detected as stale and reset rather
/// than silently under- or over-counting.
pub fn fold_egress_log(project: &str, m: &Manifest) -> Result<usize> {
    let (status, out) = crate::proc::run_capture(
        "docker",
        &[
            "exec",
            &m.egress_container(),
            "cat",
            "/var/log/tinyproxy/access.log",
        ],
    )?;
    if !status.success() {
        // Egress container not running / log not there yet — nothing to fold.
        return Ok(0);
    }

    let offset_path = paths::egress_offset_path(project)?;
    let prev_offset: usize = std::fs::read_to_string(&offset_path)
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0);

    let lines: Vec<&str> = out.lines().collect();
    let start = if prev_offset > lines.len() {
        0
    } else {
        prev_offset
    };

    let chain_path = paths::chain_log_path(project)?;
    let new_text = lines[start..].join("\n");
    let mut folded = 0;
    for event in crate::egress_log::parse_log(&new_text) {
        let kind = match event.severity {
            crate::egress_log::Severity::Tripwire => "tripwire",
            crate::egress_log::Severity::Normal => "egress",
        };
        append_chained(&chain_path, kind, serde_json::to_value(&event)?)?;
        folded += 1;
    }

    if let Some(parent) = offset_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&offset_path, lines.len().to_string())?;
    Ok(folded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    /// A fresh, never-before-used path under the OS temp dir, unique per
    /// call (parallel test threads must never collide on the same file).
    fn temp_path(label: &str) -> std::path::PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("moor-audit-test-{label}-{nanos}.jsonl"))
    }

    #[test]
    fn empty_or_missing_chain_verifies_ok_with_zero_entries() {
        let path = temp_path("missing");
        assert_eq!(
            verify_chain(&path).unwrap(),
            VerifyOutcome::Ok { entries: 0 }
        );
    }

    #[test]
    fn a_freshly_appended_chain_verifies_ok() {
        let path = temp_path("fresh");
        append_chained(&path, "exec", json!({"argv": ["keel", "status"]})).unwrap();
        append_chained(&path, "exec", json!({"argv": ["keel", "run", "spec-1"]})).unwrap();
        append_chained(&path, "egress", json!({"domain": "github.com"})).unwrap();

        assert_eq!(
            verify_chain(&path).unwrap(),
            VerifyOutcome::Ok { entries: 3 }
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn seq_and_prev_hash_link_correctly() {
        let path = temp_path("linkage");
        let e0 = append_chained(&path, "exec", json!({"n": 0})).unwrap();
        let e1 = append_chained(&path, "exec", json!({"n": 1})).unwrap();
        assert_eq!(e0.seq, 1);
        assert_eq!(e0.prev_hash, GENESIS_HASH);
        assert_eq!(e1.seq, 2);
        assert_eq!(e1.prev_hash, e0.hash);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn editing_an_entrys_data_is_detected() {
        let path = temp_path("edit-data");
        append_chained(&path, "exec", json!({"argv": ["safe", "command"]})).unwrap();
        append_chained(&path, "exec", json!({"argv": ["another", "command"]})).unwrap();

        // Tamper: rewrite the first line's data field without recomputing
        // its hash — simulating someone hand-editing the file to hide
        // what a command actually was.
        let text = std::fs::read_to_string(&path).unwrap();
        let mut lines: Vec<String> = text.lines().map(String::from).collect();
        let mut entry: ChainEntry = serde_json::from_str(&lines[0]).unwrap();
        entry.data = json!({"argv": ["rm", "-rf", "/"]});
        lines[0] = serde_json::to_string(&entry).unwrap();
        std::fs::write(&path, lines.join("\n") + "\n").unwrap();

        match verify_chain(&path).unwrap() {
            VerifyOutcome::Tampered { at_seq, .. } => assert_eq!(at_seq, 1),
            VerifyOutcome::Ok { .. } => panic!("tampering was not detected"),
        }
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn deleting_a_middle_entry_is_detected() {
        let path = temp_path("delete-middle");
        append_chained(&path, "exec", json!({"n": 0})).unwrap();
        append_chained(&path, "exec", json!({"n": 1})).unwrap();
        append_chained(&path, "exec", json!({"n": 2})).unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        // Drop the middle entry (seq 1) — first and last are left intact.
        let tampered = format!("{}\n{}\n", lines[0], lines[2]);
        std::fs::write(&path, tampered).unwrap();

        match verify_chain(&path).unwrap() {
            VerifyOutcome::Tampered { .. } => {}
            VerifyOutcome::Ok { .. } => panic!("deletion was not detected"),
        }
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn reordering_entries_is_detected() {
        let path = temp_path("reorder");
        append_chained(&path, "exec", json!({"n": 0})).unwrap();
        append_chained(&path, "exec", json!({"n": 1})).unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        let swapped = format!("{}\n{}\n", lines[1], lines[0]);
        std::fs::write(&path, swapped).unwrap();

        match verify_chain(&path).unwrap() {
            VerifyOutcome::Tampered { .. } => {}
            VerifyOutcome::Ok { .. } => panic!("reordering was not detected"),
        }
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn appending_a_forged_entry_without_the_real_chain_state_is_detected() {
        let path = temp_path("forge-append");
        append_chained(&path, "exec", json!({"n": 0})).unwrap();

        // An attacker who doesn't control this process can't call
        // append_chained (that's the point) — simulate them hand-writing
        // a plausible-looking next line with an invented prev_hash/hash.
        let forged = ChainEntry {
            schema: CHAIN_SCHEMA.to_string(),
            seq: 2,
            ts: chrono::Utc::now().to_rfc3339(),
            kind: "exec".to_string(),
            writer: WRITER.to_string(),
            data: json!({"argv": ["totally", "legitimate"]}),
            prev_hash: "f".repeat(64),
            hash: "0".repeat(64),
        };
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        use std::io::Write;
        writeln!(f, "{}", serde_json::to_string(&forged).unwrap()).unwrap();

        match verify_chain(&path).unwrap() {
            VerifyOutcome::Tampered { at_seq, .. } => assert_eq!(at_seq, 2),
            VerifyOutcome::Ok { .. } => panic!("forged append was not detected"),
        }
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn truncating_to_a_prefix_still_verifies_ok() {
        // Losing the tail of the file (e.g. a crash mid-write) is not the
        // same as tampering with what's left — the surviving prefix must
        // still verify, since every entry in it is exactly as it was
        // originally written.
        let path = temp_path("truncate");
        append_chained(&path, "exec", json!({"n": 0})).unwrap();
        append_chained(&path, "exec", json!({"n": 1})).unwrap();
        append_chained(&path, "exec", json!({"n": 2})).unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        let first_line = text.lines().next().unwrap();
        std::fs::write(&path, format!("{first_line}\n")).unwrap();

        assert_eq!(
            verify_chain(&path).unwrap(),
            VerifyOutcome::Ok { entries: 1 }
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn redact_replaces_secret_env_values_but_leaves_everything_else() {
        std::env::set_var("MOOR_TEST_SECRET_A", "sk-super-secret-value-123");
        let text =
            "curl -H 'Authorization: Bearer sk-super-secret-value-123' https://api.example.com";
        let redacted = redact(text, &["MOOR_TEST_SECRET_A".to_string()]);
        assert!(!redacted.contains("sk-super-secret-value-123"));
        assert!(redacted.contains("***REDACTED:MOOR_TEST_SECRET_A***"));
        assert!(redacted.contains("https://api.example.com"));
        std::env::remove_var("MOOR_TEST_SECRET_A");
    }

    #[test]
    fn redact_ignores_secrets_not_set_in_env() {
        std::env::remove_var("MOOR_TEST_SECRET_UNSET");
        let text = "keel run my-spec";
        let redacted = redact(text, &["MOOR_TEST_SECRET_UNSET".to_string()]);
        assert_eq!(redacted, text);
    }

    #[test]
    fn redact_does_not_mangle_trivially_short_env_values() {
        // A short/empty env value (e.g. an accidentally-blank secret)
        // must not turn into a blanket find-and-replace of common
        // substrings across the whole log line.
        std::env::set_var("MOOR_TEST_SHORT", "ok");
        let text = "echo ok, this token is ok";
        let redacted = redact(text, &["MOOR_TEST_SHORT".to_string()]);
        assert_eq!(redacted, text, "short values must not trigger redaction");
        std::env::remove_var("MOOR_TEST_SHORT");
    }

    #[test]
    fn redact_argv_redacts_each_element_independently() {
        std::env::set_var("MOOR_TEST_SECRET_B", "ghp_abcdef1234567890");
        let argv = vec![
            "curl".to_string(),
            "-H".to_string(),
            "Authorization: token ghp_abcdef1234567890".to_string(),
        ];
        let redacted = redact_argv(&argv, &["MOOR_TEST_SECRET_B".to_string()]);
        assert_eq!(redacted[0], "curl");
        assert!(redacted[2].contains("***REDACTED:MOOR_TEST_SECRET_B***"));
        assert!(!redacted[2].contains("ghp_abcdef1234567890"));
        std::env::remove_var("MOOR_TEST_SECRET_B");
    }

    /// Written by keel's own `chain::append` (keel SPEC-0009). If moor's
    /// hashing drifts from keel's by a byte, this fails.
    const KEEL_GOLDEN: &str = r#"{"schema":"keel.chain/1","seq":1,"ts":"2026-09-24T18:29:23.664455+02:00","kind":"gate","writer":"in-process","data":{"gate":"G0","run":"2026-09-24-000","sha256":"577be49e18a8cb8869b740b0143b6844c2012ae08335fef2c04b19cff590daff","spec":"golden","verdict":"fail"},"prev_hash":"0000000000000000000000000000000000000000000000000000000000000000","hash":"5ce831902245f09b547cc2d9b007b37ce6a7ce5e34b0695bf80a4cb532d17676"}"#;

    #[test]
    fn matches_a_keel_written_golden_entry() {
        let golden: ChainEntry = serde_json::from_str(KEEL_GOLDEN).unwrap();
        let recomputed = compute_hash(
            &golden.prev_hash,
            golden.seq,
            &golden.ts,
            &golden.kind,
            &golden.writer,
            &golden.data,
        );
        assert_eq!(recomputed, golden.hash, "moor hashes differently from keel");

        let path = temp_path("golden");
        std::fs::write(&path, format!("{KEEL_GOLDEN}\n")).unwrap();
        assert_eq!(
            verify_chain(&path).unwrap(),
            VerifyOutcome::Ok { entries: 1 }
        );

        // And what moor writes next is keel's shape, linked to keel's entry.
        let e = append_chained(&path, "exec", json!({"argv": ["keel", "next"]})).unwrap();
        assert_eq!(
            (e.schema.as_str(), e.writer.as_str(), e.seq),
            (CHAIN_SCHEMA, WRITER, 2)
        );
        assert_eq!(e.prev_hash, golden.hash);
        assert_eq!(
            verify_chain(&path).unwrap(),
            VerifyOutcome::Ok { entries: 2 }
        );
        let _ = std::fs::remove_file(&path);
    }

    /// Write a chain the way moor did before keel.chain/1.
    fn write_legacy(path: &Path, kinds: &[&str]) -> String {
        let mut prev = GENESIS_HASH.to_string();
        let mut out = String::new();
        for (seq, kind) in kinds.iter().enumerate() {
            let ts = "2026-09-01T00:00:00+00:00".to_string();
            let data = json!({"n": seq});
            let hash = legacy_hash(&prev, seq as u64, &ts, kind, &data);
            let e = LegacyEntry {
                seq: seq as u64,
                ts,
                kind: kind.to_string(),
                data,
                prev_hash: prev,
                hash: hash.clone(),
            };
            out.push_str(&format!("{}\n", serde_json::to_string(&e).unwrap()));
            prev = hash;
        }
        std::fs::write(path, &out).unwrap();
        prev
    }

    #[test]
    fn legacy_chain_is_sealed_into_the_new_one() {
        let path = temp_path("legacy");
        let head = write_legacy(&path, &["exec", "egress"]);
        let before = std::fs::read(&path).unwrap();

        append_chained(&path, "exec", json!({"n": "new"})).unwrap();
        append_chained(&path, "exec", json!({"n": "newer"})).unwrap();

        let legacy = legacy_path(&path);
        assert_eq!(
            std::fs::read(&legacy).unwrap(),
            before,
            "legacy chain was altered"
        );
        assert_eq!(
            verify_legacy(&legacy).unwrap(),
            VerifyOutcome::Ok { entries: 2 }
        );

        let text = std::fs::read_to_string(&path).unwrap();
        let seal: ChainEntry = serde_json::from_str(text.lines().next().unwrap()).unwrap();
        assert_eq!(seal.kind, "legacy_seal");
        assert_eq!(seal.data["legacy_head"], json!(head));
        assert_eq!(seal.data["legacy_entries"], json!(2));
        assert_eq!(seal.data["legacy_verified"], json!(true));
        assert_eq!(
            verify_chain(&path).unwrap(),
            VerifyOutcome::Ok { entries: 3 },
            "sealed once, not per append"
        );

        // A second pre-keel chain beside an existing seal is refused, not clobbered.
        write_legacy(&path, &["exec"]);
        assert!(append_chained(&path, "exec", json!({})).is_err());
        assert_eq!(std::fs::read(&legacy).unwrap(), before);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(&legacy);
    }

    #[test]
    fn sink_lines_fold_in_order_and_malformed_ones_are_kept() {
        let path = temp_path("sink");
        std::env::set_var("MOOR_TEST_SINK_SECRET", "hunter2-sink-value");
        let lines = [
            r#"{"schema":"keel.chain/1","kind":"gate","data":{"gate":"G0","verdict":"pass"}}"#,
            "not json, hunter2-sink-value",
            "",
            r#"{"schema":"keel.chain/1","kind":"approval","data":{"stage":"spec"}}"#,
        ];
        let folded =
            fold_sink_lines(&path, &lines, 1, &["MOOR_TEST_SINK_SECRET".to_string()]).unwrap();
        std::env::remove_var("MOOR_TEST_SINK_SECRET");
        assert_eq!(folded, 3);

        let text = std::fs::read_to_string(&path).unwrap();
        let entries: Vec<ChainEntry> = text
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        let kinds: Vec<&str> = entries.iter().map(|e| e.kind.as_str()).collect();
        assert_eq!(kinds, ["gate", "sink_malformed", "approval"]);
        assert!(entries.iter().all(|e| e.data["source"] == "sandbox"));
        assert_eq!(entries[0].data["verdict"], "pass");
        assert_eq!(entries[1].data["line_no"], 2);
        assert!(
            !text.contains("hunter2-sink-value"),
            "a secret reached the chain"
        );
        assert_eq!(
            verify_chain(&path).unwrap(),
            VerifyOutcome::Ok { entries: 3 }
        );
        let _ = std::fs::remove_file(&path);
    }
}
