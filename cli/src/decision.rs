//! A deterministic, rules-only risk gate for `moor run` commands. See
//! docs/adr/0001-tool-call-risk-gating.md: this table is what a future
//! model backend would sit behind, not a stand-in that gets replaced —
//! rules run first and settle what they can, a model would only ever see
//! what these rules don't.

use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::Path;

#[derive(Debug)]
pub enum Verdict {
    Allow,
    Deny {
        rule_id: &'static str,
        reason: String,
    },
}

const CREDENTIAL_MARKERS: &[&str] = &[".ssh", ".aws", ".netrc", "id_rsa", ".pem"];
const WIPE_TARGETS: &[&str] = &["/", "/workspace", "~"];

/// See docs/adr/ADR-0003-task-context-wire-format.md.
#[derive(Debug, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum GateLevel {
    Minimal,
    Standard,
    Strict,
}

/// What the agent was approved to do this session, stamped at session
/// start to `~/.moor/sessions/<session_id>/context.json`.
// `spec_title` and `started_at` go unread until the audit chain records
// them (ADR-0003, Consequences).
#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub struct TaskContext {
    pub session_id: String,
    pub spec_title: String,
    pub gate: GateLevel,
    pub expected_verbs: Vec<String>,
    #[serde(default = "default_workspace")]
    pub allowed_paths: Vec<String>,
    #[serde(default)]
    pub forbidden_paths: Vec<String>,
    pub started_at: String,
}

fn default_workspace() -> Vec<String> {
    vec!["/workspace".to_string()]
}

impl TaskContext {
    /// No file means no session was stamped, so there is nothing to
    /// enforce beyond the context-free rules. A file that is there but
    /// unreadable, malformed, or stamped for another session is an error
    /// instead: treating it as absent would silently switch off
    /// `out_of_scope` and `path_escape`.
    pub fn load(path: &Path, session_id: &str) -> Result<Option<Self>> {
        let raw = match std::fs::read_to_string(path) {
            Ok(raw) => raw,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        let ctx: TaskContext = serde_json::from_str(&raw)
            .with_context(|| format!("parsing task context {}", path.display()))?;
        if ctx.session_id != session_id {
            anyhow::bail!(
                "task context {} is stamped for session `{}`, not `{session_id}`",
                path.display(),
                ctx.session_id
            );
        }
        Ok(Some(ctx))
    }
}

fn resolve_verb(cmd: &[String]) -> &str {
    match (
        cmd.first().map(String::as_str),
        cmd.get(1).map(String::as_str),
    ) {
        (Some("cargo"), Some("check")) => "cargo_check",
        (Some("cargo"), Some("test")) => "cargo_test",
        (Some("cargo"), Some("build")) => "cargo_build",
        (Some("cat") | Some("head") | Some("tail"), _) => "read_file",
        (Some("grep") | Some("rg"), _) => "grep",
        (Some(v), _) => v,
        _ => "bash",
    }
}

fn extract_path(cmd: &[String]) -> Option<&str> {
    cmd.iter().find(|a| a.starts_with('/')).map(String::as_str)
}

pub fn evaluate(cmd: &[String], ctx: Option<&TaskContext>) -> Verdict {
    if let Some(reason) = credential_access(cmd) {
        return Verdict::Deny {
            rule_id: "credential_access",
            reason,
        };
    }
    if let Some(reason) = boundary_escape(cmd) {
        return Verdict::Deny {
            rule_id: "boundary_escape",
            reason,
        };
    }
    if let Some(reason) = destructive_wipe(cmd) {
        return Verdict::Deny {
            rule_id: "destructive",
            reason,
        };
    }
    if let Some(ctx) = ctx {
        if ctx.gate != GateLevel::Minimal {
            let verb = resolve_verb(cmd);
            if !ctx.expected_verbs.iter().any(|v| v == verb) {
                return Verdict::Deny {
                    rule_id: "out_of_scope",
                    reason: format!("verb `{verb}` is not in the session's expected verbs"),
                };
            }
            if let Some(path) = extract_path(cmd) {
                if ctx
                    .forbidden_paths
                    .iter()
                    .any(|f| path.starts_with(f.as_str()))
                    || !ctx
                        .allowed_paths
                        .iter()
                        .any(|a| path.starts_with(a.as_str()))
                {
                    return Verdict::Deny {
                        rule_id: "path_escape",
                        reason: format!("touches `{path}`, outside the session's allowed paths"),
                    };
                }
            }
        }
    }
    Verdict::Allow
}

fn credential_access(cmd: &[String]) -> Option<String> {
    cmd.iter()
        .find(|a| CREDENTIAL_MARKERS.iter().any(|m| a.contains(m)))
        .map(|a| format!("references a credential path: `{a}`"))
}

fn boundary_escape(cmd: &[String]) -> Option<String> {
    if cmd.first().map(|s| s == "sudo").unwrap_or(false) {
        return Some("invokes `sudo`".to_string());
    }
    cmd.iter()
        .find(|a| a.contains("docker.sock"))
        .map(|a| format!("references the Docker socket: `{a}`"))
}

/// `rm` with both a recursive and a force flag, targeting exactly `/`,
/// `/workspace`, or `~` — a subdirectory of either is left alone, since
/// that's ordinary build-clean territory, not a workspace wipe.
fn destructive_wipe(cmd: &[String]) -> Option<String> {
    if cmd.first().map(|s| s.as_str()) != Some("rm") {
        return None;
    }
    let mut recursive = false;
    let mut force = false;
    let mut targets = vec![];
    for arg in cmd.iter().skip(1) {
        if let Some(long) = arg.strip_prefix("--") {
            match long {
                "recursive" => recursive = true,
                "force" => force = true,
                _ => {}
            }
        } else if let Some(short) = arg.strip_prefix('-') {
            if short.contains('r') || short.contains('R') {
                recursive = true;
            }
            if short.contains('f') {
                force = true;
            }
        } else {
            targets.push(arg.as_str());
        }
    }
    if !(recursive && force) {
        return None;
    }
    targets
        .iter()
        .find(|t| WIPE_TARGETS.contains(t))
        .map(|t| format!("recursively force-removes `{t}`"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn denies_known_credential_paths() {
        for cmd in [
            "cat /home/agent/.ssh/id_rsa",
            "cp ~/.aws/credentials /tmp/out",
            "cat .netrc",
            "cat /etc/moor/service.pem",
        ] {
            match evaluate(&argv(cmd), None) {
                Verdict::Deny { rule_id, .. } => assert_eq!(rule_id, "credential_access"),
                Verdict::Allow => panic!("expected deny for `{cmd}`"),
            }
        }
    }

    #[test]
    fn denies_docker_socket_and_sudo() {
        match evaluate(&argv("sudo ls"), None) {
            Verdict::Deny { rule_id, .. } => assert_eq!(rule_id, "boundary_escape"),
            Verdict::Allow => panic!("expected deny for sudo"),
        }
        match evaluate(&argv("ls -la /var/run/docker.sock"), None) {
            Verdict::Deny { rule_id, .. } => assert_eq!(rule_id, "boundary_escape"),
            Verdict::Allow => panic!("expected deny for docker.sock"),
        }
    }

    #[test]
    fn denies_recursive_wipe_of_workspace_or_home() {
        for cmd in [
            "rm -rf /workspace",
            "rm -fr ~",
            "rm -rf /",
            "rm --recursive --force /",
        ] {
            match evaluate(&argv(cmd), None) {
                Verdict::Deny { rule_id, .. } => assert_eq!(rule_id, "destructive"),
                Verdict::Allow => panic!("expected deny for `{cmd}`"),
            }
        }
    }

    #[test]
    fn allows_ordinary_commands() {
        for cmd in [
            "git status",
            "git push origin main",
            "cargo test",
            "rm -rf /workspace/target",
            "rm file.txt",
        ] {
            assert!(
                matches!(evaluate(&argv(cmd), None), Verdict::Allow),
                "{cmd}"
            );
        }
    }

    fn standard_ctx(expected_verbs: &[&str]) -> TaskContext {
        TaskContext {
            session_id: "test-session".to_string(),
            spec_title: "test spec".to_string(),
            gate: GateLevel::Standard,
            expected_verbs: expected_verbs.iter().map(|v| v.to_string()).collect(),
            allowed_paths: vec!["/workspace".to_string()],
            forbidden_paths: vec![],
            started_at: "2026-09-29T10:00:00Z".to_string(),
        }
    }

    const SESSION: &str = "11111111-2222-3333-4444-555555555555";

    /// A fresh path under the OS temp dir, independent of `$HOME`.
    fn temp_context_path(label: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "moor-decision-test-{label}-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    fn load_str(label: &str, contents: &str) -> Result<Option<TaskContext>> {
        let path = temp_context_path(label);
        std::fs::write(&path, contents).unwrap();
        let loaded = TaskContext::load(&path, SESSION);
        std::fs::remove_file(&path).ok();
        loaded
    }

    #[test]
    fn missing_context_is_no_context() {
        let loaded = TaskContext::load(&temp_context_path("missing"), SESSION).unwrap();
        assert!(loaded.is_none());
    }

    #[test]
    fn loads_context_with_defaults() {
        let ctx = load_str(
            "valid",
            &format!(
                r#"{{"session_id":"{SESSION}","spec_title":"t","gate":"strict",
                    "expected_verbs":["cargo_test"],"started_at":"2026-09-29T10:00:00Z"}}"#
            ),
        )
        .unwrap()
        .expect("context should load");
        assert_eq!(ctx.gate, GateLevel::Strict);
        assert_eq!(ctx.allowed_paths, vec!["/workspace".to_string()]);
        assert!(ctx.forbidden_paths.is_empty());
    }

    #[test]
    fn malformed_context_is_an_error_not_absent() {
        assert!(load_str("malformed", "{not json").is_err());
        assert!(load_str(
            "bad-gate",
            &format!(
                r#"{{"session_id":"{SESSION}","spec_title":"t","gate":"lenient",
                "expected_verbs":[],"started_at":"2026-09-29T10:00:00Z"}}"#
            )
        )
        .is_err());
    }

    #[test]
    fn context_stamped_for_another_session_is_rejected() {
        let other = r#"{"session_id":"99999999-2222-3333-4444-555555555555","spec_title":"t",
            "gate":"standard","expected_verbs":[],"started_at":"2026-09-29T10:00:00Z"}"#;
        assert!(load_str("mismatch", other).is_err());
    }

    #[test]
    fn denies_verb_outside_expected_verbs() {
        let ctx = standard_ctx(&["cargo_check"]);
        match evaluate(&["grep".to_string(), "foo".to_string()], Some(&ctx)) {
            Verdict::Deny { rule_id, .. } => assert_eq!(rule_id, "out_of_scope"),
            Verdict::Allow => panic!("expected out_of_scope deny"),
        }
    }

    #[test]
    fn denies_path_outside_allowed_paths() {
        // `read_file` is expected so `out_of_scope` doesn't fire first.
        let ctx = standard_ctx(&["cargo_check", "read_file"]);
        match evaluate(&["cat".to_string(), "/etc/passwd".to_string()], Some(&ctx)) {
            Verdict::Deny { rule_id, .. } => assert_eq!(rule_id, "path_escape"),
            Verdict::Allow => panic!("expected path_escape deny"),
        }
    }
}
