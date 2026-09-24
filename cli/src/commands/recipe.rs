use crate::{audit, manifest::Manifest, paths, proc};
use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::json;
use std::path::Path;

const DEFAULT_MAX_GATE_RETRIES: u32 = 3;
const DEFAULT_MAX_RUN_ATTEMPTS: u32 = 2;

/// A recipe is a loosely-described outcome plus the handful of things
/// keel actually needs structured: a slug (becomes the spec slug) and the
/// scope globs the change may touch. Everything below the front matter is
/// free text, handed to the agent as-is — deliberately not a DSL, so the
/// operator never has to learn one to describe what they want.
#[derive(Debug, Deserialize)]
pub struct FrontMatter {
    pub slug: String,
    #[serde(default)]
    scope: Vec<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default = "default_max_gate_retries")]
    max_gate_retries: u32,
    #[serde(default = "default_max_run_attempts")]
    max_run_attempts: u32,
}

fn default_max_gate_retries() -> u32 {
    DEFAULT_MAX_GATE_RETRIES
}

fn default_max_run_attempts() -> u32 {
    DEFAULT_MAX_RUN_ATTEMPTS
}

#[derive(Debug)]
pub struct Recipe {
    pub front: FrontMatter,
    pub description: String,
}

/// Also the gate `session::emit_recipe` puts an agent-drafted recipe
/// through before the host writes it — one parser, so an emitted recipe
/// cannot be one `moor recipe` then refuses.
pub fn parse_recipe(path: &Path) -> Result<Recipe> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading recipe {}", path.display()))?;
    let rest = text
        .trim_start_matches('\u{feff}')
        .strip_prefix("---\n")
        .ok_or_else(|| {
            anyhow::anyhow!("recipe must start with a `---` front-matter block (slug, scope)")
        })?;
    let end = rest
        .find("\n---")
        .ok_or_else(|| anyhow::anyhow!("recipe's front matter has no closing `---`"))?;
    let yaml = &rest[..end];
    let description = rest[end + 4..].trim_start_matches('\n').trim().to_string();
    if description.is_empty() {
        anyhow::bail!("recipe has no description below the front matter — that's the part the agent actually reads");
    }
    let front: FrontMatter = serde_yaml::from_str(yaml)
        .context("parsing recipe front matter (need at least `slug:`)")?;
    Ok(Recipe { front, description })
}

/// Matches the shape of `keel next --json`'s "schema": "keel.next/1" — see
/// docs/decisions/0005-recipe.md for how this was derived (traced live
/// against a real spec, not guessed from keel's own source).
#[derive(Debug, Deserialize)]
struct NextSpec {
    slug: String,
    stage: String,
    command: String,
    complete: bool,
}

#[derive(Debug, Deserialize, Default)]
struct NextReport {
    #[serde(default)]
    specs: Vec<NextSpec>,
}

/// Append a short, structured "what's happening" entry to the project's
/// existing audit chain — reusing that mechanism rather than inventing a
/// second logging path, so `moor logs` has one durable, tamper-evident
/// place to read from regardless of which terminal (or whether any
/// terminal at all) is still attached to the running recipe. Deliberately
/// small fields only (slug, stage, attempt counters) — never captured
/// command output, which can be large and is already in the "recipe"
/// exec entries this same chain holds.
fn log_event(name: &str, event: &str, mut data: serde_json::Value) -> Result<()> {
    if let Some(obj) = data.as_object_mut() {
        obj.insert("event".to_string(), json!(event));
    }
    let path = paths::chain_log_path(name)?;
    audit::append_chained(&path, "recipe-event", data)?;
    Ok(())
}

fn exec_capture(name: &str, m: &Manifest, argv: &[String]) -> Result<(bool, String)> {
    let container = m.sandbox_container();
    let mut args: Vec<&str> = vec!["exec", &container];
    args.extend(argv.iter().map(|s| s.as_str()));
    let (status, out) = proc::run_capture_combined("docker", &args)?;
    audit::log_exec(name, m, "recipe", argv, status.code())?;
    Ok((status.success(), out))
}

fn next_report(name: &str, m: &Manifest, slug: &str) -> Result<NextReport> {
    let (_, out) = exec_capture(
        name,
        m,
        &["keel".into(), "next".into(), "--json".into(), slug.into()],
    )?;
    serde_json::from_str(&out)
        .with_context(|| format!("parsing `keel next --json {slug}` output:\n{out}"))
}

/// Ask an agent to author `.keel/specs/<slug>/spec.md` from the recipe's
/// free-text description. Granted the Write tool only, because a concrete
/// description of a desired outcome reliably makes Claude Code just go
/// implement it instead of writing a spec about it (verified directly: it
/// built the feature and left the spec untouched when given Bash/Edit
/// access during this feature's own development).
///
/// `--allowedTools` alone does *not* make that a boundary: measured against
/// a live sandbox, Claude Code runs `Bash` regardless of being granted only
/// other tools, under every permission mode. `session::deny_shell_argv`
/// supplies the `--disallowedTools` half that actually denies it.
fn author_spec(name: &str, m: &Manifest, slug: &str, description: &str) -> Result<()> {
    let (_, keel_prompt) = exec_capture(
        name,
        m,
        &["keel".into(), "spec".into(), "prompt".into(), slug.into()],
    )?;
    let full = format!(
        "What the operator wants (context only — this step authors the spec, it does not implement anything):\n{description}\n\n{keel_prompt}"
    );
    // `--allowedTools <tools...>` is variadic — it greedily swallows every
    // bare argument after it until the next flag, so the prompt has to
    // come *before* it or claude sees no positional prompt at all
    // (verified directly: with the flag first, claude errors "Input must
    // be provided either through stdin or as a prompt argument").
    let mut argv = vec![
        "claude".into(),
        "--print".into(),
        full,
        "--allowedTools".into(),
        "Write".into(),
    ];
    argv.extend(crate::session::deny_shell_argv());
    let (ok, out) = exec_capture(name, m, &argv)?;
    println!("{out}");
    if !ok {
        anyhow::bail!("spec-authoring agent call failed");
    }
    Ok(())
}

/// Run a `keel gate` command; on failure, feed its own output (specific
/// and actionable — "no files on: T-1", "rollback... both are empty") to
/// an agent restricted to the Edit tool, then retry. Bounded, and never
/// used for the build step itself (`keel run`) — only for fixing the
/// spec/plan scaffolding the gate is actually checking.
fn drive_gate(
    name: &str,
    m: &Manifest,
    gate_argv: &[String],
    slug: &str,
    max_retries: u32,
) -> Result<bool> {
    for attempt in 0..=max_retries {
        let (ok, out) = exec_capture(name, m, gate_argv)?;
        println!("{out}");
        log_event(
            name,
            "gate-attempt",
            json!({"slug": slug, "gate": gate_argv, "attempt": attempt + 1, "max": max_retries + 1, "result": if ok { "pass" } else { "fail" }}),
        )?;
        if ok {
            return Ok(true);
        }
        if attempt == max_retries {
            return Ok(false);
        }
        println!(
            "==> gate failed (attempt {}/{}) — asking the agent to fix exactly that",
            attempt + 1,
            max_retries + 1
        );
        let fix_prompt = format!(
            "The following keel gate failed for spec '{slug}'. Using only your Edit tool, fix exactly what it names below — do not touch any other file, do not implement the feature, do not run any command.\n\n{out}"
        );
        let mut claude_argv = vec![
            "claude".into(),
            "--print".into(),
            fix_prompt,
            "--allowedTools".into(),
            "Edit".into(),
        ];
        claude_argv.extend(crate::session::deny_shell_argv());
        let (fixed_ok, fix_out) = exec_capture(name, m, &claude_argv)?;
        println!("{fix_out}");
        if !fixed_ok {
            return Ok(false);
        }
    }
    Ok(false)
}

pub fn run(name: &str, recipe_path: &Path) -> Result<()> {
    let m = Manifest::load(&paths::manifest_path(name)?)?;
    crate::secrets::resolve_into_env(name, &m.secrets);
    let recipe = parse_recipe(recipe_path)?;
    let slug = recipe.front.slug.clone();
    let title = recipe
        .front
        .title
        .clone()
        .unwrap_or_else(|| slug.replace('-', " "));

    println!("==> recipe '{slug}' for project '{name}'");
    log_event(name, "recipe-start", json!({"slug": slug}))?;

    let mut new_argv = vec![
        "keel".into(),
        "spec".into(),
        "new".into(),
        slug.clone(),
        "--title".into(),
        title,
    ];
    for g in &recipe.front.scope {
        new_argv.push("--scope".into());
        new_argv.push(g.clone());
    }
    // `keel spec new` always runs G0 immediately afterward, and a fresh
    // scaffold's placeholder text always fails it — that's the point, not
    // an error, so a non-zero exit here is not itself "failed." What
    // actually distinguishes the three real outcomes is the text: already
    // scaffolded (idempotent — keel refuses to clobber), freshly created
    // (proceed to author it, regardless of G0's expected placeholder
    // failure), or something genuinely wrong.
    let (_, out) = exec_capture(name, &m, &new_argv)?;
    let already_existed = out.contains("already exists");
    let freshly_created = out.contains("created .keel/specs/");
    if !already_existed && !freshly_created {
        println!("{out}");
        anyhow::bail!("`keel spec new` failed — see output above");
    }
    if freshly_created {
        println!("==> scaffolded .keel/specs/{slug}/ — authoring the spec now");
        log_event(name, "spec-scaffolded", json!({"slug": slug}))?;
        author_spec(name, &m, &slug, &recipe.description)?;
        log_event(name, "spec-authored", json!({"slug": slug}))?;
    } else {
        log_event(name, "spec-resumed", json!({"slug": slug}))?;
    }

    let mut run_attempts = 0u32;
    loop {
        let report = next_report(name, &m, &slug)?;
        let Some(spec) = report.specs.iter().find(|s| s.slug == slug) else {
            println!("keel no longer lists '{slug}' — nothing left to drive.");
            return Ok(());
        };
        if spec.complete {
            println!("==> '{slug}' is complete.");
            log_event(name, "complete", json!({"slug": slug}))?;
            return Ok(());
        }

        println!("==> [{slug}] stage: {}  next: {}", spec.stage, spec.command);
        log_event(
            name,
            "stage",
            json!({"slug": slug, "stage": spec.stage, "command": spec.command}),
        )?;

        if spec.stage.contains("approval") {
            println!(
                "\nPAUSED for human approval. Review the change, then run:\n\n    moor run {name} -- {}\n\n...and re-run this recipe to continue.",
                spec.command
            );
            log_event(
                name,
                "paused-for-approval",
                json!({"slug": slug, "stage": spec.stage}),
            )?;
            return Ok(());
        }

        let argv: Vec<String> = spec.command.split_whitespace().map(String::from).collect();
        let head = (
            argv.first().map(String::as_str),
            argv.get(1).map(String::as_str),
        );

        match head {
            (Some("keel"), Some("gate")) => {
                if !drive_gate(name, &m, &argv, &slug, recipe.front.max_gate_retries)? {
                    log_event(
                        name,
                        "failed",
                        json!({"slug": slug, "reason": "gate kept failing"}),
                    )?;
                    anyhow::bail!(
                        "gate kept failing after {} attempt(s) — see .keel/specs/{slug}/gates/ for evidence; fix by hand, then re-run this recipe",
                        recipe.front.max_gate_retries + 1
                    );
                }
            }
            (Some("keel"), Some("run")) => {
                run_attempts += 1;
                log_event(
                    name,
                    "run-attempt",
                    json!({"slug": slug, "attempt": run_attempts, "max": recipe.front.max_run_attempts}),
                )?;
                if run_attempts > recipe.front.max_run_attempts {
                    log_event(
                        name,
                        "failed",
                        json!({"slug": slug, "reason": "keel run attempt cap reached"}),
                    )?;
                    anyhow::bail!(
                        "`keel run` still hasn't reached a human checkpoint after {} attempt(s) — see .keel/runs/ for evidence; drive it by hand, then re-run this recipe",
                        recipe.front.max_run_attempts
                    );
                }
                // Deliberately not gated on its own exit code: a gate
                // blocked on human-verdict (G3) exits non-zero too, and
                // is a legitimate pause, not a failure — the next
                // `keel next` call is the actual source of truth.
                let (_, out) = exec_capture(name, &m, &argv)?;
                println!("{out}");
            }
            _ => {
                // `keel plan ...`, or any future keel stage this recipe
                // runner doesn't special-case — mechanical, no agent
                // needed, just run it and let the human find out if it
                // genuinely fails.
                let (ok, out) = exec_capture(name, &m, &argv)?;
                println!("{out}");
                if !ok {
                    log_event(
                        name,
                        "failed",
                        json!({"slug": slug, "reason": format!("`{}` failed", spec.command)}),
                    )?;
                    anyhow::bail!("`{}` failed — see output above", spec.command);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_temp(label: &str, content: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("moor-recipe-test-{label}-{nanos}.md"));
        std::fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn parses_front_matter_and_description() {
        let path = write_temp(
            "ok",
            "---\nslug: greet-function\nscope:\n  - \"src/**\"\n---\n\nI want a greet function.\n",
        );
        let recipe = parse_recipe(&path).unwrap();
        assert_eq!(recipe.front.slug, "greet-function");
        assert_eq!(recipe.front.scope, vec!["src/**".to_string()]);
        assert_eq!(recipe.front.max_gate_retries, DEFAULT_MAX_GATE_RETRIES);
        assert_eq!(recipe.front.max_run_attempts, DEFAULT_MAX_RUN_ATTEMPTS);
        assert_eq!(recipe.description, "I want a greet function.");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn honors_explicit_overrides() {
        let path = write_temp(
            "overrides",
            "---\nslug: x\ntitle: Custom Title\nmax_gate_retries: 5\nmax_run_attempts: 1\n---\nDo the thing.\n",
        );
        let recipe = parse_recipe(&path).unwrap();
        assert_eq!(recipe.front.title.as_deref(), Some("Custom Title"));
        assert_eq!(recipe.front.max_gate_retries, 5);
        assert_eq!(recipe.front.max_run_attempts, 1);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn rejects_missing_opening_front_matter() {
        let path = write_temp("no-open", "slug: x\n---\nsomething\n");
        assert!(parse_recipe(&path).is_err());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn rejects_missing_closing_front_matter() {
        let path = write_temp(
            "no-close",
            "---\nslug: x\ndescription with no closing marker\n",
        );
        assert!(parse_recipe(&path).is_err());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn rejects_empty_description() {
        let path = write_temp("empty-body", "---\nslug: x\n---\n\n   \n");
        let err = parse_recipe(&path).unwrap_err();
        assert!(err.to_string().contains("no description"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn rejects_missing_slug() {
        let path = write_temp("no-slug", "---\nscope: []\n---\nDo something.\n");
        assert!(parse_recipe(&path).is_err());
        let _ = std::fs::remove_file(&path);
    }
}
