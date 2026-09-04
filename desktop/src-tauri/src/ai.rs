use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AIRequestPayload {
    pub target_mode: String, // "script" or "direct"
    pub provider: String,    // "gemini", "openai", "claude", "groq"
    pub model: String,       // "regular", "pro", or model ID
    pub prompt: String,      // User instruction
    pub token: String,       // Yemot token
    pub api_key: String,     // Gemini / Provider API key
    pub api_type: String,    // "private" or "system" (2411)
    pub is_preview: bool,    // Preview / FullAnswer
    pub logout: bool,        // Invalidate token after run
    pub script_url: String,  // Google Apps Script or Webhook URL
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

    let mut query_params: Vec<(&str, String)> = Vec::new();
    query_params.push(("token", payload.token.clone()));
    query_params.push(("text", payload.prompt.clone()));

    if payload.model.to_lowercase() == "pro" {
        query_params.push(("model", "pro".to_string()));
    }

    if payload.api_type == "private" && !payload.api_key.trim().is_empty() {
        query_params.push(("key", payload.api_key.clone()));
    }

    if payload.logout {
        query_params.push(("Logout", "yes".to_string()));
    }

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
        "openai" | "groq" => send_to_openai_compatible(payload).await,
        _ => Err(format!("ספק AI '{}' אינו נתמך עדיין באופן ישיר", payload.provider)),
    }
}

async fn send_to_gemini(payload: AIRequestPayload) -> Result<AIResponsePayload, String> {
    let effective_key = if payload.api_type == "system" {
        "2411".to_string()
    } else if !payload.api_key.trim().is_empty() {
        payload.api_key.clone()
    } else {
        return Err("נדרש מפתח Gemini API לצורך פנייה ישירה".to_string());
    };

    let model_name = match payload.model.to_lowercase().as_str() {
        "pro" => "gemini-2.5-pro",
        _ => "gemini-2.5-flash",
    };

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| format!("שגיאה: {}", e))?;

    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
        model_name, effective_key
    );

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

    let (base_url, model_name) = match payload.provider.to_lowercase().as_str() {
        "groq" => ("https://api.groq.com/openai/v1/chat/completions", "llama-3.3-70b-versatile"),
        _ => ("https://api.openai.com/v1/chat/completions", "gpt-4o"),
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
        .post(base_url)
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

        Ok(AIResponsePayload {
            success: true,
            message: format!("תשובה התקבלה מ-{}", payload.provider),
            raw_response: answer,
            is_preview: payload.is_preview,
        })
    } else {
        let err_msg = json["error"]["message"].as_str().unwrap_or("שגיאה בספק ה-AI");
        Err(format!("שגיאת ספק ({}): {}", status, err_msg))
    }
}
