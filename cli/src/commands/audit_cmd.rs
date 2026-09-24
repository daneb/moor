use crate::{audit, manifest::Manifest, paths, proc};
use anyhow::{Context, Result};
use std::path::PathBuf;

/// Show (and optionally verify or export) a project's audit trail.
/// Always folds in any new egress-gateway log lines first, so what's
/// printed reflects what's actually happened, not just what `moor
/// run`/`shell` happened to observe directly.
pub fn run(name: &str, verify: bool, export: Option<PathBuf>) -> Result<()> {
    let m = Manifest::load(&paths::manifest_path(name)?)?;
    let folded = audit::fold_egress_log(name, &m).unwrap_or(0)
        + audit::fold_sink(name, &m).unwrap_or(0);
    let path = paths::chain_log_path(name)?;

    if let Some(out_dir) = export {
        let bundle = export_bundle(name, &m, out_dir)?;
        println!("audit bundle written to {}", bundle.display());
        return Ok(());
    }

    if verify {
        // A chain sealed at the switch to keel.chain/1 is still history;
        // the new chain commits to its head, and it is checked here with
        // the hash it was written under.
        let legacy = audit::legacy_path(&path);
        if legacy.exists() {
            match audit::verify_legacy(&legacy)? {
                audit::VerifyOutcome::Ok { entries } => {
                    println!("legacy chain OK — {entries} entries ({})", legacy.display());
                }
                audit::VerifyOutcome::Tampered { at_seq, reason } => {
                    anyhow::bail!("TAMPERED: legacy chain broken at seq {at_seq}: {reason}");
                }
            }
        }
        match audit::verify_chain(&path)? {
            audit::VerifyOutcome::Ok { entries } => {
                println!("chain OK — {entries} entries, no tampering detected.");
                return Ok(());
            }
            audit::VerifyOutcome::Tampered { at_seq, reason } => {
                anyhow::bail!("TAMPERED: chain integrity broken at seq {at_seq}: {reason}");
            }
        }
    }

    if !path.exists() {
        println!("no audit entries yet for '{name}' ({})", path.display());
        return Ok(());
    }

    if folded > 0 {
        println!("(folded in {folded} new egress-log and keel-sink entries)\n");
    }

    println!("== audit chain: {} ==", path.display());
    let text = std::fs::read_to_string(&path)?;
    let mut tripwires = 0;
    for line in text.lines() {
        if line.contains("\"kind\":\"tripwire\"") {
            tripwires += 1;
            println!("[TRIPWIRE] {line}");
        } else {
            println!("{line}");
        }
    }

    println!(
        "\nkeel's own exported run bundles (.keel/bundles/, from `keel export`)\n\
         live inside the workspace volume, not here — use `moor audit\n\
         {name} --export` to pull everything (this chain + keel's bundles)\n\
         into one archive."
    );
    if tripwires > 0 {
        println!(
            "\n{tripwires} TRIPWIRE entr{} in this trail — see docs/THREAT-MODEL.md.",
            if tripwires == 1 { "y" } else { "ies" }
        );
    }
    Ok(())
}

fn export_bundle(name: &str, m: &Manifest, out_dir: PathBuf) -> Result<PathBuf> {
    std::fs::create_dir_all(&out_dir)?;
    let staging = paths::audit_dir(name)?.join(".export-staging");
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging)?;

    let chain_path = paths::chain_log_path(name)?;
    if chain_path.exists() {
        std::fs::copy(&chain_path, staging.join("chain.jsonl"))?;
    }
    std::fs::copy(paths::manifest_path(name)?, staging.join("moor.yaml"))?;

    // Best-effort: keel's own exported run bundles (`keel export <run>`
    // writes each as .keel/bundles/keel-<run-id>.tar.gz), if this project
    // has run any keel-gated work yet. Not fatal if there aren't any.
    let bundles_dest = staging.join("keel-bundles");
    let _ = proc::run_capture(
        "docker",
        &[
            "cp",
            &format!("{}:/workspace/.keel/bundles", m.sandbox_container()),
            &bundles_dest.to_string_lossy(),
        ],
    );

    let bundle_name = format!(
        "{name}-audit-{}.tar.gz",
        chrono::Utc::now().format("%Y%m%dT%H%M%SZ")
    );
    let bundle_path = out_dir.join(bundle_name);
    let staging_str = staging.to_string_lossy().to_string();
    let bundle_str = bundle_path.to_string_lossy().to_string();
    let status = proc::run_inherit("tar", &["-czf", &bundle_str, "-C", &staging_str, "."])?;
    proc::require_success("tar", status)?;

    std::fs::remove_dir_all(&staging).context("cleaning up export staging dir")?;
    Ok(bundle_path)
}
