use crate::canary;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::str::FromStr;

/// Which agent a project's sandbox is set up for. Orthogonal to the
/// *language* image: Claude ships in every image, Copilot only in the
/// opt-in `moor/copilot*` layer. `resolve_image` composes the two.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Agent {
    #[default]
    Claude,
    Copilot,
    Kiro,
}

impl FromStr for Agent {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "claude" => Ok(Agent::Claude),
            "copilot" => Ok(Agent::Copilot),
            "kiro" => Ok(Agent::Kiro),
            other => {
                anyhow::bail!("unknown agent '{other}' — expected 'claude', 'copilot' or 'kiro'")
            }
        }
    }
}

/// Compose a language image with the chosen agent. Claude leaves the
/// language image unchanged (it is in every image); Copilot maps to the
/// matching opt-in layer: `moor/base` → `moor/copilot`, and
/// `moor/<lang>` → `moor/copilot-<lang>`. A non-`moor/` image, or one
/// already a copilot image, is returned unchanged. Pure — unit-tested
/// without Docker.
pub fn resolve_image(language_image: &str, agent: Agent) -> String {
    if agent == Agent::Claude {
        return language_image.to_string();
    }
    // Copilot. Split "moor/<name>:<tag>" into its parts; anything that
    // isn't a moor image, or is already a copilot image, is left as-is.
    let (repo, tag) = match language_image.split_once(':') {
        Some((r, t)) => (r, t),
        None => (language_image, "latest"),
    };
    let Some(name) = repo.strip_prefix("moor/") else {
        return language_image.to_string();
    };
    if name == "copilot" || name.starts_with("copilot-") {
        return language_image.to_string();
    }
    if name == "base" {
        format!("moor/copilot:{tag}")
    } else {
        format!("moor/copilot-{name}:{tag}")
    }
}

/// Whether an image is one of the opt-in Copilot layers (`moor/copilot`
/// or `moor/copilot-<lang>`). This is the single place that recognises a
/// Copilot image: `resolve_image` treats such an image as already-composed
/// and leaves it unchanged, and `Manifest::agent` uses it to reconcile a
/// manifest whose `agent` field is missing or disagrees with its image.
pub fn image_is_copilot(image: &str) -> bool {
    let repo = image.split_once(':').map(|(r, _)| r).unwrap_or(image);
    let Some(name) = repo.strip_prefix("moor/") else {
        return false;
    };
    name == "copilot" || name.starts_with("copilot-")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub name: String,
    pub image: String,
    /// The agent the sandbox is set up for; see `Agent`. Defaults to
    /// Claude so manifests written before this field still load.
    #[serde(default)]
    pub agent: Agent,
    #[serde(default)]
    pub github_repo: Option<String>,
    #[serde(default)]
    pub egress: Egress,
    #[serde(default = "Resources::default")]
    pub resources: Resources,
    #[serde(default)]
    pub secrets: Vec<String>,
    /// Generated once at `new` time, never user-edited. Injected into the
    /// sandbox as MOOR_CANARY_TOKEN — see canary.rs.
    #[serde(default = "canary::generate_token")]
    pub canary_token: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Egress {
    #[serde(default)]
    pub allow: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Resources {
    pub cpu: String,
    pub mem: String,
    pub pids: u32,
}

impl Default for Resources {
    fn default() -> Self {
        Resources {
            cpu: "2.0".to_string(),
            mem: "4g".to_string(),
            pids: 512,
        }
    }
}

impl Manifest {
    pub fn new(name: &str, image: &str) -> Self {
        Manifest {
            name: name.to_string(),
            image: image.to_string(),
            agent: Agent::default(),
            github_repo: None,
            egress: Egress::default(),
            resources: Resources::default(),
            // Either credential authenticates Claude Code inside the
            // sandbox — CLAUDE_CODE_OAUTH_TOKEN (from `claude setup-token`
            // on the host, for a claude.ai subscription) takes priority
            // over ANTHROPIC_API_KEY when both are set; that precedence is
            // the `claude` CLI's own behavior, not moor's.
            //
            // COPILOT_GITHUB_TOKEN / GH_TOKEN authenticate the GitHub
            // Copilot CLI when a build runs with `keel run --driver
            // copilot` inside the sandbox. The `copilot` CLI reads them in
            // the order COPILOT_GITHUB_TOKEN > GH_TOKEN > GITHUB_TOKEN, so
            // GITHUB_TOKEN (already listed, for git push/pull) is the
            // lowest-priority fallback and the two dedicated names are
            // added ahead of it. Only whichever the operator actually
            // stores gets injected — an unset secret resolves to empty and
            // is a no-op (see secrets::resolve_into_env).
            secrets: vec![
                "ANTHROPIC_API_KEY".to_string(),
                "CLAUDE_CODE_OAUTH_TOKEN".to_string(),
                "COPILOT_GITHUB_TOKEN".to_string(),
                "GH_TOKEN".to_string(),
                "GITHUB_TOKEN".to_string(),
            ],
            canary_token: canary::generate_token(),
        }
    }

    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading manifest {}", path.display()))?;
        serde_yaml::from_str(&text).with_context(|| format!("parsing manifest {}", path.display()))
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let text = serde_yaml::to_string(self)?;
        std::fs::write(path, text)
            .with_context(|| format!("writing manifest {}", path.display()))?;
        Ok(())
    }

    /// The agent a turn should actually run, reconciled against the image.
    ///
    /// The `agent` field defaults to `Claude` so manifests written before
    /// it existed still load — but a sandbox built from a `moor/copilot*`
    /// image runs no `claude` CLI, so defaulting such a manifest to Claude
    /// sends every turn to a binary that cannot authenticate (the symptom
    /// is Claude Code's own misleading `Not logged in · /login`). When the
    /// image is a Copilot image, the agent is Copilot regardless of a
    /// missing or stale field. All turn/credential logic must read the
    /// agent through here, never the raw field.
    pub fn agent(&self) -> Agent {
        if self.agent == Agent::Claude && image_is_copilot(&self.image) {
            return Agent::Copilot;
        }
        self.agent
    }

    pub fn sandbox_container(&self) -> String {
        format!("{}-sandbox", self.name)
    }

    pub fn egress_container(&self) -> String {
        format!("{}-egress", self.name)
    }
}

/// Validate a project name: lowercase alnum + dashes, matches what's safe
/// to use as a Docker container/network/volume name and a directory name.
pub fn validate_name(name: &str) -> Result<()> {
    let ok = !name.is_empty()
        && name.len() <= 63
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && name.chars().next().is_some_and(|c| c.is_ascii_alphabetic());
    if !ok {
        anyhow::bail!(
            "invalid project name '{name}': use lowercase letters, digits, and dashes, starting with a letter"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_valid_names() {
        for name in ["a", "sample-app", "my-project-2", "x1"] {
            assert!(validate_name(name).is_ok(), "expected '{name}' to be valid");
        }
    }

    // --- agent-selection (SPEC-0010) ------------------------------------

    #[test]
    fn agent_defaults_to_claude() {
        // AC-2: no --agent given → Claude, and a manifest written without
        // an `agent:` field still loads as Claude (serde default).
        assert_eq!(Agent::default(), Agent::Claude);
        let m = Manifest::new("sample", "moor/base:latest");
        assert_eq!(m.agent, Agent::Claude);
        let no_agent_yaml = "name: x\nimage: moor/base:latest\n";
        let loaded: Manifest = serde_yaml::from_str(no_agent_yaml).unwrap();
        assert_eq!(loaded.agent, Agent::Claude);
    }

    #[test]
    fn image_is_copilot_recognises_the_opt_in_layers() {
        for img in [
            "moor/copilot:latest",
            "moor/copilot-python:latest",
            "moor/copilot-node:1.2",
            "moor/copilot", // no tag
        ] {
            assert!(image_is_copilot(img), "expected copilot: {img}");
        }
        for img in [
            "moor/base:latest",
            "moor/python:latest",
            "moor/rust:latest",
            "ubuntu:22.04",
            "copilot:latest", // not a moor/ image
        ] {
            assert!(!image_is_copilot(img), "expected not copilot: {img}");
        }
    }

    #[test]
    fn agent_reconciles_a_copilot_image_with_a_missing_field() {
        // The real-world gap: a manifest built for Copilot (its image is a
        // moor/copilot* layer) but whose `agent:` field is absent, so the
        // serde default makes the raw field Claude. Running the `claude`
        // CLI in that sandbox only ever yields "Not logged in". The
        // reconciled accessor must report Copilot so the right CLI runs.
        let yaml = "name: sc\nimage: moor/copilot-python:latest\n";
        let m: Manifest = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(m.agent, Agent::Claude, "raw field still the serde default");
        assert_eq!(m.agent(), Agent::Copilot, "reconciled against the image");
    }

    #[test]
    fn agent_leaves_non_copilot_images_on_the_stored_value() {
        // A Claude image stays Claude; an explicit agent is never
        // downgraded or otherwise second-guessed by the image.
        let claude = Manifest::new("a", "moor/python:latest");
        assert_eq!(claude.agent(), Agent::Claude);

        let mut explicit_copilot = Manifest::new("b", "moor/copilot-node:latest");
        explicit_copilot.agent = Agent::Copilot;
        assert_eq!(explicit_copilot.agent(), Agent::Copilot);

        // An explicit non-default on a plain image is honoured as-is.
        let mut kiro = Manifest::new("c", "moor/base:latest");
        kiro.agent = Agent::Kiro;
        assert_eq!(kiro.agent(), Agent::Kiro);
    }

    #[test]
    fn agent_flag_sets_manifest_agent() {
        // AC-1: the value a `--agent` flag parses to is what the manifest
        // records, and it round-trips through YAML.
        let mut m = Manifest::new("sample", "moor/python:latest");
        m.agent = "copilot".parse().unwrap();
        assert_eq!(m.agent, Agent::Copilot);
        let yaml = serde_yaml::to_string(&m).unwrap();
        assert!(yaml.contains("agent: copilot"), "{yaml}");
        let back: Manifest = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(back.agent, Agent::Copilot);
    }

    #[test]
    fn unknown_agent_is_rejected() {
        // AC-5: anything but claude/copilot errors, not silently defaults.
        assert!("claude".parse::<Agent>().is_ok());
        assert!("copilot".parse::<Agent>().is_ok());
        assert!("Copilot".parse::<Agent>().is_ok()); // case-insensitive
        for bad in ["gpt", "", "claude-code", "none"] {
            assert!(bad.parse::<Agent>().is_err(), "expected '{bad}' rejected");
        }
    }

    #[test]
    fn copilot_composes_with_language() {
        // AC-3: copilot + language image → the matching copilot layer.
        assert_eq!(
            resolve_image("moor/python:latest", Agent::Copilot),
            "moor/copilot-python:latest"
        );
        assert_eq!(
            resolve_image("moor/node:latest", Agent::Copilot),
            "moor/copilot-node:latest"
        );
        assert_eq!(
            resolve_image("moor/rust:latest", Agent::Copilot),
            "moor/copilot-rust:latest"
        );
        assert_eq!(
            resolve_image("moor/base:latest", Agent::Copilot),
            "moor/copilot:latest"
        );
        // Already a copilot image, or non-moor image: unchanged.
        assert_eq!(
            resolve_image("moor/copilot-python:latest", Agent::Copilot),
            "moor/copilot-python:latest"
        );
        assert_eq!(
            resolve_image("ubuntu:22.04", Agent::Copilot),
            "ubuntu:22.04"
        );
    }

    #[test]
    fn claude_leaves_image_unchanged() {
        // AC-4: claude returns the language image verbatim.
        for img in [
            "moor/base:latest",
            "moor/python:latest",
            "moor/node:latest",
            "moor/rust:latest",
        ] {
            assert_eq!(resolve_image(img, Agent::Claude), img);
        }
    }

    #[test]
    fn rejects_empty() {
        assert!(validate_name("").is_err());
    }

    #[test]
    fn rejects_leading_digit_or_dash() {
        assert!(validate_name("1app").is_err());
        assert!(validate_name("-app").is_err());
    }

    #[test]
    fn rejects_uppercase_and_symbols() {
        assert!(validate_name("MyApp").is_err());
        assert!(validate_name("my_app").is_err());
        assert!(validate_name("my app").is_err());
        assert!(validate_name("my/app").is_err());
    }

    #[test]
    fn rejects_dots() {
        // Dots aren't in the allowed charset at all — this also guards
        // against a name like ".." breaking the project directory path.
        assert!(validate_name("..").is_err());
        assert!(validate_name("a.b").is_err());
    }

    #[test]
    fn rejects_over_63_chars() {
        let long = "a".repeat(64);
        assert!(validate_name(&long).is_err());
        let ok_len = "a".repeat(63);
        assert!(validate_name(&ok_len).is_ok());
    }

    #[test]
    fn default_manifest_has_sane_resource_limits() {
        let m = Manifest::new("sample", "moor/base:latest");
        assert_eq!(m.resources.pids, 512);
        assert!(m.secrets.contains(&"ANTHROPIC_API_KEY".to_string()));
        assert!(m.secrets.contains(&"CLAUDE_CODE_OAUTH_TOKEN".to_string()));
        // Copilot driver auth: the two dedicated names plus GITHUB_TOKEN
        // as the documented lowest-priority fallback.
        assert!(m.secrets.contains(&"COPILOT_GITHUB_TOKEN".to_string()));
        assert!(m.secrets.contains(&"GH_TOKEN".to_string()));
        assert!(m.secrets.contains(&"GITHUB_TOKEN".to_string()));
        assert!(m.egress.allow.is_empty());
    }

    #[test]
    fn manifest_round_trips_through_yaml() {
        let m = Manifest::new("sample", "moor/node:latest");
        let text = serde_yaml::to_string(&m).unwrap();
        let back: Manifest = serde_yaml::from_str(&text).unwrap();
        assert_eq!(back.name, m.name);
        assert_eq!(back.image, m.image);
        assert_eq!(back.resources.pids, m.resources.pids);
    }
}
