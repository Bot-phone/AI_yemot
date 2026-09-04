use serde::{Deserialize, Serialize};

const YEMOT_API_BASE: &str = "https://www.call2all.co.il/ym/api/";

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct YemotSessionResult {
    pub success: bool,
    pub message: String,
    pub mfa_required: bool,
    pub mfa_token: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct YemotMfaResult {
    pub success: bool,
    pub message: String,
    pub new_token: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct YemotActionResult {
    pub success: bool,
    pub path: String,
    pub key: String,
    pub value: String,
    pub message: String,
}

#[tauri::command]
pub async fn check_yemot_token(token: String) -> Result<YemotSessionResult, String> {
    if token.trim().is_empty() {
        return Ok(YemotSessionResult {
            success: false,
            message: "טוקן ימות המשיח ריק".to_string(),
            mfa_required: false,
            mfa_token: None,
        });
    }

    let client = reqwest::Client::new();
    let url = format!("{}GetSession?token={}", YEMOT_API_BASE, token);

    let res = client
        .get(&url)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| format!("שגיאת רשת: {}", e))?;

    let json: serde_json::Value = res
        .json()
        .await
        .map_err(|e| format!("שגיאת פענוח תשובה: {}", e))?;

    let response_status = json["responseStatus"].as_str().unwrap_or("");
    let message = json["message"].as_str().unwrap_or("");

    if response_status == "OK" {
        Ok(YemotSessionResult {
            success: true,
            message: "טוקן תקין ומחובר בהצלחה".to_string(),
            mfa_required: false,
            mfa_token: None,
        })
    } else if response_status == "MFA_REQUIRED" || message.contains("mfa required") {
        let mfa_tok = json["mfaToken"].as_str().map(|s| s.to_string());
        Ok(YemotSessionResult {
            success: false,
            message: "נדרש אימות דו-שלבי (MFA)".to_string(),
            mfa_required: true,
            mfa_token: mfa_tok,
        })
    } else {
        Ok(YemotSessionResult {
            success: false,
            message: if !message.is_empty() {
                message.to_string()
            } else {
                "טוקן לא תקין או שפג תוקפו".to_string()
            },
            mfa_required: false,
            mfa_token: None,
        })
    }
}

#[tauri::command]
pub async fn request_yemot_mfa(
    token: String,
    mfa_token: String,
    method: String, // "call" or "sms"
) -> Result<YemotMfaResult, String> {
    let client = reqwest::Client::new();
    let url = format!(
        "{}SendMfaCode?token={}&mfaToken={}&method={}",
        YEMOT_API_BASE, token, mfa_token, method
    );

    let res = client
        .get(&url)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| format!("שגיאת רשת: {}", e))?;

    let json: serde_json::Value = res
        .json()
        .await
        .map_err(|e| format!("שגיאת פענוח תשובה: {}", e))?;

    let status = json["responseStatus"].as_str().unwrap_or("");
    let msg = json["message"].as_str().unwrap_or("");

    if status == "OK" {
        Ok(YemotMfaResult {
            success: true,
            message: "קוד אימות נשלח בהצלחה".to_string(),
            new_token: None,
        })
    } else {
        Ok(YemotMfaResult {
            success: false,
            message: if !msg.is_empty() {
                msg.to_string()
            } else {
                "שליחת קוד אימות נכשלה".to_string()
            },
            new_token: None,
        })
    }
}

#[tauri::command]
pub async fn verify_yemot_mfa(
    token: String,
    mfa_token: String,
    code: String,
) -> Result<YemotMfaResult, String> {
    let client = reqwest::Client::new();
    let url = format!(
        "{}VerifyMfaCode?token={}&mfaToken={}&code={}",
        YEMOT_API_BASE, token, mfa_token, code
    );

    let res = client
        .get(&url)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| format!("שגיאת רשת: {}", e))?;

    let json: serde_json::Value = res
        .json()
        .await
        .map_err(|e| format!("שגיאת פענוח תשובה: {}", e))?;

    let status = json["responseStatus"].as_str().unwrap_or("");
    let msg = json["message"].as_str().unwrap_or("");

    if status == "OK" {
        let updated_token = json["token"].as_str().unwrap_or(&token).to_string();
        Ok(YemotMfaResult {
            success: true,
            message: "אימות הצליח!".to_string(),
            new_token: Some(updated_token),
        })
    } else {
        Ok(YemotMfaResult {
            success: false,
            message: if !msg.is_empty() {
                msg.to_string()
            } else {
                "קוד אימות שגוי".to_string()
            },
            new_token: None,
        })
    }
}

#[tauri::command]
pub async fn execute_yemot_action(
    token: String,
    path: String,
    key: String,
    value: String,
) -> Result<YemotActionResult, String> {
    let client = reqwest::Client::new();
    let url = format!("{}UpdateExtension", YEMOT_API_BASE);

    let params = [
        ("token", token.as_str()),
        ("path", path.as_str()),
        (&key, value.as_str()),
    ];

    let res = client
        .post(&url)
        .form(&params)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| format!("שגיאת רשת: {}", e))?;

    if res.status().is_success() {
        Ok(YemotActionResult {
            success: true,
            path,
            key,
            value,
            message: "עודכן בהצלחה במערכת ימות המשיח".to_string(),
        })
    } else {
        let err_text = res.text().await.unwrap_or_default();
        Ok(YemotActionResult {
            success: false,
            path,
            key,
            value,
            message: format!("שגיאה בעדכון: {}", err_text),
        })
    }
}
