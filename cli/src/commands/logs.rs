use crate::paths;
use anyhow::Result;
use serde_json::Value;
use std::time::Duration;

/// Render one audit-chain line as a short status line, if (and only if)
/// it's a `recipe-event` entry — the narration `moor recipe` appends
/// alongside its normal exec log, not the exec entries themselves (those
/// are `moor audit`'s job: full argv, exit codes, evidence). Anything
/// that fails to parse or isn't a recipe-event is silently skipped, so a
/// stray malformed line can't crash what's meant to be a lightweight
/// status view.
fn format_entry(line: &str) -> Option<String> {
    let v: Value = serde_json::from_str(line).ok()?;
    if v.get("kind")?.as_str()? != "recipe-event" {
        return None;
    }
    let ts = v.get("ts")?.as_str()?;
    let time_part = ts.split('T').nth(1).unwrap_or(ts);
    let time = time_part
        .find(['.', '+'])
        .map(|i| &time_part[..i])
        .unwrap_or(time_part);
    let data = v.get("data")?;
    Some(format!("[{time}] {}", describe(data)))
}

fn describe(data: &Value) -> String {
    let event = data.get("event").and_then(|v| v.as_str()).unwrap_or("?");
    let slug = data.get("slug").and_then(|v| v.as_str()).unwrap_or("?");
    let str_field = |key: &str| data.get(key).and_then(|v| v.as_str()).unwrap_or("?");
    let num_field = |key: &str| data.get(key).and_then(|v| v.as_u64()).unwrap_or(0);

    match event {
        "recipe-start" => format!("[{slug}] recipe started"),
        "spec-scaffolded" => format!("[{slug}] spec scaffolded"),
        "spec-resumed" => format!("[{slug}] spec already exists, resuming"),
        "spec-authored" => format!("[{slug}] spec authored by agent"),
        "stage" => format!("[{slug}] stage: {}", str_field("stage")),
        "gate-attempt" => format!(
            "[{slug}] gate attempt {}/{}: {}",
            num_field("attempt"),
            num_field("max"),
            str_field("result")
        ),
        "paused-for-approval" => format!("[{slug}] PAUSED — waiting on: {}", str_field("stage")),
        "run-attempt" => format!(
            "[{slug}] build attempt {}/{}",
            num_field("attempt"),
            num_field("max")
        ),
        "complete" => format!("[{slug}] complete"),
        "failed" => format!("[{slug}] FAILED: {}", str_field("reason")),
        other => format!("[{slug}] {other}"),
    }
}

pub fn run(name: &str, follow: bool, lines: usize) -> Result<()> {
    let path = paths::chain_log_path(name)?;
    let read_all = || -> Vec<String> {
        std::fs::read_to_string(&path)
            .map(|t| t.lines().map(String::from).collect())
            .unwrap_or_default()
    };

    let existing = read_all();
    let formatted: Vec<String> = existing.iter().filter_map(|l| format_entry(l)).collect();
    if formatted.is_empty() {
        println!("no recipe activity logged yet for '{name}'");
    } else {
        let start = formatted.len().saturating_sub(lines);
        for line in &formatted[start..] {
            println!("{line}");
        }
    }

    if !follow {
        return Ok(());
    }

    println!("-- following '{name}' (Ctrl-C to stop) --");
    let mut seen = existing.len();
    loop {
        std::thread::sleep(Duration::from_millis(500));
        let all = read_all();
        if all.len() > seen {
            for line in &all[seen..] {
                if let Some(f) = format_entry(line) {
                    println!("{f}");
                }
            }
            seen = all.len();
        }
    }
}
