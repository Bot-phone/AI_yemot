/**
 * i18n infrastructure — Svelte 5 Runes based.
 *
 * How to add a new language:
 *   1. Add an entry to LOCALES below (code, nativeName, flag, dir: "rtl" | "ltr").
 *   2. Copy the "strings" object from "he" (or "en") and translate the values.
 *   3. Done — the language picker and interface direction adapt automatically.
 */

export const LOCALES = {
  he: {
    code: "he",
    nativeName: "עברית",
    flag: "🇮🇱",
    dir: "rtl",
    strings: {
      app_title: "AI yemot — עדכון שלוחות חכם",
      language: "שפה",
      footer_credit: "נוצר ע״י בוט פון — מערכות טלפוניות חכמות",
      knowledge_btn: "📚 בסיס ידע מקומי",
      update_available: "🚀 עדכון זמין: v{version}",

      settings_title: "⚙️ הגדרות יעד וחיבור",
      target_mode: "אופן העיבוד (יעד)",
      mode_script: "🌐 סקריפט (GAS)",
      mode_direct: "⚡ ספק AI ישיר",
      provider_label: "ספק AI ישיר",
      provider_gemini: "Google Gemini (מומלץ)",
      provider_openai: "OpenAI (ChatGPT)",
      provider_groq: "Groq (מהיר במיוחד)",
      model_label: "מודל AI",
      model_regular: "מודל רגיל (מהיר, פחות מורכב)",
      model_pro: "מודל Pro (מעמיק ומדויק ביותר)",
      token_label: "טוקן ימות המשיח",
      get_token: "קבלת טוקן חדש",
      clear_token: "מחיקת הטוקן השמור",
      local_note: "הטוקן נשמר מקומית בלבד ואינו נשלח לשום שרת חוץ — כל הפעולות מול המערכת מתבצעות מקומית",
      login_title: "יצירת טוקן — התחברות למערכת",
      system_number: "מספר מערכת",
      system_number_placeholder: "לדוגמה: 0773137770",
      password: "סיסמה",
      password_placeholder: "הזן סיסמת מנהל",
      login_btn: "התחברות",
      login_success: "הטוקן נוצר והוגדר בהצלחה!",
      mfa_methods: "שיטת אימות",
      no_mfa_methods: "לא נמצאו שיטות אימות זמינות",
      send_code: "שלח קוד",
      code_sent: "הקוד נשלח. הזן את הקוד שהתקבל:",
      validate_code: "אישור קוד",
      enter_system_and_password: "יש להזין מספר מערכת וסיסמה",
      show: "הצג",
      hide: "הסתר",
      token_placeholder: "הזן טוקן מערכת...",
      check: "בדוק",
      api_type_label: "מפתח AI",
      api_system: "מפתח מערכת ברירת מחדל",
      api_private: "מפתח API אישי",
      api_key_label: "מפתח API אישי",
      api_key_hint_direct: "חובה במצב ספק AI ישיר",
      api_key_hint_script: "אופציונלי — ללא מפתח ייעשה שימוש במפתח המוגדר בסקריפט (חובה למודל Pro)",
      api_key_required: "נדרש מפתח API אישי במצב ספק AI ישיר",
      api_key_placeholder: "הזן מפתח API...",
      script_url_label: "כתובת סקריפט מותאמת (GAS)",
      preview_mode: "תצוגה מקדימה ואישור פעולות (מומלץ)",
      logout_on_finish: "התנתק מהמערכת בסיום (Logout)",

      prompt_label: "הנחיות לעדכון שלוחה במערכת",
      prompt_hint: "תאר בעברית חופשית",
      prompt_placeholder:
        "לדוגמה: הגדר שלוחה 1 כתפריט שמשמיע קובץ פתיח 000 ומאפשר להקיש 1 להשמעת שיעורים ו-2 לשליחת שיחה למנהל...",
      quick_presets: "הצעות מהירות:",
      preset_menu: "תפריט בחירה",
      preset_play: "השמעת קבצים",
      preset_record: "קבלת הקלטה",
      preset_menu_prompt: "הגדר שלוחה 1 כתפריט בחירה ראשי עם מעבר לשלוחות 1, 2 ו-3",
      preset_play_prompt: "הגדר שלוחה 2 להשמעת קבצים עם אפשרות דילוג וחזרה",
      preset_record_prompt: "הגדר שלוחה 3 לקבלת נתונים והקלטה מהמאזין עם שליחה למייל",

      processing: "מעבד בקשה...",
      submit: "שגר עדכון לשלוחה 🚀",
      preview_title: "📋 תצוגה מקדימה של הפעולות המוצעות",
      preview_hint: "סמן את הפעולות שברצונך לאשר ולבצע בפועל",
      execute_selected: "בצע פעולות מסומנות ✨",
      raw_output_title: "תשובת המערכת:",

      knowledge_modal_title: "בסיס ידע מוטמע ({count} קבצים בזיכרון)",
      search_knowledge: "חיפוש קובץ ידע...",
      select_file_hint: "בחר קובץ מהרשימה לצפייה בתיעוד",
      view_formatted: "תצוגה מעוצבת",
      view_raw: "קוד גולמי",
      copy_file: "העתק תוכן",
      file_copied: "התוכן הועתק ללוח!",

      mfa_modal_title: "אימות דו-שלבי (MFA) נדרש",
      mfa_modal_desc:
        "מערכת ימות המשיח דורשת אימות טלפוני לפני ביצוע שינויים. בחר אופן קבלת הקוד:",
      mfa_call: "שיחה קולית",
      mfa_sms: "מסרון (SMS)",
      mfa_send_code: "שלח קוד אימות 📞",
      mfa_code_label: "הזן את הקוד שהתקבל:",
      mfa_code_placeholder: "קוד בן 4-6 ספרות",
      cancel: "ביטול",
      mfa_verify: "אמת והמשך ✓",

      // Status / error messages
      enter_token_first: "נא להזין טוקן תחילה",
      checking_token: "בודק תקינות טוקן מול ימות המשיח...",
      mfa_required: "נדרש אימות דו-שלבי (MFA)",
      comm_error: "שגיאת תקשורת: {error}",
      sending_code: "שולח קוד אימות...",
      error: "שגיאה: {error}",
      enter_mfa_code: "נא להזין קוד אימות",
      verifying_code: "מאמת קוד...",
      mfa_success: "האימות הושלם בהצלחה!",
      enter_prompt: "נא להזין הנחיות או בקשה לעדכון שלוחה",
      enter_yemot_token: "נא להזין טוקן של ימות המשיח",
      sending_to_script: "שולח בקשה לסקריפט...",
      processing_ai: "מעבד מול מודל ה-AI...",
      request_success: "הבקשה הושלמה בהצלחה!",
      request_failed: "אירעה שגיאה בביצוע הבקשה",
      no_actions_selected: "לא נבחרו פעולות לביצוע",
      executing_actions: "מבצע {count} פעולות מול ימות המשיח...",
      actions_done: "הסתיים! בוצעו {done} מתוך {total} פעולות.",
      file_load_error: "שגיאה בטעינת הקובץ: {error}"
    }
  },
  en: {
    code: "en",
    nativeName: "English",
    flag: "🇺🇸",
    dir: "ltr",
    strings: {
      app_title: "AI yemot — Smart extension updates",
      language: "Language",
      footer_credit: "Created by Bot Phone — Smart phone systems",
      knowledge_btn: "📚 Local knowledge base",
      update_available: "🚀 Update available: v{version}",

      settings_title: "⚙️ Target & connection settings",
      target_mode: "Processing mode (target)",
      mode_script: "🌐 Script (GAS)",
      mode_direct: "⚡ Direct AI provider",
      provider_label: "Direct AI provider",
      provider_gemini: "Google Gemini (recommended)",
      provider_openai: "OpenAI (ChatGPT)",
      provider_groq: "Groq (extra fast)",
      model_label: "AI model",
      model_regular: "Regular model (fast, less complex)",
      model_pro: "Pro model (deepest & most accurate)",
      token_label: "Yemot HaMashiach token",
      get_token: "Get new token",
      clear_token: "Clear saved token",
      local_note: "The token is stored locally only and is never sent to external servers — all actions against the system run locally",
      login_title: "Create token — system login",
      system_number: "System number",
      system_number_placeholder: "e.g. 0773137770",
      password: "Password",
      password_placeholder: "Enter admin password",
      login_btn: "Log in",
      login_success: "Token created and set successfully!",
      mfa_methods: "Verification method",
      no_mfa_methods: "No verification methods available",
      send_code: "Send code",
      code_sent: "Code sent. Enter the code you received:",
      validate_code: "Verify code",
      enter_system_and_password: "Please enter system number and password",
      show: "Show",
      hide: "Hide",
      token_placeholder: "Enter system token...",
      check: "Check",
      api_type_label: "AI key",
      api_system: "Default system key",
      api_private: "Personal API key",
      api_key_label: "Personal API key",
      api_key_hint_direct: "Required in direct AI provider mode",
      api_key_hint_script: "Optional — without it the script's configured key is used (required for Pro model)",
      api_key_required: "A personal API key is required in direct AI provider mode",
      api_key_placeholder: "Enter API key...",
      script_url_label: "Custom script URL (GAS)",
      preview_mode: "Preview & approve actions (recommended)",
      logout_on_finish: "Log out from system when done (Logout)",

      prompt_label: "Instructions for updating an extension",
      prompt_hint: "Describe freely in natural language",
      prompt_placeholder:
        "Example: Set extension 1 as a menu that plays opening file 000, allows pressing 1 to play lessons and 2 to transfer a call to the manager...",
      quick_presets: "Quick presets:",
      preset_menu: "Choice menu",
      preset_play: "Play files",
      preset_record: "Receive recording",
      preset_menu_prompt: "Set extension 1 as a main selection menu with routing to extensions 1, 2 and 3",
      preset_play_prompt: "Set extension 2 to play files with skip and replay options",
      preset_record_prompt: "Set extension 3 to receive data and a recording from the caller with email delivery",

      processing: "Processing request...",
      submit: "Launch extension update 🚀",
      preview_title: "📋 Preview of the proposed actions",
      preview_hint: "Mark the actions you want to approve and actually perform",
      execute_selected: "Run selected actions ✨",
      raw_output_title: "System response:",

      knowledge_modal_title: "Embedded knowledge base ({count} files in memory)",
      search_knowledge: "Search knowledge file...",
      select_file_hint: "Select a file from the list to view documentation",
      view_formatted: "Formatted View",
      view_raw: "Raw Text",
      copy_file: "Copy Content",
      file_copied: "Content copied to clipboard!",

      mfa_modal_title: "Two-factor authentication (MFA) required",
      mfa_modal_desc:
        "The Yemot HaMashiach system requires phone verification before making changes. Choose how to receive the code:",
      mfa_call: "Voice call",
      mfa_sms: "SMS",
      mfa_send_code: "Send verification code 📞",
      mfa_code_label: "Enter the code you received:",
      mfa_code_placeholder: "4-6 digit code",
      cancel: "Cancel",
      mfa_verify: "Verify & continue ✓",

      // Status / error messages
      enter_token_first: "Please enter a token first",
      checking_token: "Verifying token with Yemot HaMashiach...",
      mfa_required: "Two-factor authentication (MFA) required",
      comm_error: "Communication error: {error}",
      sending_code: "Sending verification code...",
      error: "Error: {error}",
      enter_mfa_code: "Please enter the verification code",
      verifying_code: "Verifying code...",
      mfa_success: "Authentication completed successfully!",
      enter_prompt: "Please enter instructions or an extension update request",
      enter_yemot_token: "Please enter a Yemot HaMashiach token",
      sending_to_script: "Sending request to script...",
      processing_ai: "Processing with the AI model...",
      request_success: "Request completed successfully!",
      request_failed: "An error occurred while executing the request",
      no_actions_selected: "No actions selected",
      executing_actions: "Executing {count} actions against Yemot HaMashiach...",
      actions_done: "Done! {done} of {total} actions were executed.",
      file_load_error: "Failed to load file: {error}"
    }
  }
};

export const DEFAULT_LOCALE = "he";
const STORAGE_KEY = "ai_yemot_locale";

/** Languages available in the picker (code, nativeName, flag, dir). */
export const availableLocales = Object.values(LOCALES).map((l) => ({
  code: l.code,
  nativeName: l.nativeName,
  flag: l.flag,
  dir: l.dir
}));

function getInitialLocale() {
  if (typeof localStorage === "undefined") return DEFAULT_LOCALE;
  try {
    const saved = localStorage.getItem(STORAGE_KEY);
    if (saved && LOCALES[saved]) return saved;
  } catch (_) {}
  return DEFAULT_LOCALE;
}

/** Reactive global language state (Svelte 5 runes). */
export const i18n = $state({ locale: getInitialLocale() });

function currentLocale() {
  return LOCALES[i18n.locale] ?? LOCALES[DEFAULT_LOCALE];
}

/**
 * Translate a key in the current language (falls back to the default
 * language, then to the key itself). Supports {param} interpolation.
 */
export function t(key, params) {
  let text =
    currentLocale().strings[key] ??
    LOCALES[DEFAULT_LOCALE].strings[key] ??
    key;
  if (params) {
    for (const [k, v] of Object.entries(params)) {
      text = text.replaceAll(`{${k}}`, String(v));
    }
  }
  return text;
}

/** Whether the current language is right-to-left. */
export function isRTL() {
  return currentLocale().dir === "rtl";
}

/** Apply language + direction to the document root (<html>). */
export function applyDocumentDirection() {
  if (typeof document === "undefined") return;
  const loc = currentLocale();
  document.documentElement.lang = loc.code;
  document.documentElement.dir = loc.dir;
}

/** Switch language, persist the choice and update direction. */
export function setLocale(code) {
  if (!LOCALES[code]) return;
  i18n.locale = code;
  try {
    localStorage.setItem(STORAGE_KEY, code);
  } catch (_) {}
  applyDocumentDirection();
}

// Apply saved direction on first load (default: Hebrew / RTL).
applyDocumentDirection();
