//! MCP server exposing keel's *verification* verbs to the agent running
//! inside a moor sandbox.
//!
//! This exists because of what it leaves out. An earlier design gave the
//! agent a scoped shell (`--allowedTools "Bash(keel gate *)"`) plus a
//! moor-maintained allow/deny list of keel verbs — exactly the
//! configuration-based control ADR-0003 rejected, and it rotted
//! immediately (the real syntax is `Bash(git *)`, with a space, not the
//! colon form that draft assumed). Here `keel approve` is not a tool at
//! all: advancing a spec past a human checkpoint is *absent* from the
//! agent's world rather than denied by a rule someone has to keep
//! current. See docs/decisions/0008-agent-session-protocol.md.
//!
//! Hand-rolled JSON-RPC 2.0 over newline-delimited stdio, serde_json
//! only. No listening socket, nothing crossing a network — the same
//! subprocess-and-pipes pattern the rest of moor uses.

use std::io::{BufRead, Write};

/// MCP revision this server speaks. Claude Code negotiates by echoing
/// back what it can use; we answer with the one shape we implement.
const PROTOCOL_VERSION: &str = "2024-11-05";
const SERVER_NAME: &str = "moor-keel";

/// The tool surface, and the only place a caller-supplied string is ever
/// allowed near a child process's argv.
mod tools {
    use serde_json::{json, Value};

    /// One tool. `verb` is the fixed, compiled-in head of the argv — a
    /// caller can never contribute to it, so no tool call can name a keel
    /// verb this table doesn't already list.
    pub struct Tool {
        pub name: &'static str,
        pub description: &'static str,
        pub verb: &'static [&'static str],
        /// Whether the call also takes a gate id (from `GATES` only).
        pub takes_gate: bool,
    }

    /// Verification only: "did what I just build hold up?" and "what does
    /// the pipeline want next?". Nothing here advances a stage.
    pub const TOOLS: &[Tool] = &[
        Tool {
            name: "keel_gate",
            description: "Run one keel gate against a spec and return its verdict. \
                          Verification only — it records a gate result, it cannot \
                          advance the spec past a human checkpoint.",
            verb: &["keel", "gate"],
            takes_gate: true,
        },
        Tool {
            name: "keel_next",
            description: "Ask keel what the next step is for a spec, as JSON.",
            verb: &["keel", "next", "--json"],
            takes_gate: false,
        },
    ];

    /// keel's own gate ids, as `keel gate --help` lists them. A gate id
    /// arrives from the caller but is only ever *matched* against this
    /// table — the string that reaches the argv is the `'static` one from
    /// here, never the caller's bytes.
    pub const GATES: &[&str] = &["g0", "g1", "g2", "g4"];

    pub fn find(name: &str) -> Option<&'static Tool> {
        TOOLS.iter().find(|t| t.name == name)
    }

    /// The `tools/list` result.
    pub fn list_json() -> Value {
        let listed: Vec<Value> = TOOLS
            .iter()
            .map(|t| {
                let mut props = json!({
                    "slug": {
                        "type": "string",
                        "description": "Spec slug: lowercase letters, digits and dashes, starting with a letter.",
                    }
                });
                let mut required = vec![json!("slug")];
                if t.takes_gate {
                    props["gate"] = json!({
                        "type": "string",
                        "enum": GATES,
                        "description": "Which gate to run.",
                    });
                    required.push(json!("gate"));
                }
                json!({
                    "name": t.name,
                    "description": t.description,
                    "inputSchema": {
                        "type": "object",
                        "properties": props,
                        "required": required,
                        "additionalProperties": false,
                    },
                })
            })
            .collect();
        json!({ "tools": listed })
    }

    /// The same rule `manifest::validate_name` applies in the CLI
    /// (cli/src/manifest.rs) — deliberately duplicated rather than shared,
    /// because this binary ships into the sandbox and must not depend on
    /// the host-side crate. Kept in step by
    /// `slug_rule_matches_manifest_validate_name`.
    pub fn validate_slug(slug: &str) -> Result<(), String> {
        let ok = !slug.is_empty()
            && slug.len() <= 63
            && slug
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
            && slug.chars().next().is_some_and(|c| c.is_ascii_alphabetic());
        if ok {
            Ok(())
        } else {
            Err(format!(
                "invalid spec slug '{slug}': use lowercase letters, digits and dashes, starting with a letter"
            ))
        }
    }

    /// Build the child argv for a tool call. Every element is either
    /// compiled in or a slug that passed `validate_slug`; an unexpected
    /// argument key is a hard error rather than something ignored, so a
    /// call can't carry a second verb or a shell fragment alongside a
    /// valid slug and have it quietly survive.
    pub fn build_argv(name: &str, args: &Value) -> Result<Vec<String>, String> {
        let tool = find(name).ok_or_else(|| format!("unknown tool '{name}'"))?;

        let obj = match args {
            Value::Object(map) => map.clone(),
            Value::Null => serde_json::Map::new(),
            _ => return Err("arguments must be an object".to_string()),
        };
        let allowed: &[&str] = if tool.takes_gate {
            &["slug", "gate"]
        } else {
            &["slug"]
        };
        for key in obj.keys() {
            if !allowed.contains(&key.as_str()) {
                return Err(format!("unexpected argument '{key}' for tool '{name}'"));
            }
        }

        let slug = obj
            .get("slug")
            .and_then(Value::as_str)
            .ok_or_else(|| "missing required string argument 'slug'".to_string())?;
        validate_slug(slug)?;

        let mut argv: Vec<String> = tool.verb.iter().map(|s| s.to_string()).collect();
        if tool.takes_gate {
            let asked = obj
                .get("gate")
                .and_then(Value::as_str)
                .ok_or_else(|| "missing required string argument 'gate'".to_string())?;
            let known = GATES
                .iter()
                .find(|g| **g == asked)
                .ok_or_else(|| format!("unknown gate '{asked}': expected one of {GATES:?}"))?;
            argv.push((*known).to_string());
        }
        argv.push(slug.to_string());
        Ok(argv)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        /// AC-1: the surface covers `gate` and `next` and defines no
        /// advancement verb. Checked three ways, because "we just didn't
        /// add it" is not a control: the names, the compiled-in argv
        /// heads, and every argv any accepted call can produce.
        #[test]
        fn surface_is_verification_only() {
            let names: Vec<&str> = TOOLS.iter().map(|t| t.name).collect();
            assert!(names.contains(&"keel_gate"), "gate must be reachable");
            assert!(names.contains(&"keel_next"), "next must be reachable");

            for t in TOOLS {
                assert_eq!(t.verb.first(), Some(&"keel"));
                let verb = t.verb.join(" ");
                for forbidden in ["approve", "approvals"] {
                    assert!(
                        !t.verb.contains(&forbidden),
                        "tool '{}' invokes '{forbidden}' ({verb})",
                        t.name
                    );
                }
            }

            // Nothing a caller can say reaches an advancement verb: try
            // every tool with every gate id and a valid slug.
            for t in TOOLS {
                let mut cases = vec![serde_json::json!({"slug": "some-spec"})];
                for g in GATES {
                    cases.push(serde_json::json!({"slug": "some-spec", "gate": g}));
                }
                for args in cases {
                    if let Ok(argv) = build_argv(t.name, &args) {
                        assert!(
                            !argv.iter().any(|a| a == "approve"),
                            "argv reached an advancement verb: {argv:?}"
                        );
                    }
                }
            }

            // And it is absent from the advertised list the agent reads.
            let listed = list_json().to_string();
            assert!(!listed.contains("approve"), "tools/list mentions approve");
        }

        /// AC-2: only a validated slug crosses into the argv.
        #[test]
        fn rejects_unvalidated_arguments() {
            // A slug that would smuggle a second verb, a flag, or a shell
            // fragment. (Nothing here would be interpreted as a shell
            // fragment anyway — there is no shell — but it must not reach
            // keel's own argument parser either.)
            for bad in [
                "spec; keel approve spec",
                "spec approve",
                "--json",
                "-p",
                "../../etc/passwd",
                "spec$(id)",
                "spec`id`",
                "Spec",
                "spec_slug",
                "1spec",
                "",
                "..",
                "spec spec",
                "spec\nkeel approve spec",
            ] {
                let args = serde_json::json!({"slug": bad, "gate": "g0"});
                assert!(
                    build_argv("keel_gate", &args).is_err(),
                    "slug '{bad}' should have been rejected"
                );
            }

            // A gate id is matched against a fixed table, not passed through.
            for bad_gate in ["g0 --force", "approve", "g5", "G0", ""] {
                let args = serde_json::json!({"slug": "valid-spec", "gate": bad_gate});
                assert!(
                    build_argv("keel_gate", &args).is_err(),
                    "gate '{bad_gate}' should have been rejected"
                );
            }

            // No other caller-supplied string is carried at all.
            let smuggled = serde_json::json!({
                "slug": "valid-spec",
                "gate": "g0",
                "extra": "--dangerously-skip-permissions",
            });
            assert!(build_argv("keel_gate", &smuggled).is_err());
            assert!(build_argv(
                "keel_next",
                &serde_json::json!({"slug": "valid-spec", "json": true})
            )
            .is_err());
            assert!(build_argv("keel_next", &serde_json::json!({"slug": 7})).is_err());
            assert!(build_argv("keel_next", &serde_json::json!("valid-spec")).is_err());
            assert!(
                build_argv("keel_approve", &serde_json::json!({"slug": "valid-spec"})).is_err()
            );

            // The happy path is exactly the fixed verb plus the slug.
            assert_eq!(
                build_argv(
                    "keel_gate",
                    &serde_json::json!({"slug": "valid-spec", "gate": "g1"})
                )
                .unwrap(),
                vec!["keel", "gate", "g1", "valid-spec"]
            );
            assert_eq!(
                build_argv("keel_next", &serde_json::json!({"slug": "valid-spec"})).unwrap(),
                vec!["keel", "next", "--json", "valid-spec"]
            );
        }

        /// The duplicated rule above must stay identical to the host's
        /// `manifest::validate_name`, so a slug moor accepts is a slug
        /// this server accepts and vice versa.
        #[test]
        fn slug_rule_matches_manifest_validate_name() {
            for valid in ["a", "sample-app", "my-project-2", "x1", &"a".repeat(63)] {
                assert!(validate_slug(valid).is_ok(), "'{valid}' should be valid");
            }
            for invalid in [
                "",
                "1app",
                "-app",
                "MyApp",
                "my_app",
                "my app",
                "my/app",
                "..",
                "a.b",
                &"a".repeat(64),
            ] {
                assert!(
                    validate_slug(invalid).is_err(),
                    "'{invalid}' should be invalid"
                );
            }
        }

        #[test]
        fn every_listed_tool_advertises_its_arguments() {
            let listed = list_json();
            let arr = listed["tools"].as_array().unwrap();
            assert_eq!(arr.len(), TOOLS.len());
            for t in arr {
                let schema = &t["inputSchema"];
                assert_eq!(schema["additionalProperties"], serde_json::json!(false));
                assert!(schema["properties"]["slug"].is_object());
            }
            let gate = arr.iter().find(|t| t["name"] == "keel_gate").unwrap();
            assert_eq!(
                gate["inputSchema"]["properties"]["gate"]["enum"],
                serde_json::json!(GATES)
            );
        }
    }
}

/// Run a keel argv and return its output as MCP tool content. stdout and
/// stderr are concatenated for the same reason
/// `proc::run_capture_combined` does it host-side: keel puts gate results
/// on stdout but a hard refusal on stderr, and the agent needs both.
fn call_keel(argv: &[String]) -> (bool, String) {
    let (head, rest) = argv.split_first().expect("argv is never empty");
    match std::process::Command::new(head)
        .args(rest)
        .stdin(std::process::Stdio::null())
        .output()
    {
        Ok(out) => {
            let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
            text.push_str(&String::from_utf8_lossy(&out.stderr));
            (out.status.success(), text)
        }
        Err(e) => (false, format!("could not run `{}`: {e}", argv.join(" "))),
    }
}

fn content(text: String, is_error: bool) -> serde_json::Value {
    serde_json::json!({
        "content": [{ "type": "text", "text": text }],
        "isError": is_error,
    })
}

fn handle(method: &str, params: &serde_json::Value) -> Result<serde_json::Value, (i64, String)> {
    match method {
        "initialize" => Ok(serde_json::json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": { "tools": { "listChanged": false } },
            "serverInfo": { "name": SERVER_NAME, "version": env!("CARGO_PKG_VERSION") },
        })),
        "tools/list" => Ok(tools::list_json()),
        "tools/call" => {
            let name = params
                .get("name")
                .and_then(serde_json::Value::as_str)
                .ok_or((-32602, "tools/call needs a string 'name'".to_string()))?;
            let args = params
                .get("arguments")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            // A rejected argument is reported as tool content, not as a
            // JSON-RPC error: the agent should read why and correct
            // itself, the way it reads a failing gate.
            match tools::build_argv(name, &args) {
                Ok(argv) => {
                    let (ok, text) = call_keel(&argv);
                    Ok(content(text, !ok))
                }
                Err(why) => Ok(content(why, true)),
            }
        }
        "ping" => Ok(serde_json::json!({})),
        other => Err((-32601, format!("method '{other}' is not implemented"))),
    }
}

fn main() {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();

    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let Ok(msg) = serde_json::from_str::<serde_json::Value>(&line) else {
            // Unparseable input has no id to answer against; JSON-RPC's
            // own advice is a null-id error, which Claude Code ignores.
            continue;
        };
        // No `id` means a notification (e.g. notifications/initialized):
        // handle nothing, answer nothing.
        let Some(id) = msg.get("id").cloned() else {
            continue;
        };
        let method = msg
            .get("method")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let params = msg
            .get("params")
            .cloned()
            .unwrap_or(serde_json::Value::Null);

        let response = match handle(method, &params) {
            Ok(result) => serde_json::json!({"jsonrpc": "2.0", "id": id, "result": result}),
            Err((code, message)) => {
                serde_json::json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
            }
        };
        if writeln!(stdout, "{response}").is_err() || stdout.flush().is_err() {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initialize_reports_tool_capability() {
        let r = handle("initialize", &serde_json::Value::Null).unwrap();
        assert_eq!(r["protocolVersion"], PROTOCOL_VERSION);
        assert_eq!(r["serverInfo"]["name"], SERVER_NAME);
        assert!(r["capabilities"]["tools"].is_object());
    }

    #[test]
    fn unknown_method_is_a_jsonrpc_error() {
        let (code, _) = handle("tools/unlock", &serde_json::Value::Null).unwrap_err();
        assert_eq!(code, -32601);
    }

    #[test]
    fn a_rejected_call_comes_back_as_error_content_not_a_transport_error() {
        // The agent has to be able to read and act on "that slug is
        // invalid"; a JSON-RPC error would just look like a broken server.
        let params = serde_json::json!({
            "name": "keel_gate",
            "arguments": {"slug": "Bad Slug", "gate": "g0"},
        });
        let r = handle("tools/call", &params).unwrap();
        assert_eq!(r["isError"], serde_json::json!(true));
        assert!(r["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("invalid spec slug"));
    }

    #[test]
    fn calling_a_tool_that_does_not_exist_says_so() {
        let params = serde_json::json!({"name": "keel_approve", "arguments": {"slug": "x"}});
        let r = handle("tools/call", &params).unwrap();
        assert_eq!(r["isError"], serde_json::json!(true));
        assert!(r["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("unknown tool"));
    }
}
