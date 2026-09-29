use crate::guide;
use crate::{audit, manifest, manifest::Manifest, paths, proc};
use anyhow::{Context, Result};

/// The project a guided command acts on, resolved once and shared by
/// `moor next` and the commands that move a spec on (`flow_cmd`).
pub struct Target {
    pub name: String,
    pub m: Manifest,
    /// `--project` for printed commands: only when they'd otherwise resolve
    /// to a different project, since naming the sticky default is redundant.
    pub flag: Option<String>,
}

impl Target {
    /// Resolves the project and checks its sandbox is up — every guided
    /// command needs to ask the sandbox where things stand.
    pub fn resolve(explicit: Option<String>) -> Result<Self> {
        let (name, _) = super::resolve_project(explicit.clone())?;
        let m = Manifest::load(&paths::manifest_path(&name)?)?;
        if super::running_projects(std::slice::from_ref(&name))?.is_empty() {
            anyhow::bail!(
                "{name} isn't running, so moor can't say where things stand.\n\n  Next:  moor up {name}"
            );
        }
        let flag =
            explicit.filter(|p| paths::read_current_project().ok().flatten().as_ref() != Some(p));
        Ok(Target { name, m, flag })
    }

    /// Where every spec stands, from the sandbox. Logged like any exec.
    pub fn report(&self) -> Result<guide::NextReport> {
        let argv: Vec<String> = ["keel", "next", "--json"].map(String::from).to_vec();
        let container = self.m.sandbox_container();
        let mut args: Vec<&str> = vec!["exec", &container];
        args.extend(argv.iter().map(String::as_str));
        let (status, out) = proc::run_capture("docker", &args)?;
        audit::log_exec(&self.name, &self.m, "next", &argv, status.code())?;
        proc::require_success(&format!("asking {container} where things stand"), status)?;
        serde_json::from_str(out.trim()).context("parsing the sandbox's pipeline status")
    }

    /// Runs `argv` in the sandbox, capturing its combined output, logged
    /// under `kind` like any exec. Returns whether it succeeded.
    pub fn exec(&self, kind: &str, argv: &[&str]) -> Result<(bool, String)> {
        let container = self.m.sandbox_container();
        let mut args = vec!["exec", container.as_str()];
        args.extend_from_slice(argv);
        let (status, out) = proc::run_capture_combined("docker", &args)?;
        let logged: Vec<String> = argv.iter().map(|a| a.to_string()).collect();
        audit::log_exec(&self.name, &self.m, kind, &logged, status.code())?;
        Ok((status.success(), out))
    }

    /// Specs whose folder has uncommitted changes: approved work that
    /// hasn't been shipped yet, when the spec is also complete.
    pub fn unshipped(&self, report: &guide::NextReport) -> Result<Vec<String>> {
        let (ok, out) = self.exec(
            "next",
            &[
                "git",
                "status",
                "--porcelain",
                "--untracked-files=all",
                "--",
                ".keel/specs",
            ],
        )?;
        if !ok {
            return Ok(vec![]);
        }
        Ok(report
            .specs
            .iter()
            .filter(|s| s.complete)
            .filter(|s| {
                let dir = format!(".keel/specs/{}/", s.slug);
                out.lines()
                    .any(|l| l.get(3..).is_some_and(|p| p.starts_with(&dir)))
            })
            .map(|s| s.slug.clone())
            .collect())
    }

    /// The spec guidance is about — see `guide::pick_active`.
    pub fn active<'a>(&self, report: &'a guide::NextReport) -> Option<guide::Active<'a>> {
        let chain: Vec<String> = paths::chain_log_path(&self.name)
            .and_then(|p| Ok(std::fs::read_to_string(p)?))
            .map(|s| s.lines().map(String::from).collect())
            .unwrap_or_default();
        guide::pick_active(report, read_pin(&self.name).as_deref(), &chain)
    }

    /// The guidance block, ending on its "Next:" line (plus a tip when
    /// the printed commands carry `--project`).
    pub fn print_guidance(&self, report: &guide::NextReport) {
        // Approved work waiting to ship comes first: starting the next spec
        // on top of it is how two specs' changes end up tangled.
        let unshipped = self.unshipped(report).unwrap_or_default();
        let active = self.active(report);
        let mut lines = match unshipped.first() {
            Some(slug) => {
                guide::render_ship(&self.name, self.flag.as_deref(), slug, unshipped.len())
            }
            None => guide::render(&self.name, self.flag.as_deref(), report, active.as_ref()),
        };
        if self.flag.is_some() {
            lines.push(String::new());
            lines.push(format!(
                "  Tip: `moor use {}` drops --project from these commands.",
                self.name
            ));
        }
        for line in lines {
            println!("{line}");
        }
    }
}

/// The pinned spec, if one is set and still a well-formed slug. A
/// hand-edited file that isn't one is treated as no pin, not an error.
fn read_pin(name: &str) -> Option<String> {
    let raw = std::fs::read_to_string(paths::active_spec_path(name).ok()?).ok()?;
    let slug = raw.trim();
    manifest::validate_name(slug).ok()?;
    Some(slug.to_string())
}

/// The closing line for a command that isn't itself guided: how to ask
/// where `name` stands, with `--project` only if it isn't the default.
pub fn next_hint(name: &str) -> String {
    let sticky = paths::read_current_project().ok().flatten();
    if sticky.as_deref() == Some(name) {
        "  Next:  moor next".to_string()
    } else {
        format!("  Next:  moor next --project {name}")
    }
}

/// `moor next`: where the project's active spec stands and the one
/// command that moves it on — see `guide`. `--all` lists every spec.
pub fn run(explicit: Option<String>, all: bool) -> Result<()> {
    let t = Target::resolve(explicit)?;
    let report = t.report()?;
    if all {
        let active = t.active(&report);
        println!("{} specs:", t.name);
        for line in guide::render_all(&report, active.as_ref()) {
            println!("{line}");
        }
    } else {
        t.print_guidance(&report);
    }
    Ok(())
}
