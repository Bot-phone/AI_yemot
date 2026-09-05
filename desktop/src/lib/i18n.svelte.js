/**
 * i18n infrastructure — Svelte 5 Runes based.
 *
 * How to add a new language:
 *   1. Add an entry to LOCALES below (code, nativeName, flag, dir: "rtl" | "ltr").
 *   2. Copy the "strings" object from "he" (or "en") and translate the values.
 *   3. Done — the language picker and interface direction adapt automatically.
 */

/** @type {Record<string, {code: string, nativeName: string, flag: string, dir: string, strings: Record<string, string>}>} */
export const LOCALES = {
  he: {
    code: "he",
    nativeName: "עברית",
    flag: "🇮🇱",
    dir: "rtl",
    strings: {
      app_title: "AI yemot — עורך קווי ימות המשיח",
      app_subtitle: "עורך קווי ימות המשיח מבוסס AI",
      language: "שפה",
      footer_credit: "נוצר ע״י בוט פון — מערכות טלפוניות חכמות",
      knowledge_btn: "📚 בסיס ידע מקומי",
      update_available: "🚀 עדכון זמין: v{version}",

      target_mode: "אופן העיבוד (יעד)",
      mode_script: "🌐 סקריפט (GAS)",
      mode_direct: "⚡ ספק AI ישיר",
      provider_label: "ספק AI ישיר",
      provider_gemini: "Google Gemini (מומלץ)",
      provider_openai: "OpenAI (ChatGPT)",
      provider_groq: "Groq (מהיר במיוחד)",
      provider_claude: "Anthropic Claude (מומלץ לסוכן)",
      model_label: "מודל AI",
      model_regular: "מודל רגיל (מהיר, פחות מורכב)",
      model_pro: "מודל Pro (מעמיק ומדויק ביותר)",
      model_small_recent: "מודל קטן עדכני",
      model_claude_regular: "Sonnet 5 (מומלץ)",
      model_claude_pro: "Opus 5",
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
      api_key_label: "מפתח API אישי",
      api_key_hint_direct: "חובה במצב ספק AI ישיר",
      api_key_hint_script: "אופציונלי — ללא מפתח ייעשה שימוש במפתח המוגדר בסקריפט (חובה למודל Pro)",
      api_key_required: "נדרש מפתח API אישי במצב ספק AI ישיר",
      api_key_placeholder: "הזן מפתח API...",
      script_url_label: "כתובת סקריפט מותאמת (GAS)",
      preview_mode: "תצוגה מקדימה ואישור פעולות (מומלץ)",
      logout_on_finish: "התנתק מהמערכת בסיום (Logout)",
      provider_custom: "ספק מותאם אישית (תואם OpenAI)",
      custom_url_label: "כתובת API מותאמת אישית (מלאה)",
      custom_url_placeholder: "לדוגמה: https://api.example.com/v1/chat/completions",
      custom_url_hint:
        "נתמך כל ספק עם API תואם OpenAI (OpenRouter, DeepSeek, Ollama וכדומה). ניתן להזין כתובת בסיס או כתובת מלאה — ההשלמה של /chat/completions תתבצע אוטומטית",
      custom_url_required: "נא להזין כתובת API מותאמת אישית המתחילה ב-http:// או https://",
      model_from_list: "בחירה מהרשימה",
      model_manual: "הזנה ידנית",
      model_manual_placeholder: "הזן מזהה מודל, לדוגמה: gpt-4o-mini",
      model_manual_required: "נא להזין שם מודל",

      task_label: "המשימה",
      task_hint: "תאר בעברית חופשית מה לשנות בקו",
      task_placeholder:
        "לדוגמה: הגדר שלוחה 1 כתפריט שמשמיע קובץ פתיח 000 ומאפשר להקיש 1 להשמעת שיעורים ו-2 לשליחת שיחה למנהל...",
      quick_presets: "משימות מוכנות:",
      preset_menu: "תפריט בחירה",
      preset_play: "השמעת קבצים",
      preset_record: "קבלת הקלטה",
      preset_human: "מענה אנושי",
      preset_menu_prompt:
        "הגדר שלוחה 1 כתפריט ראשי: הקשה 1 מעבירה לשלוחת השמעת שיעורים, 2 להשארת הודעה, 3 לניתוב למענה אנושי. אם לא הוקשה בחירה תוך 5 שניות - חזרה על התפריט.",
      preset_play_prompt:
        "הגדר שלוחה 2 להשמעת שיעורים מהחדש לישן, עם מקשי דילוג קדימה ואחורה, חזרה על הקובץ, והשמעת שם הקובץ לפני כל שיעור.",
      preset_record_prompt:
        "הגדר שלוחה 3 לקבלת הודעה מהמאזין: קודם שאל את שמו ומספר הטלפון שלו, אחר כך הקלט את ההודעה, ושלח את ההקלטה למייל.",
      preset_human_prompt:
        "הגדר שלוחה 4 לניתוב למענה אנושי בימים א-ה בין 9:00 ל-16:00, ומחוץ לשעות אלה השמע הודעה על שעות הפעילות והעבר להשארת הודעה.",

      propose_changes: "הצע שינויים",
      proposing_changes: "מבצע את המשימה…",
      preview_title: "📋 תצוגה מקדימה של הפעולות המוצעות",
      preview_hint: "סמן את הפעולות שברצונך לאשר ולבצע בפועל",
      execute_selected: "בצע פעולות מסומנות ✨",
      final_answer_title: "סיכום המערכת:",

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
      enter_mfa_code: "נא להזין קוד אימות",
      verifying_code: "מאמת קוד...",
      mfa_success: "האימות הושלם בהצלחה!",
      enter_task: "נא לתאר את המשימה לביצוע בקו",
      enter_yemot_token: "נא להזין טוקן של ימות המשיח",
      sending_to_script: "שולח בקשה לסקריפט...",
      request_success: "הבקשה הושלמה בהצלחה!",
      request_failed: "אירעה שגיאה בביצוע הבקשה",
      script_disabled: "הסקריפט המתאם הושבת על ידי המפעיל. ניתן לעבור למצב ישיר עם מפתח API משלך.",
      script_disabled_reason: "הסקריפט המתאם הושבת על ידי המפעיל: {reason}",
      no_actions_selected: "לא נבחרו פעולות לביצוע",
      executing_actions: "מבצע {count} פעולות מול ימות המשיח...",
      actions_done: "הסתיים! בוצעו {done} מתוך {total} פעולות.",
      file_load_error: "שגיאה בטעינת הקובץ: {error}",

      // ----- Agent run (direct mode) -----
      auto_apply_label: "החל שינויים אוטומטית (ללא אישור)",
      auto_apply_warning: "אזהרה: הפעולות יבוצעו מיד במערכת ללא תצוגה מקדימה ואישור.",
      include_tree_label: "צרף את עץ השלוחות לבקשה",
      include_tree_hint: "מוסיף תמונת מצב קצרה של השלוחות הקיימות כדי לשפר את דיוק המודל.",

      agent_panel_title: "מהלך העבודה",
      agent_turn_header: "שלב {turn} מתוך {max}",
      agent_starting: "מפעיל את הסוכן...",
      agent_started_with: "הסוכן פועל · {provider} · {model}",
      agent_cancel: "עצור ריצה",
      agent_cancelling: "מבטל את הריצה...",
      agent_retry_notice: "המודל עמוס, מנסה שוב ({attempt}/{max})",
      agent_thinking: "ממתין לתשובת המודל...",
      agent_tool_running: "מבצע...",
      agent_ms: "{ms} מ״ש",

      run_stats_line:
        "טוקנים: {in} נכנסים · {out} יוצאים · מטמון {cache}% · {turns} סבבים · {secs} שנ׳",
      run_cost_line: "עלות משוערת: ${cost} לפי מחירון {price_list_date}",
      run_cost_line_no_date: "עלות משוערת: ${cost}",
      run_cost_unknown: "עלות: לא ידועה (המודל אינו במחירון המובנה)",
      cost_note:
        "הערכה בלבד לפי מחירי המחירון הפומבי של הספק. חיוב בפועל עשוי להיות שונה: מסלול חינמי, הנחות או שינויי מחיר אינם ידועים לתוכנה.",
      cost_note_toggle: "מידע על אומדן העלות",
      agent_stop_end_turn: "הסתיים",
      agent_stop_max_turns: "הופסק — הגעה למספר הסבבים המרבי",
      agent_stop_cancelled: "בוטל ע״י המשתמש",
      agent_stop_error: "הסתיים בשגיאה",
      agent_stop_truncated: "התשובה נקטעה",
      agent_stop_refusal: "המודל סירב לבצע את הבקשה",
      agent_error_title: "שגיאה בריצת הסוכן",
      agent_session_expired:
        "החיבור לימות המשיח פג. יש להתחבר מחדש (אימות דו-שלבי) ולהריץ את הבקשה מחדש.",

      proposed_changes_title: "שינויים מוצעים",
      proposed_changes_hint: "סמן את השינויים לאישור. הטבלה מציגה את הערך הנוכחי מול הערך החדש.",
      applying_actions: "מבצע את הפעולות שנבחרו...",
      will_be_created: "תיווצר",
      risk_low: "סיכון נמוך",
      risk_overwrite: "דריסת ערכים קיימים",
      risk_destructive: "פעולה הרסנית",
      show_diff: "הצג שינויים",
      hide_diff: "הסתר שינויים",
      diff_key: "מפתח",
      diff_before: "ערך נוכחי",
      diff_after: "ערך חדש",
      diff_empty: "אין ערך",
      no_diff: "אין שינויים להצגה",
      warnings_title: "אזהרות",
      action_ok: "בוצע",
      action_failed: "נכשל",
      param_applied: "עודכן",
      param_not_applied: "לא עודכן",
      results_title: "תוצאות הביצוע",
      kind_set_params: "עדכון פרמטרים",
      kind_upload_file: "העלאת קובץ טקסט",
      no_run_id: "אין ריצה פעילה",

      // ----- Onboarding & settings -----
      onboarding_title: "2 צעדים להתחלה",
      onboarding_step_token: "התחברות למערכת ימות המשיח ויצירת טוקן",
      onboarding_step_api_key: "הזנת מפתח API אישי של ספק ה-AI",
      onboarding_open_login: "התחברות עכשיו",
      secrets_note: "הטוקן והמפתחות נשמרים במאגר הסודות של מערכת ההפעלה, לא בקובץ טקסט",

      // ----- Approval panel -----
      select_all: "סמן הכל",
      clear_all: "נקה הכל",
      approve_selected_count: "בצע {count} פעולות שנבחרו",
      confirm_risky_title: "אישור שינוי משמעותי",
      confirm_risky_line:
        "אתה עומד לשנות {settings} הגדרות ב-{paths} שלוחות, מתוכן {overwrites} דריסות של ערכים קיימים.",
      confirm_risky_approve: "אני מאשר — בצע",
      action_already_applied: "בוצע",
      undo_action: "בטל שינוי",
      undo_done: "השינוי בוטל",
      undo_failed: "ביטול השינוי נכשל",
      undo_unavailable: "ביטול השינוי אינו זמין בגרסה זו",
      undo_previous_params: "ערכים קודמים",
      undo_previous_content: "תוכן קודם של הקובץ",
      value_label: "ערך",
      value_empty: "(ריק)",
      value_missing: "(לא היה קיים)",
      keychain_write_failed:
        "לא ניתן לשמור את הסודות במחסן המפתחות של מערכת ההפעלה. הערכים נשמרים לשימוש בהפעלה הנוכחית בלבד ונשארים בינתיים באחסון המקומי.",

      // ----- Run feedback -----
      agent_elapsed: "{secs} שנ׳",
      retry_run: "נסה שוב",
      prompt_submit_hint: "Ctrl+Enter (או Cmd+Enter) לשליחה",
      copy_code: "העתק",
      copied: "הועתק",
      copy_output: "העתק תשובה",

      // ----- Line editor workspace -----
      connected_to: "מחובר למערכת {system}",
      connected: "מחובר",
      not_connected: "לא מחובר",
      open_settings: "הגדרות",
      settings_drawer_title: "הגדרות",
      settings_close: "סגור הגדרות",
      legacy_section: "מתקדם",
      legacy_section_hint:
        "מצב סקריפט (GAS) נשמר לתאימות לאחור בלבד. עורך הקו עובד מול ספק AI ישיר.",

      tree_title: "מבנה הקו",
      tree_refresh: "רענן",
      tree_search: "סינון שלוחות…",
      tree_loading: "טוען את מבנה הקו…",
      tree_empty: "לא נמצאו שלוחות",
      tree_no_match: "אין שלוחה התואמת את הסינון",
      tree_needs_token: "התחבר למערכת כדי לראות את מבנה הקו",
      tree_error: "טעינת מבנה הקו נכשלה: {error}",
      tree_unavailable: "תצוגת מבנה הקו אינה זמינה בגרסה זו",
      tree_expand: "פתח",
      tree_collapse: "סגור",

      inspector_title: "פרטי שלוחה",
      inspector_empty: "בחר שלוחה מהעץ כדי לראות את פרטיה",
      inspector_loading: "טוען את פרטי השלוחה…",
      inspector_error: "טעינת השלוחה נכשלה: {error}",
      inspector_unavailable: "פרטי שלוחה אינם זמינים בגרסה זו",
      inspector_path: "נתיב",
      inspector_type: "סוג",
      inspector_missing: "השלוחה אינה קיימת בקו",
      inspector_params: "פרמטרים",
      inspector_no_params: "אין פרמטרים",
      inspector_files: "קבצים",
      inspector_no_files: "אין קבצים",
      inspector_show_raw: "הצג ext.ini גולמי",
      inspector_hide_raw: "הסתר ext.ini גולמי",
      inspector_task_on_ext: "משימה על שלוחה זו",
      inspector_refresh: "רענן",
      copy_value: "העתק ערך",

      context_title: "הקשר המשימה",
      context_hint: "השלוחות האלה יצורפו לתיאור המשימה",
      context_remove: "הסר מההקשר",
      context_prefix: "[הקשר] השלוחות הנבחרות: {paths}",

      attach_audio: "צרף קובצי שמע",
      attachments_title: "קבצים מצורפים",
      attachment_remove: "הסר קובץ",
      attach_failed: "בחירת הקבצים נכשלה: {error}",
      attach_unavailable: "צירוף קבצים אינו זמין בגרסה זו",

      presets_edit: "ערוך",
      preset_add: "הוסף משימה מוכנה",
      preset_edit_btn: "ערוך",
      preset_delete: "מחק",
      preset_save: "שמור",
      preset_cancel: "בטל",
      preset_restore: "שחזר ברירת מחדל",
      preset_title_placeholder: "שם קצר",
      preset_text_placeholder: "תיאור המשימה…",
      preset_done: "סיום עריכה",

      refine_title: "דייק את המשימה",
      refine_placeholder: "מה לתקן או להוסיף?",
      refine_send: "המשך",
      refine_parent: "משימה בהמשך ל-{previous}",
      refine_unavailable: "המשך משימה אינו זמין בגרסה זו",
      refine_parent_gone: "לא ניתן להמשיך את המשימה הקודמת. התחל משימה חדשה.",

      changelog_title: "יומן שינויים",
      changelog_refresh: "רענן",
      changelog_empty: "לא בוצעו שינויים",
      changelog_error: "טעינת יומן השינויים נכשלה: {error}",
      changelog_unavailable: "יומן השינויים אינו זמין בגרסה זו",
      changelog_done: "בוצע",
      changelog_undone: "בוטל",

      kind_upload_audio: "העלאת קובץ שמע",

      // ----- Accessibility -----
      close: "סגור",
      back: "חזור"
    }
  },
  en: {
    code: "en",
    nativeName: "English",
    flag: "🇺🇸",
    dir: "ltr",
    strings: {
      app_title: "AI yemot — Yemot HaMashiach line editor",
      app_subtitle: "An AI-based editor for Yemot HaMashiach lines",
      language: "Language",
      footer_credit: "Created by Bot Phone — Smart phone systems",
      knowledge_btn: "📚 Local knowledge base",
      update_available: "🚀 Update available: v{version}",

      target_mode: "Processing mode (target)",
      mode_script: "🌐 Script (GAS)",
      mode_direct: "⚡ Direct AI provider",
      provider_label: "Direct AI provider",
      provider_gemini: "Google Gemini (recommended)",
      provider_openai: "OpenAI (ChatGPT)",
      provider_groq: "Groq (extra fast)",
      provider_claude: "Anthropic Claude (recommended for the agent)",
      model_label: "AI model",
      model_regular: "Regular model (fast, less complex)",
      model_pro: "Pro model (deepest & most accurate)",
      model_small_recent: "Small up-to-date model",
      model_claude_regular: "Sonnet 5 (recommended)",
      model_claude_pro: "Opus 5",
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
      api_key_label: "Personal API key",
      api_key_hint_direct: "Required in direct AI provider mode",
      api_key_hint_script: "Optional — without it the script's configured key is used (required for Pro model)",
      api_key_required: "A personal API key is required in direct AI provider mode",
      api_key_placeholder: "Enter API key...",
      script_url_label: "Custom script URL (GAS)",
      preview_mode: "Preview & approve actions (recommended)",
      logout_on_finish: "Log out from system when done (Logout)",
      provider_custom: "Custom provider (OpenAI-compatible)",
      custom_url_label: "Custom full API URL",
      custom_url_placeholder: "e.g. https://api.example.com/v1/chat/completions",
      custom_url_hint:
        "Any OpenAI-compatible provider is supported (OpenRouter, DeepSeek, Ollama, etc.). Enter a base URL or the full endpoint — /chat/completions is appended automatically",
      custom_url_required: "Please enter a custom API URL starting with http:// or https://",
      model_from_list: "Choose from list",
      model_manual: "Manual entry",
      model_manual_placeholder: "Enter a model ID, e.g. gpt-4o-mini",
      model_manual_required: "Please enter a model name",

      task_label: "The task",
      task_hint: "Describe freely what should change on the line",
      task_placeholder:
        "Example: Set extension 1 as a menu that plays opening file 000, allows pressing 1 to play lessons and 2 to transfer a call to the manager...",
      quick_presets: "Ready-made tasks:",
      preset_menu: "Choice menu",
      preset_play: "Play files",
      preset_record: "Receive recording",
      preset_human: "Human answering",
      preset_menu_prompt:
        "Set extension 1 as the main menu: pressing 1 goes to the lessons playback extension, 2 leaves a message, 3 routes to a human operator. If no key is pressed within 5 seconds - repeat the menu.",
      preset_play_prompt:
        "Set extension 2 to play lessons from newest to oldest, with forward and backward skip keys, repeat of the current file, and announcing the file name before each lesson.",
      preset_record_prompt:
        "Set extension 3 to take a message from the caller: first ask for their name and phone number, then record the message, and send the recording by email.",
      preset_human_prompt:
        "Set extension 4 to route to a human operator Sunday to Thursday between 9:00 and 16:00; outside those hours play a message with the opening hours and transfer to leaving a message.",

      propose_changes: "Propose changes",
      proposing_changes: "Working on the task…",
      preview_title: "📋 Preview of the proposed actions",
      preview_hint: "Mark the actions you want to approve and actually perform",
      execute_selected: "Run selected actions ✨",
      final_answer_title: "System summary:",

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
      enter_mfa_code: "Please enter the verification code",
      verifying_code: "Verifying code...",
      mfa_success: "Authentication completed successfully!",
      enter_task: "Please describe the task to perform on the line",
      enter_yemot_token: "Please enter a Yemot HaMashiach token",
      sending_to_script: "Sending request to script...",
      request_success: "Request completed successfully!",
      request_failed: "An error occurred while executing the request",
      script_disabled: "The relay script was disabled by its operator. You can switch to direct mode with your own API key.",
      script_disabled_reason: "The relay script was disabled by its operator: {reason}",
      no_actions_selected: "No actions selected",
      executing_actions: "Executing {count} actions against Yemot HaMashiach...",
      actions_done: "Done! {done} of {total} actions were executed.",
      file_load_error: "Failed to load file: {error}",

      // ----- Agent run (direct mode) -----
      auto_apply_label: "Apply changes automatically (no approval)",
      auto_apply_warning: "Warning: actions run against the system immediately, with no preview or approval.",
      include_tree_label: "Attach the extension tree to the request",
      include_tree_hint: "Adds a short snapshot of the existing extensions to improve the model's accuracy.",

      agent_panel_title: "Run progress",
      agent_turn_header: "Step {turn} of {max}",
      agent_starting: "Starting the agent...",
      agent_started_with: "Agent running · {provider} · {model}",
      agent_cancel: "Stop run",
      agent_cancelling: "Cancelling the run...",
      agent_retry_notice: "The model is busy, retrying ({attempt}/{max})",
      agent_thinking: "Waiting for the model's response...",
      agent_tool_running: "Running...",
      agent_ms: "{ms} ms",

      run_stats_line:
        "Tokens: {in} in · {out} out · cache {cache}% · {turns} turns · {secs}s",
      run_cost_line: "Estimated cost: ${cost} per the {price_list_date} price list",
      run_cost_line_no_date: "Estimated cost: ${cost}",
      run_cost_unknown: "Cost: unknown (the model is not in the built-in price list)",
      cost_note:
        "An estimate only, based on the provider's public list prices. The actual charge may differ: free tiers, discounts or price changes are not known to the app.",
      cost_note_toggle: "About the cost estimate",
      agent_stop_end_turn: "Finished",
      agent_stop_max_turns: "Stopped — maximum number of turns reached",
      agent_stop_cancelled: "Cancelled by the user",
      agent_stop_error: "Finished with an error",
      agent_stop_truncated: "The response was truncated",
      agent_stop_refusal: "The model refused to perform the request",
      agent_error_title: "Agent run error",
      agent_session_expired:
        "The Yemot HaMashiach session expired. Please sign in again (two-factor) and re-run the request.",

      proposed_changes_title: "Proposed changes",
      proposed_changes_hint: "Mark the changes to approve. The table shows the current value against the new one.",
      applying_actions: "Running the selected actions...",
      will_be_created: "will be created",
      risk_low: "Low risk",
      risk_overwrite: "Overwrites existing values",
      risk_destructive: "Destructive action",
      show_diff: "Show changes",
      hide_diff: "Hide changes",
      diff_key: "Key",
      diff_before: "Current value",
      diff_after: "New value",
      diff_empty: "no value",
      no_diff: "No changes to show",
      warnings_title: "Warnings",
      action_ok: "Done",
      action_failed: "Failed",
      param_applied: "updated",
      param_not_applied: "not updated",
      results_title: "Execution results",
      kind_set_params: "Update parameters",
      kind_upload_file: "Upload text file",
      no_run_id: "No active run",

      // ----- Onboarding & settings -----
      onboarding_title: "2 steps to get started",
      onboarding_step_token: "Sign in to Yemot HaMashiach and create a token",
      onboarding_step_api_key: "Enter your personal AI provider API key",
      onboarding_open_login: "Sign in now",
      secrets_note: "The token and keys are kept in the operating system's credential store, not in a text file",

      // ----- Approval panel -----
      select_all: "Select all",
      clear_all: "Clear all",
      approve_selected_count: "Run {count} selected actions",
      confirm_risky_title: "Confirm a significant change",
      confirm_risky_line:
        "You are about to change {settings} settings across {paths} extensions, {overwrites} of them overwriting existing values.",
      confirm_risky_approve: "I confirm — run it",
      action_already_applied: "Applied",
      undo_action: "Undo change",
      undo_done: "The change was undone",
      undo_failed: "Undo failed",
      undo_unavailable: "Undo is not available in this version",
      undo_previous_params: "Previous values",
      undo_previous_content: "Previous file contents",
      value_label: "Value",
      value_empty: "(empty)",
      value_missing: "(did not exist)",
      keychain_write_failed:
        "The secrets could not be stored in the OS keychain. They are kept for this session only and stay in local storage for now.",

      // ----- Run feedback -----
      agent_elapsed: "{secs}s",
      retry_run: "Try again",
      prompt_submit_hint: "Ctrl+Enter (or Cmd+Enter) to submit",
      copy_code: "Copy",
      copied: "Copied",
      copy_output: "Copy answer",

      // ----- Line editor workspace -----
      connected_to: "Connected to system {system}",
      connected: "Connected",
      not_connected: "Not connected",
      open_settings: "Settings",
      settings_drawer_title: "Settings",
      settings_close: "Close settings",
      legacy_section: "Advanced",
      legacy_section_hint:
        "Script (GAS) mode is kept for backwards compatibility only. The line editor works against a direct AI provider.",

      tree_title: "Line structure",
      tree_refresh: "Refresh",
      tree_search: "Filter extensions…",
      tree_loading: "Loading the line structure…",
      tree_empty: "No extensions found",
      tree_no_match: "No extension matches the filter",
      tree_needs_token: "Sign in to the system to see the line structure",
      tree_error: "Failed to load the line structure: {error}",
      tree_unavailable: "The line structure view is not available in this version",
      tree_expand: "Expand",
      tree_collapse: "Collapse",

      inspector_title: "Extension details",
      inspector_empty: "Pick an extension from the tree to see its details",
      inspector_loading: "Loading the extension…",
      inspector_error: "Failed to load the extension: {error}",
      inspector_unavailable: "Extension details are not available in this version",
      inspector_path: "Path",
      inspector_type: "Type",
      inspector_missing: "This extension does not exist on the line",
      inspector_params: "Parameters",
      inspector_no_params: "No parameters",
      inspector_files: "Files",
      inspector_no_files: "No files",
      inspector_show_raw: "Show raw ext.ini",
      inspector_hide_raw: "Hide raw ext.ini",
      inspector_task_on_ext: "Task on this extension",
      inspector_refresh: "Refresh",
      copy_value: "Copy value",

      context_title: "Task context",
      context_hint: "These extensions are attached to the task description",
      context_remove: "Remove from context",
      context_prefix: "[Context] Selected extensions: {paths}",

      attach_audio: "Attach audio files",
      attachments_title: "Attached files",
      attachment_remove: "Remove file",
      attach_failed: "Choosing files failed: {error}",
      attach_unavailable: "Attaching files is not available in this version",

      presets_edit: "Edit",
      preset_add: "Add a ready-made task",
      preset_edit_btn: "Edit",
      preset_delete: "Delete",
      preset_save: "Save",
      preset_cancel: "Cancel",
      preset_restore: "Restore defaults",
      preset_title_placeholder: "Short name",
      preset_text_placeholder: "Task description…",
      preset_done: "Done editing",

      refine_title: "Refine the task",
      refine_placeholder: "What should be fixed or added?",
      refine_send: "Continue",
      refine_parent: "Task continuing from {previous}",
      refine_unavailable: "Task continuation is not available in this version",
      refine_parent_gone:
        "The previous task can no longer be continued. Start a new task.",

      changelog_title: "Change log",
      changelog_refresh: "Refresh",
      changelog_empty: "No changes were applied",
      changelog_error: "Failed to load the change log: {error}",
      changelog_unavailable: "The change log is not available in this version",
      changelog_done: "Applied",
      changelog_undone: "Undone",

      kind_upload_audio: "Upload audio file",

      // ----- Accessibility -----
      close: "Close",
      back: "Back"
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

/** Keys already reported by `warnMissingKey`, so the console stays readable. */
const warnedKeys = new Set();

/**
 * In development, tell the developer once about a key that has no translation
 * in either the active or the default language.
 * @param {string} key
 */
function warnMissingKey(key) {
  if (!import.meta.env?.DEV) return;
  if (warnedKeys.has(key)) return;
  warnedKeys.add(key);
  console.warn(`[i18n] missing translation key: "${key}"`);
}

/**
 * Translate a key in the current language (falls back to the default
 * language, then to the key itself). Supports {param} interpolation.
 * @param {string} key
 * @param {Record<string, unknown>} [params]
 * @returns {string}
 */
export function t(key, params) {
  const active = currentLocale().strings[key];
  const fallback = LOCALES[DEFAULT_LOCALE].strings[key];
  if (active === undefined && fallback === undefined) warnMissingKey(key);
  else if (active === undefined) warnMissingKey(`${key} (${i18n.locale})`);
  let text = active ?? fallback ?? key;
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

/**
 * Switch language, persist the choice and update direction.
 * @param {string} code
 */
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
