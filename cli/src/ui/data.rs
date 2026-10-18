//! What the console reads, and the few things it writes directly. Every
//! sandbox read goes through `docker exec` and is logged through
//! `audit::log_exec` under kind `ui`, exactly as `moor studio` logs its
//! own reads under `studio`: the console opens no record of its own and
//! skips none. Host files (manifests, the audit chain, transcripts) are
//! read in place.
//!
//! Everything that came out of a sandbox is passed through
//! `studio::render::sanitize` before it is returned — the page renders it
//! as text, but a control sequence has no business leaving this module.

use crate::commands::doctor_cmd;
use crate::manifest::{self, Agent, Manifest};
use crate::studio::render::sanitize;
use crate::{audit, commands, guide, paths, proc, secrets};
use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub fn load(name: &str) -> Result<Manifest> {
    manifest::validate_name(name).context("project name")?;
    let path = paths::manifest_path(name)?;
    if !path.exists() {
        anyhow::bail!("no project named '{name}'");
    }
    Manifest::load(&path)
}

pub fn is_up(name: &str) -> Result<bool> {
    Ok(!commands::running_projects(std::slice::from_ref(&name.to_string()))?.is_empty())
}

/// Run `argv` in the project's sandbox, logged like every other exec.
/// `combined` folds stderr into the output; a JSON read wants stdout only.
fn exec(name: &str, m: &Manifest, argv: &[&str], combined: bool) -> Result<(bool, String)> {
    let container = m.sandbox_container();
    let mut args = vec!["exec", container.as_str()];
    args.extend_from_slice(argv);
    let (status, out) = if combined {
        proc::run_capture_combined("docker", &args)?
    } else {
        proc::run_capture("docker", &args)?
    };
    let logged: Vec<String> = argv.iter().map(|a| a.to_string()).collect();
    audit::log_exec(name, m, "ui", &logged, status.code())?;
    Ok((status.success(), sanitize(&out)))
}

fn exec_json(name: &str, m: &Manifest, argv: &[&str]) -> Result<Value> {
    let (ok, out) = exec(name, m, argv, false)?;
    if !ok {
        anyhow::bail!("`{}` failed in the sandbox", argv.join(" "));
    }
    serde_json::from_str(out.trim()).with_context(|| format!("parsing `{}`", argv.join(" ")))
}

fn sh(name: &str, m: &Manifest, script: &str) -> Result<(bool, String)> {
    exec(name, m, &["sh", "-c", script], true)
}

/// Write `text` to `dest` inside the sandbox through the child's stdin —
/// the same `docker exec -i ... cat > dest` path `moor spec push` and the
/// studio use. No bind mount, no `docker cp`.
fn write_into(name: &str, m: &Manifest, dest: &str, text: &str) -> Result<()> {
    let container = m.sandbox_container();
    let script = format!("cat > {dest}");
    let argv = [
        "exec",
        "-i",
        container.as_str(),
        "sh",
        "-c",
        script.as_str(),
    ];
    let mut child = Command::new("docker")
        .args(argv)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(text.as_bytes())?;
    }
    let out = child.wait_with_output()?;
    let logged: Vec<String> = argv.iter().map(|a| a.to_string()).collect();
    audit::log_exec(name, m, "ui", &logged, out.status.code())?;
    if !out.status.success() {
        anyhow::bail!(
            "writing {dest} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    Ok(())
}

fn agent_name(a: Agent) -> &'static str {
    match a {
        Agent::Claude => "claude",
        Agent::Copilot => "copilot",
        Agent::Kiro => "kiro",
    }
}

fn source_name(s: &secrets::Source) -> &'static str {
    match s {
        secrets::Source::Env => "env",
        secrets::Source::Keychain => "keychain",
        secrets::Source::Missing => "missing",
    }
}

fn manifest_json(m: &Manifest) -> Value {
    json!({
        "name": m.name,
        "image": m.image,
        "agent": agent_name(m.agent()),
        "github_repo": m.github_repo,
        "egress": m.egress.allow,
        "resources": { "cpu": m.resources.cpu, "mem": m.resources.mem, "pids": m.resources.pids },
        "secrets": m.secrets,
        "sandbox_container": m.sandbox_container(),
    })
}

fn approval_json(a: &guide::Approval) -> Value {
    json!({
        "stage": a.stage,
        "standing": a.standing,
        "by": a.by.as_deref().map(sanitize),
        "note": a.note.as_deref().map(sanitize),
        "recheck": a.recheck,
    })
}

/// One spec as keel reports it, with what moor's guidance adds: the step
/// number and title, what to read, what it decides, whether `moor go`
/// can take it.
pub fn spec_json(s: &guide::NextSpec) -> Value {
    let step = guide::step(&s.stage).map(|st| {
        json!({
            "n": st.n,
            "of": guide::TOTAL_STEPS,
            "title": st.title,
            "what": st.what,
            "read": st.read,
            "decides": st.decides,
        })
    });
    json!({
        "slug": s.slug,
        "stage": s.stage,
        "command": s.command,
        "complete": s.complete,
        "step": step,
        "label": guide::step_label(s),
        "automatic": guide::is_automatic(s),
        "pending_decision": guide::pending_decision(s),
        "read": guide::artifact_to_read(s),
        "approval": s.approval.as_ref().map(approval_json),
    })
}

fn chain_lines(name: &str) -> Vec<String> {
    paths::chain_log_path(name)
        .and_then(|p| Ok(std::fs::read_to_string(p)?))
        .map(|s| s.lines().map(String::from).collect())
        .unwrap_or_default()
}

fn active_json(name: &str, report: &guide::NextReport) -> Value {
    let pin = commands::next_cmd::read_pin(name);
    let chain = chain_lines(name);
    match guide::pick_active(report, pin.as_deref(), &chain) {
        Some(a) => json!({
            "slug": a.spec.slug,
            "why": match a.why {
                guide::Why::Pinned => "pinned",
                guide::Why::OnlyOne => "only_one",
                guide::Why::MostRecent => "most_recent",
                guide::Why::FurthestAlong => "furthest_along",
            },
            "stale_pin": a.stale_pin,
            "pin": pin,
        }),
        None => json!({ "slug": null, "why": null, "stale_pin": null, "pin": pin }),
    }
}

fn report(name: &str, m: &Manifest) -> Result<guide::NextReport> {
    let v = exec_json(name, m, &["keel", "next", "--json"])?;
    serde_json::from_value(v).context("parsing keel next")
}

fn report_json(report: &guide::NextReport) -> Value {
    json!({
        "blockers": report.blockers.iter().map(|b| json!({"reason": sanitize(&b.reason), "command": b.command})).collect::<Vec<_>>(),
        "specs": report.specs.iter().map(spec_json).collect::<Vec<_>>(),
    })
}

/// Specs whose folder has uncommitted work: approved, not yet shipped.
fn unshipped(name: &str, m: &Manifest, report: &guide::NextReport) -> Vec<String> {
    let Ok((ok, out)) = exec(
        name,
        m,
        &[
            "git",
            "status",
            "--porcelain",
            "--untracked-files=all",
            "--",
            ".keel/specs",
        ],
        false,
    ) else {
        return vec![];
    };
    if !ok {
        return vec![];
    }
    report
        .specs
        .iter()
        .filter(|s| s.complete)
        .filter(|s| {
            let dir = format!(".keel/specs/{}/", s.slug);
            out.lines()
                .any(|l| l.get(3..).is_some_and(|p| p.starts_with(&dir)))
        })
        .map(|s| s.slug.clone())
        .collect()
}

/// Every project at a glance: the manifest, whether it is up, and — for
/// a sandbox that is up — where its active spec stands.
pub fn overview() -> Result<Value> {
    let names = paths::all_project_names()?;
    let running = commands::running_projects(&names)?;
    let default = paths::read_current_project().ok().flatten();
    let mut projects = vec![];
    for name in &names {
        let Ok(m) = load(name) else { continue };
        let up = running.contains(name);
        let mut p = manifest_json(&m);
        p["up"] = json!(up);
        p["is_default"] = json!(default.as_deref() == Some(name.as_str()));
        p["pin"] = json!(commands::next_cmd::read_pin(name));
        if up {
            match report(name, &m) {
                Ok(r) => {
                    let active = active_json(name, &r);
                    let active_spec = active["slug"]
                        .as_str()
                        .and_then(|slug| r.specs.iter().find(|s| s.slug == slug))
                        .map(spec_json);
                    let open = r.specs.iter().filter(|s| !s.complete).count();
                    p["active"] = active;
                    p["active_spec"] = json!(active_spec);
                    p["open_specs"] = json!(open);
                    p["spec_count"] = json!(r.specs.len());
                    p["unshipped"] = json!(unshipped(name, &m, &r));
                    p["blockers"] = report_json(&r)["blockers"].clone();
                }
                Err(e) => p["error"] = json!(format!("{e:#}")),
            }
        }
        projects.push(p);
    }
    Ok(json!({
        "projects": projects,
        "default": default,
        "home": paths::moor_home()?.display().to_string(),
    }))
}

fn git_state(name: &str, m: &Manifest) -> Value {
    match exec(
        name,
        m,
        &["git", "status", "--porcelain=v2", "--branch"],
        false,
    ) {
        Ok((true, out)) => {
            let st = doctor_cmd::parse_porcelain_v2(&out);
            json!({
                "branch": st.branch,
                "detached": st.detached,
                "upstream": st.upstream,
                "ahead": st.ahead,
                "behind": st.behind,
                "dirty": st.dirty,
            })
        }
        _ => Value::Null,
    }
}

pub fn project(name: &str) -> Result<Value> {
    let m = load(name)?;
    let up = is_up(name)?;
    let mut out = manifest_json(&m);
    out["up"] = json!(up);
    out["is_default"] =
        json!(paths::read_current_project().ok().flatten().as_deref() == Some(name));
    out["secrets_status"] = json!(secrets::status(name, &m.secrets)
        .iter()
        .map(|(n, s)| json!({"name": n, "source": source_name(s)}))
        .collect::<Vec<_>>());
    out["posture"] = read_json_file(&paths::posture_path(name)?);
    out["session"] = json!(paths::session_path(name)
        .ok()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| crate::session::valid_session_id(s)));
    if !up {
        return Ok(out);
    }
    let r = report(name, &m)?;
    out["report"] = report_json(&r);
    out["active"] = active_json(name, &r);
    out["unshipped"] = json!(unshipped(name, &m, &r));
    out["runs"] = exec_json(name, &m, &["keel", "runs", "--json"])
        .map(|v| v["runs"].clone())
        .unwrap_or(json!([]));
    out["git"] = git_state(name, &m);
    Ok(out)
}

fn read_json_file(path: &Path) -> Value {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(Value::Null)
}

const SEP: &str = "@@moor-artifact:";

/// A spec's three artifacts, its gate history and approvals, from keel.
pub fn spec(name: &str, slug: &str) -> Result<Value> {
    manifest::validate_name(slug).context("spec name")?;
    let m = load(name)?;
    if !is_up(name)? {
        anyhow::bail!("{name} isn't running");
    }
    let script = format!(
        "for f in spec plan tasks; do echo '{SEP}'$f; cat .keel/specs/{slug}/$f.md 2>/dev/null; done"
    );
    let (_, out) = sh(name, &m, &script)?;
    let mut artifacts = json!({ "spec": null, "plan": null, "tasks": null });
    let mut current: Option<String> = None;
    let mut buf = String::new();
    let flush = |artifacts: &mut Value, current: &Option<String>, buf: &str| {
        if let Some(k) = current {
            if !buf.is_empty() {
                artifacts[k] = json!(buf.trim_end_matches('\n'));
            }
        }
    };
    for line in out.lines() {
        if let Some(k) = line.strip_prefix(SEP) {
            flush(&mut artifacts, &current, &buf);
            current = Some(k.trim().to_string());
            buf.clear();
        } else {
            buf.push_str(line);
            buf.push('\n');
        }
    }
    flush(&mut artifacts, &current, &buf);

    let report = exec_json(name, &m, &["keel", "report", slug, "--json"])
        .ok()
        .and_then(|v| v["specs"].as_array().and_then(|a| a.first().cloned()))
        .unwrap_or(Value::Null);
    let approvals =
        exec_json(name, &m, &["keel", "approvals", slug, "--json"]).unwrap_or(Value::Null);
    Ok(json!({
        "slug": slug,
        "artifacts": artifacts,
        "report": report,
        "approvals": approvals,
    }))
}

pub fn diff(name: &str, slug: &str) -> Result<Value> {
    manifest::validate_name(slug).context("spec name")?;
    let m = load(name)?;
    let (_, patch) = exec(
        name,
        &m,
        &[
            "git",
            "--no-pager",
            "diff",
            "HEAD",
            "--",
            ".",
            ":(exclude).keel",
        ],
        false,
    )?;
    let (_, stat) = exec(
        name,
        &m,
        &[
            "git",
            "--no-pager",
            "diff",
            "--stat",
            "HEAD",
            "--",
            ".",
            ":(exclude).keel",
        ],
        false,
    )?;
    let (_, new_files) = exec(
        name,
        &m,
        &[
            "git",
            "ls-files",
            "--others",
            "--exclude-standard",
            "--",
            ".",
            ":(exclude).keel",
        ],
        false,
    )?;
    Ok(json!({
        "patch": patch,
        "stat": stat,
        "new_files": new_files.lines().filter(|l| !l.trim().is_empty()).collect::<Vec<_>>(),
    }))
}

pub fn write_artifact(name: &str, slug: &str, kind: &str, text: &str) -> Result<()> {
    manifest::validate_name(slug).context("spec name")?;
    let file = match kind {
        "spec" => "spec.md",
        "plan" => "plan.md",
        "tasks" => "tasks.md",
        other => anyhow::bail!("unknown artifact '{other}'"),
    };
    let m = load(name)?;
    write_into(
        name,
        &m,
        &format!("/workspace/.keel/specs/{slug}/{file}"),
        text,
    )
}

fn safe_run_id(id: &str) -> Result<()> {
    if id.is_empty() || id.len() > 64 || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        anyhow::bail!("'{id}' isn't a run id");
    }
    Ok(())
}

fn safe_file(f: &str) -> Result<()> {
    if f.is_empty()
        || f.len() > 128
        || f.starts_with('.')
        || !f
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    {
        anyhow::bail!("'{f}' isn't a file name this will read");
    }
    Ok(())
}

/// A run's metadata, gate verdicts, evidence files and trajectory.
pub fn run(name: &str, id: &str) -> Result<Value> {
    safe_run_id(id)?;
    let m = load(name)?;
    let dir = format!(".keel/runs/{id}");
    let script = format!(
        "cd {dir} 2>/dev/null || exit 3; echo '{SEP}run'; cat run.json 2>/dev/null; \
         for g in gates/*.json; do [ -f \"$g\" ] && echo \"{SEP}gate:$g\" && cat \"$g\"; done; \
         echo '{SEP}evidence'; ls -l evidence 2>/dev/null | tail -n +2; \
         echo '{SEP}trajectory'; head -c 2000000 trajectory.jsonl 2>/dev/null"
    );
    let (ok, out) = sh(name, &m, &script)?;
    if !ok && out.trim().is_empty() {
        anyhow::bail!("no run {id} in {name}");
    }
    let mut run = Value::Null;
    let mut gates = vec![];
    let mut evidence = vec![];
    let mut trajectory = vec![];
    let mut section = String::new();
    let mut buf = String::new();
    let finish = |section: &str, buf: &str, gates: &mut Vec<Value>, run: &mut Value| {
        if section == "run" {
            *run = serde_json::from_str(buf).unwrap_or(Value::Null);
        } else if section.starts_with("gate:") {
            if let Ok(v) = serde_json::from_str::<Value>(buf) {
                gates.push(v);
            }
        }
    };
    for line in out.lines() {
        if let Some(k) = line.strip_prefix(SEP) {
            finish(&section, &buf, &mut gates, &mut run);
            section = k.trim().to_string();
            buf.clear();
            continue;
        }
        match section.as_str() {
            "evidence" => {
                // `ls -l`: perms links owner group size date... name
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 9 {
                    evidence.push(json!({ "name": parts[8..].join(" "), "size": parts[4].parse::<u64>().unwrap_or(0) }));
                }
            }
            "trajectory" => {
                if let Ok(v) = serde_json::from_str::<Value>(line) {
                    trajectory.push(v);
                }
            }
            _ => {
                buf.push_str(line);
                buf.push('\n');
            }
        }
    }
    finish(&section, &buf, &mut gates, &mut run);
    Ok(json!({
        "id": id,
        "run": run,
        "gates": gates,
        "evidence": evidence,
        "trajectory": trajectory,
    }))
}

pub fn evidence(name: &str, id: &str, file: &str) -> Result<String> {
    safe_run_id(id)?;
    safe_file(file)?;
    let m = load(name)?;
    let path = format!(".keel/runs/{id}/evidence/{file}");
    let (ok, out) = sh(name, &m, &format!("head -c 4000000 {path}"))?;
    if !ok {
        anyhow::bail!("couldn't read {path}");
    }
    Ok(out)
}

/// The host's audit chain for a project, newest last. `after` is a seq:
/// only entries past it are returned, so the page can poll cheaply.
pub fn audit(name: &str, after: Option<u64>, limit: usize, fold: bool) -> Result<Value> {
    let m = load(name)?;
    let mut folded = 0;
    if fold && is_up(name)? {
        folded =
            audit::fold_egress_log(name, &m).unwrap_or(0) + audit::fold_sink(name, &m).unwrap_or(0);
    }
    let lines = chain_lines(name);
    let mut entries: Vec<Value> = lines
        .iter()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .collect();
    let total = entries.len();
    let tripwires = entries.iter().filter(|e| e["kind"] == "tripwire").count();
    let mut kinds = std::collections::BTreeMap::<String, usize>::new();
    for e in &entries {
        if let Some(k) = e["kind"].as_str() {
            *kinds.entry(k.to_string()).or_default() += 1;
        }
    }
    if let Some(after) = after {
        entries.retain(|e| e["seq"].as_u64().unwrap_or(0) > after);
    }
    let start = entries.len().saturating_sub(limit);
    let entries: Vec<Value> = entries.drain(start..).map(sanitize_value).collect();
    Ok(json!({
        "entries": entries,
        "total": total,
        "tripwires": tripwires,
        "kinds": kinds,
        "folded": folded,
        "path": paths::chain_log_path(name)?.display().to_string(),
        "legacy": audit::legacy_path(&paths::chain_log_path(name)?).exists(),
    }))
}

/// Control characters out of every string in a value, recursively — the
/// chain holds argv and notes that came from the sandbox.
fn sanitize_value(v: Value) -> Value {
    match v {
        Value::String(s) => Value::String(sanitize(&s)),
        Value::Array(a) => Value::Array(a.into_iter().map(sanitize_value).collect()),
        Value::Object(o) => {
            Value::Object(o.into_iter().map(|(k, v)| (k, sanitize_value(v))).collect())
        }
        other => other,
    }
}

pub fn transcript(name: &str) -> Result<Value> {
    load(name)?;
    let path = paths::transcript_path(name)?;
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let turns: Vec<Value> = text
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .map(sanitize_value)
        .collect();
    Ok(json!({ "turns": turns }))
}

/// `keel driver list`'s table, parsed: id, path, whether it is the default.
fn parse_drivers(text: &str) -> Vec<Value> {
    text.lines()
        .filter(|l| l.starts_with("  ") && !l.trim_start().starts_with('`'))
        .filter_map(|l| {
            let mut parts = l.split_whitespace();
            let id = parts.next()?;
            let path = parts.next().unwrap_or("");
            let default = parts.any(|p| p == "default");
            Some(json!({ "id": id, "path": path, "default": default }))
        })
        .collect()
}

pub fn settings(name: &str) -> Result<Value> {
    let m = load(name)?;
    let up = is_up(name)?;
    let mut out = manifest_json(&m);
    out["up"] = json!(up);
    out["secrets_status"] = json!(secrets::status(name, &m.secrets)
        .iter()
        .map(|(n, s)| json!({"name": n, "source": source_name(s)}))
        .collect::<Vec<_>>());
    out["copilot_device_token"] = json!(secrets::copilot_device_token().is_some());
    out["images"] = json!(moor_images());
    out["drivers"] = if up {
        exec(name, &m, &["keel", "driver", "list"], true)
            .map(|(_, t)| json!(parse_drivers(&t)))
            .unwrap_or(json!([]))
    } else {
        json!([])
    };
    out["keel_config"] = if up {
        sh(name, &m, "cat .keel/keel.toml 2>/dev/null")
            .map(|(_, t)| json!(t))
            .unwrap_or(Value::Null)
    } else {
        Value::Null
    };
    out["manifest_path"] = json!(paths::manifest_path(name)?.display().to_string());
    Ok(out)
}

pub fn moor_images() -> Vec<String> {
    proc::run_capture(
        "docker",
        &["images", "--format", "{{.Repository}}:{{.Tag}}", "moor/*"],
    )
    .map(|(_, out)| {
        let mut v: Vec<String> = out
            .lines()
            .map(str::trim)
            .filter(|l| l.starts_with("moor/") && !l.ends_with(":<none>"))
            .map(String::from)
            .collect();
        v.sort();
        v
    })
    .unwrap_or_default()
}

pub fn valid_image(image: &str) -> bool {
    let (repo, tag) = image.split_once(':').unwrap_or((image, "latest"));
    !repo.is_empty()
        && repo.len() < 200
        && repo.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'/' | b'-' | b'_' | b'.')
        })
        && !tag.is_empty()
        && tag
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

fn valid_host(h: &str) -> bool {
    !h.is_empty()
        && h.len() < 254
        && h.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'*'))
}

/// Apply a settings change to the manifest on the host. Takes effect on
/// the next `moor up`, which the page offers right after.
pub fn update_manifest(name: &str, patch: &Value) -> Result<Value> {
    let mut m = load(name)?;
    if let Some(agent) = patch["agent"].as_str() {
        m.agent = agent.parse()?;
    }
    if let Some(image) = patch["image"].as_str() {
        if !valid_image(image) {
            anyhow::bail!("'{image}' isn't an image name");
        }
        m.image = manifest::resolve_image(image, m.agent);
    }
    if let Some(allow) = patch["egress"].as_array() {
        let mut hosts = vec![];
        for h in allow {
            let h = h.as_str().unwrap_or("").trim();
            if !valid_host(h) {
                anyhow::bail!("'{h}' isn't a hostname");
            }
            hosts.push(h.to_string());
        }
        m.egress.allow = hosts;
    }
    if let Some(r) = patch["resources"].as_object() {
        if let Some(cpu) = r.get("cpu").and_then(Value::as_str) {
            cpu.parse::<f64>().context("cpu must be a number")?;
            m.resources.cpu = cpu.to_string();
        }
        if let Some(mem) = r.get("mem").and_then(Value::as_str) {
            let ok = mem.len() < 8
                && mem.ends_with(['g', 'm', 'G', 'M'])
                && mem[..mem.len() - 1].parse::<u32>().is_ok();
            if !ok {
                anyhow::bail!("mem must look like 4g or 512m");
            }
            m.resources.mem = mem.to_string();
        }
        if let Some(p) = r.get("pids").and_then(Value::as_u64) {
            m.resources.pids = p as u32;
        }
    }
    if let Some(repo) = patch.get("github_repo") {
        m.github_repo = repo
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from);
    }
    if let Some(list) = patch["secrets"].as_array() {
        let mut names = vec![];
        for s in list {
            let s = s.as_str().unwrap_or("").trim();
            if !valid_secret_name(s) {
                anyhow::bail!("'{s}' isn't an environment variable name");
            }
            names.push(s.to_string());
        }
        m.secrets = names;
    }
    m.save(&paths::manifest_path(name)?)?;
    Ok(manifest_json(&m))
}

pub fn valid_secret_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() < 64
        && s.bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
        && !s.as_bytes()[0].is_ascii_digit()
}

pub fn set_driver(name: &str, id: &str) -> Result<String> {
    if !id
        .bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        || id.is_empty()
    {
        anyhow::bail!("'{id}' isn't a driver id");
    }
    let m = load(name)?;
    let (ok, out) = exec(name, &m, &["keel", "driver", "default", id], true)?;
    if !ok {
        anyhow::bail!("{}", out.trim());
    }
    Ok(out)
}

/// Make `name` the default project and pin (or unpin) its active spec —
/// what `moor use` does, without the printing.
pub fn pin(name: &str, slug: Option<&str>) -> Result<()> {
    load(name)?;
    paths::write_current_project(name)?;
    let path = paths::active_spec_path(name)?;
    match slug {
        Some(s) => {
            manifest::validate_name(s).context("spec name")?;
            std::fs::write(&path, s)?;
        }
        None => {
            let _ = std::fs::remove_file(&path);
        }
    }
    Ok(())
}

/// Where the console keeps files it writes on the host for a project:
/// specs and recipes about to be sent in.
pub fn drafts_dir(name: &str) -> Result<PathBuf> {
    let dir = paths::project_dir(name)?.join("drafts");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Directories under `path` (or $HOME), for picking a repo to import.
pub fn list_dirs(path: Option<&str>) -> Result<Value> {
    let home = std::env::var("HOME").context("HOME is not set")?;
    let base = PathBuf::from(path.filter(|p| !p.is_empty()).unwrap_or(&home));
    let base = base
        .canonicalize()
        .with_context(|| format!("{} doesn't exist", base.display()))?;
    if !base.is_dir() {
        anyhow::bail!("{} isn't a directory", base.display());
    }
    let mut dirs = vec![];
    for entry in std::fs::read_dir(&base)? {
        let entry = entry?;
        let fname = entry.file_name().to_string_lossy().to_string();
        if fname.starts_with('.') || !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let p = entry.path();
        dirs.push(json!({
            "name": fname,
            "path": p.display().to_string(),
            "git": p.join(".git").exists(),
            "kind": detect_kind(&p),
        }));
    }
    dirs.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    Ok(json!({
        "path": base.display().to_string(),
        "parent": base.parent().map(|p| p.display().to_string()),
        "home": home,
        "dirs": dirs,
    }))
}

fn detect_kind(p: &Path) -> &'static str {
    if p.join("Cargo.toml").exists() {
        "rust"
    } else if p.join("package.json").exists() {
        "node"
    } else if p.join("pyproject.toml").exists() || p.join("requirements.txt").exists() {
        "python"
    } else {
        ""
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn driver_table_is_parsed() {
        let text = "  noop           .keel/drivers/noop                          \n  claude-code    .keel/drivers/claude-code          default  \n\n  `keel driver check <id>` runs the conformance suite\n";
        let d = parse_drivers(text);
        assert_eq!(d.len(), 2);
        assert_eq!(d[1]["id"], "claude-code");
        assert_eq!(d[1]["default"], true);
        assert_eq!(d[0]["default"], false);
    }

    #[test]
    fn image_and_secret_names_are_shaped() {
        assert!(valid_image("moor/node:latest"));
        assert!(valid_image("moor/base"));
        assert!(!valid_image("moor/node:la test"));
        assert!(!valid_image("Moor/Node"));
        assert!(valid_secret_name("ANTHROPIC_API_KEY"));
        assert!(!valid_secret_name("1BAD"));
        assert!(!valid_secret_name("lower"));
    }

    #[test]
    fn run_ids_and_evidence_files_are_shaped() {
        assert!(safe_run_id("2026-09-24-002").is_ok());
        assert!(safe_run_id("../x").is_err());
        assert!(safe_file("build.txt").is_ok());
        assert!(safe_file("../run.json").is_err());
        assert!(safe_file(".hidden").is_err());
    }

    #[test]
    fn strings_are_sanitized_recursively() {
        let v = json!({"a": ["x\x1b[31m", {"b": "y\r"}]});
        let s = sanitize_value(v);
        assert_eq!(s, json!({"a": ["x", {"b": "y"}]}));
    }
}
