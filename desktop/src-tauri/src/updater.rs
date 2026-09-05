//! Update check against the GitHub releases of the org repo.
//!
//! The desktop app is distributed as GitHub releases created by
//! `.github/workflows/desktop.yml` on `v*` tags, so `releases/latest`
//! always carries a `tag_name` of the form `v<version>`.

use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

const RELEASES_LATEST_URL: &str =
    "https://api.github.com/repos/Bot-phone/AI_yemot/releases/latest";

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct UpdateCheckResult {
    pub has_update: bool,
    pub current_version: String,
    pub latest_version: String,
    pub release_url: String,
    pub release_notes: String,
}

/// One client for the whole process: building a `reqwest::Client` per call
/// throws away the connection pool and the TLS session cache.
fn client() -> Result<&'static reqwest::Client, String> {
    static CLIENT: OnceLock<Result<reqwest::Client, String>> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .user_agent("AI_yemot_desktop")
                .timeout(std::time::Duration::from_secs(6))
                .build()
                .map_err(|e| e.to_string())
        })
        .as_ref()
        .map_err(|e| e.clone())
}

#[tauri::command]
pub async fn check_for_updates() -> Result<UpdateCheckResult, String> {
    let current_version = env!("CARGO_PKG_VERSION").to_string();

    let no_update = || UpdateCheckResult {
        has_update: false,
        current_version: current_version.clone(),
        latest_version: current_version.clone(),
        release_url: String::new(),
        release_notes: String::new(),
    };

    let res = match client()?.get(RELEASES_LATEST_URL).send().await {
        Ok(res) if res.status().is_success() => res,
        // Offline / rate-limited / no release yet: not an error for the caller.
        _ => return Ok(no_update()),
    };

    let json = match res.json::<serde_json::Value>().await {
        Ok(json) => json,
        Err(_) => return Ok(no_update()),
    };

    let tag = json["tag_name"]
        .as_str()
        .unwrap_or("")
        .trim()
        .trim_start_matches('v')
        .trim();

    if tag.is_empty() || !is_newer_version(tag, &current_version) {
        return Ok(no_update());
    }

    Ok(UpdateCheckResult {
        has_update: true,
        current_version,
        latest_version: tag.to_string(),
        release_url: json["html_url"].as_str().unwrap_or("").to_string(),
        release_notes: json["body"].as_str().unwrap_or("").to_string(),
    })
}

/// Splits `1.2.3-rc1+build` into the numeric core `[1, 2, 3]` and whether a
/// pre-release suffix was present.
fn parse_version(v: &str) -> (Vec<u32>, bool) {
    let v = v.trim().trim_start_matches('v');
    let core = v.split(['-', '+']).next().unwrap_or("");
    let pre = v.len() > core.len();
    let parts = core
        .split('.')
        .map(|s| s.trim().parse::<u32>().unwrap_or(0))
        .collect();
    (parts, pre)
}

/// `true` when `latest` should be offered as an update over `current`.
/// A pre-release is never newer than the same numeric core released.
fn is_newer_version(latest: &str, current: &str) -> bool {
    let (l_core, l_pre) = parse_version(latest);
    let (c_core, c_pre) = parse_version(current);

    for i in 0..l_core.len().max(c_core.len()) {
        let l = l_core.get(i).copied().unwrap_or(0);
        let c = c_core.get(i).copied().unwrap_or(0);
        if l != c {
            return l > c;
        }
    }

    // Same numeric core: only a release over a pre-release counts as newer.
    c_pre && !l_pre
}

#[cfg(test)]
mod tests {
    use super::is_newer_version;

    #[test]
    fn plain_versions() {
        assert!(is_newer_version("1.0.1", "1.0.0"));
        assert!(is_newer_version("1.1.0", "1.0.9"));
        assert!(is_newer_version("2.0.0", "1.9.9"));
        assert!(!is_newer_version("1.0.0", "1.0.0"));
        assert!(!is_newer_version("1.0.0", "1.0.1"));
        assert!(!is_newer_version("0.9.9", "1.0.0"));
    }

    #[test]
    fn tag_prefix_and_whitespace() {
        assert!(is_newer_version("v1.0.1", "1.0.0"));
        assert!(!is_newer_version(" v1.0.0 ", "1.0.0"));
    }

    #[test]
    fn differing_lengths_are_zero_padded() {
        assert!(!is_newer_version("1.0", "1.0.0"));
        assert!(!is_newer_version("1.0.0", "1.0"));
        assert!(is_newer_version("1.0.1", "1.0"));
        assert!(!is_newer_version("1.0", "1.0.1"));
    }

    #[test]
    fn prerelease_is_not_newer_than_same_release() {
        assert!(!is_newer_version("1.0.0-rc1", "1.0.0"));
        assert!(!is_newer_version("1.0.0-rc2", "1.0.0-rc1"));
        assert!(is_newer_version("1.0.0", "1.0.0-rc1"));
        assert!(is_newer_version("1.1.0-rc1", "1.0.0"));
        assert!(!is_newer_version("1.0.0-rc1", "1.0.1"));
    }

    #[test]
    fn garbage_does_not_trigger_an_update() {
        assert!(!is_newer_version("", "1.0.0"));
        assert!(!is_newer_version("not-a-version", "1.0.0"));
    }
}
