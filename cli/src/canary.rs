use std::time::{SystemTime, UNIX_EPOCH};

/// A hostname reserved as a tripwire: it is never added to any project's
/// egress allow-list, so any request the sandbox makes to it is always a
/// deny — and folded into the audit trail as a TRIPWIRE, not a routine
/// deny, because there is no legitimate reason for it to ever be
/// attempted. See docs/THREAT-MODEL.md.
pub const CANARY_DOMAIN: &str = "canary.moor.invalid";

/// A decoy value planted in the sandbox's environment as
/// `MOOR_CANARY_TOKEN`. It looks like a credential but authenticates
/// nothing — its only purpose is to exist somewhere a real secret would.
/// Nothing watches for it yet: the proxy doesn't inspect request bodies
/// (see proxy/README.md), and `moor selftest` probes only the canary
/// domain.
pub fn generate_token() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("moor-canary-{:x}-{:x}", nanos, std::process::id())
}
