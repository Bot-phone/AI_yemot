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
    /// This process runs from the portable single-file build. The updater
    /// plugin would download and run the NSIS installer, turning a portable
    /// copy into an installed app, so the UI only offers a download instead.
    pub portable: bool,
    /// Direct link to the portable asset of the latest release, when it has
    /// one; empty otherwise (the UI falls back to `release_url`).
    pub portable_url: String,
}

/// Marker file that turns any copy of the app into a portable one when it
/// sits next to the executable (useful for people who rename the file).
const PORTABLE_MARKER: &str = "portable";

/// `true` when the executable at `exe` is the portable build: its file name
/// carries "portable" (CI names it `AI_yemot_<version>_x64-portable.exe`) or
/// a `portable` marker file lies beside it.
fn is_portable_exe(exe: &std::path::Path, marker_beside: impl Fn(&std::path::Path) -> bool) -> bool {
    let by_name = exe
        .file_stem()
        .and_then(|s| s.to_str())
        .map(|s| s.to_ascii_lowercase().contains("portable"))
        .unwrap_or(false);
    by_name || exe.parent().map(|d| marker_beside(&d.join(PORTABLE_MARKER))).unwrap_or(false)
}

/// Whether this process is the portable build. Evaluated once.
pub fn is_portable() -> bool {
    static PORTABLE: OnceLock<bool> = OnceLock::new();
    *PORTABLE.get_or_init(|| {
        std::env::current_exe()
            .map(|exe| is_portable_exe(&exe, |m| m.is_file()))
            .unwrap_or(false)
    })
}

/// The `browser_download_url` of the release asset whose name marks it as the
/// portable build, if any.
fn portable_asset_url(release: &serde_json::Value) -> String {
    release["assets"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|a| {
            a["name"]
                .as_str()
                .map(|n| n.to_ascii_lowercase().contains("portable"))
                .unwrap_or(false)
        })
        .and_then(|a| a["browser_download_url"].as_str())
        .unwrap_or("")
        .to_string()
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
        portable: is_portable(),
        portable_url: String::new(),
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
        portable: is_portable(),
        portable_url: portable_asset_url(&json),
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
    use super::{is_newer_version, is_portable_exe, portable_asset_url};
    use std::path::Path;

    #[test]
    fn portable_by_file_name_or_marker() {
        let no_marker = |_: &Path| false;
        assert!(is_portable_exe(Path::new(r"D:\usb\AI_yemot_0.1.0_x64-portable.exe"), no_marker));
        assert!(is_portable_exe(Path::new("/tmp/ai_yemot-PORTABLE"), no_marker));
        assert!(!is_portable_exe(Path::new(r"C:\Program Files\AI_yemot\AI_yemot.exe"), no_marker));
        // renamed copy with a `portable` file beside it
        assert!(is_portable_exe(Path::new("usb/yemot.exe"), |m| m == Path::new("usb/portable")));
        assert!(!is_portable_exe(Path::new("usb/yemot.exe"), |m| m == Path::new("other/portable")));
    }

    #[test]
    fn portable_asset_is_picked_from_the_release() {
        let release = serde_json::json!({
            "assets": [
                {"name": "AI_yemot_0.2.0_x64-setup.exe", "browser_download_url": "https://x/setup.exe"},
                {"name": "AI_yemot_0.2.0_x64-portable.exe", "browser_download_url": "https://x/portable.exe"}
            ]
        });
        assert_eq!(portable_asset_url(&release), "https://x/portable.exe");
        assert_eq!(portable_asset_url(&serde_json::json!({"assets": []})), "");
        assert_eq!(portable_asset_url(&serde_json::json!({})), "");
    }

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

#[cfg(test)]
mod live_tests {
    //! Network test, run by hand: `cargo test --lib live_release -- --ignored --nocapture`.
    use super::*;

    #[tokio::test]
    #[ignore]
    async fn live_release_offers_an_update_to_an_older_build() {
        let json: serde_json::Value = client()
            .unwrap()
            .get(RELEASES_LATEST_URL)
            .send()
            .await
            .expect("GitHub reachable")
            .json()
            .await
            .expect("release JSON");
        let tag = json["tag_name"].as_str().unwrap_or("").trim_start_matches('v');
        assert!(!tag.is_empty(), "no release published");
        assert!(is_newer_version(tag, "0.0.1"), "{tag} should be offered to 0.0.1");
        assert!(!is_newer_version(tag, env!("CARGO_PKG_VERSION")), "current build is up to date");
        assert!(json["html_url"].as_str().unwrap_or("").starts_with("https://github.com/Bot-phone/AI_yemot/releases/"));
        println!("latest release tag = v{tag}");
    }
}
