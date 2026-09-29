use crate::{audit, decision, manifest::Manifest, paths, proc, session};
use anyhow::Result;
use std::io::{BufRead, IsTerminal, Write};
use std::path::Path;

/// True if the invoked command looks like a `git push` — the one channel
/// through which code actually leaves the sandbox for real (as opposed
/// to routine egress like `git fetch`/`clone`/package installs). Tagged
/// distinctly in the audit chain so it stands out from routine execs.
/// Best-effort by nature: it only sees commands run via `moor run`,
/// not ones typed inside an interactive `moor shell` session — see
/// docs/THREAT-MODEL.md for why a git hook baked into the image isn't a
/// stronger alternative (it runs inside the untrusted sandbox and the
/// agent can simply reconfigure or bypass it).
fn looks_like_git_push(cmd: &[String]) -> bool {
    cmd.first().map(|s| s == "git").unwrap_or(false) && cmd.iter().any(|a| a == "push")
}

/// Prints the rule that fired and asks whether to run the command
/// anyway. Only an explicit `y`/`yes` (trimmed, case-insensitive) is a
/// yes; a read error is treated the same as a decline, not a crash.
fn confirm(
    reader: &mut impl BufRead,
    writer: &mut impl Write,
    rule_id: &str,
    reason: &str,
) -> bool {
    let _ = writeln!(
        writer,
        "moor run: rule `{rule_id}` flagged this command: {reason}"
    );
    let _ = write!(writer, "Run it anyway? [y/N] ");
    let _ = writer.flush();
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() {
        return false;
    }
    matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

/// `moor run`'s stdin — an interactive confirm only makes sense when a
/// human is actually there to answer it, not when `moor run` is driven
/// by a script or keel's own driver.
fn stdin_is_tty() -> bool {
    std::io::stdin().is_terminal()
}

/// Who to attribute an override to. Best-effort, same fallback as keel's
/// own approval records when the host has no git identity configured.
fn operator_identity() -> String {
    proc::run_capture("git", &["config", "--get", "user.name"])
        .ok()
        .filter(|(status, _)| status.success())
        .map(|(_, out)| out.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

/// The host-side enforcement point for `decision::evaluate` — see
/// docs/adr/0001-tool-call-risk-gating.md and
/// docs/adr/0002-tool-call-ask-verdict.md. A rule match is resolved by
/// prompting `reader`/`writer` when `interactive`, else denied without
/// reading them at all; either way the resolution is logged before `run`
/// propagates an error, so a denied command never reaches `docker exec`.
/// IO, identity and task context are parameters, not read directly, so
/// this is testable without a real terminal, `$HOME`, or a running
/// container.
#[allow(clippy::too_many_arguments)]
fn gate(
    chain_path: &Path,
    name: &str,
    secrets: &[String],
    cmd: &[String],
    ctx: Option<&decision::TaskContext>,
    interactive: bool,
    reader: &mut impl BufRead,
    writer: &mut impl Write,
    operator: &str,
) -> Result<()> {
    let decision::Verdict::Deny { rule_id, reason } = decision::evaluate(cmd, ctx) else {
        return Ok(());
    };
    let allowed = interactive && confirm(reader, writer, rule_id, &reason);
    audit::log_decision(chain_path, name, cmd, secrets, rule_id, allowed, operator)?;
    if allowed {
        return Ok(());
    }
    anyhow::bail!("moor run: blocked by rule `{rule_id}`: {reason}");
}

/// The task context stamped for the project's current `moor ask`
/// session, if there is one. No session, or no context stamped for it,
/// leaves only the context-free rules in force; a context that exists but
/// can't be trusted is an error, not a silent downgrade.
fn task_context(name: &str) -> Result<Option<decision::TaskContext>> {
    let Some(session_id) = session::read_session_id(&paths::session_path(name)?) else {
        return Ok(None);
    };
    decision::TaskContext::load(&paths::task_context_path(&session_id)?, &session_id)
}

pub fn run(name: &str, cmd: &[String]) -> Result<()> {
    if cmd.is_empty() {
        anyhow::bail!("usage: moor run <project> -- <command...>");
    }
    let m = Manifest::load(&paths::manifest_path(name)?)?;
    // So audit::redact below can scrub a secret's value even when it was
    // only ever set via Keychain, never exported into this shell.
    crate::secrets::resolve_into_env(name, &m.secrets);
    let ctx = task_context(name)?;
    gate(
        &paths::chain_log_path(name)?,
        name,
        &m.secrets,
        cmd,
        ctx.as_ref(),
        stdin_is_tty(),
        &mut std::io::stdin().lock(),
        &mut std::io::stderr(),
        &operator_identity(),
    )?;
    let container = m.sandbox_container();

    // What a push sends is resolved before it runs: afterwards the ref may
    // have moved, and the question is what left, not what is there now.
    let pushed = looks_like_git_push(cmd).then(|| {
        let (dir, git_ref) = push_target(cmd);
        let commit = resolve_commit(&container, &dir, &git_ref);
        push_data(&git_ref, commit.as_deref())
    });

    let mut args: Vec<&str> = vec!["exec", &container];
    args.extend(cmd.iter().map(|s| s.as_str()));

    let status = proc::run_inherit("docker", &args)?;
    match pushed {
        Some(data) => audit::log_exec_with(name, &m, "push", cmd, status.code(), data)?,
        None => audit::log_exec(name, &m, "exec", cmd, status.code())?,
    }
    proc::require_success(&format!("`{}` in {container}", cmd.join(" ")), status)?;
    Ok(())
}

/// The repository a `git push` runs in (`-C <dir>`, else `/workspace`) and
/// the ref it sends: the source side of an explicit refspec, else `HEAD`.
fn push_target(cmd: &[String]) -> (String, String) {
    let mut dir = "/workspace".to_string();
    let mut args = cmd.iter().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "-C" => {
                if let Some(d) = args.next() {
                    dir = d.clone();
                }
            }
            "push" => break,
            _ => {}
        }
    }
    // After `push`: options, then [<remote> [<refspec>...]].
    let positional: Vec<&String> = args.filter(|a| !a.starts_with('-')).collect();
    let git_ref = positional
        .get(1)
        .map(|spec| {
            spec.trim_start_matches('+')
                .split(':')
                .next()
                .unwrap_or("")
                .to_string()
        })
        .filter(|r| !r.is_empty())
        .unwrap_or_else(|| "HEAD".to_string());
    (dir, git_ref)
}

fn resolve_commit(container: &str, dir: &str, git_ref: &str) -> Option<String> {
    let rev = format!("{git_ref}^{{commit}}");
    let (status, out) = proc::run_capture(
        "docker",
        &[
            "exec",
            container,
            "git",
            "-C",
            dir,
            "rev-parse",
            "--verify",
            "--quiet",
            &rev,
        ],
    )
    .ok()?;
    let sha = out.trim();
    (status.success() && sha.len() == 40 && sha.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| sha.to_string())
}

/// The fields a `push` entry adds to an exec entry. `commit` is null when
/// the ref did not resolve — recorded as unknown rather than omitted.
fn push_data(git_ref: &str, commit: Option<&str>) -> serde_json::Value {
    serde_json::json!({ "ref": git_ref, "commit": commit })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_plain_git_push() {
        assert!(looks_like_git_push(&["git".into(), "push".into()]));
        assert!(looks_like_git_push(&[
            "git".into(),
            "push".into(),
            "origin".into(),
            "main".into()
        ]));
    }

    #[test]
    fn does_not_flag_other_git_commands() {
        assert!(!looks_like_git_push(&[
            "git".into(),
            "clone".into(),
            "x".into()
        ]));
        assert!(!looks_like_git_push(&["git".into(), "fetch".into()]));
        assert!(!looks_like_git_push(&["git".into(), "status".into()]));
    }

    #[test]
    fn does_not_flag_non_git_commands() {
        assert!(!looks_like_git_push(&["keel".into(), "run".into()]));
        assert!(!looks_like_git_push(&[
            "npm".into(),
            "run".into(),
            "push".into()
        ]));
    }

    #[test]
    fn empty_command_is_not_a_push() {
        assert!(!looks_like_git_push(&[]));
    }

    fn argv(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    /// A fresh, never-before-used chain path under the OS temp dir —
    /// independent of `$HOME`, so this stays a real unit test.
    fn temp_chain_path(label: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "moor-run-cmd-test-{label}-{}-{}.jsonl",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    /// Runs `gate` against a throwaway chain path with the given stdin
    /// content and interactivity, and returns (result, chain contents).
    fn run_gate(interactive: bool, stdin: &str, cmd: &[String]) -> (Result<()>, String) {
        run_gate_with(None, interactive, stdin, cmd)
    }

    fn run_gate_with(
        ctx: Option<&decision::TaskContext>,
        interactive: bool,
        stdin: &str,
        cmd: &[String],
    ) -> (Result<()>, String) {
        let chain_path = temp_chain_path("gate");
        let mut reader = std::io::Cursor::new(stdin.as_bytes());
        let mut writer = Vec::new();
        let result = gate(
            &chain_path,
            "test-project",
            &[],
            cmd,
            ctx,
            interactive,
            &mut reader,
            &mut writer,
            "test-operator",
        );
        let logged = std::fs::read_to_string(&chain_path).unwrap_or_default();
        std::fs::remove_file(&chain_path).ok();
        (result, logged)
    }

    #[test]
    fn deny_verdict_blocks_docker_exec() {
        let (result, logged) = run_gate(false, "", &argv("cat /home/agent/.ssh/id_rsa"));

        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("credential_access"), "{msg}");
        assert!(logged.contains("\"kind\":\"decision\""), "{logged}");
        assert!(
            logged.contains("\"rule_id\":\"credential_access\""),
            "{logged}"
        );
    }

    #[test]
    fn allow_verdict_is_a_no_op() {
        let chain_path = temp_chain_path("allow");
        let mut reader = std::io::Cursor::new(&b""[..]);
        let mut writer = Vec::new();

        assert!(gate(
            &chain_path,
            "test-project",
            &[],
            &argv("git status"),
            None,
            true,
            &mut reader,
            &mut writer,
            "test-operator"
        )
        .is_ok());
        assert!(!chain_path.exists());
        assert!(writer.is_empty(), "Allow must never prompt");
    }

    #[test]
    fn interactive_yes_allows() {
        for answer in ["y\n", "Y\n", "yes\n", "YES\n"] {
            let (result, logged) = run_gate(true, answer, &argv("sudo ls"));
            assert!(result.is_ok(), "answer `{answer:?}` should allow");
            assert!(logged.contains("\"resolution\":\"allow\""), "{logged}");
            assert!(logged.contains("\"by\":\"test-operator\""), "{logged}");
        }
    }

    #[test]
    fn interactive_no_denies() {
        for answer in ["n\n", "no\n", "\n", "whatever\n"] {
            let (result, logged) = run_gate(true, answer, &argv("sudo ls"));
            assert!(result.is_err(), "answer `{answer:?}` should deny");
            assert!(logged.contains("\"resolution\":\"deny\""), "{logged}");
        }
    }

    #[test]
    fn non_interactive_denies_without_reading_stdin() {
        // stdin contains "y\n" but interactive is false: it must never be
        // consulted, so the answer it would give is irrelevant.
        let (result, logged) = run_gate(false, "y\n", &argv("sudo ls"));
        assert!(result.is_err());
        assert!(logged.contains("\"resolution\":\"deny\""), "{logged}");
    }

    #[test]
    fn decision_entry_records_resolution_and_operator() {
        let (_, logged) = run_gate(true, "y\n", &argv("sudo ls"));
        assert!(
            logged.contains("\"rule_id\":\"boundary_escape\""),
            "{logged}"
        );
        assert!(logged.contains("\"resolution\":\"allow\""), "{logged}");
        assert!(logged.contains("\"by\":\"test-operator\""), "{logged}");
    }

    fn standard_ctx(expected_verbs: &[&str]) -> decision::TaskContext {
        decision::TaskContext {
            session_id: "11111111-2222-3333-4444-555555555555".to_string(),
            spec_title: "test spec".to_string(),
            gate: decision::GateLevel::Standard,
            expected_verbs: expected_verbs.iter().map(|v| v.to_string()).collect(),
            allowed_paths: vec!["/workspace".to_string()],
            forbidden_paths: vec![],
            started_at: "2026-09-29T10:00:00Z".to_string(),
        }
    }

    #[test]
    fn task_context_reaches_the_rules() {
        let ctx = standard_ctx(&["cargo_test"]);

        let (result, logged) = run_gate_with(Some(&ctx), false, "", &argv("git status"));
        assert!(result.is_err());
        assert!(logged.contains("\"rule_id\":\"out_of_scope\""), "{logged}");

        let (result, _) = run_gate_with(Some(&ctx), false, "", &argv("cargo test"));
        assert!(result.is_ok(), "an expected verb must still run");
    }

    #[test]
    fn confirm_prompts_with_rule_and_reason() {
        let mut reader = std::io::Cursor::new(&b"y\n"[..]);
        let mut writer = Vec::new();
        assert!(confirm(&mut reader, &mut writer, "destructive", "wipes /"));
        let printed = String::from_utf8(writer).unwrap();
        assert!(printed.contains("destructive"), "{printed}");
        assert!(printed.contains("wipes /"), "{printed}");
    }

    #[test]
    fn push_entry_names_ref_and_commit() {
        assert_eq!(
            push_target(&argv("git push")),
            ("/workspace".into(), "HEAD".into())
        );
        assert_eq!(
            push_target(&argv("git push origin")),
            ("/workspace".into(), "HEAD".into())
        );
        assert_eq!(
            push_target(&argv("git push -u origin feature")),
            ("/workspace".into(), "feature".into())
        );
        assert_eq!(
            push_target(&argv(
                "git -C /workspace/app push --force origin +main:release"
            )),
            ("/workspace/app".into(), "main".into())
        );

        let sha = "0123456789abcdef0123456789abcdef01234567";
        let data = push_data("main", Some(sha));
        assert_eq!(data["ref"], "main");
        assert_eq!(data["commit"], sha);
        assert!(push_data("main", None)["commit"].is_null());
    }
}
