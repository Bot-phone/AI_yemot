<script>
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";

  // State (Svelte 5 Runes)
  let targetMode = $state("script"); // "script" | "direct"
  let aiProvider = $state("gemini"); // "gemini" | "openai" | "groq"
  let modelType = $state("regular"); // "regular" | "pro"
  let promptText = $state("");
  let yemotToken = $state("");
  let showToken = $state(false);
  let apiType = $state("system"); // "system" (2411) | "private"
  let apiKey = $state("");
  let showApiKey = $state(false);
  let isPreviewMode = $state(true);
  let logoutOnFinish = $state(false);
  let scriptUrl = $state("https://script.google.com/macros/s/AKfycbz_REPLACE_ME/exec");

  // Status & Progress
  let isLoading = $state(false);
  let statusMessage = $state("");
  let errorMessage = $state("");
  let resultOutput = $state("");
  let parsedActions = $state([]);

  // Token verification status
  let tokenStatus = $state(null); // null | { valid: bool, message: string }

  // Knowledge Explorer
  let knowledgeFiles = $state([]);
  let searchQuery = $state("");
  let selectedFileContent = $state(null);
  let selectedFileName = $state("");
  let showKnowledgeModal = $state(false);

  // GitHub Update info
  let updateInfo = $state(null);

  // MFA State
  let showMfaModal = $state(false);
  let mfaToken = $state("");
  let mfaMethod = $state("call"); // "call" | "sms"
  let mfaCode = $state("");
  let mfaStatus = $state("");

  onMount(async () => {
    // Load local storage if previously saved
    try {
      const savedToken = localStorage.getItem("ai_yemot_token");
      if (savedToken) yemotToken = savedToken;

      const savedKey = localStorage.getItem("ai_yemot_api_key");
      if (savedKey) apiKey = savedKey;

      const savedScriptUrl = localStorage.getItem("ai_yemot_script_url");
      if (savedScriptUrl) scriptUrl = savedScriptUrl;
    } catch (_) {}

    // Load embedded knowledge files count
    try {
      knowledgeFiles = await invoke("get_knowledge_files");
    } catch (e) {
      console.error("Failed to load knowledge files:", e);
    }

    // Check for GitHub updates in background
    try {
      const res = await invoke("check_for_updates");
      if (res && res.has_update) {
        updateInfo = res;
      }
    } catch (e) {
      console.error("Failed to check updates:", e);
    }
  });

  function saveSettings() {
    try {
      localStorage.setItem("ai_yemot_token", yemotToken);
      localStorage.setItem("ai_yemot_api_key", apiKey);
      localStorage.setItem("ai_yemot_script_url", scriptUrl);
    } catch (_) {}
  }

  async function checkToken() {
    if (!yemotToken.trim()) {
      tokenStatus = { valid: false, message: "נא להזין טוקן תחילה" };
      return;
    }
    statusMessage = "בודק תקינות טוקן מול ימות המשיח...";
    try {
      const res = await invoke("check_yemot_token", { token: yemotToken.trim() });
      if (res.success) {
        tokenStatus = { valid: true, message: res.message };
      } else if (res.mfa_required) {
        tokenStatus = { valid: false, message: "נדרש אימות דו-שלבי (MFA)" };
        mfaToken = res.mfa_token || "";
        showMfaModal = true;
      } else {
        tokenStatus = { valid: false, message: res.message };
      }
    } catch (e) {
      tokenStatus = { valid: false, message: "שגיאת תקשורת: " + e };
    } finally {
      statusMessage = "";
    }
  }

  async function requestMfa() {
    mfaStatus = "שולח קוד אימות...";
    try {
      const res = await invoke("request_yemot_mfa", {
        token: yemotToken.trim(),
        mfaToken: mfaToken,
        method: mfaMethod
      });
      mfaStatus = res.message;
    } catch (e) {
      mfaStatus = "שגיאה: " + e;
    }
  }

  async function verifyMfa() {
    if (!mfaCode.trim()) {
      mfaStatus = "נא להזין קוד אימות";
      return;
    }
    mfaStatus = "מאמת קוד...";
    try {
      const res = await invoke("verify_yemot_mfa", {
        token: yemotToken.trim(),
        mfaToken: mfaToken,
        code: mfaCode.trim()
      });
      if (res.success) {
        if (res.new_token) {
          yemotToken = res.new_token;
          saveSettings();
        }
        showMfaModal = false;
        tokenStatus = { valid: true, message: "האימות הושלם בהצלחה!" };
      } else {
        mfaStatus = res.message;
      }
    } catch (e) {
      mfaStatus = "שגיאה: " + e;
    }
  }

  async function handleSubmit(event) {
    event.preventDefault();
    if (!promptText.trim()) {
      errorMessage = "נא להזין הנחיות או בקשה לעדכון שלוחה";
      return;
    }
    if (!yemotToken.trim()) {
      errorMessage = "נא להזין טוקן של ימות המשיח";
      return;
    }

    saveSettings();
    errorMessage = "";
    resultOutput = "";
    parsedActions = [];
    isLoading = true;
    statusMessage = targetMode === "script" ? "שולח בקשה לסקריפט..." : "מעבד מול מודל ה-AI...";

    try {
      const payload = {
        target_mode: targetMode,
        provider: aiProvider,
        model: modelType,
        prompt: promptText.trim(),
        token: yemotToken.trim(),
        api_key: apiKey.trim(),
        api_type: apiType,
        is_preview: isPreviewMode,
        logout: logoutOnFinish,
        script_url: scriptUrl.trim()
      };

      const res = await invoke("send_ai_request", { payload });
      if (res.success) {
        resultOutput = res.raw_response;
        if (res.is_preview) {
          parseActionList(res.raw_response);
        }
        statusMessage = "הבקשה הושלמה בהצלחה!";
      } else {
        errorMessage = res.message || "אירעה שגיאה בביצוע הבקשה";
      }
    } catch (e) {
      errorMessage = "שגיאה בתקשורת: " + e;
    } finally {
      isLoading = false;
    }
  }

  function parseActionList(raw) {
    try {
      // Try parsing as JSON array
      const json = JSON.parse(raw);
      if (Array.isArray(json)) {
        parsedActions = json.map(item => ({
          path: item.path || "",
          key: item.key || "",
          value: item.value || "",
          description: item.description || item.desc || "",
          selected: true
        }));
        return;
      }
    } catch (_) {}

    // Fallback: Line-by-line parsing
    const lines = raw.split("\n");
    const actions = [];
    for (const line of lines) {
      const trimmed = line.trim();
      if (trimmed.includes("=") || trimmed.includes("->")) {
        actions.push({
          path: "",
          key: trimmed,
          value: "",
          description: "",
          selected: true
        });
      }
    }
    if (actions.length > 0) {
      parsedActions = actions;
    }
  }

  async function executeSelectedActions() {
    const selected = parsedActions.filter(a => a.selected);
    if (selected.length === 0) {
      alert("לא נבחרו פעולות לביצוע");
      return;
    }

    isLoading = true;
    statusMessage = `מבצע ${selected.length} פעולות מול ימות המשיח...`;
    let successCount = 0;

    for (const act of selected) {
      try {
        const res = await invoke("execute_yemot_action", {
          token: yemotToken.trim(),
          path: act.path,
          key: act.key,
          value: act.value
        });
        if (res.success) successCount++;
      } catch (e) {
        console.error("Action failed:", e);
      }
    }

    isLoading = false;
    statusMessage = `הסתיים! בוצעו ${successCount} מתוך ${selected.length} פעולות.`;
  }

  async function openKnowledgeFile(fileName) {
    selectedFileName = fileName;
    try {
      selectedFileContent = await invoke("get_knowledge_file_content", { fileName });
    } catch (e) {
      selectedFileContent = "שגיאה בטעינת הקובץ: " + e;
    }
  }

  function setPreset(text) {
    promptText = text;
  }
</script>

<div class="min-h-screen bg-slate-50 text-slate-800 pb-12">
  <!-- Top Navigation Bar -->
  <header class="bg-white border-b border-slate-200 sticky top-0 z-30 shadow-sm">
    <div class="max-w-6xl mx-auto px-4 py-3 flex items-center justify-between">
      <div class="flex items-center space-x-3 space-x-reverse">
        <div class="w-10 h-10 rounded-xl bg-gradient-to-tr from-blue-600 to-indigo-500 flex items-center justify-center text-white text-xl shadow-md">
          🤖
        </div>
        <div>
          <h1 class="text-lg font-bold text-slate-900 leading-tight">AI yemot</h1>
          <p class="text-xs text-slate-500">ניהול שלוחות חכם מקומפל (Tauri 2.0 + Rust)</p>
        </div>
      </div>

      <div class="flex items-center space-x-3 space-x-reverse">
        <button
          type="button"
          onclick={() => showKnowledgeModal = true}
          class="px-3 py-1.5 rounded-lg border border-slate-200 text-xs font-medium text-slate-700 bg-slate-50 hover:bg-slate-100 flex items-center gap-1.5 transition"
        >
          <span>📚 בסיס ידע מקומי</span>
          <span class="bg-blue-100 text-blue-800 text-[10px] px-1.5 py-0.5 rounded-full font-bold">{knowledgeFiles.length}</span>
        </button>

        {#if updateInfo && updateInfo.has_update}
          <a
            href={updateInfo.release_url}
            target="_blank"
            class="px-3 py-1.5 rounded-lg bg-emerald-50 border border-emerald-300 text-xs font-semibold text-emerald-700 flex items-center gap-1 animate-pulse"
          >
            <span>🚀 עדכון זמין: v{updateInfo.latest_version}</span>
          </a>
        {/if}
      </div>
    </div>
  </header>

  <!-- Main Container -->
  <main class="max-w-6xl mx-auto px-4 py-6">
    <div class="grid grid-cols-1 lg:grid-cols-3 gap-6">

      <!-- Left / Config Column -->
      <div class="lg:col-span-1 space-y-6">
        <div class="bg-white rounded-2xl border border-slate-200 p-5 shadow-sm space-y-5">
          <h2 class="text-sm font-bold text-slate-800 flex items-center gap-2 border-b pb-3">
            <span>⚙️ הגדרות יעד וחיבור</span>
          </h2>

          <!-- Target Mode Selector -->
          <div>
            <label class="block text-xs font-semibold text-slate-600 mb-2">אופן העיבוד (יעד)</label>
            <div class="grid grid-cols-2 gap-2 p-1 bg-slate-100 rounded-xl">
              <button
                type="button"
                onclick={() => targetMode = 'script'}
                class="py-1.5 px-3 text-xs font-medium rounded-lg transition {targetMode === 'script' ? 'bg-white text-blue-700 shadow-sm font-bold' : 'text-slate-600 hover:text-slate-900'}"
              >
                🌐 סקריפט (GAS)
              </button>
              <button
                type="button"
                onclick={() => targetMode = 'direct'}
                class="py-1.5 px-3 text-xs font-medium rounded-lg transition {targetMode === 'direct' ? 'bg-white text-blue-700 shadow-sm font-bold' : 'text-slate-600 hover:text-slate-900'}"
              >
                ⚡ ספק AI ישיר
              </button>
            </div>
          </div>

          <!-- Provider selection if direct mode -->
          {#if targetMode === 'direct'}
            <div>
              <label class="block text-xs font-semibold text-slate-600 mb-1.5">ספק AI ישיר</label>
              <select
                bind:value={aiProvider}
                class="w-full text-xs rounded-lg border border-slate-300 p-2 focus:ring-2 focus:ring-blue-500 focus:outline-none bg-white"
              >
                <option value="gemini">Google Gemini (מומלץ)</option>
                <option value="openai">OpenAI (ChatGPT)</option>
                <option value="groq">Groq (מהיר במיוחד)</option>
              </select>
            </div>
          {/if}

          <!-- Model Type (Regular / Pro) -->
          <div>
            <label class="block text-xs font-semibold text-slate-600 mb-1.5">מודל AI</label>
            <div class="space-y-1.5">
              <label class="flex items-center gap-2 text-xs text-slate-700 cursor-pointer">
                <input type="radio" bind:group={modelType} value="regular" class="text-blue-600 focus:ring-blue-500">
                <span>מודל רגיל (מהיר, פחות מורכב)</span>
              </label>
              <label class="flex items-center gap-2 text-xs text-slate-700 cursor-pointer">
                <input type="radio" bind:group={modelType} value="pro" class="text-blue-600 focus:ring-blue-500">
                <span>מודל Pro (מעמיק ומדויק ביותר)</span>
              </label>
            </div>
          </div>

          <!-- Yemot Token -->
          <div>
            <div class="flex items-center justify-between mb-1.5">
              <label class="text-xs font-semibold text-slate-600">טוקן ימות המשיח</label>
              <button
                type="button"
                onclick={() => showToken = !showToken}
                class="text-[11px] text-blue-600 hover:underline"
              >
                {showToken ? 'הסתר' : 'הצג'}
              </button>
            </div>
            <div class="relative">
              <input
                type={showToken ? "text" : "password"}
                bind:value={yemotToken}
                placeholder="הזן טוקן מערכת..."
                class="w-full text-xs rounded-lg border border-slate-300 p-2 pr-2 pl-14 focus:ring-2 focus:ring-blue-500 focus:outline-none"
              />
              <button
                type="button"
                onclick={checkToken}
                class="absolute left-1 top-1 bottom-1 px-2.5 bg-slate-100 hover:bg-slate-200 text-slate-700 text-[11px] font-medium rounded-md transition"
              >
                בדוק
              </button>
            </div>
            {#if tokenStatus}
              <p class="text-[11px] mt-1.5 {tokenStatus.valid ? 'text-emerald-600' : 'text-rose-600'} font-medium">
                {tokenStatus.valid ? '✅ ' : '❌ '}{tokenStatus.message}
              </p>
            {/if}
          </div>

          <!-- API Key Option -->
          <div class="border-t pt-4">
            <label class="block text-xs font-semibold text-slate-600 mb-1.5">סוג מפתח AI</label>
            <div class="space-y-2">
              <label class="flex items-center gap-2 text-xs text-slate-700 cursor-pointer">
                <input type="radio" bind:group={apiType} value="system" class="text-blue-600">
                <span>מפתח מערכת ברירת מחדל</span>
              </label>
              <label class="flex items-center gap-2 text-xs text-slate-700 cursor-pointer">
                <input type="radio" bind:group={apiType} value="private" class="text-blue-600">
                <span>מפתח API אישי</span>
              </label>
            </div>

            {#if apiType === 'private'}
              <div class="mt-2.5">
                <input
                  type={showApiKey ? "text" : "password"}
                  bind:value={apiKey}
                  placeholder="הזן מפתח API..."
                  class="w-full text-xs rounded-lg border border-slate-300 p-2 focus:ring-2 focus:ring-blue-500 focus:outline-none"
                />
              </div>
            {/if}
          </div>

          <!-- Script URL if in script mode -->
          {#if targetMode === 'script'}
            <div class="border-t pt-4">
              <label class="block text-xs font-semibold text-slate-600 mb-1">כתובת סקריפט מותאמת (GAS)</label>
              <input
                type="text"
                bind:value={scriptUrl}
                class="w-full text-[11px] rounded-lg border border-slate-300 p-2 focus:ring-2 focus:ring-blue-500 focus:outline-none text-slate-600"
              />
            </div>
          {/if}

          <!-- Execution Mode & Logout -->
          <div class="border-t pt-4 space-y-2">
            <label class="flex items-center gap-2 text-xs text-slate-700 cursor-pointer">
              <input type="checkbox" bind:checked={isPreviewMode} class="rounded text-blue-600 focus:ring-blue-500">
              <span class="font-medium">תצוגה מקדימה ואישור פעולות (מומלץ)</span>
            </label>
            <label class="flex items-center gap-2 text-xs text-slate-600 cursor-pointer">
              <input type="checkbox" bind:checked={logoutOnFinish} class="rounded text-blue-600 focus:ring-blue-500">
              <span>התנתק מהמערכת בסיום (Logout)</span>
            </label>
          </div>
        </div>
      </div>

      <!-- Right / Main Action Column -->
      <div class="lg:col-span-2 space-y-6">
        <!-- Input Form Card -->
        <div class="bg-white rounded-2xl border border-slate-200 p-6 shadow-sm">
          <form onsubmit={handleSubmit} class="space-y-4">
            <div>
              <div class="flex items-center justify-between mb-2">
                <label for="prompt-textarea" class="text-sm font-bold text-slate-800">הנחיות לעדכון שלוחה במערכת</label>
                <span class="text-xs text-slate-400">תאר בעברית חופשית</span>
              </div>
              <textarea
                id="prompt-textarea"
                rows="5"
                bind:value={promptText}
                placeholder="לדוגמה: הגדר שלוחה 1 כתפריט שמשמיע קובץ פתיח 000 ומאפשר להקיש 1 להשמעת שיעורים ו-2 לשליחת שיחה למנהל..."
                class="w-full rounded-xl border border-slate-300 p-3.5 text-sm focus:ring-2 focus:ring-blue-500 focus:outline-none leading-relaxed resize-y"
              ></textarea>
            </div>

            <!-- Quick Presets -->
            <div class="flex flex-wrap gap-2 pt-1">
              <span class="text-xs text-slate-400 self-center">הצעות מהירות:</span>
              <button
                type="button"
                onclick={() => setPreset("הגדר שלוחה 1 כתפריט בחירה ראשי עם מעבר לשלוחות 1, 2 ו-3")}
                class="text-xs bg-slate-100 hover:bg-slate-200 text-slate-700 px-2.5 py-1 rounded-lg transition"
              >
                תפריט בחירה
              </button>
              <button
                type="button"
                onclick={() => setPreset("הגדר שלוחה 2 להשמעת קבצים עם אפשרות דילוג וחזרה")}
                class="text-xs bg-slate-100 hover:bg-slate-200 text-slate-700 px-2.5 py-1 rounded-lg transition"
              >
                השמעת קבצים
              </button>
              <button
                type="button"
                onclick={() => setPreset("הגדר שלוחה 3 לקבלת נתונים והקלטה מהמאזין עם שליחה למייל")}
                class="text-xs bg-slate-100 hover:bg-slate-200 text-slate-700 px-2.5 py-1 rounded-lg transition"
              >
                קבלת הקלטה
              </button>
            </div>

            {#if errorMessage}
              <div class="p-3 bg-rose-50 border border-rose-200 text-rose-700 text-xs rounded-xl flex items-center gap-2">
                <span>⚠️</span>
                <span>{errorMessage}</span>
              </div>
            {/if}

            {#if statusMessage}
              <div class="p-3 bg-blue-50 border border-blue-200 text-blue-800 text-xs rounded-xl flex items-center gap-2">
                <span class="animate-spin text-sm">⏳</span>
                <span>{statusMessage}</span>
              </div>
            {/if}

            <div class="pt-2 flex justify-end">
              <button
                type="submit"
                disabled={isLoading}
                class="w-full sm:w-auto px-7 py-3 bg-gradient-to-r from-blue-600 to-indigo-600 hover:from-blue-700 hover:to-indigo-700 text-white font-bold text-sm rounded-xl shadow-md transition disabled:opacity-50 flex items-center justify-center gap-2"
              >
                {#if isLoading}
                  <span class="animate-spin">⏳</span>
                  <span>מעבד בקשה...</span>
                {:else}
                  <span>שגר עדכון לשלוחה 🚀</span>
                {/if}
              </button>
            </div>
          </form>
        </div>

        <!-- Action Preview List Card (If parsed actions exist) -->
        {#if parsedActions.length > 0}
          <div class="bg-white rounded-2xl border border-slate-200 p-6 shadow-sm space-y-4">
            <div class="flex items-center justify-between border-b pb-3">
              <div>
                <h3 class="text-sm font-bold text-slate-800">📋 תצוגה מקדימה של הפעולות המוצעות</h3>
                <p class="text-xs text-slate-500">סמן את הפעולות שברצונך לאשר ולבצע בפועל</p>
              </div>
              <button
                type="button"
                onclick={executeSelectedActions}
                disabled={isLoading}
                class="px-4 py-2 bg-emerald-600 hover:bg-emerald-700 text-white text-xs font-bold rounded-lg shadow transition disabled:opacity-50"
              >
                בצע פעולות מסומנות ✨
              </button>
            </div>

            <div class="divide-y divide-slate-100 max-h-96 overflow-y-auto">
              {#each parsedActions as action, idx}
                <div class="py-3 flex items-start gap-3 hover:bg-slate-50 p-2 rounded-lg transition">
                  <input
                    type="checkbox"
                    bind:checked={action.selected}
                    class="mt-1 rounded text-blue-600 focus:ring-blue-500"
                  />
                  <div class="flex-1 text-xs">
                    <div class="flex items-center gap-2 font-mono">
                      {#if action.path}
                        <span class="bg-slate-100 text-slate-700 px-1.5 py-0.5 rounded font-bold">{action.path}</span>
                      {/if}
                      <span class="text-blue-700 font-bold">{action.key}</span>
                      {#if action.value}
                        <span class="text-slate-400">=</span>
                        <span class="text-emerald-700 bg-emerald-50 px-1.5 py-0.5 rounded">{action.value}</span>
                      {/if}
                    </div>
                    {#if action.description}
                      <p class="text-slate-500 mt-1">{action.description}</p>
                    {/if}
                  </div>
                </div>
              {/each}
            </div>
          </div>
        {/if}

        <!-- Raw Result Output (If no parsed actions or in addition) -->
        {#if resultOutput}
          <div class="bg-white rounded-2xl border border-slate-200 p-5 shadow-sm space-y-3">
            <h3 class="text-xs font-bold text-slate-700">תשובת המערכת:</h3>
            <pre class="bg-slate-900 text-slate-100 p-4 rounded-xl text-xs font-mono whitespace-pre-wrap overflow-x-auto max-h-80">{resultOutput}</pre>
          </div>
        {/if}
      </div>
    </div>
  </main>

  <!-- Knowledge Explorer Modal -->
  {#if showKnowledgeModal}
    <div class="fixed inset-0 bg-black/50 z-50 flex items-center justify-center p-4 backdrop-blur-sm">
      <div class="bg-white rounded-2xl max-w-4xl w-full max-h-[85vh] flex flex-col shadow-2xl overflow-hidden">
        <div class="p-4 border-b flex items-center justify-between bg-slate-50">
          <div class="flex items-center gap-2">
            <span class="text-xl">📚</span>
            <h3 class="font-bold text-sm text-slate-800">בסיס ידע מוטמע ({knowledgeFiles.length} קבצים בזיכרון)</h3>
          </div>
          <button
            type="button"
            onclick={() => { showKnowledgeModal = false; selectedFileContent = null; }}
            class="text-slate-400 hover:text-slate-600 text-lg font-bold"
          >
            ✕
          </button>
        </div>

        <div class="grid grid-cols-1 md:grid-cols-3 flex-1 overflow-hidden">
          <!-- File List -->
          <div class="p-3 border-l border-slate-200 overflow-y-auto max-h-[70vh]">
            <input
              type="text"
              bind:value={searchQuery}
              placeholder="חיפוש קובץ ידע..."
              class="w-full text-xs rounded-lg border border-slate-300 p-2 mb-2 focus:ring-2 focus:ring-blue-500 focus:outline-none"
            />
            <div class="space-y-1">
              {#each knowledgeFiles.filter(f => f.name.includes(searchQuery)) as file}
                <button
                  type="button"
                  onclick={() => openKnowledgeFile(file.name)}
                  class="w-full text-right p-2 text-xs rounded-lg hover:bg-blue-50 hover:text-blue-700 transition flex items-center justify-between {selectedFileName === file.name ? 'bg-blue-100 font-bold text-blue-800' : 'text-slate-700'}"
                >
                  <span class="truncate">{file.name.replace('.txt', '')}</span>
                  <span class="text-[10px] text-slate-400">{(file.size / 1024).toFixed(1)}k</span>
                </button>
              {/each}
            </div>
          </div>

          <!-- Content Viewer -->
          <div class="md:col-span-2 p-4 overflow-y-auto max-h-[70vh] bg-slate-50">
            {#if selectedFileContent}
              <h4 class="text-xs font-bold text-slate-800 mb-2 border-b pb-1">{selectedFileName}</h4>
              <pre class="text-xs text-slate-700 font-sans whitespace-pre-wrap leading-relaxed">{selectedFileContent}</pre>
            {:else}
              <div class="h-full flex items-center justify-center text-slate-400 text-xs">
                בחר קובץ מהרשימה לצפייה בתיעוד
              </div>
            {/if}
          </div>
        </div>
      </div>
    </div>
  {/if}

  <!-- MFA Modal -->
  {#if showMfaModal}
    <div class="fixed inset-0 bg-black/50 z-50 flex items-center justify-center p-4 backdrop-blur-sm">
      <div class="bg-white rounded-2xl max-w-md w-full p-6 shadow-2xl space-y-4">
        <div class="flex items-center justify-between border-b pb-3">
          <h3 class="font-bold text-sm text-slate-800 flex items-center gap-2">
            <span>🔐</span>
            <span>אימות דו-שלבי (MFA) נדרש</span>
          </h3>
          <button
            type="button"
            onclick={() => showMfaModal = false}
            class="text-slate-400 hover:text-slate-600"
          >
            ✕
          </button>
        </div>

        <p class="text-xs text-slate-600 leading-relaxed">
          מערכת ימות המשיח דורשת אימות טלפוני לפני ביצוע שינויים. בחר אופן קבלת הקוד:
        </p>

        <div class="flex gap-4 text-xs text-slate-700">
          <label class="flex items-center gap-1.5 cursor-pointer">
            <input type="radio" bind:group={mfaMethod} value="call" class="text-blue-600">
            <span>שיחה קולית</span>
          </label>
          <label class="flex items-center gap-1.5 cursor-pointer">
            <input type="radio" bind:group={mfaMethod} value="sms" class="text-blue-600">
            <span>מסרון (SMS)</span>
          </label>
        </div>

        <button
          type="button"
          onclick={requestMfa}
          class="w-full py-2 bg-slate-100 hover:bg-slate-200 text-slate-800 text-xs font-semibold rounded-lg transition"
        >
          שלח קוד אימות 📞
        </button>

        <div class="pt-2">
          <label for="mfa-code-input" class="block text-xs font-semibold text-slate-600 mb-1">הזן את הקוד שהתקבל:</label>
          <input
            id="mfa-code-input"
            type="text"
            bind:value={mfaCode}
            placeholder="קוד בן 4-6 ספרות"
            class="w-full text-center text-sm font-mono tracking-widest rounded-lg border border-slate-300 p-2 focus:ring-2 focus:ring-blue-500 focus:outline-none"
          />
        </div>

        {#if mfaStatus}
          <p class="text-xs text-blue-700 font-medium">{mfaStatus}</p>
        {/if}

        <div class="pt-2 flex justify-end gap-2">
          <button
            type="button"
            onclick={() => showMfaModal = false}
            class="px-4 py-2 text-xs font-medium text-slate-600 hover:bg-slate-100 rounded-lg transition"
          >
            ביטול
          </button>
          <button
            type="button"
            onclick={verifyMfa}
            class="px-5 py-2 bg-blue-600 hover:bg-blue-700 text-white text-xs font-bold rounded-lg shadow transition"
          >
            אמת והמשך ✓
          </button>
        </div>
      </div>
    </div>
  {/if}
</div>
