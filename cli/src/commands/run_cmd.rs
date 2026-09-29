use crate::{audit, manifest::Manifest, paths, proc};
use anyhow::Result;

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

pub fn run(name: &str, cmd: &[String]) -> Result<()> {
    if cmd.is_empty() {
        anyhow::bail!("usage: moor run <project> -- <command...>");
    }
    let m = Manifest::load(&paths::manifest_path(name)?)?;
    // So audit::redact below can scrub a secret's value even when it was
    // only ever set via Keychain, never exported into this shell.
    crate::secrets::resolve_into_env(name, &m.secrets);
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
