use crate::paths;
use anyhow::{Context, Result};

/// Rejects a name that isn't among the known projects, listing what is
/// known so the operator can correct a typo immediately. Split out from
/// `run` so it's testable without touching `~/.moor`.
fn check_known(name: &str, names: &[String]) -> Result<()> {
    if names.iter().any(|n| n == name) {
        return Ok(());
    }
    anyhow::bail!(
        "no such project '{name}' — known projects: {}",
        if names.is_empty() {
            "(none yet)".to_string()
        } else {
            names.join(", ")
        }
    );
}

/// Set the sticky default project that `moor keel`/`moor view` (and any
/// other command taking an optional `--project`) fall back to when it's
/// omitted, so switching your working project doesn't mean typing its
/// name on every subsequent command. `spec` also pins the spec `moor
/// next` guides this project through; the slug is checked for shape only,
/// since whether keel knows it is `moor next`'s to report.
pub fn run(name: &str, spec: Option<&str>) -> Result<()> {
    check_known(name, &paths::all_project_names()?)?;
    if let Some(slug) = spec {
        crate::manifest::validate_name(slug).context("spec name")?;
    }
    paths::write_current_project(name)?;
    println!("default project set to '{name}'");
    if let Some(slug) = spec {
        let path = paths::active_spec_path(name)?;
        std::fs::write(&path, slug).with_context(|| format!("writing {}", path.display()))?;
        println!("active spec set to '{slug}'");
    }
    println!("\n  Next:  moor next");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_a_known_project() {
        let names = vec!["banner-demo".to_string(), "keel".to_string()];
        assert!(check_known("keel", &names).is_ok());
    }

    #[test]
    fn rejects_an_unknown_project_and_lists_the_known_ones() {
        let names = vec!["banner-demo".to_string(), "keel".to_string()];
        let err = check_known("typo-app", &names).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("typo-app"));
        assert!(msg.contains("banner-demo"));
        assert!(msg.contains("keel"));
    }

    #[test]
    fn rejects_with_a_clear_message_when_there_are_no_projects_yet() {
        let err = check_known("anything", &[]).unwrap_err();
        assert!(err.to_string().contains("none yet"));
    }
}
