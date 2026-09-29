use crate::guide;
use crate::{audit, manifest, manifest::Manifest, paths, proc};
use anyhow::{Context, Result};

/// The pinned spec, if one is set and still a well-formed slug. A
/// hand-edited file that isn't one is treated as no pin, not an error.
fn read_pin(name: &str) -> Option<String> {
    let raw = std::fs::read_to_string(paths::active_spec_path(name).ok()?).ok()?;
    let slug = raw.trim();
    manifest::validate_name(slug).ok()?;
    Some(slug.to_string())
}

/// `moor next`: where the project's active spec stands and the one
/// command that moves it on — see `guide`. `--all` lists every spec.
pub fn run(explicit: Option<String>, all: bool) -> Result<()> {
    let (name, _) = super::resolve_project(explicit.clone())?;
    let m = Manifest::load(&paths::manifest_path(&name)?)?;
    if super::running_projects(std::slice::from_ref(&name))?.is_empty() {
        anyhow::bail!(
            "{name} isn't running, so keel can't say where things stand.\n\n  Next:  moor up {name}"
        );
    }

    let argv: Vec<String> = ["keel", "next", "--json"].map(String::from).to_vec();
    let container = m.sandbox_container();
    let mut args: Vec<&str> = vec!["exec", &container];
    args.extend(argv.iter().map(String::as_str));
    let (status, out) = proc::run_capture("docker", &args)?;
    audit::log_exec(&name, &m, "next", &argv, status.code())?;
    proc::require_success(&format!("`keel next` in {container}"), status)?;
    let report: guide::NextReport =
        serde_json::from_str(out.trim()).context("parsing `keel next --json`")?;

    let chain: Vec<String> = std::fs::read_to_string(paths::chain_log_path(&name)?)
        .map(|s| s.lines().map(String::from).collect())
        .unwrap_or_default();
    let pin = read_pin(&name);
    let active = guide::pick_active(&report, pin.as_deref(), &chain);

    // Printed commands carry `--project` only when they'd otherwise resolve
    // to a different project; naming the sticky default is redundant.
    let flag =
        explicit.filter(|p| paths::read_current_project().ok().flatten().as_ref() != Some(p));
    let lines = if all {
        let mut lines = vec![format!("{name} specs:")];
        lines.extend(guide::render_all(&report, active.as_ref()));
        lines
    } else {
        let mut lines = guide::render(&name, flag.as_deref(), &report, active.as_ref());
        if flag.is_some() {
            lines.push(String::new());
            lines.push(format!(
                "  Tip: `moor use {name}` drops --project from these commands."
            ));
        }
        lines
    };
    for line in lines {
        println!("{line}");
    }
    Ok(())
}
