//! A deterministic, rules-only risk gate for `moor run` commands. See
//! docs/adr/0001-tool-call-risk-gating.md: this table is what a future
//! model backend would sit behind, not a stand-in that gets replaced —
//! rules run first and settle what they can, a model would only ever see
//! what these rules don't.

#[derive(Debug)]
pub enum Verdict {
    Allow,
    Deny { rule_id: &'static str, reason: String },
}

const CREDENTIAL_MARKERS: &[&str] = &[".ssh", ".aws", ".netrc", "id_rsa", ".pem"];
const WIPE_TARGETS: &[&str] = &["/", "/workspace", "~"];

pub fn evaluate(cmd: &[String]) -> Verdict {
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
            match evaluate(&argv(cmd)) {
                Verdict::Deny { rule_id, .. } => assert_eq!(rule_id, "credential_access"),
                Verdict::Allow => panic!("expected deny for `{cmd}`"),
            }
        }
    }

    #[test]
    fn denies_docker_socket_and_sudo() {
        match evaluate(&argv("sudo ls")) {
            Verdict::Deny { rule_id, .. } => assert_eq!(rule_id, "boundary_escape"),
            Verdict::Allow => panic!("expected deny for sudo"),
        }
        match evaluate(&argv("ls -la /var/run/docker.sock")) {
            Verdict::Deny { rule_id, .. } => assert_eq!(rule_id, "boundary_escape"),
            Verdict::Allow => panic!("expected deny for docker.sock"),
        }
    }

    #[test]
    fn denies_recursive_wipe_of_workspace_or_home() {
        for cmd in ["rm -rf /workspace", "rm -fr ~", "rm -rf /", "rm --recursive --force /"] {
            match evaluate(&argv(cmd)) {
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
            assert!(matches!(evaluate(&argv(cmd)), Verdict::Allow), "{cmd}");
        }
    }
}
