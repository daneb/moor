//! One conversational turn with the agent inside a project's sandbox.
//!
//! `moor shell` puts an operator in front of the agent but leaves the
//! whole exchange outside the audit chain (its single entry's redacted
//! argv is the literal string "shell"). `moor recipe` is chained but has
//! no conversational step at all — the recipe is written blind. This
//! module is the middle: a request/response turn that is chained before
//! the operator ever sees the answer, resumable across turns, and
//! tool-bounded by role. See docs/decisions/0008-agent-session-protocol.md.

use crate::{audit, manifest::Manifest, paths, proc};
use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::path::Path;

/// Where the MCP config lands in the image (images/base/Dockerfile).
const MCP_CONFIG: &str = "/etc/moor/mcp-config.json";

/// What a turn may do, as an actual tool-access boundary rather than a
/// prompt-worded request for restraint — the same distinction
/// `recipe::author_spec` already relies on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// Think a change through against the real repository. Read-only.
    Brainstorm,
    /// Make the change and check it, through the MCP server's
    /// verification verbs. Deliberately no `Bash`: a shell would hand
    /// back `keel approve`, which is the one thing the MCP surface exists
    /// to keep out of the agent's world.
    Build,
}

impl Role {
    pub fn parse(s: &str) -> Result<Role> {
        match s {
            "brainstorm" => Ok(Role::Brainstorm),
            "build" => Ok(Role::Build),
            other => anyhow::bail!("unknown role '{other}': expected 'brainstorm' or 'build'"),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Role::Brainstorm => "brainstorm",
            Role::Build => "build",
        }
    }

    /// The exact `--allowedTools` set for this role.
    ///
    /// On its own this grants *auto-approval*, not exclusivity — see
    /// `denied_tools`, and ADR-0008's correction. A tool absent from here
    /// is not thereby unavailable.
    pub fn tools(&self) -> &'static [&'static str] {
        match self {
            Role::Brainstorm => &["Read", "Glob", "Grep"],
            Role::Build => &[
                "Read",
                "Glob",
                "Grep",
                "Edit",
                "Write",
                "mcp__moor-keel__keel_gate",
                "mcp__moor-keel__keel_next",
            ],
        }
    }

    /// The `--disallowedTools` set — the part that is actually a boundary.
    ///
    /// Measured against a live sandbox, not assumed: with `--allowedTools
    /// Read Glob Grep` and nothing else, Claude Code ran `Bash` anyway and
    /// returned its output, `permission_denials: []`. `--allowedTools` is
    /// an auto-approval list; under `--print` there is nobody to ask about
    /// anything else, and every permission mode (`auto`, `manual`,
    /// `dontAsk`, `plan`) let it through. `--disallowedTools` denied it —
    /// the agent then reported having no shell tool at all.
    ///
    /// So this list is load-bearing, and it is a list, which is the kind of
    /// control ADR-0003 wanted to avoid. That is a property of Claude
    /// Code's permission model rather than a choice moor gets to make: the
    /// MCP server still keeps `keel approve` off the tool surface, but only
    /// this keeps it from being reachable through a shell. Over-denying is
    /// the safe direction — naming a tool that does not exist costs
    /// nothing, missing one that does costs the checkpoint.
    pub fn denied_tools(&self) -> Vec<&'static str> {
        let mut denied: Vec<&'static str> = SHELL_AND_SUBAGENT.to_vec();
        denied.extend_from_slice(NETWORK);
        if *self == Role::Brainstorm {
            denied.extend_from_slice(MUTATING);
        }
        denied
    }
}

/// Tools that reach a shell, spawn a subagent, or run a slash command —
/// any one of which hands back everything the tool list was supposed to
/// withhold. `Bash` is the decisive one: with it, the agent can run `keel
/// approve` directly and the MCP server's whole reason for existing is
/// moot. `Task` is here because a subagent's tool set is not this one's.
const SHELL_AND_SUBAGENT: &[&str] = &[
    "Bash",
    "BashOutput",
    "KillShell",
    "KillBash",
    "Task",
    "SlashCommand",
];

/// Tools that change the workspace.
const MUTATING: &[&str] = &["Write", "Edit", "MultiEdit", "NotebookEdit"];

/// Tools that fetch from the network. The egress proxy default-denies
/// anyway, but a turn has no business reaching past it.
const NETWORK: &[&str] = &["WebFetch", "WebSearch"];

/// `--disallowedTools` plus every route to a shell or a subagent, ready to
/// append to any `claude` argv. Shared with `commands::recipe`, whose
/// agent steps make the same assumption this corrects: that granting one
/// tool withholds the others. It does not.
pub fn deny_shell_argv() -> Vec<String> {
    let mut argv = vec!["--disallowedTools".to_string()];
    argv.extend(SHELL_AND_SUBAGENT.iter().map(|t| t.to_string()));
    argv
}

/// The fields of `claude --print --output-format json` this depends on,
/// verified directly against the image's Claude Code 2.1.270 rather than
/// assumed. `subtype` is deliberately absent: a failed turn there returned
/// `"is_error": true` alongside `"subtype": "success"`, so reading
/// `subtype` as a success signal would record that turn as a success.
#[derive(Debug, Default, Deserialize)]
pub struct TurnResult {
    #[serde(default)]
    pub is_error: bool,
    #[serde(default)]
    pub result: String,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub num_turns: Option<u64>,
    #[serde(default)]
    pub permission_denials: Vec<serde_json::Value>,
}

pub fn parse_turn(stdout: &str) -> Result<TurnResult> {
    serde_json::from_str(stdout.trim())
        .with_context(|| format!("parsing `claude --output-format json` output:\n{stdout}"))
}

/// A turn failed if it says so, whatever the process exit status said.
pub fn turn_failed(t: &TurnResult, exit_ok: bool) -> bool {
    t.is_error || !exit_ok
}

/// A session id is only usable if it looks like the UUID Claude Code
/// actually returns — 8-4-4-4-12 lowercase hex. Anything else is either a
/// different shape of output or something that came from the wrong place,
/// and in both cases resuming from it is worse than starting fresh.
pub fn valid_session_id(id: &str) -> bool {
    let groups = [8usize, 4, 4, 4, 12];
    let parts: Vec<&str> = id.split('-').collect();
    parts.len() == groups.len()
        && parts
            .iter()
            .zip(groups)
            .all(|(p, n)| p.len() == n && p.chars().all(|c| c.is_ascii_hexdigit()))
}

/// The id to carry into the next turn — taken from the parsed
/// `session_id` field and nowhere else. Text the agent produced is never
/// consulted, so a response that prints a plausible UUID cannot redirect
/// the next turn at another session.
pub fn next_session_id(t: &TurnResult) -> Option<String> {
    t.session_id
        .as_deref()
        .filter(|id| valid_session_id(id))
        .map(str::to_string)
}

fn store_session_id(path: &Path, id: &str) -> Result<()> {
    if !valid_session_id(id) {
        anyhow::bail!("refusing to store malformed session id '{id}'");
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, id).with_context(|| format!("writing {}", path.display()))
}

fn read_session_id(path: &Path) -> Option<String> {
    let raw = std::fs::read_to_string(path).ok()?;
    let id = raw.trim();
    // Re-validated on the way out as well as in: a hand-edited file is
    // the same problem as a malformed response.
    valid_session_id(id).then(|| id.to_string())
}

fn sha256_hex(text: &str) -> String {
    let mut h = Sha256::new();
    h.update(text.as_bytes());
    hex::encode(h.finalize())
}

/// Append this turn's `agent-turn` entry to the project's existing chain.
/// Hashes of the prompt and response, never the text — the chain is
/// tamper-evidence over *what happened*, and it is exported by `moor
/// audit --export`, so putting conversation content in it would leak
/// through every bundle. The full text goes to the redacted transcript
/// instead (`write_transcript`).
fn append_turn_to(path: &Path, turn: &ChainedTurn) -> Result<audit::ChainEntry> {
    audit::append_chained(
        path,
        "agent-turn",
        json!({
            "project": turn.project,
            "role": turn.role.as_str(),
            "tools": turn.role.tools(),
            "session_id": turn.session_id,
            "prompt_sha256": sha256_hex(turn.prompt),
            "response_sha256": sha256_hex(turn.response),
            "result": if turn.failed { "error" } else { "ok" },
            "num_turns": turn.result.num_turns,
            // The count only: a denial record carries the argument the
            // agent was denied, which is content.
            "permission_denials": turn.result.permission_denials.len(),
        }),
    )
}

/// Everything one chain entry commits to, gathered in one place so the
/// prompt and the response can't drift apart from the hashes taken of
/// them.
struct ChainedTurn<'a> {
    project: &'a str,
    role: Role,
    session_id: Option<&'a str>,
    prompt: &'a str,
    response: &'a str,
    failed: bool,
    result: &'a TurnResult,
}

/// Append the turn's full text to the project's transcript, redacted with
/// the project's declared secret names the same way `log_exec` masks
/// argv. The agent can echo a secret it was given back at us; this is
/// what stops that landing in a file at rest.
fn write_transcript(
    path: &Path,
    role: Role,
    prompt: &str,
    response: &str,
    secret_names: &[String],
) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let line = json!({
        "ts": chrono::Utc::now().to_rfc3339(),
        "role": role.as_str(),
        "prompt": audit::redact(prompt, secret_names),
        "response": audit::redact(response, secret_names),
    });
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("opening transcript {}", path.display()))?;
    writeln!(f, "{line}")?;
    Ok(())
}

/// Flags whose value is `<thing...>` in `claude --help` — variadic, and so
/// greedy that they swallow a bare positional that follows them. The
/// prompt must come before every one of these or `claude` never sees a
/// prompt at all. Found the hard way twice: `recipe::author_spec`
/// documents it for `--allowedTools`, and `--mcp-config <configs...>` then
/// did exactly the same thing to `moor ask` in live use, reporting the
/// prompt as a missing config file ("MCP config file not found:
/// /workspace/say ok").
pub const VARIADIC_FLAGS: &[&str] = &[
    "--allowedTools",
    "--allowed-tools",
    "--disallowedTools",
    "--disallowed-tools",
    "--mcp-config",
];

/// The argv for one turn: single-value flags, then the prompt, then every
/// variadic flag. `--resume <session-id>` takes exactly one value (checked
/// against `claude --help`, and against a live container) so it sits with
/// the single-value flags ahead of the prompt.
pub fn build_turn_argv(role: Role, prompt: &str, resume: Option<&str>) -> Vec<String> {
    let mut argv: Vec<String> = vec![
        "claude".into(),
        "--print".into(),
        "--output-format".into(),
        "json".into(),
    ];
    if let Some(id) = resume {
        argv.push("--resume".into());
        argv.push(id.to_string());
    }
    // The prompt, and then every variadic flag — in that order, which is
    // the whole point. Routing them through one loop after the prompt is
    // pushed is what makes the ordering structural rather than a thing to
    // remember: a new variadic flag gets added to this table, and lands on
    // the correct side of the prompt by construction.
    argv.push(prompt.to_string());
    let variadic: [(&str, Vec<String>); 3] = [
        ("--mcp-config", vec![MCP_CONFIG.to_string()]),
        (
            "--allowedTools",
            role.tools().iter().map(|t| t.to_string()).collect(),
        ),
        (
            "--disallowedTools",
            role.denied_tools().iter().map(|t| t.to_string()).collect(),
        ),
    ];
    for (flag, values) in variadic {
        debug_assert!(
            VARIADIC_FLAGS.contains(&flag),
            "{flag} is appended after the prompt, so it must be listed in VARIADIC_FLAGS"
        );
        argv.push(flag.to_string());
        argv.extend(values);
    }
    argv
}

/// Translate an agent failure moor can recognise into the command that
/// fixes it. Claude Code reports a missing credential as "Not logged in ·
/// Please run /login" — accurate for an interactive session, and useless
/// here: there is no interactive session to run `/login` in, and the fix
/// is host-side. Secrets are scoped per project+secret pair, so a token
/// stored for one project says nothing about another.
pub fn failure_hint(project: &str, text: &str) -> Option<String> {
    if text.contains("Not logged in") || text.contains("/login") {
        return Some(format!(
            "this sandbox has no Claude Code credential. `/login` can't help — auth is \
             injected from the host at `moor up` (ADR-0002). Check where it resolves from:\n\n    \
             moor secrets status {project}\n\n\
             then store one and recreate the container so it gets injected:\n\n    \
             moor secrets set {project} CLAUDE_CODE_OAUTH_TOKEN\n    \
             moor up {project}\n\n\
             The value comes from `claude setup-token` on the host; the same token works \
             for every project, but has to be set per project."
        ));
    }
    None
}

/// Turn a turn that produced no parseable JSON into something an operator
/// can act on. `claude` writes its own refusals to stderr and nothing to
/// stdout, so without this the only symptom is a JSON parse error about
/// column 0 of an empty string.
fn unparseable_turn(
    project: &str,
    stdout: &str,
    stderr: &str,
    parse_err: anyhow::Error,
) -> anyhow::Error {
    let stderr = stderr.trim();
    if stderr.contains("MCP config file not found") && stderr.contains(MCP_CONFIG) {
        return anyhow::anyhow!(
            "this sandbox predates moor's MCP server — {MCP_CONFIG} is not in its image.\n\
             Rebuild the images and recreate the container:\n\n    \
             make images && moor up {project}\n\n\
             claude said:\n{stderr}"
        );
    }
    if stdout.trim().is_empty() {
        return anyhow::anyhow!(
            "`claude` produced no output and did not start a turn. It said:\n{}",
            if stderr.is_empty() {
                "(nothing on stderr either)"
            } else {
                stderr
            }
        );
    }
    if stderr.is_empty() {
        return parse_err;
    }
    parse_err.context(format!("claude also wrote to stderr:\n{stderr}"))
}

/// Write a recipe the agent drafted, but only once it parses under the
/// same parser `moor recipe` uses. The agent contributes text; the host
/// creates the file that drives its own next run — via a temp file beside
/// the destination, so a draft that doesn't parse leaves nothing behind.
pub fn emit_recipe(dest: &Path, text: &str) -> Result<()> {
    let body = strip_code_fence(text);
    let tmp = dest.with_extension("recipe-draft");
    if let Some(parent) = dest.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    std::fs::write(&tmp, &body).with_context(|| format!("writing {}", tmp.display()))?;
    let parsed = crate::commands::recipe::parse_recipe(&tmp);
    match parsed {
        Ok(_) => {
            std::fs::rename(&tmp, dest)
                .with_context(|| format!("moving draft into {}", dest.display()))?;
            Ok(())
        }
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e).with_context(|| {
                format!(
                    "the agent's recipe draft does not parse — nothing was written to {}",
                    dest.display()
                )
            })
        }
    }
}

/// Agents wrap file content in a fenced block more often than not; the
/// fence is presentation, not part of the recipe.
fn strip_code_fence(text: &str) -> String {
    let trimmed = text.trim();
    let Some(rest) = trimmed.strip_prefix("```") else {
        return format!("{trimmed}\n");
    };
    let body = rest.split_once('\n').map(|(_lang, b)| b).unwrap_or("");
    let body = body.trim_end().strip_suffix("```").unwrap_or(body);
    format!("{}\n", body.trim_end())
}

/// What one completed turn amounts to, once it has been chained and
/// transcribed. Returned rather than printed so a caller that owns the
/// terminal (`moor studio`) can draw it itself — the point being that
/// there is one chaining implementation, not one per front end.
pub struct TurnOutcome {
    pub text: String,
    pub failed: bool,
    pub denials: usize,
    /// moor's own read of *why* a turn failed, when the agent's message
    /// names a cause moor can act on. Kept separate from `text` so the
    /// chain entry and the transcript record what the agent actually said,
    /// never moor's commentary on it.
    pub hint: Option<String>,
}

/// Issue one turn: resume from the host's stored session id, run the agent
/// in the sandbox, then chain and transcribe it. Writes nothing to the
/// operator's terminal, so "chained before shown" holds for every caller
/// by construction — there is no path to a response that skips this
/// function.
pub fn run_turn(project: &str, role: Role, prompt: &str, new_session: bool) -> Result<TurnOutcome> {
    let m = Manifest::load(&paths::manifest_path(project)?)?;
    crate::secrets::resolve_into_env(project, &m.secrets);
    paths::ensure_project_dirs(project)?;

    let session_path = paths::session_path(project)?;
    let resume = if new_session {
        None
    } else {
        read_session_id(&session_path)
    };
    let argv = build_turn_argv(role, prompt, resume.as_deref());

    let container = m.sandbox_container();
    let mut args: Vec<&str> = vec!["exec", &container];
    args.extend(argv.iter().map(String::as_str));
    let (status, out, err) = proc::run_capture_split("docker", &args)?;

    let turn = parse_turn(&out).map_err(|e| unparseable_turn(project, &out, &err, e))?;
    let failed = turn_failed(&turn, status.success());
    let session_id = next_session_id(&turn);

    // Chained before the caller can show anything — AC-6 of
    // `agent-session-protocol`.
    append_turn_to(
        &paths::chain_log_path(project)?,
        &ChainedTurn {
            project,
            role,
            session_id: session_id.as_deref(),
            prompt,
            response: &turn.result,
            failed,
            result: &turn,
        },
    )?;
    write_transcript(
        &paths::transcript_path(project)?,
        role,
        prompt,
        &turn.result,
        &m.secrets,
    )?;
    if let Some(id) = &session_id {
        store_session_id(&session_path, id)?;
    }

    let hint = failed
        .then(|| failure_hint(project, &turn.result))
        .flatten();
    Ok(TurnOutcome {
        text: turn.result,
        failed,
        denials: turn.permission_denials.len(),
        hint,
    })
}

/// `moor ask`: one turn, printed for a human.
pub fn ask(
    project: &str,
    role: Role,
    prompt: &str,
    new_session: bool,
    emit: Option<&Path>,
) -> Result<()> {
    println!("==> [{}] turn in project {project}", role.as_str());
    let outcome = run_turn(project, role, prompt, new_session)?;

    println!("{}", outcome.text);
    if outcome.denials > 0 {
        println!(
            "==> {} tool call(s) were denied — the role's tool set is {:?}",
            outcome.denials,
            role.tools()
        );
    }
    if outcome.failed {
        if let Some(hint) = &outcome.hint {
            println!("\n==> {hint}");
        }
        anyhow::bail!("the agent reported an error for this turn (is_error)");
    }
    if let Some(dest) = emit {
        emit_recipe(dest, &outcome.text)?;
        println!("==> wrote {} — run it with `moor recipe`", dest.display());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_path(label: &str, ext: &str) -> std::path::PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("moor-session-test-{label}-{nanos}.{ext}"))
    }

    fn result_json(is_error: bool, subtype: &str, session_id: &str, text: &str) -> String {
        json!({
            "type": "result",
            "subtype": subtype,
            "is_error": is_error,
            "result": text,
            "session_id": session_id,
            "num_turns": 3,
            "permission_denials": [],
        })
        .to_string()
    }

    /// AC-3. Note what this asserts twice over: that the mutating tools are
    /// not *granted*, and that they are explicitly *denied*. Granting alone
    /// is not a boundary — measured live, Claude Code ran `Bash` under
    /// `--allowedTools Read Glob Grep` and returned its output. See
    /// `Role::denied_tools`.
    #[test]
    fn brainstorm_grants_no_write_tool() {
        let tools = Role::Brainstorm.tools();
        assert_eq!(tools, &["Read", "Glob", "Grep"]);
        let denied = Role::Brainstorm.denied_tools();
        for forbidden in [
            "Write",
            "Edit",
            "Bash",
            "NotebookEdit",
            "MultiEdit",
            "WebFetch",
        ] {
            assert!(
                !tools.contains(&forbidden),
                "brainstorm must not grant {forbidden}"
            );
            assert!(
                denied.contains(&forbidden),
                "brainstorm must explicitly deny {forbidden} — not granting it is not enough"
            );
        }
        // The deny list reaches the argv, or it is decoration.
        let argv = build_turn_argv(Role::Brainstorm, "think about this", None);
        let at = argv.iter().position(|a| a == "--disallowedTools").unwrap();
        for forbidden in ["Bash", "Write", "Edit", "Task"] {
            assert!(argv[at..].iter().any(|a| a == forbidden));
        }
        // And the granted set in the argv is exactly the role's — the
        // values of `--allowedTools` up to whatever flag comes next.
        let argv = build_turn_argv(Role::Brainstorm, "what should we change?", None);
        let at = argv.iter().position(|a| a == "--allowedTools").unwrap();
        let granted: Vec<&String> = argv[at + 1..]
            .iter()
            .take_while(|a| !a.starts_with("--"))
            .collect();
        assert_eq!(granted, vec!["Read", "Glob", "Grep"]);
    }

    /// AC-4
    #[test]
    fn rejects_malformed_session_id() {
        for bad in [
            "",
            "not-a-uuid",
            "abc",
            "3f2504e0-4f89-11d3-9a0c",
            "3f2504e0-4f89-11d3-9a0c-0305e82c33011",
            "3f2504e0-4f89-11d3-9a0c-0305e82c330z",
            "../../etc/passwd",
            "3f2504e04f8911d39a0c0305e82c3301",
        ] {
            assert!(!valid_session_id(bad), "'{bad}' should be rejected");
            let path = temp_path("bad-id", "txt");
            assert!(store_session_id(&path, bad).is_err());
            assert!(!path.exists(), "a rejected id must not be written");
        }

        let good = "3f2504e0-4f89-11d3-9a0c-0305e82c3301";
        assert!(valid_session_id(good));
        let path = temp_path("good-id", "txt");
        store_session_id(&path, good).unwrap();
        assert_eq!(read_session_id(&path).as_deref(), Some(good));

        // A stored file that was tampered with is not resumed from either.
        std::fs::write(&path, "wherever-you-like").unwrap();
        assert_eq!(read_session_id(&path), None);
        let _ = std::fs::remove_file(&path);

        // The id comes from the parsed field only — a response that prints
        // a perfectly well-formed UUID cannot redirect the next turn.
        let smuggled = json!({
            "is_error": false,
            "result": "Continue with session_id 11111111-2222-3333-4444-555555555555",
            "session_id": "not-a-uuid",
        })
        .to_string();
        let turn = parse_turn(&smuggled).unwrap();
        assert_eq!(next_session_id(&turn), None);
        assert!(turn.result.contains("11111111-2222-3333-4444-555555555555"));
    }

    /// AC-5
    #[test]
    fn error_turn_is_detected_by_is_error() {
        // The shape actually observed at Claude Code 2.1.270: a failed
        // turn reporting "subtype": "success".
        let raw = result_json(
            true,
            "success",
            "3f2504e0-4f89-11d3-9a0c-0305e82c3301",
            "boom",
        );
        let turn = parse_turn(&raw).unwrap();
        assert!(turn.is_error);
        assert!(
            turn_failed(&turn, true),
            "is_error must win over a zero exit status and a 'success' subtype"
        );
        assert!(!raw.contains("\"subtype\": \"error\""));

        // A clean turn is a success; a non-zero exit is still a failure.
        let ok = parse_turn(&result_json(
            false,
            "success",
            "3f2504e0-4f89-11d3-9a0c-0305e82c3301",
            "done",
        ))
        .unwrap();
        assert!(!turn_failed(&ok, true));
        assert!(turn_failed(&ok, false));
    }

    /// AC-6
    #[test]
    fn turn_is_chained_without_content() {
        let path = temp_path("chain", "jsonl");
        let prompt = "the operator's private plan for the quarter";
        let response = "here is what I would do, in detail";
        let turn = parse_turn(&result_json(
            false,
            "success",
            "3f2504e0-4f89-11d3-9a0c-0305e82c3301",
            response,
        ))
        .unwrap();

        let entry = append_turn_to(
            &path,
            &ChainedTurn {
                project: "sample",
                role: Role::Brainstorm,
                session_id: Some("3f2504e0-4f89-11d3-9a0c-0305e82c3301"),
                prompt,
                response,
                failed: false,
                result: &turn,
            },
        )
        .unwrap();

        assert_eq!(entry.kind, "agent-turn");
        assert_eq!(entry.data["role"], json!("brainstorm"));
        assert_eq!(entry.data["tools"], json!(["Read", "Glob", "Grep"]));
        assert_eq!(
            entry.data["session_id"],
            json!("3f2504e0-4f89-11d3-9a0c-0305e82c3301")
        );
        assert_eq!(entry.data["prompt_sha256"], json!(sha256_hex(prompt)));
        assert_eq!(entry.data["response_sha256"], json!(sha256_hex(response)));
        assert_eq!(entry.data["result"], json!("ok"));

        // No content anywhere in the file on disk, and the chain still verifies.
        let on_disk = std::fs::read_to_string(&path).unwrap();
        assert!(!on_disk.contains(prompt));
        assert!(!on_disk.contains(response));
        assert!(!on_disk.contains("private plan"));
        assert!(matches!(
            audit::verify_chain(&path).unwrap(),
            audit::VerifyOutcome::Ok { entries: 1 }
        ));
        let _ = std::fs::remove_file(&path);
    }

    /// AC-7
    #[test]
    fn transcript_is_redacted() {
        std::env::set_var("MOOR_TEST_SESSION_SECRET", "ghp_livetoken1234567890");
        let path = temp_path("transcript", "jsonl");
        write_transcript(
            &path,
            Role::Build,
            "use the token ghp_livetoken1234567890 to push",
            "I ran it with ghp_livetoken1234567890 and it worked",
            &["MOOR_TEST_SESSION_SECRET".to_string()],
        )
        .unwrap();

        let on_disk = std::fs::read_to_string(&path).unwrap();
        assert!(
            !on_disk.contains("ghp_livetoken1234567890"),
            "the secret survived into the transcript:\n{on_disk}"
        );
        assert!(on_disk.contains("***REDACTED:MOOR_TEST_SESSION_SECRET***"));
        // Redacted, not dropped — the surrounding text is still readable.
        assert!(on_disk.contains("to push"));
        assert!(on_disk.contains("\"role\":\"build\""));
        std::env::remove_var("MOOR_TEST_SESSION_SECRET");
        let _ = std::fs::remove_file(&path);
    }

    /// AC-8
    #[test]
    fn emitted_recipe_round_trips() {
        let dest = temp_path("emitted", "md");
        let drafted = "```markdown\n---\nslug: greet-function\nscope:\n  - \"src/**\"\n---\n\nI want a greet function.\n```";
        emit_recipe(&dest, drafted).unwrap();

        // Written by the host, and it parses under the parser `moor
        // recipe` itself uses — so the emitted file can actually be run.
        let written = std::fs::read_to_string(&dest).unwrap();
        assert!(written.starts_with("---\n"));
        assert!(!written.contains("```"));
        let recipe = crate::commands::recipe::parse_recipe(&dest).unwrap();
        assert_eq!(recipe.front.slug, "greet-function");
        assert_eq!(recipe.description, "I want a greet function.");
        let _ = std::fs::remove_file(&dest);

        // A draft that doesn't parse writes nothing at all — no half-file
        // for `moor recipe` to trip over later.
        let bad = temp_path("emitted-bad", "md");
        let err = emit_recipe(&bad, "Sure! Here's how I'd approach that...").unwrap_err();
        assert!(err.to_string().contains("does not parse"));
        assert!(!bad.exists());
        assert!(!bad.with_extension("recipe-draft").exists());
    }

    #[test]
    fn build_role_has_verification_tools_but_no_shell() {
        let tools = Role::Build.tools();
        assert!(tools.contains(&"mcp__moor-keel__keel_gate"));
        assert!(tools.contains(&"mcp__moor-keel__keel_next"));
        assert!(!tools.iter().any(|t| t.starts_with("Bash")));
        assert!(!tools.iter().any(|t| t.contains("approve")));
        // But `build` can still edit — that is the point of the role.
        assert!(tools.contains(&"Edit"));
        assert!(tools.contains(&"Write"));
    }

    /// The one that matters: with a shell, the agent runs `keel approve`
    /// itself and every human checkpoint in keel's pipeline is advisory.
    /// Verified live before this existed — the `build` role reached
    /// `keel approve --help` through Bash and quoted it back — so every
    /// role must deny every route to a shell or a subagent, in the argv.
    #[test]
    fn no_role_can_reach_a_shell_or_a_subagent() {
        for role in [Role::Brainstorm, Role::Build] {
            let denied = role.denied_tools();
            let argv = build_turn_argv(role, "do the thing", None);
            let at = argv
                .iter()
                .position(|a| a == "--disallowedTools")
                .unwrap_or_else(|| panic!("{role:?} passes no deny list at all"));
            let denied_in_argv = &argv[at + 1..];
            for escape in [
                "Bash",
                "BashOutput",
                "KillShell",
                "KillBash",
                "Task",
                "SlashCommand",
            ] {
                assert!(denied.contains(&escape), "{role:?} does not deny {escape}");
                assert!(
                    denied_in_argv.iter().any(|a| a == escape),
                    "{role:?}'s argv does not deny {escape}: {argv:?}"
                );
            }
            // Nothing is both granted and denied — that would be ambiguous
            // and its resolution is not moor's to guess.
            for granted in role.tools() {
                assert!(
                    !denied.contains(granted),
                    "{role:?} both grants and denies {granted}"
                );
            }
        }
    }

    /// Every variadic flag swallows a bare positional that follows it, so
    /// the prompt has to precede all of them. This is asserted against the
    /// whole `VARIADIC_FLAGS` list rather than one flag because the bug has
    /// now happened twice: first with `--allowedTools`, then — in live use,
    /// after this was believed fixed — with `--mcp-config <configs...>`,
    /// which reported the prompt as a missing config file and left `claude`
    /// writing nothing to stdout at all.
    #[test]
    fn the_prompt_precedes_every_variadic_flag() {
        let prompt = "can we add a CI pipeline with security scans";
        for role in [Role::Brainstorm, Role::Build] {
            for resume in [None, Some("3f2504e0-4f89-11d3-9a0c-0305e82c3301")] {
                let argv = build_turn_argv(role, prompt, resume);
                let at_prompt = argv
                    .iter()
                    .position(|a| a == prompt)
                    .expect("the prompt is in the argv at all");
                for flag in VARIADIC_FLAGS {
                    if let Some(at_flag) = argv.iter().position(|a| a == flag) {
                        assert!(
                            at_prompt < at_flag,
                            "{flag} precedes the prompt, so claude will eat it: {argv:?}"
                        );
                    }
                }
                // Nothing may trail the last variadic flag's values either.
                let last_variadic = VARIADIC_FLAGS
                    .iter()
                    .filter_map(|f| argv.iter().position(|a| a == f))
                    .max()
                    .expect("a turn always passes at least one variadic flag");
                assert!(at_prompt < last_variadic);
            }
        }
    }

    /// The MCP config path is a value of `--mcp-config`, and must be the
    /// argument immediately after it — not somewhere a variadic flag
    /// earlier in the line could have absorbed.
    #[test]
    fn mcp_config_is_the_value_of_its_own_flag() {
        let argv = build_turn_argv(Role::Build, "go", None);
        let at = argv.iter().position(|a| a == "--mcp-config").unwrap();
        assert_eq!(argv[at + 1], MCP_CONFIG);
    }

    /// A turn that produced nothing parseable must hand the operator
    /// `claude`'s own words, not a JSON parse error about column 0.
    #[test]
    fn an_empty_turn_reports_what_claude_actually_said() {
        let parse_err = parse_turn("").unwrap_err();
        let stderr = "Error: Invalid MCP configuration:\nMCP config file not found: /etc/moor/mcp-config.json";
        let reported = format!("{:?}", unparseable_turn("personil", "", stderr, parse_err));
        // Names the cause and the exact command that fixes it.
        assert!(
            reported.contains("predates moor's MCP server"),
            "{reported}"
        );
        assert!(
            reported.contains("make images && moor up personil"),
            "{reported}"
        );

        // Any other silent failure still surfaces stderr verbatim.
        let parse_err = parse_turn("").unwrap_err();
        let reported = format!(
            "{:?}",
            unparseable_turn("demo", "", "Error: Invalid API key", parse_err)
        );
        assert!(reported.contains("Invalid API key"), "{reported}");
        assert!(reported.contains("produced no output"), "{reported}");

        // Output that is present but not JSON keeps the parse error and
        // adds stderr rather than replacing it.
        let parse_err = parse_turn("not json").unwrap_err();
        let reported = format!(
            "{:?}",
            unparseable_turn("demo", "not json", "a warning", parse_err)
        );
        assert!(reported.contains("not json"), "{reported}");
        assert!(reported.contains("a warning"), "{reported}");
    }

    #[test]
    fn resume_is_passed_through_when_there_is_a_stored_session() {
        let argv = build_turn_argv(
            Role::Build,
            "carry on",
            Some("3f2504e0-4f89-11d3-9a0c-0305e82c3301"),
        );
        let at = argv.iter().position(|a| a == "--resume").unwrap();
        assert_eq!(argv[at + 1], "3f2504e0-4f89-11d3-9a0c-0305e82c3301");
        // `--resume <session-id>` takes exactly one value (per `claude
        // --help`, and verified live), so it belongs ahead of the prompt.
        assert!(at < argv.iter().position(|a| a == "carry on").unwrap());
        assert!(argv.contains(&MCP_CONFIG.to_string()));
    }

    /// "Not logged in · Please run /login" is what Claude Code says when a
    /// sandbox has no credential — advice that cannot be followed from a
    /// non-interactive turn. moor has to answer the question the operator
    /// actually has: which command fixes this, for which project.
    #[test]
    fn a_missing_credential_points_at_the_host_side_fix() {
        let hint = failure_hint("omniscient", "Not logged in · Please run /login")
            .expect("a missing credential is recognised");
        assert!(hint.contains("moor secrets set omniscient CLAUDE_CODE_OAUTH_TOKEN"));
        assert!(hint.contains("moor up omniscient"));
        assert!(hint.contains("moor secrets status omniscient"));
        // Per project+secret scoping is the part that surprises people.
        assert!(hint.contains("set per project"));

        // An ordinary failure gets no invented advice.
        assert_eq!(failure_hint("demo", "I could not find that file."), None);
        assert_eq!(failure_hint("demo", ""), None);
    }

    #[test]
    fn unknown_role_is_rejected() {
        assert!(Role::parse("brainstorm").is_ok());
        assert!(Role::parse("build").is_ok());
        assert!(Role::parse("approve").is_err());
        assert!(Role::parse("").is_err());
    }
}
