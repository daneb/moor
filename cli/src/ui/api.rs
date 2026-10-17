//! The console's routes. Every `/api` call carries the per-launch token;
//! every action is a fixed `moor` argv built here from validated fields,
//! so nothing the page sends is ever a command line.

use super::http::{Request, Response};
use super::{data, State};
use crate::commands::spec_cmd;
use crate::manifest::validate_name;
use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

const INDEX_HTML: &str = include_str!("../../ui/index.html");
const APP_JS: &str = include_str!("../../ui/app.js");
const APP_CSS: &str = include_str!("../../ui/app.css");

pub fn route(state: &State, req: &Request) -> Response {
    let segs: Vec<&str> = req
        .path
        .trim_matches('/')
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();
    match (req.method.as_str(), segs.as_slice()) {
        ("GET", []) | ("GET", ["index.html"]) => {
            asset("index.html", "text/html; charset=utf-8", INDEX_HTML)
        }
        ("GET", ["app.js"]) => asset("app.js", "text/javascript; charset=utf-8", APP_JS),
        ("GET", ["app.css"]) => asset("app.css", "text/css; charset=utf-8", APP_CSS),
        (_, ["api", rest @ ..]) => {
            if req.header("x-moor-token") != Some(state.token.as_str()) {
                return Response::error(
                    401,
                    "missing or wrong console token — reopen the URL moor ui printed",
                );
            }
            match api(state, req, rest) {
                Ok(v) => Response::ok(v),
                Err(e) => {
                    let msg = format!("{e:#}");
                    let status =
                        if msg.contains("isn't") || msg.contains("unknown") || msg.contains("must")
                        {
                            400
                        } else if msg.starts_with("no ") {
                            404
                        } else {
                            500
                        };
                    Response::error(status, msg)
                }
            }
        }
        _ => Response::error(404, "not found"),
    }
}

/// The embedded page — or, with `MOOR_UI_DEV=<dir>` set, the same file
/// read from that directory on every request, so the page can be worked
/// on without rebuilding the binary. Never set in normal use.
fn asset(name: &str, content_type: &'static str, embedded: &'static str) -> Response {
    if let Ok(dir) = std::env::var("MOOR_UI_DEV") {
        if let Ok(text) = std::fs::read_to_string(Path::new(&dir).join(name)) {
            return Response::dynamic(content_type, text);
        }
    }
    Response::asset(content_type, embedded)
}

fn body(req: &Request) -> Result<Value> {
    if req.body.is_empty() {
        return Ok(json!({}));
    }
    req.json()
}

fn str_field<'a>(v: &'a Value, key: &str) -> Result<&'a str> {
    v[key]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .with_context(|| format!("'{key}' must be given"))
}

fn api(state: &State, req: &Request, segs: &[&str]) -> Result<Value> {
    match (req.method.as_str(), segs) {
        ("GET", ["overview"]) => data::overview(),
        ("GET", ["fs"]) => data::list_dirs(req.query_str("path")),
        ("GET", ["images"]) => Ok(json!({ "images": data::moor_images() })),
        ("GET", ["projects", p]) => data::project(p),
        ("GET", ["projects", p, "settings"]) => data::settings(p),
        ("PUT", ["projects", p, "manifest"]) => data::update_manifest(p, &body(req)?),
        ("POST", ["projects", p, "pin"]) => {
            let b = body(req)?;
            data::pin(p, b["slug"].as_str().filter(|s| !s.is_empty()))?;
            Ok(json!({ "ok": true }))
        }
        ("POST", ["projects", p, "drivers", "default"]) => {
            let b = body(req)?;
            let out = data::set_driver(p, str_field(&b, "id")?)?;
            Ok(json!({ "ok": true, "output": out }))
        }
        ("GET", ["projects", p, "specs", slug]) => data::spec(p, slug),
        ("GET", ["projects", p, "specs", slug, "diff"]) => data::diff(p, slug),
        ("PUT", ["projects", p, "specs", slug, "artifact", kind]) => {
            data::write_artifact(p, slug, kind, &req.text())?;
            Ok(json!({ "ok": true }))
        }
        ("POST", ["specs", "starter"]) => {
            let b = body(req)?;
            let slug = str_field(&b, "slug")?;
            validate_name(slug).context("spec name")?;
            let title = b["title"]
                .as_str()
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .unwrap_or(slug);
            let scope: Vec<String> = b["scope"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(String::from)
                        .collect()
                })
                .unwrap_or_default();
            Ok(json!({ "text": spec_cmd::starter(slug, title, &scope) }))
        }
        ("POST", ["projects", p, "recipes"]) => write_recipe(p, &body(req)?),
        ("GET", ["projects", p, "runs", id]) => data::run(p, id),
        ("GET", ["projects", p, "runs", id, "evidence", file]) => {
            Ok(json!({ "name": file, "text": data::evidence(p, id, file)? }))
        }
        ("GET", ["projects", p, "audit"]) => data::audit(
            p,
            req.query_str("after").and_then(|a| a.parse().ok()),
            req.query_usize("limit").unwrap_or(300).min(5000),
            req.query_str("fold") != Some("0"),
        ),
        ("GET", ["projects", p, "transcript"]) => data::transcript(p),
        ("GET", ["jobs"]) => {
            Ok(json!({ "jobs": state.jobs.list().iter().map(|j| j.summary()).collect::<Vec<_>>() }))
        }
        ("GET", ["jobs", id]) => {
            let job = job(state, id)?;
            Ok(job.snapshot(req.query_usize("after").unwrap_or(0)))
        }
        ("POST", ["jobs", id, "cancel"]) => {
            job(state, id)?.cancel()?;
            Ok(json!({ "ok": true }))
        }
        ("POST", ["jobs"]) => start_job(state, &body(req)?),
        _ => anyhow::bail!("no route for {} /api/{}", req.method, segs.join("/")),
    }
}

fn job(state: &State, id: &str) -> Result<std::sync::Arc<super::jobs::Job>> {
    let id: u64 = id.parse().context("job id")?;
    state.jobs.get(id).with_context(|| format!("no job {id}"))
}

fn project_field(b: &Value) -> Result<String> {
    let p = str_field(b, "project")?;
    validate_name(p).context("project name")?;
    if !crate::paths::manifest_path(p)?.exists() {
        anyhow::bail!("no project named '{p}'");
    }
    Ok(p.to_string())
}

fn existing_dir(v: &Value, key: &str) -> Result<PathBuf> {
    let p = PathBuf::from(str_field(v, key)?);
    if !p.is_absolute() {
        anyhow::bail!("'{key}' must be an absolute path");
    }
    if !p.is_dir() {
        anyhow::bail!("{} isn't a directory", p.display());
    }
    Ok(p)
}

/// Text from the page, with control characters out, for a note or a
/// prompt that goes into an argv.
fn clean(s: &str) -> String {
    crate::studio::render::sanitize(s)
        .replace('\t', "    ")
        .trim()
        .to_string()
}

/// The slug a spec file declares in its front matter.
fn slug_of(text: &str) -> Result<String> {
    let slug = text
        .lines()
        .find_map(|l| l.strip_prefix("slug:"))
        .map(|s| s.trim().trim_matches('"').to_string())
        .context("the spec has no `slug:` line in its front matter")?;
    validate_name(&slug).context("spec name")?;
    Ok(slug)
}

/// Write a recipe on the host from a draft the page composed, or from an
/// agent's answer — through `session::emit_recipe`, so it is parsed
/// before it is kept, like `moor ask --emit-recipe`.
fn write_recipe(p: &str, b: &Value) -> Result<Value> {
    let p = {
        validate_name(p).context("project name")?;
        p.to_string()
    };
    let text = if let Some(text) = b["text"].as_str() {
        text.to_string()
    } else {
        let slug = str_field(b, "slug")?;
        validate_name(slug).context("spec name")?;
        let scope: Vec<String> = b["scope"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(String::from)
                    .collect()
            })
            .unwrap_or_default();
        let scope = if scope.is_empty() {
            vec!["src/**".to_string()]
        } else {
            scope
        };
        let description = str_field(b, "description")?;
        let scope_yaml: String = scope.iter().map(|s| format!("  - \"{s}\"\n")).collect();
        format!("---\nslug: {slug}\nscope:\n{scope_yaml}---\n\n{description}\n")
    };
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let dest = data::drafts_dir(&p)?.join(format!("{stamp}.recipe.md"));
    crate::session::emit_recipe(&dest, &text)?;
    Ok(json!({ "path": dest.display().to_string(), "text": text }))
}

fn draft_in_project(p: &str, path: &str) -> Result<PathBuf> {
    let dir = data::drafts_dir(p)?.canonicalize()?;
    let path = Path::new(path)
        .canonicalize()
        .context("that draft no longer exists")?;
    if !path.starts_with(&dir) {
        anyhow::bail!("recipes run from the project's drafts folder only");
    }
    Ok(path)
}

fn agent_field(b: &Value) -> Result<String> {
    let a = b["agent"].as_str().unwrap_or("claude").trim();
    a.parse::<crate::manifest::Agent>()?;
    Ok(a.to_string())
}

fn image_field(b: &Value) -> Result<Option<String>> {
    match b["image"].as_str().map(str::trim).filter(|s| !s.is_empty()) {
        Some(i) if data::valid_image(i) => Ok(Some(i.to_string())),
        Some(i) => anyhow::bail!("'{i}' isn't an image name"),
        None => Ok(None),
    }
}

/// One job kind → one `moor` argv. The table is the whole surface: a
/// kind not here is not something the console can run.
fn start_job(state: &State, b: &Value) -> Result<Value> {
    let kind = str_field(b, "kind")?;
    let mut stdin: Option<String> = None;
    let (label, project, argv): (String, Option<String>, Vec<String>) = match kind {
        "up" | "down" | "selftest" => {
            let p = project_field(b)?;
            (
                format!("moor {kind} {p}"),
                Some(p.clone()),
                vec![kind.into(), p],
            )
        }
        "go" => {
            let p = project_field(b)?;
            let check = b["check"].as_bool().unwrap_or(false);
            let mut argv = vec!["go".into(), "-p".into(), p.clone()];
            if check {
                argv.push("--check".into());
            }
            (
                format!("moor go{}", if check { " --check" } else { "" }),
                Some(p),
                argv,
            )
        }
        "approve" => {
            let p = project_field(b)?;
            (
                "moor approve".into(),
                Some(p.clone()),
                vec!["approve".into(), "-p".into(), p, "--yes".into()],
            )
        }
        "reject" => {
            let p = project_field(b)?;
            let why = clean(str_field(b, "why")?);
            if why.is_empty() {
                anyhow::bail!("say why, so the next attempt can address it");
            }
            (
                "moor reject".into(),
                Some(p.clone()),
                vec!["reject".into(), "-p".into(), p, why],
            )
        }
        "ship" => {
            let p = project_field(b)?;
            let mut argv = vec!["ship".into(), "-p".into(), p.clone()];
            if let Some(slug) = b["spec"].as_str().filter(|s| !s.is_empty()) {
                validate_name(slug).context("spec name")?;
                argv.push(slug.into());
            }
            argv.push("--yes".into());
            ("moor ship".into(), Some(p), argv)
        }
        "ask" => {
            let p = project_field(b)?;
            let role = b["role"].as_str().unwrap_or("brainstorm");
            crate::session::Role::parse(role)?;
            let prompt = clean(str_field(b, "prompt")?);
            let mut argv = vec![
                "ask".into(),
                "-p".into(),
                p.clone(),
                "--role".into(),
                role.into(),
            ];
            if b["new"].as_bool().unwrap_or(false) {
                argv.push("--new".into());
            }
            argv.push(prompt);
            (format!("moor ask --role {role}"), Some(p), argv)
        }
        "spec_push" => {
            let p = project_field(b)?;
            let text = str_field(b, "text")?.to_string();
            let slug = slug_of(&text)?;
            let path = data::drafts_dir(&p)?.join(format!("{slug}.spec.md"));
            std::fs::write(&path, &text)?;
            (
                format!("moor spec push {slug}"),
                Some(p.clone()),
                vec![
                    "spec".into(),
                    "push".into(),
                    "-p".into(),
                    p,
                    path.display().to_string(),
                ],
            )
        }
        "recipe" => {
            let p = project_field(b)?;
            let path = draft_in_project(&p, str_field(b, "path")?)?;
            (
                "moor recipe".into(),
                Some(p.clone()),
                vec!["recipe".into(), p, path.display().to_string()],
            )
        }
        "gate" => {
            let p = project_field(b)?;
            let slug = str_field(b, "slug")?;
            validate_name(slug).context("spec name")?;
            let gate = match b["gate"].as_str() {
                Some("g0") => "g0",
                Some("g1") => "g1",
                _ => anyhow::bail!("gate must be g0 or g1"),
            };
            (
                format!("moor keel gate {gate} {slug}"),
                Some(p.clone()),
                vec![
                    "keel".into(),
                    "-p".into(),
                    p,
                    "gate".into(),
                    gate.into(),
                    slug.into(),
                ],
            )
        }
        "new" => {
            let name = str_field(b, "name")?;
            validate_name(name).context("project name")?;
            let image = image_field(b)?.unwrap_or_else(|| "moor/base:latest".into());
            let agent = agent_field(b)?;
            let mut argv = vec![
                "new".into(),
                name.into(),
                "--image".into(),
                image,
                "--agent".into(),
                agent,
            ];
            if b["github"].as_bool().unwrap_or(false) {
                argv.push("--github".into());
                stdin = Some("y\n".into());
            }
            (format!("moor new {name}"), None, argv)
        }
        "import" => {
            let name = str_field(b, "name")?;
            validate_name(name).context("project name")?;
            let from = existing_dir(b, "from")?;
            let agent = agent_field(b)?;
            let mut argv = vec![
                "import".into(),
                name.into(),
                "--from".into(),
                from.display().to_string(),
                "--agent".into(),
                agent,
            ];
            if let Some(image) = image_field(b)? {
                argv.push("--image".into());
                argv.push(image);
            }
            if b["github"].as_bool().unwrap_or(false) {
                argv.push("--github".into());
                stdin = Some("y\n".into());
            }
            (format!("moor import {name}"), None, argv)
        }
        "audit_verify" => {
            let p = project_field(b)?;
            (
                "moor audit --verify".into(),
                Some(p.clone()),
                vec!["audit".into(), p, "--verify".into()],
            )
        }
        "audit_export" => {
            let p = project_field(b)?;
            let out = existing_dir(b, "out")?;
            (
                "moor audit --export".into(),
                Some(p.clone()),
                vec![
                    "audit".into(),
                    p,
                    "--export".into(),
                    out.display().to_string(),
                ],
            )
        }
        "bundle" => {
            let p = project_field(b)?;
            let out = existing_dir(b, "out")?;
            let mut argv = vec!["bundle".into(), "-p".into(), p.clone()];
            if let Some(run) = b["run"].as_str().filter(|s| !s.is_empty()) {
                if !run.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-') {
                    anyhow::bail!("'{run}' isn't a run id");
                }
                argv.push(run.into());
            }
            argv.push("--out".into());
            argv.push(out.display().to_string());
            ("moor bundle".into(), Some(p), argv)
        }
        "pull" | "doctor" => {
            let p = project_field(b)?;
            (
                format!("moor {kind}"),
                Some(p.clone()),
                vec![kind.into(), "-p".into(), p],
            )
        }
        "driver_check" => {
            let p = project_field(b)?;
            let id = str_field(b, "id")?;
            if !id
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
            {
                anyhow::bail!("'{id}' isn't a driver id");
            }
            (
                format!("keel driver check {id}"),
                Some(p.clone()),
                vec![
                    "keel".into(),
                    "-p".into(),
                    p,
                    "driver".into(),
                    "check".into(),
                    id.into(),
                ],
            )
        }
        "secret_set" => {
            let p = project_field(b)?;
            let name = str_field(b, "name")?;
            if !data::valid_secret_name(name) {
                anyhow::bail!("'{name}' isn't an environment variable name");
            }
            let value = b["value"].as_str().unwrap_or("").trim();
            if value.is_empty() || value.contains('\n') {
                anyhow::bail!("the value must be one non-empty line");
            }
            // The value travels on stdin and is never in argv, so it is
            // never in `ps`, the job list, or the audit chain.
            stdin = Some(format!("{value}\n"));
            (
                format!("moor secrets set {name}"),
                Some(p.clone()),
                vec!["secrets".into(), "set".into(), p, name.into()],
            )
        }
        "secret_unset" => {
            let p = project_field(b)?;
            let name = str_field(b, "name")?;
            if !data::valid_secret_name(name) {
                anyhow::bail!("'{name}' isn't an environment variable name");
            }
            (
                format!("moor secrets unset {name}"),
                Some(p.clone()),
                vec!["secrets".into(), "unset".into(), p, name.into()],
            )
        }
        other => anyhow::bail!("unknown job kind '{other}'"),
    };

    if let Some(p) = &project {
        if let Some(running) = state.jobs.busy(p) {
            if is_exclusive(kind) {
                anyhow::bail!(
                    "{p} is busy: {} is still running (job {})",
                    running.label,
                    running.id
                );
            }
        }
    }
    let job = state
        .jobs
        .spawn(kind, &label, project.as_deref(), argv, stdin)?;
    Ok(job.snapshot(0))
}

fn is_exclusive(kind: &str) -> bool {
    matches!(
        kind,
        "go" | "approve" | "reject" | "ship" | "spec_push" | "recipe" | "up" | "down" | "gate"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_comes_from_front_matter_and_is_checked() {
        assert_eq!(
            slug_of("---\nslug: login-endpoint\nstatus: draft\n---\n").unwrap(),
            "login-endpoint"
        );
        assert!(slug_of("---\nstatus: draft\n---\n").is_err());
        assert!(slug_of("---\nslug: ../etc\n---\n").is_err());
    }

    #[test]
    fn clean_strips_control_characters_but_keeps_newlines() {
        assert_eq!(clean("  a\x1b[31mb\r\nc  "), "ab\nc");
    }

    #[test]
    fn unknown_job_kinds_are_refused() {
        let state = State::for_tests();
        let err = start_job(&state, &json!({"kind": "rm-rf"})).unwrap_err();
        assert!(err.to_string().contains("unknown job kind"));
    }

    #[test]
    fn a_reject_needs_a_reason() {
        let state = State::for_tests();
        let err = start_job(
            &state,
            &json!({"kind": "reject", "project": "no-such-project-xyz"}),
        )
        .unwrap_err();
        assert!(err.to_string().contains("no project") || err.to_string().contains("why"));
    }
}
