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

// ---------------------------------------------------------------------------
// Login (create token from system number + password), MFA & logout
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct YemotLoginResult {
    pub success: bool,
    pub message: String,
    pub token: Option<String>,
    pub mfa_required: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MfaMethod {
    pub id: String,
    pub label: String,
    pub send_types: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MfaMethodsResult {
    pub success: bool,
    pub message: String,
    pub methods: Vec<MfaMethod>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct YemotSimpleResult {
    pub success: bool,
    pub message: String,
}

async fn yemot_get(client: &reqwest::Client, url: &str) -> Result<serde_json::Value, String> {
    let res = client
        .get(url)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| format!("שגיאת רשת: {}", e))?;
    let json: serde_json::Value = res
        .json()
        .await
        .map_err(|e| format!("שגיאת פענוח תשובה: {}", e))?;
    Ok(json)
}

/// Minimal URL-encoding for query parameter values.
fn urlencode(input: &str) -> String {
    let mut out = String::new();
    for b in input.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char)
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

/// Login with system number + password. Returns a token; indicates whether
/// MFA (two-factor) verification is still required before using it.
#[tauri::command]
pub async fn login_yemot(username: String, password: String) -> Result<YemotLoginResult, String> {
    if username.trim().is_empty() || password.trim().is_empty() {
        return Ok(YemotLoginResult {
            success: false,
            message: "יש להזין מספר מערכת וסיסמה".to_string(),
            token: None,
            mfa_required: false,
        });
    }

    let client = reqwest::Client::new();
    let url = format!(
        "{}Login?username={}&password={}",
        YEMOT_API_BASE,
        urlencode(&username),
        urlencode(&password)
    );

    let json = yemot_get(&client, &url).await?;
    let status = json["responseStatus"].as_str().unwrap_or("");
    let msg = json["message"].as_str().unwrap_or("");

    if status != "OK" {
        return Ok(YemotLoginResult {
            success: false,
            message: if !msg.is_empty() {
                msg.to_string()
            } else {
                "ההתחברות נכשלה".to_string()
            },
            token: None,
            mfa_required: false,
        });
    }

    let token = json["token"].as_str().unwrap_or("").to_string();
    if token.is_empty() {
        return Ok(YemotLoginResult {
            success: false,
            message: "טוקן לא התקבל מהמערכת".to_string(),
            token: None,
            mfa_required: false,
        });
    }

    // Check global MFA status for this session
    let mfa_url = format!(
        "{}MFASession?token={}&action=isPass",
        YEMOT_API_BASE,
        urlencode(&token)
    );
    let mfa_json = yemot_get(&client, &mfa_url).await?;
    let is_pass = mfa_json["isPass"].as_bool().unwrap_or(false);

    Ok(YemotLoginResult {
        success: true,
        message: if is_pass {
            "התחברות הושלמה בהצלחה".to_string()
        } else {
            "נדרש אימות דו-שלבי (MFA)".to_string()
        },
        token: Some(token),
        mfa_required: !is_pass,
    })
}

/// Get available MFA verification methods (call / SMS etc.) for a session.
#[tauri::command]
pub async fn get_mfa_methods(token: String) -> Result<MfaMethodsResult, String> {
    let client = reqwest::Client::new();
    let url = format!(
        "{}MFASession?token={}&action=getMFAMethods",
        YEMOT_API_BASE,
        urlencode(&token)
    );

    let json = yemot_get(&client, &url).await?;

    let mut methods = Vec::new();
    if let Some(arr) = json["mfaMethods"].as_array() {
        for m in arr {
            let id = m["ID"].as_i64().map(|v| v.to_string()).unwrap_or_default();
            let value = m["VALUE"].as_str().unwrap_or("");
            let nike = m["NIKE"].as_str().unwrap_or("");
            let label = if nike.is_empty() {
                value.to_string()
            } else {
                format!("{} ({})", value, nike)
            };
            let send_types: Vec<String> = m["SEND_TYPE"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default();
            if !id.is_empty() {
                methods.push(MfaMethod { id, label, send_types });
            }
        }
    }

    if methods.is_empty() {
        Ok(MfaMethodsResult {
            success: false,
            message: "לא נמצאו שיטות אימות זמינות. יש להגדיר שיטות אימות במערכת".to_string(),
            methods,
        })
    } else {
        Ok(MfaMethodsResult {
            success: true,
            message: "OK".to_string(),
            methods,
        })
    }
}

/// Send an MFA verification code using the selected method & send type.
#[tauri::command]
pub async fn send_mfa_code(
    token: String,
    mfa_id: String,
    send_type: String,
) -> Result<YemotSimpleResult, String> {
    let client = reqwest::Client::new();
    let url = format!(
        "{}MFASession?token={}&action=sendMFA&mfaId={}&mfaSendType={}&lang=HE",
        YEMOT_API_BASE,
        urlencode(&token),
        urlencode(&mfa_id),
        urlencode(&send_type)
    );

    let json = yemot_get(&client, &url).await?;
    let status = json["responseStatus"].as_str().unwrap_or("");
    let msg = json["message"].as_str().unwrap_or("");

    if status == "OK" {
        Ok(YemotSimpleResult {
            success: true,
            message: "קוד אימות נשלח בהצלחה".to_string(),
        })
    } else {
        Ok(YemotSimpleResult {
            success: false,
            message: if !msg.is_empty() {
                format!("שגיאה: {}", msg)
            } else {
                "שליחת הקוד נכשלה".to_string()
            },
        })
    }
}

/// Validate the MFA code the user received.
#[tauri::command]
pub async fn validate_mfa_code(token: String, code: String) -> Result<YemotSimpleResult, String> {
    let client = reqwest::Client::new();
    let url = format!(
        "{}MFASession?token={}&action=validMFA&mfaCode={}&mfaRememberMe=false",
        YEMOT_API_BASE,
        urlencode(&token),
        urlencode(&code)
    );

    let json = yemot_get(&client, &url).await?;
    let valid_status = json["mfa_valid_status"].as_str().unwrap_or("");

    if valid_status == "VALID" {
        Ok(YemotSimpleResult {
            success: true,
            message: "האימות הושלם בהצלחה!".to_string(),
        })
    } else if valid_status == "OVERTRY" {
        Ok(YemotSimpleResult {
            success: false,
            message: "חרגת ממספר הניסיונות. יש לשלוח קוד חדש".to_string(),
        })
    } else {
        let left = json["mfa_valid_left"].as_i64();
        let message = match left {
            Some(n) => format!("קוד שגוי. נותרו {} ניסיונות", n),
            None => "הקוד שהוזן שגוי".to_string(),
        };
        Ok(YemotSimpleResult { success: false, message })
    }
}

/// Logout (invalidate the token) — performed locally, never via the script.
#[tauri::command]
pub async fn logout_yemot(token: String) -> Result<YemotSimpleResult, String> {
    let client = reqwest::Client::new();
    let url = format!("{}Logout?token={}", YEMOT_API_BASE, urlencode(&token));

    let res = client
        .get(&url)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| format!("שגיאת רשת: {}", e))?;

    if res.status().is_success() {
        Ok(YemotSimpleResult {
            success: true,
            message: "התנתקות בוצעה בהצלחה".to_string(),
        })
    } else {
        Ok(YemotSimpleResult {
            success: false,
            message: format!(
                "התנתקות נכשלה ({}): {}",
                res.status(),
                res.text().await.unwrap_or_default()
            ),
        })
    }
}
