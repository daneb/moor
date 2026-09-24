use crate::manifest::Manifest;

const TEMPLATE: &str = include_str!("../templates/project.compose.yml.tmpl");

/// Render the per-project docker-compose file from the manifest. Pure
/// string substitution — no templating engine — so
/// `cli/templates/project.compose.yml.tmpl` stays the readable ground
/// truth for exactly what gets applied. Lives inside the crate directory
/// (not a top-level `compose/`, which it did before ADR-0004) because
/// `include_str!` can't reach outside the package a published crate
/// actually ships.
pub fn render(m: &Manifest) -> String {
    let secret_env_lines = if m.secrets.is_empty() {
        String::new()
    } else {
        let mut lines = String::new();
        for s in &m.secrets {
            lines.push_str(&format!("      {s}: \"${{{s}:-}}\"\n"));
        }
        // drop the trailing newline; the template line already provides one
        lines.trim_end_matches('\n').to_string()
    };

    let extra_allow = m.egress.allow.join(",");

    TEMPLATE
        .replace("{{PROJECT_NAME}}", &m.name)
        .replace("{{IMAGE}}", &m.image)
        .replace("{{CPU_LIMIT}}", &m.resources.cpu)
        .replace("{{MEM_LIMIT}}", &m.resources.mem)
        .replace("{{PIDS_LIMIT}}", &m.resources.pids.to_string())
        .replace("{{SECRET_ENV_LINES}}", &secret_env_lines)
        .replace("{{EXTRA_ALLOW_DOMAINS}}", &extra_allow)
        .replace("{{CANARY_TOKEN}}", &m.canary_token)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::Manifest;

    #[test]
    fn no_known_placeholder_survives_rendering() {
        // Checked against the specific tokens `render()` substitutes, not
        // a blanket "{{" scan — the template's own header comment uses
        // that literal string to explain the mechanism.
        let m = Manifest::new("sample-app", "moor/node:latest");
        let rendered = render(&m);
        for token in [
            "{{PROJECT_NAME}}",
            "{{IMAGE}}",
            "{{CPU_LIMIT}}",
            "{{MEM_LIMIT}}",
            "{{PIDS_LIMIT}}",
            "{{SECRET_ENV_LINES}}",
            "{{EXTRA_ALLOW_DOMAINS}}",
            "{{CANARY_TOKEN}}",
        ] {
            assert!(
                !rendered.contains(token),
                "unreplaced placeholder {token} left in rendered compose:\n{rendered}"
            );
        }
    }

    #[test]
    fn project_name_is_substituted_everywhere_it_appears() {
        let m = Manifest::new("sample-app", "moor/node:latest");
        let rendered = render(&m);
        assert!(rendered.contains("sample-app-sandbox"));
        assert!(rendered.contains("sample-app-egress"));
        assert!(rendered.contains("sample-app-internal"));
        assert!(rendered.contains("sample-app-workspace"));
    }

    #[test]
    fn secrets_become_compose_default_empty_env_lines() {
        let mut m = Manifest::new("sample-app", "moor/node:latest");
        m.secrets = vec![
            "ANTHROPIC_API_KEY".to_string(),
            "CLAUDE_CODE_OAUTH_TOKEN".to_string(),
            "GITHUB_TOKEN".to_string(),
        ];
        let rendered = render(&m);
        assert!(rendered.contains("ANTHROPIC_API_KEY: \"${ANTHROPIC_API_KEY:-}\""));
        assert!(rendered.contains("CLAUDE_CODE_OAUTH_TOKEN: \"${CLAUDE_CODE_OAUTH_TOKEN:-}\""));
        assert!(rendered.contains("GITHUB_TOKEN: \"${GITHUB_TOKEN:-}\""));
    }

    #[test]
    fn no_secrets_still_renders_valid_yaml_shape() {
        let mut m = Manifest::new("sample-app", "moor/node:latest");
        m.secrets = vec![];
        let rendered = render(&m);
        assert!(!rendered.contains("{{SECRET_ENV_LINES}}"));
    }

    #[test]
    fn extra_allow_domains_joined_with_commas() {
        let mut m = Manifest::new("sample-app", "moor/node:latest");
        m.egress.allow = vec![
            "registry.npmjs.org".to_string(),
            "example-registry.dev".to_string(),
        ];
        let rendered = render(&m);
        assert!(
            rendered.contains("EXTRA_ALLOW_DOMAINS: \"registry.npmjs.org,example-registry.dev\"")
        );
    }

    #[test]
    fn sandbox_has_no_bind_mount_volume_entries() {
        // Guards the "no host bind mount, ever" decision at the compose
        // level: every volumes: entry must reference a named volume
        // (declared under top-level `volumes:`), never a host path.
        let m = Manifest::new("sample-app", "moor/base:latest");
        let rendered = render(&m);
        assert!(rendered.contains("workspace:/workspace"));
        assert!(rendered.contains("cache:/home/agent/.cache"));
        assert!(rendered.contains("claude-state:/home/agent/.claude"));
        assert!(!rendered.contains("/Users/"));
        assert!(!rendered.contains("${HOME}"));
    }

    #[test]
    fn sandbox_gets_attestation_and_sink_without_bind_mounts() {
        let m = Manifest::new("sample-app", "moor/base:latest");
        let rendered = render(&m);
        let sandbox = rendered.split("\n  egress:").next().unwrap();
        assert!(sandbox.contains("KEEL_RUNTIME_ATTESTATION: \"/run/moor/posture.json\""));
        assert!(sandbox.contains("KEEL_CHAIN_SINK: \"/run/moor-sink/keel.jsonl\""));
        assert!(sandbox.contains("- /run/moor:mode=0755"));
        assert!(sandbox.contains("- /run/moor-sink:mode=1777"));
        // Both paths are tmpfs; every volume is still a named volume.
        for line in sandbox
            .lines()
            .map(str::trim)
            .filter(|l| l.starts_with("- ") && l.contains(':'))
        {
            let source = line.trim_start_matches("- ").split(':').next().unwrap();
            assert!(
                !source.starts_with('/') || line.contains("mode="),
                "host path mounted: {line}"
            );
            assert!(
                !source.starts_with('.') && !source.starts_with('~') && !source.contains('$'),
                "host path mounted: {line}"
            );
        }
    }

    #[test]
    fn sandbox_network_is_internal_only() {
        let m = Manifest::new("sample-app", "moor/base:latest");
        let rendered = render(&m);
        assert!(rendered.contains("internal: true"));
    }
}
