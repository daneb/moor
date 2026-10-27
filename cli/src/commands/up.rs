use crate::manifest::{Agent, Manifest};
use crate::{paths, secrets};
use anyhow::Result;

pub fn run(name: &str) -> Result<()> {
    if !paths::manifest_path(name)?.exists() {
        anyhow::bail!("no project '{name}' — run `moor new {name}` first");
    }
    // For a copilot-agent project, fall back to the host's `copilot /login`
    // device-flow token (its own Keychain entry) when no explicit
    // COPILOT_GITHUB_TOKEN was set — so logging in once on the host is all
    // the operator does. Resolved into the env before compose_up, whose
    // own resolve_into_env then leaves this already-set var alone.
    let m = Manifest::load(&paths::manifest_path(name)?)?;
    secrets::resolve_copilot_device_token(name, m.agent() == Agent::Copilot);
    // A kiro-agent project reads the host's fresh IAM Identity Center SSO
    // token (from `~/.aws/sso/cache`) into the env, so a single host
    // `kiro-cli login` is all the operator does. Read fresh each up and
    // never persisted; a non-kiro project, or no host credential, is a
    // no-op. compose_up's own resolve_into_env then leaves it set.
    secrets::resolve_kiro_credential(m.agent() == Agent::Kiro);
    super::compose_up(name)?;
    println!("'{name}' is up.\n");
    println!("{}", super::next_cmd::next_hint(name));
    Ok(())
}
