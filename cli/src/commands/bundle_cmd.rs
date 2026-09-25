use crate::{audit, manifest::Manifest, paths, proc};
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// Where the host chain lands inside the throwaway container.
const CHAIN_IN: &str = "/tmp/moor-host-chain.jsonl";

/// keel's `export --chain` for a project under moor: the bundle carries the
/// chain moor wrote on the host, which keel inside the sandbox never holds
/// (keel ADR-0001). Built in a throwaway container from the project's
/// image — not the sandbox, where the agent could hand keel a chain of its
/// own — with no network, the workspace's named volume, and the host chain
/// on stdin. keel prints the bundle's path on stdout, so that goes to
/// stderr and stdout carries nothing but the archive.
fn export_args(m: &Manifest, run: Option<&str>) -> Vec<String> {
    let script = format!(
        "cat > {CHAIN_IN} && keel export \"$@\" --chain {CHAIN_IN} --out /tmp/moor-out >&2 \
         && cat /tmp/moor-out/*.tar.gz"
    );
    let mut args: Vec<String> = [
        "run",
        "--rm",
        "-i",
        "--network",
        "none",
        "-v",
        &format!("{}-workspace:/workspace", m.name),
        "-w",
        "/workspace",
        "--entrypoint",
        "sh",
        &m.image,
        "-c",
        &script,
        "sh",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    args.extend(run.map(String::from));
    args
}

/// keel's own verifier over the archive, in a second throwaway container
/// that gets nothing but the archive on stdin.
fn verify_args(m: &Manifest) -> Vec<String> {
    [
        "run",
        "--rm",
        "-i",
        "--network",
        "none",
        "--entrypoint",
        "sh",
        &m.image,
        "-c",
        "cat > /tmp/bundle.tar.gz && keel bundle verify /tmp/bundle.tar.gz",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// Run the export into `archive`, removing it if the export fails: a
/// truncated archive on disk would look like evidence and be none.
fn export_to(program: &str, args: &[String], chain: &Path, archive: &Path) -> Result<()> {
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let status = proc::run_stdin_to_file(program, &args, chain, archive);
    match status {
        Ok(s) if s.success() => Ok(()),
        other => {
            let _ = std::fs::remove_file(archive);
            match other {
                Ok(s) => anyhow::bail!("keel export failed ({s}) — see its output above"),
                Err(e) => Err(e),
            }
        }
    }
}

pub fn run(name: &str, run: Option<&str>, out: Option<PathBuf>) -> Result<()> {
    let m = Manifest::load(&paths::manifest_path(name)?)?;
    // keel's latest run_end has to be in the chain before the chain ships.
    audit::fold_sink(name, &m).context("folding keel's sink before bundling")?;
    let chain = paths::chain_log_path(name)?;
    if !chain.exists() {
        anyhow::bail!("'{name}' has no audit chain yet — nothing to bundle");
    }

    let out = out.unwrap_or(std::env::current_dir()?);
    std::fs::create_dir_all(&out)?;
    let archive = out.join(format!(
        "keel-{name}-{}.tar.gz",
        chrono::Utc::now().format("%Y%m%dT%H%M%SZ")
    ));
    export_to("docker", &export_args(&m, run), &chain, &archive)?;
    println!("{}", archive.display());

    let args = verify_args(&m);
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let status = proc::run_with_stdin_file("docker", &args, &archive)?;
    if !status.success() {
        // keel's own verdict is the exit code: 1 fail, 3 blocked.
        std::process::exit(status.code().unwrap_or(1));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m() -> Manifest {
        Manifest::new("sample-app", "moor/rust:latest")
    }

    /// No network, no host path; only the named workspace volume.
    fn assert_throwaway(args: &[String]) {
        assert_eq!(&args[..3], ["run", "--rm", "-i"]);
        let pos = args
            .iter()
            .position(|a| a == "--network")
            .expect("no --network");
        assert_eq!(args[pos + 1], "none");
        for (i, a) in args.iter().enumerate() {
            if a == "-v" || a == "--volume" || a == "--mount" {
                let source = args[i + 1].split(':').next().unwrap();
                assert!(
                    !source.starts_with('/')
                        && !source.starts_with('.')
                        && !source.starts_with('~'),
                    "host path mounted: {}",
                    args[i + 1]
                );
            }
        }
        assert!(
            !args.iter().any(|a| a.contains("sandbox")),
            "ran in the sandbox: {args:?}"
        );
    }

    #[test]
    fn export_runs_in_a_throwaway_container_with_no_network() {
        let args = export_args(&m(), None);
        assert_throwaway(&args);
        assert!(args.contains(&"sample-app-workspace:/workspace".to_string()));
        assert!(
            args.contains(&"moor/rust:latest".to_string()),
            "not the project's image"
        );
    }

    #[test]
    fn export_hands_keel_the_host_chain() {
        let script = |a: &[String]| a[a.iter().position(|x| x == "-c").unwrap() + 1].clone();
        let latest = export_args(&m(), None);
        let s = script(&latest);
        assert!(s.starts_with(&format!("cat > {CHAIN_IN} &&")), "{s}");
        assert!(
            s.contains(&format!("keel export \"$@\" --chain {CHAIN_IN}")),
            "{s}"
        );
        assert!(
            s.contains(">&2"),
            "keel's path would pollute the archive: {s}"
        );
        assert_eq!(
            latest.last().unwrap(),
            "sh",
            "a run id was passed when none was given"
        );

        let named = export_args(&m(), Some("2026-09-25-000"));
        assert_eq!(&named[named.len() - 2..], ["sh", "2026-09-25-000"]);
    }

    #[test]
    fn a_failed_export_removes_the_partial_archive() {
        let dir = std::env::temp_dir().join(format!("moor-bundle-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let chain = dir.join("chain.jsonl");
        std::fs::write(&chain, "{}\n").unwrap();
        let archive = dir.join("out.tar.gz");

        let failing = ["-c".to_string(), "printf partial; exit 1".to_string()];
        assert!(export_to("sh", &failing, &chain, &archive).is_err());
        assert!(!archive.exists(), "a partial archive was left behind");

        let working = ["-c".to_string(), "cat".to_string()];
        export_to("sh", &working, &chain, &archive).unwrap();
        assert_eq!(std::fs::read_to_string(&archive).unwrap(), "{}\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn verify_runs_in_a_throwaway_container_with_no_network() {
        let args = verify_args(&m());
        assert_throwaway(&args);
        assert!(
            !args.iter().any(|a| a == "-v"),
            "the verifier needs no volume: {args:?}"
        );
        assert!(args.last().unwrap().contains("keel bundle verify"));
    }
}
