/**
 * ===================================================================
 * ⚠️ קוד מורשה קפוא (script mode) — אין פיתוח חדש כאן.
 * ===================================================================
 * המסלול העיקרי הוא האפליקציה השולחנית שב-desktop/ (Tauri + Svelte).
 * הקובץ הזה נשמר לצורך משתמשים קיימים בלבד:
 *   • אין להוסיף כאן פיצורות — מממשים אותן ב-desktop/.
 *   • הפריסה (deploy-gas.yml) היא workflow_dispatch ידנית בלבד.
 *   • תיקוני אבטחה ותקלות קריטיות — כן; כל השאר — לא.
 * ===================================================================
 */

/**
 * ===================================================================
 * AI_yemot - Google Apps Script Backend
 * מערכת AI מתקדמת לעדכון אוטומטי של שלוחות במערכות ימות המשיח
 * ===================================================================
 */

// -------------------------------------------------------------------
// הגדרות וקבועים
// -------------------------------------------------------------------

// טעינת הגדרות מ-Script Properties עם אפשרות למשתנים גלובליים
const SCRIPT_PROPS = PropertiesService.getScriptProperties().getProperties();

const GEMINI_API_KEY = (typeof globalThis.GEMINI_API_KEY !== 'undefined' ? globalThis.GEMINI_API_KEY : '') || SCRIPT_PROPS['GEMINI_API_KEY'] || '';
const GOOGLE_CLOUD_API_KEY = (typeof globalThis.GOOGLE_CLOUD_API_KEY !== 'undefined' ? globalThis.GOOGLE_CLOUD_API_KEY : '') || SCRIPT_PROPS['GOOGLE_CLOUD_API_KEY'] || '';
const DRIVE_FOLDER_ID = (typeof globalThis.DRIVE_FOLDER_ID !== 'undefined' ? globalThis.DRIVE_FOLDER_ID : '') || SCRIPT_PROPS['DRIVE_FOLDER_ID'] || '';

const YEMOT_API_BASE_URL = 'https://www.call2all.co.il/ym/api/';
const GEMINI_API_BASE_URL = 'https://generativelanguage.googleapis.com/v1beta/models/';
const SPEECH_API_BASE_URL = 'https://speech.googleapis.com/v1/speech:recognize';

const MAX_GEMINI_TURNS = 14;
const MAX_HTTP_RETRIES = 5;

// -------------------------------------------------------------------
// נקודות כניסה ל-Web App
// -------------------------------------------------------------------

function doGet(e) {
  return handleRequest(e ? e.parameter : {});
}

function doPost(e) {
  let params = {};
  if (e) {
    if (e.postData && e.postData.contents) {
      try {
        params = JSON.parse(e.postData.contents);
      } catch (err) {
        params = e.parameter || {};
      }
    } else {
      params = e.parameter || {};
    }
  }
  return handleRequest(params);
}

// -------------------------------------------------------------------
// פונקציית הטיפול המרכזית בבקשות
// -------------------------------------------------------------------

// Kill switch. Set the Script Property AI_YEMOT_DISABLED (any value other than
// "", "0" or "false"; the value itself is shown to the user as the reason) to
// disable this script without redeploying it. Every request, including admin
// alerts, then gets the agreed signal below, which the desktop app turns into a
// dedicated "the script was disabled" notice instead of a generic error.
function disabledNotice() {
  const raw = SCRIPT_PROPS.AI_YEMOT_DISABLED;
  if (raw === undefined || raw === null) return "";
  const v = String(raw).trim();
  if (v === "" || v === "0" || v.toLowerCase() === "false") return "";
  return v;
}

function handleRequest(params) {
  params = params || {};

  const disabledReason = disabledNotice();
  if (disabledReason) {
    return ContentService.createTextOutput(JSON.stringify({
      status: "disabled",
      code: "SCRIPT_DISABLED",
      message: disabledReason
    })).setMimeType(ContentService.MimeType.JSON);
  }

  // טיפול בהתראות מייל מהוורקר על נושאים חדשים שטרם סווגו
  if (params.action === 'notifyNewTopics' || params.action === 'sendAlert') {
    return handleNewTopicsAlert(params);
  }

  const text = params.text;
  const path = params.path;
  const token = params.token;
  const downloadToken = params.Downloadtoken || params.downloadToken;
  const model = params.model;
  const userProvidedKey = params.key;
  const fullAnswer = params.Fullanswer || params.fullAnswer;
  const shouldLogout = params.Logout && String(params.Logout).toLowerCase() === 'yes';

  let finalTranscript = "";
  const logMessages = [];
  const clarifications = [];
  logMessages.push("Script started.");

  // בדיקת מפתח למודל Pro
  if (model && String(model).toLowerCase() === 'pro' && (!userProvidedKey || userProvidedKey.trim() === '')) {
    Logger.log("FATAL: Pro model selected without a user-provided API key ('key' parameter).");
    return ContentService.createTextOutput("שגיאה: השימוש במודל 'pro' מחייב העברת מפתח API אישי דרך הפרמטר 'key'.");
  }

  // קביעת מפתח Gemini אפקטיבי
  let effectiveGeminiKey;
  if (userProvidedKey) {
    if (userProvidedKey === "2411") {
      effectiveGeminiKey = GOOGLE_CLOUD_API_KEY;
      logMessages.push("Using GOOGLE_CLOUD_API_KEY for Gemini.");
    } else {
      effectiveGeminiKey = userProvidedKey;
      logMessages.push("Using user-provided 'key' parameter for Gemini.");
    }
  } else {
    effectiveGeminiKey = GEMINI_API_KEY;
    logMessages.push("Using configured GEMINI_API_KEY.");
  }

  try {
    // בדיקת טוקן ואימות MFA מול ימות המשיח — הטוקן אינו חובה יותר:
    // האפליקציה המקומית אינה שולחת טוקן לסקריפט מטעמי אבטחה, וכל הפעולות
    // מול ימות מתבצעות אצל הלקוח באופן מקומי. הסקריפט משמש רק לתקשורת עם ה-AI.
    let tokenIsValid = false;
    if (token && token.trim() !== "") {
      logMessages.push("Attempting token validity check using MFASession?action=isPass...");
      const tokenCheckResult = checkTokenValidity(token);

      if (!tokenCheckResult.isValid) {
        logMessages.push("FATAL: Token check failed. Message: " + tokenCheckResult.message);
        Logger.log("FATAL: Token check failed. Message: " + tokenCheckResult.message);
        return ContentService.createTextOutput("שגיאה: הטוקן שסופק לא תקין או לא עבר אימות דו-שלבי");
      }
      tokenIsValid = true;
      logMessages.push("Token check passed (MFA isPass: true).");
    } else {
      logMessages.push("No token provided — AI-only mode (all Yemot actions are executed locally by the client).");
    }

    // בדיקת הגדרות מערכת
    if (!GOOGLE_CLOUD_API_KEY || !DRIVE_FOLDER_ID ||
        GOOGLE_CLOUD_API_KEY.includes("YOUR_") || DRIVE_FOLDER_ID.includes("YOUR_")) {
      logMessages.push("FATAL: GOOGLE_CLOUD_API_KEY or DRIVE_FOLDER_ID are not set.");
      return ContentService.createTextOutput(JSON.stringify({
        status: "fatal_error",
        message: "Cloud API key or Drive ID are not configured.",
        log: logMessages
      }));
    }

    if (!effectiveGeminiKey || effectiveGeminiKey.includes("YOUR_")) {
      logMessages.push("FATAL: Effective Gemini API key is missing or invalid.");
      return ContentService.createTextOutput(JSON.stringify({
        status: "fatal_error",
        message: "Gemini API key is not configured or invalid.",
        log: logMessages
      }));
    }

    // קביעת הטקסט הסופי לבקשה: טקסט ישיר או הורדת קובץ קול ותמלול
    if (text) {
      finalTranscript = text;
      logMessages.push("Using provided 'text' parameter.");
      Logger.log("Using provided text: " + finalTranscript);
    } else if (path) {
      logMessages.push("Using 'path' parameter. Attempting audio download and transcription...");
      Logger.log("Path provided: " + path);

      const effectiveToken = downloadToken || token;
      const fileUrl = `${YEMOT_API_BASE_URL}DownloadFile?token=${encodeURIComponent(effectiveToken)}&path=ivr2:${encodeURIComponent(path)}`;
      logMessages.push("Downloading file from: " + fileUrl);

      const fileResponse = fetchWithRetry(fileUrl, { muteHttpExceptions: true });
      if (fileResponse.getResponseCode() !== 200) {
        throw new Error(`Failed to download file. Status: ${fileResponse.getResponseCode()} | Response: ${fileResponse.getContentText()}`);
      }

      const fileBlob = fileResponse.getBlob();
      fileBlob.setName(path.replace(/[/\\:]/g, '_') + ".wav");
      logMessages.push("File downloaded successfully.");

      finalTranscript = transcribeAudioWithGoogleCloud(fileBlob, GOOGLE_CLOUD_API_KEY);
      logMessages.push("Transcription result: '" + finalTranscript + "'");
      Logger.log("Transcription result: " + finalTranscript);
    } else {
      throw new Error("'text' or 'path' parameter must be provided.");
    }

    if (!finalTranscript) {
      throw new Error("No transcript available (text was empty or transcription failed).");
    }

    // שליפת רשימת קבצי ידע מ-Google Drive
    let knowledgeFileList = [];
    if (DRIVE_FOLDER_ID) {
      logMessages.push("Fetching knowledge file list from Drive folder: " + DRIVE_FOLDER_ID);
      knowledgeFileList = getKnowledgeFileList(DRIVE_FOLDER_ID);
    } else {
      logMessages.push("DRIVE_FOLDER_ID not set. Proceeding without dynamic knowledge.");
    }

    logMessages.push(`Sending text to Gemini for processing (Model: ${model || 'default'})...`);

    // שליחה ל-Gemini כולל Tool Calling
    const geminiResponseText = callGemini(finalTranscript, model, knowledgeFileList, effectiveGeminiKey, DRIVE_FOLDER_ID, token);

    logMessages.push("Received final response from Gemini.");
    Logger.log("Gemini Response (raw): " + geminiResponseText);

    // אם הפרמטר Fullanswer=Yes, החזרת התשובה ללא ביצוע קישורי ה-API
    if (fullAnswer && String(fullAnswer).toLowerCase() === 'yes') {
      logMessages.push("Fullanswer=Yes detected. Returning raw Gemini response without execution.");
      return ContentService.createTextOutput(geminiResponseText);
    }

    // ללא טוקן תקין אין אפשרות לבצע פעולות מול ימות מהסקריפט —
    // מחזירים את התשובה הגולמית והלקוח מבצע את הפעולות באופן מקומי.
    if (!tokenIsValid) {
      logMessages.push("No valid token — returning raw AI response for local execution by the client.");
      return ContentService.createTextOutput(geminiResponseText);
    }

    // ניתוח וביצוע קישורי ה-API שהוחזרו מ-Gemini
    const lines = geminiResponseText.trim().split('\n');
    const executionResults = [];
    const unrecognizedLines = [];
    let currentExplanation = "פעולה לא מוסברת";
    let executionCount = 0;

    for (const line of lines) {
      const trimmedLine = line.trim();

      if (trimmedLine.startsWith("הסבר:")) {
        currentExplanation = trimmedLine.substring(4).trim();
      } else if (trimmedLine.startsWith(YEMOT_API_BASE_URL)) {
        const finalUrl = trimmedLine + "&token=" + encodeURIComponent(token);
        logMessages.push(`Executing (${currentExplanation}): ${trimmedLine}`);
        Logger.log(`Executing: ${finalUrl}`);

        let icon = "❌";
        let statusMessage = "";
        try {
          const apiResponse = fetchWithRetry(finalUrl, { muteHttpExceptions: true });
          const responseCode = apiResponse.getResponseCode();
          const responseContent = apiResponse.getContentText();
          logMessages.push(` -> Status: ${responseCode} | Response: ${responseContent}`);

          try {
            const yemotResponse = JSON.parse(responseContent);
            if (yemotResponse.responseStatus === "OK") {
              icon = "✅";
              statusMessage = "סטטוס: בוצע";
            } else {
              const errorMessage = yemotResponse.message || "שגיאה לא ידועה מימות המשיח";
              statusMessage = `סטטוס: שגיאה. פרטי השגיאה: ${errorMessage}`;
            }
          } catch (jsonParseError) {
            statusMessage = `סטטוס: שגיאה. התקבלה תגובה לא תקינה מהשרת: ${responseContent.substring(0, 100)}`;
          }
          executionCount++;
        } catch (execErr) {
          logMessages.push(` -> Error executing URL: ${execErr.message}`);
          statusMessage = `סטטוס: שגיאה. פרטי השגיאה: ${execErr.message}`;
        }

        executionResults.push(`${icon} ${currentExplanation}. ${statusMessage}`);
        currentExplanation = "פעולה לא מוסברת";
      } else if (trimmedLine.startsWith("הבהרה:")) {
        clarifications.push(trimmedLine.substring(5).trim());
      } else if (trimmedLine) {
        logMessages.push("Unrecognized line from Gemini: " + trimmedLine);
        unrecognizedLines.push(trimmedLine);
      }
    }

    logMessages.push(`Script completed. Executed ${executionCount} links.`);
    Logger.log(`Script completed. Executed ${executionCount} links.`);

    let finalOutput = executionResults.join('\n');
    if (clarifications.length > 0) {
      if (finalOutput.length > 0) finalOutput += "\n\n";
      finalOutput += "🔔 הבהרות:\n" + clarifications.map(c => `- ${c}`).join('\n');
    }
    if (unrecognizedLines.length > 0) {
      if (finalOutput.length > 0) finalOutput += "\n\n";
      finalOutput += "⚠️ הבהרות:\n" + unrecognizedLines.join('\n');
    }

    // ביצוע ניתוק טוקן אם נדרש
    if (shouldLogout && token) {
      try {
        const logoutUrl = `${YEMOT_API_BASE_URL}Logout?token=${encodeURIComponent(token)}`;
        Logger.log("Executing Logout request: " + logoutUrl);
        fetchWithRetry(logoutUrl, { muteHttpExceptions: true });
        logMessages.push("Logout executed successfully.");
      } catch (logoutErr) {
        Logger.log("Logout failed: " + logoutErr.message);
        logMessages.push("Logout failed: " + logoutErr.message);
      }
    }

    return ContentService.createTextOutput(finalOutput);

  } catch (e) {
    logMessages.push("FATAL ERROR: " + e.message);
    Logger.log("FATAL ERROR: " + e.message + " | Stack: " + e.stack);
    return ContentService.createTextOutput(e.message);
  }
}

// -------------------------------------------------------------------
// בדיקת תקינות ואימות MFA של טוקן ימות המשיח
// -------------------------------------------------------------------

function checkTokenValidity(token) {
  const checkUrl = `${YEMOT_API_BASE_URL}MFASession?token=${encodeURIComponent(token)}&action=isPass`;
  Logger.log("Token check URL: " + checkUrl);

  let response;
  try {
    response = fetchWithRetry(checkUrl, { muteHttpExceptions: true });
  } catch (e) {
    Logger.log("Error fetching token check URL: " + e.message);
    return { isValid: false, message: "כשל ברמת הרשת: " + e.message };
  }

  const responseCode = response.getResponseCode();
  const responseBody = response.getContentText();
  Logger.log("Token check response status: " + responseCode);

  if (responseCode !== 200) {
    return { isValid: false, message: `שגיאת HTTP: קוד ${responseCode}` };
  }

  try {
    const jsonResponse = JSON.parse(responseBody);

    if (jsonResponse.responseStatus !== "OK") {
      const errorMessage = jsonResponse.message || "שגיאת API כללית או טוקן לא חוקי";
      return { isValid: false, message: `responseStatus הוא ${jsonResponse.responseStatus}. פרטים: ${errorMessage}` };
    }

    if (jsonResponse.isPass === true) {
      return { isValid: true, message: "OK. הסשן עבר MFA. סיבת המעבר: " + jsonResponse.passReason };
    } else {
      const reason = jsonResponse.passReason || "לא צוין";
      let errorMessage = `הסשן לא עבר MFA (isPass: false). סיבה אחרונה: ${reason}. `;
      if (jsonResponse.isAvailable === false) {
        errorMessage += "לא ניתן לבצע אימות כיוון שאין שיטות אימות זמינות (isAvailable: false).";
      }
      return { isValid: false, message: errorMessage };
    }
  } catch (e) {
    Logger.log("Error parsing token check response JSON: " + e.message);
    return { isValid: false, message: "שגיאה בפענוח תגובת השרת (JSON לא תקין)" };
  }
}

// -------------------------------------------------------------------
// הורדת קובץ הגדרות (INI) מימות המשיח
// -------------------------------------------------------------------

function downloadYemotFileContent(path, token) {
  if (!path || !token) {
    return "שגיאה: חסר נתיב או טוקן לפונקציית ההורדה.";
  }

  const fileUrl = `${YEMOT_API_BASE_URL}DownloadFile?token=${encodeURIComponent(token)}&path=ivr2:${encodeURIComponent(path)}`;
  Logger.log("Attempting to download Yemot file: " + path);

  try {
    const fileResponse = fetchWithRetry(fileUrl, { muteHttpExceptions: true });
    const responseCode = fileResponse.getResponseCode();

    if (responseCode !== 200) {
      const errorText = fileResponse.getContentText();
      Logger.log(`Failed to download Yemot file. Status: ${responseCode} | Response: ${errorText}`);

      try {
        const errorJson = JSON.parse(errorText);
        if (errorJson.message) {
          if (errorJson.message.toLowerCase().includes("file not found")) {
            Logger.log("Yemot reported: File not found.");
            return `הערה: הקובץ המבוקש לא נמצא בנתיב ${path}. (זה תקין אם זו יצירת קובץ חדש).`;
          }
          return `שגיאה: הקובץ לא נמצא או אין הרשאה. פרטי שגיאה: ${errorJson.message}`;
        }
      } catch (e) {
        // התגובה אינה JSON
      }
      return `שגיאה: נכשל בהורדת הקובץ. סטטוס: ${responseCode} | תגובה: ${errorText.substring(0, 100)}`;
    }

    const fileContent = fileResponse.getBlob().getDataAsString("UTF-8");
    Logger.log("Successfully downloaded Yemot file. Content length: " + fileContent.length);
    return fileContent;

  } catch (e) {
    Logger.log("Exception during Yemot file download: " + e.message);
    return `שגיאה: אירעה חריגה במהלך ניסיון ההורדה. ${e.message}`;
  }
}

// -------------------------------------------------------------------
// מאגר ידע מ-Google Drive
// -------------------------------------------------------------------

function getKnowledgeFileList(folderId) {
  if (!folderId) return [];

  try {
    const folder = DriveApp.getFolderById(folderId);
    const files = folder.getFiles();
    const fileList = [];

    while (files.hasNext()) {
      const file = files.next();
      const mimeType = file.getMimeType();
      if (mimeType.startsWith("text/") || mimeType === MimeType.PLAIN_TEXT || mimeType === MimeType.GOOGLE_DOCS) {
        fileList.push(file.getName());
      }
    }
    return fileList;
  } catch (e) {
    Logger.log("Error getting file list from Drive: " + e.message);
    return [];
  }
}

function getKnowledgeFileContent(fileName, folderId) {
  if (!folderId || !fileName) {
    return "שגיאה: לא סופק שם קובץ או מזהה תיקייה.";
  }

  const fileNames = fileName.split(',').map(name => name.trim()).filter(name => name.length > 0);
  if (fileNames.length === 0) {
    return "שגיאה: לא סופקו שמות קבצים חוקיים.";
  }

  let combinedContent = "";

  try {
    const folder = DriveApp.getFolderById(folderId);

    for (const name of fileNames) {
      const files = folder.getFilesByName(name);

      if (files.hasNext()) {
        const file = files.next();
        const mimeType = file.getMimeType();
        let content = "";

        if (mimeType.startsWith("text/") || mimeType === MimeType.PLAIN_TEXT) {
          content = file.getBlob().getDataAsString("UTF-8");
        } else if (mimeType === MimeType.GOOGLE_DOCS) {
          content = DocumentApp.openById(file.getId()).getBody().getText();
        } else {
          Logger.log(`Skipping file '${name}' as it is not text or Google Doc: ${mimeType}`);
          combinedContent += `שגיאה: הקובץ '${name}' אינו קובץ טקסט או Google Doc (${mimeType}).\n\n`;
          continue;
        }

        combinedContent += `--- התחלת תוכן קובץ: ${name} ---\n${content}\n\n`;
        Logger.log("Successfully fetched content for: " + name);
      } else {
        Logger.log("File not found in Drive: " + name);
        combinedContent += `שגיאה: הקובץ '${name}' לא נמצא במאגר הידע.\n\n`;
      }
    }

    return combinedContent.trim();
  } catch (e) {
    Logger.log("Error during file processing: " + e.message);
    return "שגיאה כללית במהלך עיבוד הקבצים: " + e.message;
  }
}

// -------------------------------------------------------------------
// אינטגרציה עם מודל Gemini (כולל Tool Calling)
// -------------------------------------------------------------------

function callGemini(userText, modelName, knowledgeFileList, geminiApiKey, driveFolderId, token) {
  knowledgeFileList = knowledgeFileList || [];
  const isPro = modelName && String(modelName).toLowerCase() === 'pro';
  const modelToUse = isPro ? 'gemini-2.5-pro' : 'gemini-2.5-flash';

  const geminiUrl = `${GEMINI_API_BASE_URL}${modelToUse}:generateContent?key=${geminiApiKey}`;

  const tools = [
    {
      functionDeclarations: [
        {
          name: "get_knowledge_file_content",
          description: "מחזיר את הטקסט המלא של מסמך ידע אחד ממאגר הידע (המסמך עלול להיות ארוך מאוד). השתמש בכלי רק לאחר שבחרת מסמך מתוך רשימת המסמכים שניתנה לך, ובקש קובץ אחד בלבד בכל קריאה.",
          parameters: {
            type: "OBJECT",
            properties: {
              fileName: {
                type: "STRING",
                description: "שם הקובץ המדויק מתוך הרשימה, כולל הסיומת .txt (לדוגמה: 'תפריט.txt'). קובץ אחד בכל קריאה."
              }
            },
            required: ["fileName"]
          }
        },
        {
          name: "get_yemot_ini_file_content",
          description: "קריאת תוכן קובץ INI קיים ממערכת הטלפון (ימות המשיח), כדי לשמר הגדרות קיימות לפני עדכון. זמין רק כאשר הבקשה כוללת טוקן; ללא טוקן הכלי מחזיר הודעה שהוא אינו זמין.",
          parameters: {
            type: "OBJECT",
            properties: {
              filePath: {
                type: "STRING",
                description: "הנתיב המלא לקובץ ה-ini במערכת הטלפונית, לדוגמה: '3/4/5/ivr.ini' או 'Messages.ini'"
              }
            },
            required: ["filePath"]
          }
        }
      ]
    }
  ];

  const fileListString = (knowledgeFileList.length > 0)
    ? "רשימת מסמכי הידע הזמינים (העבר לכלי את השם עם הסיומת \".txt\"):\n" +
      knowledgeFileList.map(function (n) { return String(n).replace(/\.txt$/i, ''); }).join('\n')
    : "אין קבצי ידע דינמיים זמינים.";

  // הנחיות מערכת למודל: פלט פעולות כשורות URL (ללא token) עם שורות הסבר
  const systemInstructions =
    "אתה מומחה להגדרת שלוחות במערכת ימות המשיח (IVR). ענה בעברית תמציתית ומדויקת.\n" +
    "עובדות יסוד: שלוחה היא תיקייה בנתיב כגון /1/2, וההגדרות שלה נמצאות בקובץ ext.ini שבתוכה. "
    + "לשלוחה חדשה חובה להגדיר type=. בחר תמיד את המודול הספציפי ביותר למשימה — למשל לניתוב שיחות קיימים "
    + "routing, routing_time, routing_yemot, nitoviya ו-queue, וכל אחד למטרה אחרת.\n" +
    "איסור המצאה: אין להמציא שמות פרמטרים או ערכים. כל מפתח שאתה כותב חייב להופיע במסמך ידע שקראת בשיחה הזו "
    + "באמצעות הכלי. אם אינך יודע — קרא את המסמך המתאים או שאל.\n" +
    "פורמט הפלט לעדכון הגדרות: לכל שלוחה שורת הסבר אחת ואחריה כתובת URL אחת שמרכזת את כל הפרמטרים של אותה שלוחה:\n" +
    "הסבר: <תיאור קצר של הפעולה>\n" +
    "https://www.call2all.co.il/ym/api/UpdateExtension?path=ivr2:/<שלוחה>&<param>=<value>&<param2>=<value2>\n" +
    "אין לכלול token בכתובת. אל תפצל פרמטרים של אותה שלוחה לכמה כתובות, ואל תכתוב יותר משורת 'הסבר:' אחת לכל כתובת.\n" +
    "הוסף שורת 'הבהרה:' להערות חשובות למשתמש. אל תצטט ואל תשכתב את תוכן מסמכי הידע. "
    + "אם חסר מידע חיוני — שאל שאלת הבהרה ממוקדת אחת בלבד.";

  const fullSystemInstructions = systemInstructions + "\n\n" + fileListString;

  const history = [
    {
      role: "user",
      parts: [{ text: userText }]
    }
  ];

  let currentTurn = 0;
  while (currentTurn < MAX_GEMINI_TURNS) {
    currentTurn++;
    Logger.log(`Gemini Turn ${currentTurn}`);

    const payload = {
      contents: history,
      tools: tools,
      systemInstruction: {
        parts: [{ text: fullSystemInstructions }]
      },
      generationConfig: {
        temperature: 0.0,
        maxOutputTokens: 8192
      },
      safetySettings: [
        { category: "HARM_CATEGORY_HARASSMENT", threshold: "BLOCK_NONE" },
        { category: "HARM_CATEGORY_HATE_SPEECH", threshold: "BLOCK_NONE" },
        { category: "HARM_CATEGORY_DANGEROUS_CONTENT", threshold: "BLOCK_NONE" }
      ]
    };

    const options = {
      method: 'post',
      contentType: 'application/json',
      payload: JSON.stringify(payload),
      muteHttpExceptions: true
    };

    let geminiResponse;
    try {
      geminiResponse = fetchWithRetry(geminiUrl, options);
    } catch (err) {
      const errorMessage = err.message || "";
      if (errorMessage.includes('code": 503') || errorMessage.includes("UNAVAILABLE")) {
        Logger.log("Caught 503 Overload Error.");
        throw new Error("⚠️ שגיאה: מודל זה עמוס כרגע (קוד 503). אנא נסה שוב מאוחר יותר או נסה מודל אחר.");
      }
      Logger.log("Error calling Gemini API: " + errorMessage);
      throw new Error("שגיאת API כללית: " + errorMessage);
    }

    const responseCode = geminiResponse.getResponseCode();
    const responseBody = geminiResponse.getContentText();

    if (responseCode !== 200) {
      Logger.log(`Gemini API Error: ${responseCode} ${responseBody}`);
      let errorMessageForUser = `שגיאה לא ידועה (קוד ${responseCode}). אנא נסה שוב.`;

      try {
        const originalJsonError = JSON.parse(responseBody);
        if (originalJsonError.error && originalJsonError.error.message) {
          errorMessageForUser = translateToHebrew(originalJsonError.error.message);
        }
      } catch (e) {
        Logger.log("Could not parse error JSON. Using default message.");
      }

      throw new Error(errorMessageForUser);
    }

    let jsonResponse;
    try {
      jsonResponse = JSON.parse(responseBody);
    } catch (e) {
      Logger.log(`Error parsing Gemini JSON response: ${e} | Response body: ${responseBody}`);
      throw new Error("Error parsing Gemini response: " + e.message);
    }

    if (!jsonResponse.candidates || jsonResponse.candidates.length === 0) {
      Logger.log("Gemini returned no candidates. Body: " + responseBody);
      throw new Error("Gemini returned an empty or invalid response.");
    }

    const candidate = jsonResponse.candidates[0];

    if (candidate.content) {
      history.push(candidate.content);
    } else {
      Logger.log("Warning: Candidate content was empty.");
    }

    const parts = (candidate.content && candidate.content.parts) ? candidate.content.parts : [];
    const functionCalls = parts
      .map(function (part) { return part && part.functionCall; })
      .filter(function (fc) { return !!fc; });

    if (functionCalls.length > 0) {
      const responseParts = [];

      for (let i = 0; i < functionCalls.length; i++) {
        const functionCall = functionCalls[i];
        const toolName = functionCall.name;
        let toolResultContent = "";
        Logger.log(`Gemini requested Tool: ${toolName} (FinishReason: ${candidate.finishReason})`);

        if (toolName === "get_knowledge_file_content") {
          const fileName = functionCall.args ? functionCall.args.fileName : "";
          Logger.log("Tool: get_knowledge_file_content with file: " + fileName);
          toolResultContent = getKnowledgeFileContent(fileName, driveFolderId);
        } else if (toolName === "get_yemot_ini_file_content") {
          const filePath = functionCall.args ? functionCall.args.filePath : "";
          Logger.log("Tool: get_yemot_ini_file_content with path: " + filePath);
          if (!token) {
            Logger.log("No token in request - returning local-mode notice for ini tool.");
            toolResultContent = "לא זמין במצב זה: האפליקציה קוראת את קובצי ה-ini באופן מקומי. הנח שהשלוחה חדשה, או בקש מהמשתמש את ההגדרות הקיימות.";
          } else {
            toolResultContent = downloadYemotFileContent(filePath, token);
          }
        } else {
          Logger.log("Gemini requested an unknown tool: " + toolName);
          toolResultContent = `שגיאה: הכלי '${toolName}' אינו מוכר.`;
        }

        responseParts.push({
          functionResponse: {
            name: toolName,
            response: {
              content: toolResultContent
            }
          }
        });
      }

      history.push({
        role: "function",
        parts: responseParts
      });

      Logger.log(`Added ${responseParts.length} tool result(s) to history. Continuing loop.`);

    } else if (candidate.finishReason === "STOP") {
      Logger.log("Gemini finished with STOP (and no tool call).");
      const responseText = parts
        .map(function (part) { return (part && typeof part.text === 'string') ? part.text : ''; })
        .join('')
        .trim();

      if (responseText) {
        return responseText;
      }

      const errorMessage = "⚠️ שגיאה: המודל סיים את הפעולה בהצלחה אך לא יצר שום תוכן. ייתכן שיש בעיה בבקשה או בבניית הפורמט הנדרש.";
      Logger.log(errorMessage);
      throw new Error(errorMessage);

    } else {
      throw new Error(`Gemini response was blocked or incomplete. Reason: ${candidate.finishReason} (and no tool call).`);
    }
  }

  throw new Error(`Failed to get a final response from Gemini after ${MAX_GEMINI_TURNS} turns.`);
}

// -------------------------------------------------------------------
// תרגום הודעות שגיאה לעברית
// -------------------------------------------------------------------

function translateToHebrew(textToTranslate) {
  if (!textToTranslate) return "";
  try {
    return LanguageApp.translate(textToTranslate, 'auto', 'iw');
  } catch (e) {
    return textToTranslate;
  }
}

// -------------------------------------------------------------------
// תמלול קבצי שמע באמצעות Google Cloud Speech-to-Text
// -------------------------------------------------------------------

function transcribeAudioWithGoogleCloud(audioBlob, googleCloudApiKey) {
  const speechApiUrl = `${SPEECH_API_BASE_URL}?key=${googleCloudApiKey}`;

  let audioBytes;
  try {
    audioBytes = audioBlob.getBytes();
  } catch (e) {
    Logger.log("Error getting bytes from blob: " + e);
    throw new Error("Could not read audio file bytes.");
  }

  const audioBase64 = Utilities.base64Encode(audioBytes);

  const payload = {
    config: {
      encoding: "LINEAR16",
      sampleRateHertz: 8000,
      languageCode: "he-IL",
      enableAutomaticPunctuation: true
    },
    audio: {
      content: audioBase64
    }
  };

  const options = {
    method: 'post',
    contentType: 'application/json',
    payload: JSON.stringify(payload),
    muteHttpExceptions: true
  };

  Logger.log("Sending audio to Google Speech-to-Text API...");
  const response = fetchWithRetry(speechApiUrl, options);

  const responseCode = response.getResponseCode();
  const responseBody = response.getContentText();

  if (responseCode !== 200) {
    Logger.log(`Google Speech API Error - Status: ${responseCode} | Body: ${responseBody}`);
    throw new Error(`Google Speech API returned status ${responseCode}: ${responseBody}`);
  }

  try {
    const jsonResponse = JSON.parse(responseBody);
    if (jsonResponse.results && jsonResponse.results.length > 0 &&
        jsonResponse.results[0].alternatives && jsonResponse.results[0].alternatives.length > 0) {
      const transcript = jsonResponse.results[0].alternatives[0].transcript;
      Logger.log("Transcription successful: " + transcript);
      return transcript;
    } else {
      Logger.log("Google Speech API returned no results (audio might be empty).");
      return "";
    }
  } catch (e) {
    Logger.log(`Error parsing Speech API response: ${e} | Body: ${responseBody}`);
    throw new Error("Failed to parse Google Speech API response.");
  }
}

// -------------------------------------------------------------------
// ביצוע קריאות HTTP עם ניסיונות חוזרים (Exponential Backoff + Jitter)
// -------------------------------------------------------------------

function fetchWithRetry(url, options) {
  let baseWaitTime = 1000;
  let response;

  const fetchOptions = options ? { ...options } : {};
  fetchOptions.muteHttpExceptions = true;

  for (let i = 0; i < MAX_HTTP_RETRIES; i++) {
    try {
      response = UrlFetchApp.fetch(url, fetchOptions);
      const responseCode = response.getResponseCode();

      // בדיקה לקודי שגיאה 503 (Unavailable) או 429 (Rate Limit) הדורשים ניסיון חוזר
      if (responseCode === 503 || responseCode === 429) {
        Logger.log(`Attempt ${i + 1}/${MAX_HTTP_RETRIES} failed with ${responseCode} for ${url}. Retrying in ${baseWaitTime / 1000}s...`);
        const jitter = Math.random() * 1000;
        Utilities.sleep(baseWaitTime + jitter);
        baseWaitTime *= 2;
      } else {
        return response;
      }
    } catch (e) {
      Logger.log(`Attempt ${i + 1}/${MAX_HTTP_RETRIES} failed with network exception: ${e.message}. Retrying in ${baseWaitTime / 1000}s...`);
      Utilities.sleep(baseWaitTime + Math.random() * 1000);
      baseWaitTime *= 2;
      if (i === MAX_HTTP_RETRIES - 1) {
        throw e;
      }
    }
  }

  Logger.log(`Failed all ${MAX_HTTP_RETRIES} retries for: ${url}`);
  return response;
}

// -------------------------------------------------------------------
// שליחת התראת מייל על נושאים חדשים שזוהו בפורום ע"י הוורקר
// -------------------------------------------------------------------

function handleNewTopicsAlert(params) {
  const secret = params.secret || params.admin_secret;
  // fail-closed: בלי ADMIN_SECRET מוגדר ב-Script Properties הבקשה נדחית.
  const expectedSecret = SCRIPT_PROPS['ADMIN_SECRET'] || '';

  if (!expectedSecret || !secret || secret !== expectedSecret) {
    Logger.log(expectedSecret
      ? "Unauthorized alert attempt."
      : "ADMIN_SECRET is not configured in Script Properties — alert rejected.");
    return ContentService.createTextOutput(JSON.stringify({
      status: "error",
      message: "Unauthorized: Invalid admin secret"
    })).setMimeType(ContentService.MimeType.JSON);
  }

  let topics = params.topics || [];
  if (typeof topics === 'string') {
    try {
      topics = JSON.parse(topics);
    } catch (e) {
      topics = [];
    }
  }

  if (!topics || topics.length === 0) {
    return ContentService.createTextOutput(JSON.stringify({
      status: "ok",
      message: "No topics provided"
    })).setMimeType(ContentService.MimeType.JSON);
  }

  const alertEmail = SCRIPT_PROPS['ALERT_EMAIL'] || Session.getActiveUser().getEmail();
  if (!alertEmail) {
    Logger.log("No recipient email found for alerts.");
    return ContentService.createTextOutput(JSON.stringify({
      status: "error",
      message: "ALERT_EMAIL is not configured"
    })).setMimeType(ContentService.MimeType.JSON);
  }

  const subject = `[התראת ימות המשיח] זוהו ${topics.length} נושאים חדשים בפורום ללא סיווג`;

  let htmlBody = `<h3>שלום,</h3>`;
  htmlBody += `<p>וורקר סנכרון התיעוד זיהה <b>${topics.length} נושאים חדשים</b> בקטגוריה 1 בפורום ימות המשיח שטרם סווגו:</p>`;
  htmlBody += `<ul>`;
  for (let i = 0; i < topics.length; i++) {
    const t = topics[i];
    const url = t.url || `https://f2.freeivr.co.il/topic/${t.tid}`;
    htmlBody += `<li><b>[TID ${t.tid}]</b> <a href="${url}" target="_blank">${t.title}</a> (מספר פוסטים: ${t.postcount || 1})</li>`;
  }
  htmlBody += `</ul>`;
  htmlBody += `<p>יש לעדכן את קובץ <code>scraper/topics_config.json</code> עם הסיווג הרצוי (התעלמות / יצירת קובץ / שרשור לקובץ קיים).</p>`;
  htmlBody += `<hr><small>נשלח אוטומטית ממערכת AI_yemot</small>`;

  try {
    MailApp.sendEmail({
      to: alertEmail,
      subject: subject,
      htmlBody: htmlBody
    });
    Logger.log(`Alert email sent to ${alertEmail} for ${topics.length} topics.`);
    return ContentService.createTextOutput(JSON.stringify({
      status: "ok",
      sent_to: alertEmail,
      count: topics.length
    })).setMimeType(ContentService.MimeType.JSON);
  } catch (err) {
    Logger.log(`Failed to send alert email: ${err.message}`);
    return ContentService.createTextOutput(JSON.stringify({
      status: "error",
      message: err.message
    })).setMimeType(ContentService.MimeType.JSON);
  }
}

