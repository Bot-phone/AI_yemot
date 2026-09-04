use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct UpdateCheckResult {
    pub has_update: bool,
    pub current_version: String,
    pub latest_version: String,
    pub release_url: String,
    pub release_notes: String,
}

#[tauri::command]
pub async fn check_for_updates() -> Result<UpdateCheckResult, String> {
    let current_version = env!("CARGO_PKG_VERSION").to_string();

    let client = reqwest::Client::builder()
        .user_agent("AI_yemot_desktop")
        .timeout(std::time::Duration::from_secs(6))
        .build()
        .map_err(|e| e.to_string())?;

    let repos = [
        "https://api.github.com/repos/palmoni5/AI_yemot/releases/latest",
        "https://api.github.com/repos/Bot-phone/AI_yemot/releases/latest",
    ];

    for url in repos {
        if let Ok(res) = client.get(url).send().await {
            if res.status().is_success() {
                if let Ok(json) = res.json::<serde_json::Value>().await {
                    let tag = json["tag_name"]
                        .as_str()
                        .unwrap_or("")
                        .trim_start_matches('v')
                        .trim();
                    let html_url = json["html_url"].as_str().unwrap_or("").to_string();
                    let body = json["body"].as_str().unwrap_or("").to_string();

                    if !tag.is_empty() && is_newer_version(tag, &current_version) {
                        return Ok(UpdateCheckResult {
                            has_update: true,
                            current_version,
                            latest_version: tag.to_string(),
                            release_url: html_url,
                            release_notes: body,
                        });
                    }
                }
            }
        }
    }

    Ok(UpdateCheckResult {
        has_update: false,
        current_version: current_version.clone(),
        latest_version: current_version,
        release_url: String::new(),
        release_notes: String::new(),
    })
}

fn is_newer_version(latest: &str, current: &str) -> bool {
    let latest_parts: Vec<u32> = latest.split('.').filter_map(|s| s.parse().ok()).collect();
    let current_parts: Vec<u32> = current.split('.').filter_map(|s| s.parse().ok()).collect();

    for (l, c) in latest_parts.iter().zip(current_parts.iter()) {
        if l > c {
            return true;
        }
        if l < c {
            return false;
        }
    }
    latest_parts.len() > current_parts.len()
}
