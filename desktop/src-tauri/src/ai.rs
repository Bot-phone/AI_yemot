use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AIRequestPayload {
    pub target_mode: String, // "script" or "direct"
    pub provider: String,    // "gemini", "openai", "claude", "groq"
    pub model: String,       // "regular", "pro", or model ID
    pub prompt: String,      // User instruction
    pub token: String,       // Yemot token — NEVER sent to any external script/AI
    pub api_key: String,     // Gemini / Provider API key (personal)
    pub is_preview: bool,    // Preview / FullAnswer
    pub logout: bool,        // Invalidate token after run
    pub script_url: String,  // Google Apps Script or Webhook URL
    #[serde(default)]
    pub base_url: String,    // Optional custom API URL (direct mode) — full endpoint or base URL
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AIResponsePayload {
    pub success: bool,
    pub message: String,
    pub raw_response: String,
    pub is_preview: bool,
}

#[tauri::command]
pub async fn send_ai_request(payload: AIRequestPayload) -> Result<AIResponsePayload, String> {
    if payload.target_mode == "script" {
        send_to_script(payload).await
    } else {
        send_to_direct_ai(payload).await
    }
}

async fn send_to_script(payload: AIRequestPayload) -> Result<AIResponsePayload, String> {
    if payload.script_url.trim().is_empty() {
        return Err("כתובת הסקריפט (Script URL) לא הוגדרה".to_string());
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|e| format!("שגיאת יצירת לקוח רשת: {}", e))?;

    // SECURITY: the Yemot token is never sent to the script — only the AI
    // prompt (and optional personal API key) leave this machine. All Yemot
    // actions are executed locally by this app (execute_yemot_action).
    let mut query_params: Vec<(&str, String)> = Vec::new();
    query_params.push(("text", payload.prompt.clone()));

    if payload.model.to_lowercase() == "pro" {
        query_params.push(("model", "pro".to_string()));
    }

    if !payload.api_key.trim().is_empty() {
        query_params.push(("key", payload.api_key.clone()));
    }

    // Logout is handled locally (logout_yemot command), never by the script.

    if payload.is_preview {
        query_params.push(("Fullanswer", "yes".to_string()));
    }

    let res = client
        .get(&payload.script_url)
        .query(&query_params)
        .header("Accept", "application/json, text/plain, */*")
        .send()
        .await
        .map_err(|e| format!("שגיאת תקשורת מול הסקריפט: {}", e))?;

    let status = res.status();
    let text = res
        .text()
        .await
        .map_err(|e| format!("שגיאת קריאת תשובה: {}", e))?;

    if status.is_success() {
        Ok(AIResponsePayload {
            success: true,
            message: "הבקשה עובדה בהצלחה ע\"י הסקריפט".to_string(),
            raw_response: text,
            is_preview: payload.is_preview,
        })
    } else {
        Err(format!("שגיאת שרת סקריפט ({}): {}", status, text))
    }
}

async fn send_to_direct_ai(payload: AIRequestPayload) -> Result<AIResponsePayload, String> {
    match payload.provider.to_lowercase().as_str() {
        "gemini" => send_to_gemini(payload).await,
        // "custom" = any OpenAI-compatible provider supplied by the user (base_url required)
        "openai" | "groq" | "custom" => send_to_openai_compatible(payload).await,
        _ => Err(format!("ספק AI '{}' אינו נתמך עדיין באופן ישיר", payload.provider)),
    }
}

async fn send_to_gemini(payload: AIRequestPayload) -> Result<AIResponsePayload, String> {
    // Direct mode: there is no system key at all — a personal API key is required.
    let effective_key = payload.api_key.trim().to_string();
    if effective_key.is_empty() {
        return Err("נדרש מפתח API אישי לצורך פנייה ישירה לספק ה-AI".to_string());
    }

    // "regular" / "pro" map to the recommended models; anything else is treated
    // as an explicit model ID entered manually by the user.
    let model_name = match payload.model.trim().to_lowercase().as_str() {
        "pro" => "gemini-2.5-pro".to_string(),
        "regular" | "" => "gemini-2.5-flash".to_string(),
        _ => payload.model.trim().to_string(),
    };

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| format!("שגיאה: {}", e))?;

    // Custom URL support: either a full ".../models/X:generateContent" endpoint
    // or a base URL (e.g. proxy/gateway root up to /v1beta) that we complete.
    let custom_url = payload.base_url.trim().trim_end_matches('/').to_string();
    let url = if custom_url.is_empty() {
        format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
            model_name, effective_key
        )
    } else if custom_url.ends_with(":generateContent") {
        if custom_url.contains("key=") {
            custom_url
        } else {
            format!("{}?key={}", custom_url, effective_key)
        }
    } else {
        format!(
            "{}/models/{}:generateContent?key={}",
            custom_url, model_name, effective_key
        )
    };

    let system_prompt = "אתה עוזר AI מקצועי להגדרת שלוחות במערכת ימות המשיח. \
    ענה בעברית ברורה ומדויקת. אם המשתמש ביקש הגדרות שלוחה, פרט את שמות הפרמטרים וערכיהם בקובץ ext.ini.";

    let body = serde_json::json!({
        "systemInstruction": {
            "parts": [{ "text": system_prompt }]
        },
        "contents": [{
            "role": "user",
            "parts": [{ "text": payload.prompt }]
        }]
    });

    let res = client
        .post(&url)
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("שגיאת תקשורת מול Gemini: {}", e))?;

    let status = res.status();
    let json: serde_json::Value = res
        .json()
        .await
        .map_err(|e| format!("שגיאת קריאת JSON מ-Gemini: {}", e))?;

    if status.is_success() {
        let answer_text = json["candidates"][0]["content"]["parts"][0]["text"]
            .as_str()
            .unwrap_or("לא התקבלה תשובה טקסטואלית")
            .to_string();

        Ok(AIResponsePayload {
            success: true,
            message: "תשובה התקבלה ישירות מ-Gemini".to_string(),
            raw_response: answer_text,
            is_preview: payload.is_preview,
        })
    } else {
        let err_msg = json["error"]["message"].as_str().unwrap_or("שגיאת API לא ידועה");
        Err(format!("שגיאת Gemini API: {}", err_msg))
    }
}

async fn send_to_openai_compatible(payload: AIRequestPayload) -> Result<AIResponsePayload, String> {
    if payload.api_key.trim().is_empty() {
        return Err("נדרש מפתח API עבור ספק זה".to_string());
    }

    let (default_url, default_model) = match payload.provider.to_lowercase().as_str() {
        "groq" => ("https://api.groq.com/openai/v1/chat/completions", "llama-3.3-70b-versatile"),
        _ => ("https://api.openai.com/v1/chat/completions", "gpt-4o"),
    };

    // Custom provider (or custom URL on a known provider): the user supplies a
    // full endpoint URL, or a base URL that we complete with /chat/completions.
    let custom_url = payload.base_url.trim().trim_end_matches('/').to_string();
    if custom_url.is_empty() && payload.provider.eq_ignore_ascii_case("custom") {
        return Err("נדרשת כתובת API מותאמת אישית עבור ספק מותאם".to_string());
    }
    let base_url = if custom_url.is_empty() {
        default_url.to_string()
    } else if custom_url.ends_with("/chat/completions") {
        custom_url
    } else {
        format!("{}/chat/completions", custom_url)
    };

    // "regular" / "pro" map to the recommended model; anything else is treated
    // as an explicit model ID entered manually by the user (or chosen from the list).
    let model_name = match payload.model.trim().to_lowercase().as_str() {
        "regular" | "pro" | "" => default_model.to_string(),
        _ => payload.model.trim().to_string(),
    };

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| format!("שגיאה: {}", e))?;

    let body = serde_json::json!({
        "model": model_name,
        "messages": [
            {
                "role": "system",
                "content": "אתה עוזר AI מקצועי להגדרת שלוחות במערכת ימות המשיח. ענה בעברית ברורה."
            },
            {
                "role": "user",
                "content": payload.prompt
            }
        ]
    });

    let res = client
        .post(&base_url)
        .header("Authorization", format!("Bearer {}", payload.api_key))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("שגיאת תקשורת מול ספק ה-AI: {}", e))?;

    let status = res.status();
    let json: serde_json::Value = res
        .json()
        .await
        .map_err(|e| format!("שגיאת פענוח תשובה: {}", e))?;

    if status.is_success() {
        let answer = json["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("לא התקבלה תשובה")
            .to_string();

        // For a custom provider, show the server host instead of "custom".
        let provider_display = if payload.provider.eq_ignore_ascii_case("custom") {
            reqwest::Url::parse(&base_url)
                .ok()
                .and_then(|u| u.host_str().map(|h| h.to_string()))
                .unwrap_or_else(|| "ספק מותאם".to_string())
        } else {
            payload.provider.clone()
        };

        Ok(AIResponsePayload {
            success: true,
            message: format!("תשובה התקבלה מ-{}", provider_display),
            raw_response: answer,
            is_preview: payload.is_preview,
        })
    } else {
        let err_msg = json["error"]["message"].as_str().unwrap_or("שגיאה בספק ה-AI");
        Err(format!("שגיאת ספק ({}): {}", status, err_msg))
    }
}
