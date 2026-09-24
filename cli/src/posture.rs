//! The sandbox's posture, attested from outside it (`keel.posture/1`).
//!
//! keel inside the sandbox cannot inspect the box it runs in, so moor does
//! it from the host with `docker inspect` and hands keel the result. keel
//! records the claim and blocks a run whose required properties are not
//! `proven` (keel SPEC-0010). Every property here is read from what Docker
//! reports about the running container — never from anything the sandbox
//! says about itself — and anything the output does not establish is
//! `unproven`, not assumed.

use crate::{audit, manifest::Manifest, paths, proc};
use anyhow::{Context, Result};
use serde_json::{json, Value};

pub const POSTURE_SCHEMA: &str = "keel.posture/1";
/// Root-owned tmpfs: the `agent` user can read it, not replace it.
pub const IN_SANDBOX_PATH: &str = "/run/moor/posture.json";

fn verdict(status: &str, evidence: String) -> Value {
    json!({ "status": status, "evidence": evidence })
}

fn unproven(what: &str) -> Value {
    verdict("unproven", format!("docker inspect does not report {what}"))
}

/// A true/false field where `good` is the value that holds.
fn flag(v: &Value, what: &str, good: bool) -> Value {
    match v.as_bool() {
        Some(b) if b == good => verdict("proven", format!("{what} is {b}")),
        Some(b) => verdict("violated", format!("{what} is {b}")),
        None => unproven(what),
    }
}

/// Derive the attestation's properties from `docker inspect <container>`
/// (one object) and `docker network inspect` of each attached network.
/// Pure over JSON, so each rule is tested without a container.
pub fn derive(container: &Value, networks: &[Value]) -> Value {
    let host = &container["HostConfig"];
    let mut p = serde_json::Map::new();

    p.insert(
        "fs.read_only_root".into(),
        flag(&host["ReadonlyRootfs"], "HostConfig.ReadonlyRootfs", true),
    );
    p.insert(
        "privileged.off".into(),
        flag(&host["Privileged"], "HostConfig.Privileged", false),
    );

    p.insert(
        "mounts.no_host_bind".into(),
        match container["Mounts"].as_array() {
            None => unproven("Mounts"),
            Some(mounts) => {
                let binds: Vec<&str> = mounts
                    .iter()
                    .filter(|m| m["Type"] == "bind")
                    .filter_map(|m| m["Destination"].as_str())
                    .collect();
                if binds.is_empty() {
                    verdict(
                        "proven",
                        format!("{} mount(s), none of type bind", mounts.len()),
                    )
                } else {
                    verdict("violated", format!("bind mount(s) at {}", binds.join(", ")))
                }
            }
        },
    );

    p.insert(
        "user.non_root".into(),
        match container["Config"]["User"].as_str() {
            None => unproven("Config.User"),
            Some(u) => {
                let name = u.split(':').next().unwrap_or("");
                if name.is_empty() || name == "root" || name == "0" {
                    verdict("violated", format!("Config.User is {u:?}"))
                } else {
                    verdict("proven", format!("Config.User is {u:?}"))
                }
            }
        },
    );

    let listed =
        |field: &str, wanted: &dyn Fn(&str) -> bool, what: &str| match host[field].as_array() {
            None => unproven(&format!("HostConfig.{field}")),
            Some(items) => {
                let items: Vec<&str> = items.iter().filter_map(Value::as_str).collect();
                let status = if items.iter().any(|i| wanted(i)) {
                    "proven"
                } else {
                    "violated"
                };
                verdict(
                    status,
                    format!("HostConfig.{field} is [{}] ({what})", items.join(", ")),
                )
            }
        };
    p.insert(
        "caps.dropped_all".into(),
        listed("CapDrop", &|c| c.eq_ignore_ascii_case("ALL"), "needs ALL"),
    );
    p.insert(
        "privileges.no_new".into(),
        listed(
            "SecurityOpt",
            &|o| o == "no-new-privileges" || o == "no-new-privileges:true",
            "needs no-new-privileges",
        ),
    );

    p.insert(
        "network.internal_only".into(),
        match container["NetworkSettings"]["Networks"].as_object() {
            None => unproven("NetworkSettings.Networks"),
            Some(attached) if attached.is_empty() => unproven("any attached network"),
            Some(attached) => {
                let mut open = vec![];
                let mut unknown = vec![];
                for name in attached.keys() {
                    match networks
                        .iter()
                        .find(|n| n["Name"] == name.as_str())
                        .map(|n| &n["Internal"])
                    {
                        Some(Value::Bool(true)) => {}
                        Some(Value::Bool(false)) => open.push(name.as_str()),
                        _ => unknown.push(name.as_str()),
                    }
                }
                if !open.is_empty() {
                    verdict("violated", format!("not internal: {}", open.join(", ")))
                } else if !unknown.is_empty() {
                    unproven(&format!("Internal for {}", unknown.join(", ")))
                } else {
                    verdict(
                        "proven",
                        format!(
                            "internal: {}",
                            attached.keys().cloned().collect::<Vec<_>>().join(", ")
                        ),
                    )
                }
            }
        },
    );

    Value::Object(p)
}

fn inspect(args: &[&str]) -> Result<Value> {
    let (status, out) = proc::run_capture("docker", args)?;
    if !status.success() {
        anyhow::bail!("`docker {}` failed: {}", args.join(" "), out.trim());
    }
    let v: Value = serde_json::from_str(&out).context("parsing docker inspect output")?;
    Ok(v.as_array().and_then(|a| a.first()).cloned().unwrap_or(v))
}

/// Inspect the running sandbox, write the attestation into it as root, and
/// record the claim — with the hash of exactly what keel will read — in the
/// host chain.
pub fn attest(name: &str, m: &Manifest) -> Result<()> {
    let sandbox = m.sandbox_container();
    let container = inspect(&["inspect", &sandbox])?;
    let networks: Vec<Value> = container["NetworkSettings"]["Networks"]
        .as_object()
        .map(|nets| {
            nets.keys()
                .filter_map(|n| inspect(&["network", "inspect", n]).ok())
                .collect()
        })
        .unwrap_or_default();

    let doc = json!({
        "schema": POSTURE_SCHEMA,
        "runtime": "moor",
        "image_digest": container["Image"],
        "attested_at": chrono::Utc::now().to_rfc3339(),
        "properties": derive(&container, &networks),
    });
    let text = serde_json::to_string_pretty(&doc)?;
    let host_copy = paths::posture_path(name)?;
    std::fs::write(&host_copy, &text)
        .with_context(|| format!("writing {}", host_copy.display()))?;

    let status = proc::run_with_stdin_file(
        "docker",
        &[
            "exec",
            "-i",
            "-u",
            "root",
            &sandbox,
            "sh",
            "-c",
            &format!("cat > {IN_SANDBOX_PATH} && chmod 0444 {IN_SANDBOX_PATH}"),
        ],
        &host_copy,
    )?;
    proc::require_success("writing the posture attestation into the sandbox", status)?;

    audit::append_chained(
        &paths::chain_log_path(name)?,
        "attest",
        json!({
            "sha256": hex::encode(<sha2::Sha256 as sha2::Digest>::digest(text.as_bytes())),
            "runtime": "moor",
            "image_digest": doc["image_digest"],
            "properties": doc["properties"],
        }),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape of `docker inspect` for a sandbox rendered from moor's own
    /// compose template.
    fn hardened() -> (Value, Vec<Value>) {
        let container = json!({
            "Image": "sha256:0123abcd",
            "Config": { "User": "agent" },
            "HostConfig": {
                "ReadonlyRootfs": true,
                "Privileged": false,
                "CapDrop": ["ALL"],
                "SecurityOpt": ["no-new-privileges:true"],
            },
            "Mounts": [
                { "Type": "volume", "Destination": "/workspace" },
                { "Type": "tmpfs", "Destination": "/run/moor" },
            ],
            "NetworkSettings": { "Networks": { "demo-internal": {} } },
        });
        let networks = vec![json!({ "Name": "demo-internal", "Internal": true })];
        (container, networks)
    }

    fn status(props: &Value, name: &str) -> String {
        props[name]["status"]
            .as_str()
            .unwrap_or("<absent>")
            .to_string()
    }

    const ALL: [&str; 7] = [
        "fs.read_only_root",
        "privileged.off",
        "mounts.no_host_bind",
        "user.non_root",
        "caps.dropped_all",
        "privileges.no_new",
        "network.internal_only",
    ];

    #[test]
    fn attestation_is_derived_from_inspect_output() {
        let (c, n) = hardened();
        let props = derive(&c, &n);
        for name in ALL {
            assert_eq!(status(&props, name), "proven", "{name}: {}", props[name]);
        }

        // Each property turns on what inspect reports, not on a default.
        let mut c2 = c.clone();
        c2["HostConfig"]["ReadonlyRootfs"] = json!(false);
        c2["HostConfig"]["Privileged"] = json!(true);
        c2["HostConfig"]["CapDrop"] = json!(["NET_RAW"]);
        c2["HostConfig"]["SecurityOpt"] = json!([]);
        c2["Config"]["User"] = json!("0:0");
        c2["Mounts"] = json!([{ "Type": "bind", "Destination": "/host" }]);
        let open = vec![json!({ "Name": "demo-internal", "Internal": false })];
        let props = derive(&c2, &open);
        for name in ALL {
            assert_eq!(status(&props, name), "violated", "{name}: {}", props[name]);
        }
    }

    #[test]
    fn missing_inspect_fields_are_unproven() {
        let props = derive(&json!({}), &[]);
        for name in ALL {
            assert_eq!(status(&props, name), "unproven", "{name}: {}", props[name]);
        }

        // Wrong types and unknown networks are no better than absence.
        let (mut c, _) = hardened();
        c["HostConfig"]["ReadonlyRootfs"] = json!("yes");
        c["HostConfig"]["CapDrop"] = Value::Null;
        let props = derive(&c, &[]);
        assert_eq!(status(&props, "fs.read_only_root"), "unproven");
        assert_eq!(status(&props, "caps.dropped_all"), "unproven");
        assert_eq!(status(&props, "network.internal_only"), "unproven");
        assert_eq!(status(&props, "user.non_root"), "proven");
    }
}
