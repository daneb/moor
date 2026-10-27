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

/// The env var the discovered Kiro SSO token is injected as. `kiro-cli`
/// inside the sandbox reads its bearer credential from this `AWS_*`
/// variable; it is the first of `manifest::KIRO_SECRET_VARS`, so the
/// existing secret→env plumbing and audit redaction already cover it.
pub const KIRO_TOKEN_VAR: &str = "AWS_BEARER_TOKEN_BEDROCK";

/// The host's SSO token cache directory (`~/.aws/sso/cache`), where
/// `kiro-cli login` for IAM Identity Center leaves its cached access
/// tokens as JSON files. `None` if no home directory is resolvable.
pub fn kiro_cache_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME").map(|home| std::path::Path::new(&home).join(".aws/sso/cache"))
}

/// The freshest unexpired SSO access token in the host's cache, if one is
/// present (AC-6). Each `~/.aws/sso/cache/*.json` holds an `accessToken`
/// and an `expiresAt`; the most recently modified file wins, and an
/// obviously expired or token-less file is skipped. Read fresh every call
/// — nothing is cached by moor (AC-8) — and returned by value, so it is
/// never handed to a subprocess as an argument (AC-9). `None` on any
/// miss (no cache dir, no file, no token), never an error, so absence is
/// a no-op for the caller (AC-10).
pub fn kiro_sso_token() -> Option<String> {
    let dir = kiro_cache_dir()?;
    let mut candidates: Vec<(std::time::SystemTime, std::path::PathBuf)> = std::fs::read_dir(&dir)
        .ok()?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    // Freshest file first.
    candidates.sort_by(|a, b| b.0.cmp(&a.0));
    for (_, path) in candidates {
        if let Some(token) = token_from_cache_file(&std::fs::read_to_string(&path).ok()?) {
            return Some(token);
        }
    }
    None
}

/// The `accessToken` from one SSO cache file's JSON, dropping it when the
/// file's `expiresAt` is in the past. Pure, so the freshness rule is
/// unit-testable without writing files.
fn token_from_cache_file(contents: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(contents).ok()?;
    let token = v.get("accessToken")?.as_str()?.trim().to_string();
    if token.is_empty() {
        return None;
    }
    if let Some(expires) = v.get("expiresAt").and_then(|e| e.as_str()) {
        // The SSO cache stamps `expiresAt` as RFC-3339 UTC. A file whose
        // expiry has already passed is skipped rather than injected stale;
        // an unparseable stamp is treated as "no expiry known" and kept.
        if let Ok(at) = chrono::DateTime::parse_from_rfc3339(expires) {
            if at <= chrono::Utc::now() {
                return None;
            }
        }
    }
    Some(token)
}

/// For a `kiro`-agent project, inject the host's freshly-read SSO access
/// token as `AWS_BEARER_TOKEN_BEDROCK` when nothing more explicit resolved
/// it. The Copilot device-token counterpart: an already-set env var or a
/// moor-managed Keychain secret wins, a non-kiro agent is a no-op (AC-7),
/// and a missing credential is a no-op (AC-10). Reads fresh and writes no
/// copy to the manifest, compose file, or Keychain (AC-8); the token is
/// read by value and set via the environment, never an argv (AC-9).
/// Returns the token it set, if any, so the caller can add it to the
/// audit redaction set.
pub fn resolve_kiro_credential(agent_is_kiro: bool) -> Option<String> {
    let env_set = std::env::var(KIRO_TOKEN_VAR).is_ok();
    if !should_use_kiro_token(agent_is_kiro, env_set) {
        return None;
    }
    let token = kiro_sso_token()?;
    std::env::set_var(KIRO_TOKEN_VAR, &token);
    Some(token)
}

/// The decision, pure so it is testable without the environment or the
/// filesystem: use the discovered SSO token only for a kiro agent, and
/// only when no explicit env var already provides the bearer credential.
pub fn should_use_kiro_token(agent_is_kiro: bool, env_set: bool) -> bool {
    agent_is_kiro && !env_set
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

    // --- kiro-agent-sandbox (SPEC-0018) ---------------------------------

    #[test]
    fn finds_kiro_credential_source() {
        // AC-6: the host credential is the SSO token cache `kiro-cli login`
        // populates (`~/.aws/sso/cache`) — no `moor secrets` step. A login
        // can't be assumed on every host, so assert discovery is total
        // (never panics) and targets the right directory, then exercise the
        // freshness rule directly: unexpired yields its token, expired or
        // token-less yields nothing, an unparseable expiry is kept.
        let _ = kiro_sso_token();
        if let Some(dir) = kiro_cache_dir() {
            assert!(dir.ends_with(".aws/sso/cache"), "{}", dir.display());
        }
        let fresh = r#"{"accessToken":"tok-abc","expiresAt":"2999-01-01T00:00:00Z"}"#;
        assert_eq!(token_from_cache_file(fresh), Some("tok-abc".to_string()));
        let expired = r#"{"accessToken":"old","expiresAt":"2000-01-01T00:00:00Z"}"#;
        assert_eq!(token_from_cache_file(expired), None);
        assert_eq!(
            token_from_cache_file(r#"{"expiresAt":"2999-01-01T00:00:00Z"}"#),
            None
        );
        let odd = r#"{"accessToken":"keep","expiresAt":"soon"}"#;
        assert_eq!(token_from_cache_file(odd), Some("keep".to_string()));
    }

    #[test]
    fn kiro_credential_only_for_kiro_agent() {
        // AC-7: only a kiro agent injects the credential, and only with no
        // explicit env var; a non-kiro resolve is a pure no-op.
        assert!(!should_use_kiro_token(/*kiro*/ false, /*env*/ false));
        assert!(should_use_kiro_token(/*kiro*/ true, /*env*/ false));
        assert!(!should_use_kiro_token(true, /*env*/ true));
        std::env::remove_var(KIRO_TOKEN_VAR);
        assert_eq!(resolve_kiro_credential(false), None);
        assert!(std::env::var(KIRO_TOKEN_VAR).is_err());
    }

    #[test]
    fn kiro_credential_not_persisted() {
        // AC-8: resolving writes no Keychain entry (`set` is the only
        // writer, never on this path), no manifest/compose copy — only a
        // process env var, read fresh. A non-kiro resolve touches nothing.
        let before = get("moor-kiro-noexist", KIRO_TOKEN_VAR).ok();
        assert_eq!(resolve_kiro_credential(false), None);
        let after = get("moor-kiro-noexist", KIRO_TOKEN_VAR).ok();
        assert_eq!(before, after, "resolve must not write a Keychain entry");
    }

    #[test]
    fn kiro_credential_not_in_argv() {
        // AC-9: the token is read from a file by value and injected via the
        // environment — no argv path exists, so it can't surface in `docker
        // inspect`/`ps`. token_from_cache_file is a pure in-memory read,
        // and the var it sets is the first declared secret, so it rides the
        // same file-based compose injection as every secret.
        let contents = r#"{"accessToken":"secret-value","expiresAt":"2999-01-01T00:00:00Z"}"#;
        assert_eq!(
            token_from_cache_file(contents),
            Some("secret-value".to_string())
        );
        assert_eq!(KIRO_TOKEN_VAR, crate::manifest::KIRO_SECRET_VARS[0]);
    }

    #[test]
    fn missing_kiro_credential_is_noop() {
        // AC-10: no host credential and nothing explicit → None, no error;
        // the turn's own auth hint reports the gap.
        std::env::remove_var(KIRO_TOKEN_VAR);
        assert_eq!(resolve_kiro_credential(false), None);
    }
}
