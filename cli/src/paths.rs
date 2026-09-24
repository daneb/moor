use anyhow::{Context, Result};
use std::path::PathBuf;

pub fn moor_home() -> Result<PathBuf> {
    let home = std::env::var("HOME").context("HOME is not set")?;
    Ok(PathBuf::from(home).join(".moor"))
}

pub fn project_dir(name: &str) -> Result<PathBuf> {
    Ok(moor_home()?.join("projects").join(name))
}

pub fn manifest_path(name: &str) -> Result<PathBuf> {
    Ok(project_dir(name)?.join("moor.yaml"))
}

pub fn compose_path(name: &str) -> Result<PathBuf> {
    Ok(project_dir(name)?.join("compose.yml"))
}

pub fn audit_dir(name: &str) -> Result<PathBuf> {
    Ok(project_dir(name)?.join("audit"))
}

/// The single hash-chained audit trail for a project — exec commands,
/// folded-in egress verdicts, and tripwire hits all append here. See
/// audit.rs.
pub fn chain_log_path(name: &str) -> Result<PathBuf> {
    Ok(audit_dir(name)?.join("chain.jsonl"))
}

/// How many raw lines of the egress gateway's access log have already
/// been folded into chain.jsonl — lets `moor audit` be idempotent.
pub fn egress_offset_path(name: &str) -> Result<PathBuf> {
    Ok(audit_dir(name)?.join(".egress-offset"))
}

/// The session id the next `moor ask` turn resumes from. Host-side and
/// host-written only: nothing inside a sandbox has this path mounted, so
/// the agent cannot nominate which session it continues.
pub fn session_path(name: &str) -> Result<PathBuf> {
    Ok(project_dir(name)?.join("session"))
}

/// The full, redacted text of every `moor ask` turn. Separate from
/// chain.jsonl on purpose: the chain carries hashes and is exported by
/// `moor audit --export`, this carries conversation.
pub fn transcript_path(name: &str) -> Result<PathBuf> {
    Ok(project_dir(name)?.join("transcript.jsonl"))
}

pub fn ensure_project_dirs(name: &str) -> Result<()> {
    std::fs::create_dir_all(audit_dir(name)?)?;
    Ok(())
}

/// Where `moor use` records the sticky default project, so commands
/// like `moor keel`/`moor view` don't need `--project` spelled out on
/// every call just because more than one project happens to exist.
pub fn current_project_path() -> Result<PathBuf> {
    Ok(moor_home()?.join("current"))
}

pub fn read_current_project() -> Result<Option<String>> {
    let path = current_project_path()?;
    match std::fs::read_to_string(&path) {
        Ok(contents) => {
            let name = contents.trim();
            Ok(if name.is_empty() {
                None
            } else {
                Some(name.to_string())
            })
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

pub fn write_current_project(name: &str) -> Result<()> {
    let path = current_project_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, name).with_context(|| format!("writing {}", path.display()))
}

pub fn all_project_names() -> Result<Vec<String>> {
    let projects_dir = moor_home()?.join("projects");
    if !projects_dir.exists() {
        return Ok(vec![]);
    }
    let mut names = vec![];
    for entry in std::fs::read_dir(projects_dir)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            if let Some(name) = entry.file_name().to_str() {
                names.push(name.to_string());
            }
        }
    }
    names.sort();
    Ok(names)
}
