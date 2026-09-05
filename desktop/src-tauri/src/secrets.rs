//! OS keychain storage for the app's secrets (Yemot token, provider API keys).
//!
//! The frontend used to keep these in `localStorage` in plaintext; they now live
//! in the platform's native credential store (Windows Credential Manager, macOS
//! Keychain, Secret Service on Linux) under the service name `ai-yemot`.
//!
//! Only *secrets* belong here — provider/model/language/preset choices stay in
//! localStorage, because they are not sensitive and must survive a keychain that
//! is locked or unavailable.

use keyring::Entry;

/// Service name registered in the OS credential store.
const SERVICE: &str = "ai-yemot";

/// Names the frontend is allowed to address, so a compromised webview cannot
/// walk the whole credential store through this command surface.
const ALLOWED: &[&str] = &[
    "yemot_token",
    "api_key_claude",
    "api_key_gemini",
    "api_key_openai",
    "api_key_groq",
    "api_key_custom",
    "custom_base_url",
];

fn entry(name: &str) -> Result<Entry, String> {
    if !ALLOWED.contains(&name) {
        return Err(format!("שם סוד לא מוכר: {}", name));
    }
    Entry::new(SERVICE, name).map_err(|e| format!("שגיאת גישה למאגר הסודות: {}", e))
}

/// Store (or replace) a secret. An empty value deletes the entry instead.
#[tauri::command]
pub fn secret_set(name: &str, value: &str) -> Result<(), String> {
    if value.is_empty() {
        return secret_delete(name);
    }
    entry(name)?
        .set_password(value)
        .map_err(|e| format!("שמירת הסוד נכשלה: {}", e))
}

/// Read a secret; `None` when it was never stored (or was deleted).
#[tauri::command]
pub fn secret_get(name: &str) -> Result<Option<String>, String> {
    match entry(name)?.get_password() {
        Ok(v) => Ok(Some(v)),
        Err(keyring::Error::NoEntry) => Ok(None),
        // A locked / missing / unsupported credential store is not a failure of
        // *this* read: the app must still start, just without a stored secret.
        // On write the same conditions stay errors — silently losing a secret
        // the user asked to save would be worse.
        Err(e @ (keyring::Error::NoStorageAccess(_) | keyring::Error::PlatformFailure(_))) => {
            eprintln!("secret_get({}): מאגר הסודות אינו זמין: {}", name, e);
            Ok(None)
        }
        Err(e) => Err(format!("קריאת הסוד נכשלה: {}", e)),
    }
}

/// Remove a secret. Deleting a missing entry is a no-op, not an error.
#[tauri::command]
pub fn secret_delete(name: &str) -> Result<(), String> {
    match entry(name)?.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(format!("מחיקת הסוד נכשלה: {}", e)),
    }
}
