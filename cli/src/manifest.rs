use crate::canary;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub name: String,
    pub image: String,
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
