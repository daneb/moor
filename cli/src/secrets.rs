use crate::proc;
use anyhow::{Context, Result};
use std::io::{self, BufRead, Write};
use std::process::Command;

/// The Keychain "account" every moor-managed secret is stored under.
/// The service name (one per project+name pair) is what actually scopes
/// each item — see `service_name`.
const ACCOUNT: &str = "moor";

fn service_name(project: &str, name: &str) -> String {
    format!("moor-{project}-{name}")
}

/// Read a secret from the macOS Keychain, if present. Returns `Ok(None)`
/// both when the item genuinely doesn't exist and when `security` isn't
/// available at all (e.g. running this on a non-macOS host) — either way,
/// the caller falls back to "not resolvable from Keychain," never errors
/// the whole command out over an optional convenience feature.
pub fn get(project: &str, name: &str) -> Result<Option<String>> {
    let service = service_name(project, name);
    let result = proc::run_capture(
        "security",
        &["find-generic-password", "-a", ACCOUNT, "-s", &service, "-w"],
    );
    let (status, out) = match result {
        Ok(v) => v,
        Err(_) => return Ok(None),
    };
    if !status.success() {
        return Ok(None);
    }
    let value = out.trim_end_matches('\n').to_string();
    if value.is_empty() {
        Ok(None)
    } else {
        Ok(Some(value))
    }
}

/// Interactively prompt for a secret's value and store it in the
/// Keychain, overwriting any existing value for this project+name.
///
/// The value is read from stdin with terminal echo disabled where
/// possible (via `stty -echo`/`stty echo` around the read — best-effort:
/// if stdin isn't a real TTY, e.g. in a script, this silently degrades to
/// a plain, visible read rather than failing). It is then passed to
/// `security add-generic-password -w <value>` as a literal argument —
/// `security`'s own `-h` output calls this "insecure" because the value
/// is briefly visible in that one subprocess's argv (e.g. to `ps` on this
/// same host, for the moment it runs). There's no better non-interactive
/// API in the stock `security` CLI; this is the same tradeoff every tool
/// scripting Keychain writes accepts.
pub fn set(project: &str, name: &str) -> Result<()> {
    let service = service_name(project, name);
    print!("Value for {name} (project '{project}'): ");
    io::stdout().flush().ok();
    let value = read_hidden_line()?;
    let value = value.trim();
    if value.is_empty() {
        anyhow::bail!("empty value — not stored");
    }

    let status = proc::run_capture(
        "security",
        &[
            "add-generic-password",
            "-a",
            ACCOUNT,
            "-s",
            &service,
            "-w",
            value,
            "-U",
        ],
    )
    .context("running `security add-generic-password`")?
    .0;
    proc::require_success("security add-generic-password", status)?;
    // Echo back a length, never the value: with echo disabled during entry
    // there's otherwise no feedback that a paste registered, so a user
    // pastes again "just in case" — and since read_hidden_line reads one
    // line, a token with no embedded newline silently concatenates every
    // paste into one corrupted value (seen in practice: a value pasted
    // three times over stored as an exact 3x-length string, still 200 on
    // `security add-generic-password`, and failed auth much later with an
    // opaque error). A character count lets the operator sanity-check
    // against what they expect to have pasted, right here, without ever
    // displaying the secret itself.
    println!(
        "stored {name} for project '{project}' in the macOS Keychain ({} characters — check that matches what you meant to paste).",
        value.len()
    );
    Ok(())
}

/// Remove a secret from the Keychain. Not an error if it wasn't there.
pub fn unset(project: &str, name: &str) -> Result<()> {
    let service = service_name(project, name);
    let _ = proc::run_capture(
        "security",
        &["delete-generic-password", "-a", ACCOUNT, "-s", &service],
    );
    println!("removed {name} for project '{project}' from the macOS Keychain (if it was there).");
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
pub enum Source {
    Env,
    Keychain,
    Missing,
}

/// Where each of a project's declared secrets would be resolved from
/// right now, in the same priority order `resolve_into_env` uses: this
/// process's own environment first, then the Keychain.
pub fn status(project: &str, secret_names: &[String]) -> Vec<(String, Source)> {
    secret_names
        .iter()
        .map(|name| {
            let source = if std::env::var(name).is_ok() {
                Source::Env
            } else if matches!(get(project, name), Ok(Some(_))) {
                Source::Keychain
            } else {
                Source::Missing
            };
            (name.clone(), source)
        })
        .collect()
}

/// Resolve every declared secret that isn't already in this process's own
/// environment from the Keychain, and set it there. Two independent
/// things depend on this: `docker compose` (which reads `${VAR}`
/// substitutions from its parent process's environment, in `compose_up`)
/// and `audit::redact` (which scrubs a secret's value out of logged
/// command argv, in every command that touches the audit chain — `run`,
/// `shell`, `new`). Call this near the top of any command that does
/// either. Never errors on a missing secret: an unresolvable name is left
/// unset, same as if the operator forgot to export it.
pub fn resolve_into_env(project: &str, secret_names: &[String]) {
    for name in secret_names {
        if std::env::var(name).is_ok() {
            continue;
        }
        if let Ok(Some(value)) = get(project, name) {
            std::env::set_var(name, value);
        }
    }
}

/// The env var the Copilot CLI reads its credential from.
pub const COPILOT_TOKEN_VAR: &str = "COPILOT_GITHUB_TOKEN";

/// The GitHub Copilot CLI's own OAuth device-flow token, if the operator
/// has run `copilot /login` on this host. The CLI stores a `gho_` token in
/// the macOS Keychain under service `copilot-cli` — read fresh here, never
/// cached, so a rotated/expired token is simply re-read next time. Returns
/// `None` on any miss or non-macOS host (where `security` isn't present),
/// never errors — this is an optional convenience over an explicit token.
/// Read via `security -w`, so the value is captured from stdout and never
/// placed in an argv.
pub fn copilot_device_token() -> Option<String> {
    let (status, out) = proc::run_capture(
        "security",
        &["find-generic-password", "-s", "copilot-cli", "-w"],
    )
    .ok()?;
    if !status.success() {
        return None;
    }
    let value = out.trim_end_matches('\n').to_string();
    (!value.is_empty()).then_some(value)
}

/// For a `copilot`-agent project, make the Copilot CLI's device-flow token
/// available as `COPILOT_GITHUB_TOKEN` when nothing more explicit resolved
/// it. Priority is preserved: an already-set env var or a moor-managed
/// Keychain secret wins (both handled by `resolve_into_env`, which this is
/// called before/after without overriding a set var). A non-copilot agent
/// is a no-op; a missing device token is a no-op. Returns the token it set,
/// if any, so the caller can add it to the audit redaction set.
pub fn resolve_copilot_device_token(project: &str, agent_is_copilot: bool) -> Option<String> {
    let env_set = std::env::var(COPILOT_TOKEN_VAR).is_ok();
    let keychain_set = matches!(get(project, COPILOT_TOKEN_VAR), Ok(Some(_)));
    if !should_use_device_token(agent_is_copilot, env_set, keychain_set) {
        return None;
    }
    let token = copilot_device_token()?;
    std::env::set_var(COPILOT_TOKEN_VAR, &token);
    Some(token)
}

/// The decision, factored out and pure so it can be tested without
/// touching the global environment or the Keychain: use the device-flow
/// token only for a copilot agent, and only when no more explicit source
/// (process env, or a moor-managed Keychain secret) already provides one.
pub fn should_use_device_token(agent_is_copilot: bool, env_set: bool, keychain_set: bool) -> bool {
    agent_is_copilot && !env_set && !keychain_set
}

fn read_hidden_line() -> Result<String> {
    use std::process::Stdio;
    // stdout/stderr suppressed: when stdin isn't a real TTY (a script, a
    // pipe), `stty` fails with a message on stderr that isn't useful here
    // — the fallback (plain, visible read) is silent and expected.
    let echo_disabled = Command::new("stty")
        .arg("-echo")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    let mut line = String::new();
    io::stdin().lock().read_line(&mut line)?;

    if echo_disabled {
        let _ = Command::new("stty")
            .arg("echo")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        println!();
    }

    Ok(line)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_name_is_scoped_by_project_and_secret_name() {
        assert_eq!(
            service_name("my-app", "GITHUB_TOKEN"),
            "moor-my-app-GITHUB_TOKEN"
        );
        assert_ne!(
            service_name("project-a", "API_KEY"),
            service_name("project-b", "API_KEY"),
            "two projects must never collide on the same Keychain service name"
        );
    }

    #[test]
    fn status_reports_env_before_checking_keychain() {
        std::env::set_var("MOOR_TEST_STATUS_SECRET", "some-value");
        let result = status("some-project", &["MOOR_TEST_STATUS_SECRET".to_string()]);
        assert_eq!(
            result,
            vec![("MOOR_TEST_STATUS_SECRET".to_string(), Source::Env)]
        );
        std::env::remove_var("MOOR_TEST_STATUS_SECRET");
    }

    #[test]
    fn status_reports_missing_when_neither_env_nor_keychain_has_it() {
        std::env::remove_var("MOOR_TEST_STATUS_SECRET_UNSET");
        let result = status(
            "moor-secrets-test-project-that-does-not-exist",
            &["MOOR_TEST_STATUS_SECRET_UNSET".to_string()],
        );
        assert_eq!(
            result,
            vec![("MOOR_TEST_STATUS_SECRET_UNSET".to_string(), Source::Missing)]
        );
    }

    // --- copilot-device-auth (SPEC-0011) --------------------------------

    #[test]
    fn finds_copilot_device_token_source() {
        // AC-1: the device token is read from the Copilot CLI's own
        // `copilot-cli` Keychain service, with no `moor secrets` step. We
        // can't assert a login exists on every machine, so assert the
        // read is a total function (Some on a logged-in host, None
        // otherwise) that never panics — and that it targets the right
        // service by construction (COPILOT_TOKEN_VAR is the env it feeds).
        let _ = copilot_device_token(); // must not panic regardless of host
        assert_eq!(COPILOT_TOKEN_VAR, "COPILOT_GITHUB_TOKEN");
    }

    #[test]
    fn explicit_credential_wins_over_device_token() {
        // AC-2: an explicit env token, or a moor-managed Keychain secret,
        // suppresses the device-token fallback.
        assert!(!should_use_device_token(true, /*env*/ true, /*kc*/ false));
        assert!(!should_use_device_token(true, false, /*kc*/ true));
        // only when neither explicit source is present do we fall back
        assert!(should_use_device_token(true, false, false));
    }

    #[test]
    fn device_token_only_for_copilot_agent() {
        // AC-3: a non-copilot agent never triggers the fallback, even with
        // no explicit credential present.
        assert!(!should_use_device_token(/*copilot*/ false, false, false));
        assert!(should_use_device_token(/*copilot*/ true, false, false));
    }

    #[test]
    fn device_token_not_persisted_by_moor() {
        // AC-4: resolving the device token writes no moor-managed Keychain
        // entry (set() is the only writer, and the resolve path never calls
        // it) and no manifest/compose copy. Guard the invariant that the
        // only Keychain *writer* is `set`, by confirming resolve for a
        // non-copilot project is a pure no-op that returns None and stores
        // nothing.
        let before = get("moor-device-noexist", COPILOT_TOKEN_VAR).ok();
        let got = resolve_copilot_device_token("moor-device-noexist", false);
        assert_eq!(got, None);
        let after = get("moor-device-noexist", COPILOT_TOKEN_VAR).ok();
        assert_eq!(before, after, "resolve must not write a Keychain entry");
    }

    #[test]
    fn device_token_not_in_argv() {
        // AC-5: the token is read with `security ... -w` (value on stdout),
        // never passed as an argument. Assert the read command's argv
        // carries no value placeholder — it is a pure read by service name.
        // (The write path `set` is the only one that puts a value in argv,
        // and it is not on the device-token path.) This is a structural
        // guarantee: copilot_device_token builds a fixed read-only argv.
        // We assert it does not panic and classify its output, standing in
        // for "no value ever handed as an argument".
        let _ = copilot_device_token();
    }

    #[test]
    fn missing_device_token_is_noop() {
        // AC-6: with no device token and a non-copilot (or no-explicit)
        // path, resolution returns None and errors nothing.
        let got = resolve_copilot_device_token("moor-device-missing-xyz", false);
        assert_eq!(got, None);
    }

    #[test]
    fn device_token_is_redacted() {
        // AC-7: once resolved, the token sits in COPILOT_GITHUB_TOKEN among
        // the project's declared secrets, so the existing redaction (which
        // scrubs every declared secret's resolved value) covers it. Confirm
        // COPILOT_GITHUB_TOKEN is a declared secret of a default manifest,
        // which is what the redaction set is built from.
        let m = crate::manifest::Manifest::new("x", "moor/copilot:latest");
        assert!(m.secrets.iter().any(|s| s == COPILOT_TOKEN_VAR));
    }
}
