<script>
  import { onMount, tick } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { marked } from "marked";
  import { t, i18n, isRTL, setLocale, availableLocales } from "$lib/i18n.svelte.js";

  marked.setOptions({
    gfm: true,
    breaks: true,
  });

  // Interface direction (reactive to language changes)
  let rtl = $derived(isRTL());

  // State (Svelte 5 Runes)
  let targetMode = $state("direct"); // "direct" | "script" — default: direct (persisted locally)
  let aiProvider = $state("gemini"); // "gemini" | "openai" | "groq" | "custom"
  let modelType = $state("regular"); // "regular" | "pro" (script mode)
  // Direct mode model selection: from a known list, or manually entered ID
  let modelSource = $state("list"); // "list" | "manual"
  let selectedModel = $state("gemini-2.5-flash"); // model ID chosen from the list
  let customModel = $state(""); // model ID entered manually
  let customBaseUrl = $state(""); // custom full API URL (custom provider)
  let promptText = $state("");
  let yemotToken = $state("");
  let showToken = $state(false);
  let apiKey = $state("");
  let showApiKey = $state(false);
  let isPreviewMode = $state(true);
  let logoutOnFinish = $state(false);
  let scriptUrl = $state("https://script.google.com/macros/s/AKfycbz_REPLACE_ME/exec");

  // Known model list per direct provider (first two are the classic Regular/Pro).
  // tag = optional i18n key appended to the label.
  /** @type {Record<string, {id: string, tag?: string}[]>} */
  const PROVIDER_MODELS = {
    gemini: [
      { id: "gemini-2.5-flash", tag: "model_regular" },
      { id: "gemini-2.5-pro", tag: "model_pro" },
      { id: "gemini-2.0-flash" },
      { id: "gemini-2.5-flash-lite" },
      { id: "gemini-1.5-flash" },
      { id: "gemini-1.5-pro" },
    ],
    openai: [
      { id: "gpt-4o-mini", tag: "model_regular" },
      { id: "gpt-4o", tag: "model_pro" },
      { id: "gpt-4.1" },
      { id: "gpt-4.1-mini" },
      { id: "gpt-4.1-nano" },
      { id: "o4-mini" },
    ],
    groq: [
      { id: "llama-3.3-70b-versatile", tag: "model_regular" },
      { id: "llama-3.1-8b-instant" },
      { id: "openai/gpt-oss-120b", tag: "model_pro" },
      { id: "openai/gpt-oss-20b" },
      { id: "qwen/qwen3-32b" },
      { id: "gemma2-9b-it" },
    ],
  };

  // Status & Progress
  let isLoading = $state(false);
  let statusMessage = $state("");
  let errorMessage = $state("");
  let resultOutput = $state("");
  let parsedActions = $state([]);

  // Token verification status
  let tokenStatus = $state(null); // null | { valid: bool, message: string }

  // Login (create token) state
  let showLoginModal = $state(false);
  let loginUsername = $state("");
  let loginPassword = $state("");
  let loginShowPassword = $state(false);
  let loginToken = $state("");
  let loginStep = $state("credentials"); // "credentials" | "mfa"
  let loginMethods = $state([]);
  let loginMethodId = $state("");
  let loginSendType = $state("");
  let loginCode = $state("");
  let loginCodeSent = $state(false);
  let loginStatus = $state("");
  let loginError = $state("");
  let loginLoading = $state(false);

  // Knowledge Explorer
  let knowledgeFiles = $state([]);
  let searchQuery = $state("");
  let selectedFileContent = $state(null);
  let selectedFileName = $state("");
  let showKnowledgeModal = $state(false);
  let isRawView = $state(false);
  let copyFeedback = $state(false);
  let contentContainerRef = $state(null);

  function preprocessMarkdown(content) {
    if (!content) return "";
    return content.replace(/\[([^\]]+)\]\(([^)\n]+)\)/g, (match, text, href) => {
      const trimmed = href.trim();
      if (trimmed.includes(" ") && !trimmed.startsWith("<") && !trimmed.endsWith(">")) {
        return `[${text}](<${trimmed}>)`;
      }
      return match;
    });
  }

  let renderedMarkdownHtml = $derived.by(() => {
    if (!selectedFileContent) return "";
    try {
      const preprocessed = preprocessMarkdown(selectedFileContent);
      return marked.parse(preprocessed);
    } catch (e) {
      console.error("Markdown parse error:", e);
      return selectedFileContent;
    }
  });

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

      const savedTargetMode = localStorage.getItem("ai_yemot_target_mode");
      if (savedTargetMode === "script" || savedTargetMode === "direct") {
        targetMode = savedTargetMode;
      }

      const savedProvider = localStorage.getItem("ai_yemot_provider");
      if (savedProvider && ["gemini", "openai", "groq", "custom"].includes(savedProvider)) {
        aiProvider = savedProvider;
      }

      const savedModelSource = localStorage.getItem("ai_yemot_model_source");
      if (savedModelSource === "list" || savedModelSource === "manual") {
        modelSource = savedModelSource;
      }

      const savedSelectedModel = localStorage.getItem("ai_yemot_selected_model");
      if (savedSelectedModel) selectedModel = savedSelectedModel;

      const savedCustomModel = localStorage.getItem("ai_yemot_custom_model");
      if (savedCustomModel) customModel = savedCustomModel;

      const savedCustomBaseUrl = localStorage.getItem("ai_yemot_custom_base_url");
      if (savedCustomBaseUrl) customBaseUrl = savedCustomBaseUrl;

      normalizeModelSelection();
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
      localStorage.setItem("ai_yemot_target_mode", targetMode);
      localStorage.setItem("ai_yemot_provider", aiProvider);
      localStorage.setItem("ai_yemot_model_source", modelSource);
      localStorage.setItem("ai_yemot_selected_model", selectedModel);
      localStorage.setItem("ai_yemot_custom_model", customModel);
      localStorage.setItem("ai_yemot_custom_base_url", customBaseUrl);
    } catch (_) {}
  }

  // ----- Direct-mode provider / model helpers -----

  // Switch processing mode and persist the choice immediately.
  /**
   * @param {"direct" | "script"} mode
   */
  function setTargetMode(mode) {
    targetMode = mode;
    try {
      localStorage.setItem("ai_yemot_target_mode", mode);
    } catch (_) {}
  }

  /**
   * Known models for a provider (empty for a custom provider).
   * @param {string} provider
   * @returns {{id: string, tag?: string}[]}
   */
  function getProviderModels(provider) {
    return PROVIDER_MODELS[provider] || [];
  }

  // Keep the selected model consistent with the current provider.
  function normalizeModelSelection() {
    if (aiProvider === "custom") {
      // A custom provider has no known list — manual entry only.
      modelSource = "manual";
      return;
    }
    const models = getProviderModels(aiProvider);
    if (models.length === 0) return;
    if (!models.some((m) => m.id === selectedModel)) {
      selectedModel = models[0].id;
    }
  }

  function handleProviderChange() {
    normalizeModelSelection();
  }

  async function checkToken() {
    if (!yemotToken.trim()) {
      tokenStatus = { valid: false, message: t("enter_token_first") };
      return;
    }
    statusMessage = t("checking_token");
    try {
      const res = await invoke("check_yemot_token", { token: yemotToken.trim() });
      if (res.success) {
        tokenStatus = { valid: true, message: res.message };
        try {
          localStorage.setItem("ai_yemot_token", yemotToken.trim());
        } catch (_) {}
      } else if (res.mfa_required) {
        tokenStatus = { valid: false, message: t("mfa_required") };
        mfaToken = res.mfa_token || "";
        showMfaModal = true;
      } else {
        tokenStatus = { valid: false, message: res.message };
      }
    } catch (e) {
      tokenStatus = { valid: false, message: t("comm_error", { error: e }) };
    } finally {
      statusMessage = "";
    }
  }

  async function requestMfa() {
    mfaStatus = t("sending_code");
    try {
      const res = await invoke("request_yemot_mfa", {
        token: yemotToken.trim(),
        mfaToken: mfaToken,
        method: mfaMethod
      });
      mfaStatus = res.message;
    } catch (e) {
      mfaStatus = t("error", { error: e });
    }
  }

  async function verifyMfa() {
    if (!mfaCode.trim()) {
      mfaStatus = t("enter_mfa_code");
      return;
    }
    mfaStatus = t("verifying_code");
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
        tokenStatus = { valid: true, message: t("mfa_success") };
      } else {
        mfaStatus = res.message;
      }
    } catch (e) {
      mfaStatus = t("error", { error: e });
    }
  }

  async function handleSubmit(event) {
    event.preventDefault();
    if (!promptText.trim()) {
      errorMessage = t("enter_prompt");
      return;
    }
    if (!yemotToken.trim()) {
      errorMessage = t("enter_yemot_token");
      return;
    }
    if (targetMode === "direct" && !apiKey.trim()) {
      errorMessage = t("api_key_required");
      return;
    }

    // Direct-mode: validate the custom provider URL and the chosen model.
    let payloadModel = modelType;
    if (targetMode === "direct") {
      if (aiProvider === "custom") {
        const url = customBaseUrl.trim();
        if (!url || !/^https?:\/\//i.test(url)) {
          errorMessage = t("custom_url_required");
          return;
        }
      }
      if (aiProvider === "custom" || modelSource === "manual") {
        if (!customModel.trim()) {
          errorMessage = t("model_manual_required");
          return;
        }
        payloadModel = customModel.trim();
      } else {
        payloadModel = selectedModel;
      }
    }

    saveSettings();
    errorMessage = "";
    resultOutput = "";
    parsedActions = [];
    isLoading = true;
    statusMessage = targetMode === "script" ? t("sending_to_script") : t("processing_ai");

    try {
      const payload = {
        target_mode: targetMode,
        provider: aiProvider,
        model: payloadModel,
        prompt: promptText.trim(),
        token: yemotToken.trim(),
        api_key: apiKey.trim(),
        is_preview: isPreviewMode,
        logout: logoutOnFinish,
        script_url: scriptUrl.trim(),
        base_url: targetMode === "direct" ? customBaseUrl.trim() : ""
      };

      const res = await invoke("send_ai_request", { payload });
      if (res.success) {
        resultOutput = res.raw_response;
        if (res.is_preview) {
          parseActionList(res.raw_response);
        }
        statusMessage = t("request_success");
      } else {
        errorMessage = res.message || t("request_failed");
      }
    } catch (e) {
      errorMessage = t("comm_error", { error: e });
    } finally {
      isLoading = false;
      // Logout is performed locally — the token never reaches the script/AI.
      if (logoutOnFinish) {
        await localLogout();
      }
    }
  }

  function parseActionList(raw) {
    // Try parsing as JSON array
    try {
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

    // Fallback: line-by-line parsing
    const lines = raw.split("\n");
    const actions = [];
    let currentDescription = "";
    for (const line of lines) {
      const trimmed = line.trim();
      if (trimmed.startsWith("הסבר:")) {
        currentDescription = trimmed.substring(4).trim();
        continue;
      }
      // Parse Yemot API URL lines into local actions (path + key=value params)
      if (/^https?:\/\//i.test(trimmed) && trimmed.includes("?")) {
        try {
          const url = new URL(trimmed);
          const pathParam = url.searchParams.get("path") || "";
          for (const [param, value] of url.searchParams.entries()) {
            if (param === "path" || param === "token") continue;
            actions.push({
              path: pathParam,
              key: param,
              value: value,
              description: currentDescription,
              selected: true
            });
          }
          currentDescription = "";
        } catch (_) {}
        continue;
      }
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
      alert(t("no_actions_selected"));
      return;
    }

    isLoading = true;
    statusMessage = t("executing_actions", { count: selected.length });
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
    statusMessage = t("actions_done", { done: successCount, total: selected.length });
    // Logout is performed locally — never via the script.
    if (logoutOnFinish) {
      await localLogout();
    }
  }

  async function openKnowledgeFile(fileName, targetAnchor = null) {
    let cleanName = (fileName || "").trim();
    if (!cleanName) return;

    if (!cleanName.endsWith(".txt")) {
      cleanName += ".txt";
    }

    const match = knowledgeFiles.find(
      (f) =>
        f.name === cleanName ||
        f.name.replace(".txt", "") === cleanName.replace(".txt", "")
    );
    selectedFileName = match ? match.name : cleanName;

    try {
      selectedFileContent = await invoke("get_knowledge_file_content", {
        fileName: selectedFileName,
      });
      await tick();
      if (targetAnchor) {
        setTimeout(() => scrollToAnchor(targetAnchor), 80);
      } else if (contentContainerRef) {
        contentContainerRef.scrollTop = 0;
      }
    } catch (e) {
      selectedFileContent = t("file_load_error", { error: e });
    }
  }

  function scrollToAnchor(targetId) {
    if (!contentContainerRef || !targetId) return;
    const cleanId = decodeURIComponent(targetId).replace(/^#/, "").trim();
    if (!cleanId) return;

    let el = null;
    try {
      el =
        contentContainerRef.querySelector(`[id="${CSS.escape(cleanId)}"]`) ||
        contentContainerRef.querySelector(`a[name="${CSS.escape(cleanId)}"]`);
    } catch (_) {}

    if (!el && cleanId.startsWith("post-")) {
      const pid = cleanId.replace("post-", "");
      try {
        el = contentContainerRef.querySelector(`[id*="${CSS.escape(pid)}"]`);
      } catch (_) {}
    }

    if (!el) {
      const headings = contentContainerRef.querySelectorAll("h1, h2, h3, h4");
      for (const h of headings) {
        if (h.textContent && h.textContent.includes(cleanId)) {
          el = h;
          break;
        }
      }
    }

    if (el) {
      const targetEl =
        el.tagName === "A" && el.textContent.trim() === "" && el.nextElementSibling
          ? el.nextElementSibling
          : el;

      targetEl.scrollIntoView({ behavior: "smooth", block: "start" });
      targetEl.classList.remove("target-highlight");
      void targetEl.offsetWidth;
      targetEl.classList.add("target-highlight");
    }
  }

  async function handleContentClick(e) {
    const anchor = e.target.closest("a");
    if (!anchor) return;

    const href = anchor.getAttribute("href");
    if (!href) return;

    e.preventDefault();

    if (
      href.startsWith("http://") ||
      href.startsWith("https://") ||
      href.startsWith("mailto:")
    ) {
      try {
        await openUrl(href);
      } catch (err) {
        console.error("Failed to open URL with plugin-opener:", err);
        window.open(href, "_blank");
      }
      return;
    }

    if (href.startsWith("#")) {
      scrollToAnchor(href);
      return;
    }

    let decodedHref = decodeURIComponent(href).trim();
    if (decodedHref.startsWith("<") && decodedHref.endsWith(">")) {
      decodedHref = decodedHref.slice(1, -1).trim();
    }
    const hashIndex = decodedHref.indexOf("#");
    let targetFile =
      hashIndex >= 0 ? decodedHref.slice(0, hashIndex) : decodedHref;
    const targetAnchor =
      hashIndex >= 0 ? decodedHref.slice(hashIndex + 1) : null;

    if (targetFile) {
      if (!targetFile.endsWith(".txt")) {
        targetFile += ".txt";
      }
      if (searchQuery && !targetFile.includes(searchQuery)) {
        searchQuery = "";
      }
      await openKnowledgeFile(targetFile, targetAnchor);
    }
  }

  async function copySelectedContent() {
    if (!selectedFileContent) return;
    try {
      await navigator.clipboard.writeText(selectedFileContent);
      copyFeedback = true;
      setTimeout(() => {
        copyFeedback = false;
      }, 2000);
    } catch (e) {
      console.error("Failed to copy:", e);
    }
  }

  function setPreset(promptKey) {
    promptText = t(promptKey);
  }

  // ----- Token management (stored locally, deletable) -----

  function clearToken() {
    yemotToken = "";
    tokenStatus = null;
    try {
      localStorage.removeItem("ai_yemot_token");
    } catch (_) {}
  }

  // ----- Login (create token) flow: system number + password + MFA -----

  function openLoginModal() {
    loginUsername = "";
    loginPassword = "";
    loginToken = "";
    loginStep = "credentials";
    loginMethods = [];
    loginMethodId = "";
    loginSendType = "";
    loginCode = "";
    loginCodeSent = false;
    loginStatus = "";
    loginError = "";
    loginLoading = false;
    showLoginModal = true;
  }

  function closeLoginModal() {
    showLoginModal = false;
  }

  function applyLoginToken(token) {
    yemotToken = token;
    try {
      localStorage.setItem("ai_yemot_token", token);
    } catch (_) {}
    tokenStatus = { valid: true, message: t("login_success") };
    closeLoginModal();
  }

  async function handleLogin() {
    loginError = "";
    if (!loginUsername.trim() || !loginPassword.trim()) {
      loginError = t("enter_system_and_password");
      return;
    }
    loginLoading = true;
    loginStatus = t("login_btn") + "...";
    try {
      const res = await invoke("login_yemot", {
        username: loginUsername.trim(),
        password: loginPassword.trim()
      });
      if (res.success && res.token) {
        loginToken = res.token;
        if (!res.mfa_required) {
          applyLoginToken(res.token);
        } else {
          await loadLoginMethods();
        }
      } else {
        loginError = res.message;
      }
    } catch (e) {
      loginError = t("comm_error", { error: e });
    } finally {
      loginLoading = false;
      loginStatus = "";
    }
  }

  async function loadLoginMethods() {
    loginLoading = true;
    try {
      const res = await invoke("get_mfa_methods", { token: loginToken });
      if (res.success && res.methods && res.methods.length > 0) {
        loginMethods = res.methods;
        loginMethodId = res.methods[0].id;
        loginSendType = res.methods[0].send_types && res.methods[0].send_types[0] ? res.methods[0].send_types[0] : "";
        loginStep = "mfa";
        loginCodeSent = false;
        loginCode = "";
        loginStatus = "";
      } else {
        loginError = res.message || t("no_mfa_methods");
      }
    } catch (e) {
      loginError = t("comm_error", { error: e });
    } finally {
      loginLoading = false;
    }
  }

  async function handleSendLoginCode() {
    loginError = "";
    loginLoading = true;
    loginStatus = t("sending_code");
    try {
      const res = await invoke("send_mfa_code", {
        token: loginToken,
        mfaId: loginMethodId,
        sendType: loginSendType
      });
      if (res.success) {
        loginCodeSent = true;
        loginStatus = "";
      } else {
        loginError = res.message;
      }
    } catch (e) {
      loginError = t("comm_error", { error: e });
    } finally {
      loginLoading = false;
      loginStatus = "";
    }
  }

  async function handleValidateLoginCode() {
    loginError = "";
    if (!loginCode.trim()) {
      loginError = t("enter_mfa_code");
      return;
    }
    loginLoading = true;
    loginStatus = t("verifying_code");
    try {
      const res = await invoke("validate_mfa_code", {
        token: loginToken,
        code: loginCode.trim()
      });
      if (res.success) {
        applyLoginToken(loginToken);
      } else {
        loginError = res.message;
      }
    } catch (e) {
      loginError = t("comm_error", { error: e });
    } finally {
      loginLoading = false;
      loginStatus = "";
    }
  }

  // ----- Local logout (token is never invalidated via the script) -----

  async function localLogout() {
    if (!yemotToken.trim()) return;
    try {
      await invoke("logout_yemot", { token: yemotToken.trim() });
    } catch (e) {
      console.error("Logout failed:", e);
    }
  }
</script>

<svelte:head>
  <title>{t("app_title")}</title>
</svelte:head>

<div class="min-h-screen bg-slate-50 text-slate-800 pb-12" dir={rtl ? "rtl" : "ltr"}>
  <!-- Top Navigation Bar -->
  <header class="bg-white border-b border-slate-200 sticky top-0 z-30 shadow-sm">
    <div class="max-w-6xl mx-auto px-4 py-3 flex items-center justify-between">
      <div class="flex items-center space-x-3 space-x-reverse">
        <div class="w-10 h-10 rounded-xl bg-gradient-to-tr from-blue-600 to-indigo-500 flex items-center justify-center text-white text-xl shadow-md">
          🤖
        </div>
        <div>
          <h1 class="text-lg font-bold text-slate-900 leading-tight">AI yemot</h1>
        </div>
      </div>

      <div class="flex items-center gap-3">
        <!-- Language selector -->
        <select
          value={i18n.locale}
          onchange={(e) => setLocale(e.target.value)}
          aria-label={t("language")}
          title={t("language")}
          class="text-xs rounded-lg border border-slate-300 bg-white p-1.5 focus:ring-2 focus:ring-blue-500 focus:outline-none cursor-pointer"
        >
          {#each availableLocales as loc}
            <option value={loc.code}>{loc.flag} {loc.nativeName}</option>
          {/each}
        </select>

        <button
          type="button"
          onclick={() => showKnowledgeModal = true}
          class="px-3 py-1.5 rounded-lg border border-slate-200 text-xs font-medium text-slate-700 bg-slate-50 hover:bg-slate-100 flex items-center gap-1.5 transition"
        >
          <span>{t("knowledge_btn")}</span>
          <span class="bg-blue-100 text-blue-800 text-[10px] px-1.5 py-0.5 rounded-full font-bold">{knowledgeFiles.length}</span>
        </button>

        {#if updateInfo && updateInfo.has_update}
          <a
            href={updateInfo.release_url}
            target="_blank"
            class="px-3 py-1.5 rounded-lg bg-emerald-50 border border-emerald-300 text-xs font-semibold text-emerald-700 flex items-center gap-1 animate-pulse"
          >
            <span>{t("update_available", { version: updateInfo.latest_version })}</span>
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
            <span>{t("settings_title")}</span>
          </h2>

          <!-- Target Mode Selector -->
          <div>
            <label class="block text-xs font-semibold text-slate-600 mb-2">{t("target_mode")}</label>
            <div class="grid grid-cols-2 gap-2 p-1 bg-slate-100 rounded-xl">
              <button
                type="button"
                onclick={() => setTargetMode('direct')}
                class="py-1.5 px-3 text-xs font-medium rounded-lg transition {targetMode === 'direct' ? 'bg-white text-blue-700 shadow-sm font-bold' : 'text-slate-600 hover:text-slate-900'}"
              >
                {t("mode_direct")}
              </button>
              <button
                type="button"
                onclick={() => setTargetMode('script')}
                class="py-1.5 px-3 text-xs font-medium rounded-lg transition {targetMode === 'script' ? 'bg-white text-blue-700 shadow-sm font-bold' : 'text-slate-600 hover:text-slate-900'}"
              >
                {t("mode_script")}
              </button>
            </div>
          </div>

          <!-- Provider selection if direct mode -->
          {#if targetMode === 'direct'}
            <div>
              <label class="block text-xs font-semibold text-slate-600 mb-1.5">{t("provider_label")}</label>
              <select
                bind:value={aiProvider}
                onchange={handleProviderChange}
                class="w-full text-xs rounded-lg border border-slate-300 p-2 focus:ring-2 focus:ring-blue-500 focus:outline-none bg-white"
              >
                <option value="gemini">{t("provider_gemini")}</option>
                <option value="openai">{t("provider_openai")}</option>
                <option value="groq">{t("provider_groq")}</option>
                <option value="custom">{t("provider_custom")}</option>
              </select>
            </div>

            {#if aiProvider === 'custom'}
              <!-- Custom provider: full API URL -->
              <div>
                <label for="custom-api-url" class="block text-xs font-semibold text-slate-600 mb-1.5">{t("custom_url_label")}</label>
                <input
                  id="custom-api-url"
                  type="text"
                  dir="ltr"
                  bind:value={customBaseUrl}
                  placeholder={t("custom_url_placeholder")}
                  class="w-full text-[11px] rounded-lg border border-slate-300 p-2 focus:ring-2 focus:ring-blue-500 focus:outline-none text-slate-600"
                />
                <p class="text-[10px] text-slate-400 mt-1.5 leading-relaxed">{t("custom_url_hint")}</p>
              </div>
            {/if}

            <!-- Model: pick from the known list or enter a manual ID -->
            <div>
              <label for="model-select" class="block text-xs font-semibold text-slate-600 mb-1.5">{t("model_label")}</label>
              <div class="grid grid-cols-2 gap-2 p-1 bg-slate-100 rounded-xl mb-2">
                <button
                  type="button"
                  onclick={() => modelSource = 'list'}
                  disabled={aiProvider === 'custom'}
                  class="py-1.5 px-3 text-xs font-medium rounded-lg transition disabled:opacity-40 {modelSource === 'list' ? 'bg-white text-blue-700 shadow-sm font-bold' : 'text-slate-600 hover:text-slate-900'}"
                >
                  {t("model_from_list")}
                </button>
                <button
                  type="button"
                  onclick={() => modelSource = 'manual'}
                  class="py-1.5 px-3 text-xs font-medium rounded-lg transition {modelSource === 'manual' ? 'bg-white text-blue-700 shadow-sm font-bold' : 'text-slate-600 hover:text-slate-900'}"
                >
                  {t("model_manual")}
                </button>
              </div>

              {#if aiProvider !== 'custom' && modelSource === 'list'}
                <select
                  id="model-select"
                  bind:value={selectedModel}
                  class="w-full text-xs rounded-lg border border-slate-300 p-2 focus:ring-2 focus:ring-blue-500 focus:outline-none bg-white"
                >
                  {#each getProviderModels(aiProvider) as m}
                    <option value={m.id}>{m.id}{m.tag ? ` — ${t(m.tag)}` : ""}</option>
                  {/each}
                </select>
              {:else}
                <input
                  id="model-select"
                  type="text"
                  dir="ltr"
                  bind:value={customModel}
                  placeholder={t("model_manual_placeholder")}
                  class="w-full text-[11px] rounded-lg border border-slate-300 p-2 focus:ring-2 focus:ring-blue-500 focus:outline-none text-slate-600"
                />
              {/if}
            </div>
          {:else}
            <!-- Model Type (Regular / Pro) — script mode -->
            <div>
              <label class="block text-xs font-semibold text-slate-600 mb-1.5">{t("model_label")}</label>
              <div class="space-y-1.5">
                <label class="flex items-center gap-2 text-xs text-slate-700 cursor-pointer">
                  <input type="radio" bind:group={modelType} value="regular" class="text-blue-600 focus:ring-blue-500">
                  <span>{t("model_regular")}</span>
                </label>
                <label class="flex items-center gap-2 text-xs text-slate-700 cursor-pointer">
                  <input type="radio" bind:group={modelType} value="pro" class="text-blue-600 focus:ring-blue-500">
                  <span>{t("model_pro")}</span>
                </label>
              </div>
            </div>
          {/if}

          <!-- Yemot Token -->
          <div>
            <div class="flex items-center justify-between mb-1.5">
              <label class="text-xs font-semibold text-slate-600">{t("token_label")}</label>
              <div class="flex items-center gap-2">
                <button
                  type="button"
                  onclick={openLoginModal}
                  class="text-[11px] text-blue-600 hover:underline"
                >
                  {t("get_token")}
                </button>
                {#if yemotToken}
                  <button
                    type="button"
                    onclick={clearToken}
                    title={t("clear_token")}
                    aria-label={t("clear_token")}
                    class="text-[11px] text-rose-500 hover:text-rose-700"
                  >
                    🗑 {t("clear_token")}
                  </button>
                {/if}
                <button
                  type="button"
                  onclick={() => showToken = !showToken}
                  class="text-[11px] text-blue-600 hover:underline"
                >
                  {showToken ? t("hide") : t("show")}
                </button>
              </div>
            </div>
            <div class="relative">
              <input
                type={showToken ? "text" : "password"}
                bind:value={yemotToken}
                placeholder={t("token_placeholder")}
                class="w-full text-xs rounded-lg border border-slate-300 p-2 {rtl ? 'pr-2 pl-14' : 'pl-2 pr-14'} focus:ring-2 focus:ring-blue-500 focus:outline-none"
              />
              <button
                type="button"
                onclick={checkToken}
                class="absolute {rtl ? 'left-1' : 'right-1'} top-1 bottom-1 px-2.5 bg-slate-100 hover:bg-slate-200 text-slate-700 text-[11px] font-medium rounded-md transition"
              >
                {t("check")}
              </button>
            </div>
            {#if tokenStatus}
              <p class="text-[11px] mt-1.5 {tokenStatus.valid ? 'text-emerald-600' : 'text-rose-600'} font-medium">
                {tokenStatus.valid ? '✅ ' : '❌ '}{tokenStatus.message}
              </p>
            {/if}
            <p class="text-[10px] text-slate-400 mt-1.5 leading-relaxed">🔒 {t("local_note")}</p>
          </div>

          <!-- Personal API Key (no system key exists) -->
          <div class="border-t pt-4">
            <div class="flex items-center justify-between mb-1.5">
              <label class="text-xs font-semibold text-slate-600">{t("api_key_label")}</label>
              <button
                type="button"
                onclick={() => showApiKey = !showApiKey}
                class="text-[11px] text-blue-600 hover:underline"
              >
                {showApiKey ? t("hide") : t("show")}
              </button>
            </div>
            <input
              type={showApiKey ? "text" : "password"}
              bind:value={apiKey}
              placeholder={t("api_key_placeholder")}
              class="w-full text-xs rounded-lg border border-slate-300 p-2 focus:ring-2 focus:ring-blue-500 focus:outline-none"
            />
            <p class="text-[10px] text-slate-400 mt-1.5">
              {targetMode === 'direct' ? t("api_key_hint_direct") : t("api_key_hint_script")}
            </p>
          </div>

          <!-- Script URL if in script mode -->
          {#if targetMode === 'script'}
            <div class="border-t pt-4">
              <label class="block text-xs font-semibold text-slate-600 mb-1">{t("script_url_label")}</label>
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
              <span class="font-medium">{t("preview_mode")}</span>
            </label>
            <label class="flex items-center gap-2 text-xs text-slate-600 cursor-pointer">
              <input type="checkbox" bind:checked={logoutOnFinish} class="rounded text-blue-600 focus:ring-blue-500">
              <span>{t("logout_on_finish")}</span>
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
                <label for="prompt-textarea" class="text-sm font-bold text-slate-800">{t("prompt_label")}</label>
                <span class="text-xs text-slate-400">{t("prompt_hint")}</span>
              </div>
              <textarea
                id="prompt-textarea"
                rows="5"
                bind:value={promptText}
                placeholder={t("prompt_placeholder")}
                class="w-full rounded-xl border border-slate-300 p-3.5 text-sm focus:ring-2 focus:ring-blue-500 focus:outline-none leading-relaxed resize-y"
              ></textarea>
            </div>

            <!-- Quick Presets -->
            <div class="flex flex-wrap gap-2 pt-1">
              <span class="text-xs text-slate-400 self-center">{t("quick_presets")}</span>
              <button
                type="button"
                onclick={() => setPreset("preset_menu_prompt")}
                class="text-xs bg-slate-100 hover:bg-slate-200 text-slate-700 px-2.5 py-1 rounded-lg transition"
              >
                {t("preset_menu")}
              </button>
              <button
                type="button"
                onclick={() => setPreset("preset_play_prompt")}
                class="text-xs bg-slate-100 hover:bg-slate-200 text-slate-700 px-2.5 py-1 rounded-lg transition"
              >
                {t("preset_play")}
              </button>
              <button
                type="button"
                onclick={() => setPreset("preset_record_prompt")}
                class="text-xs bg-slate-100 hover:bg-slate-200 text-slate-700 px-2.5 py-1 rounded-lg transition"
              >
                {t("preset_record")}
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
                  <span>{t("processing")}</span>
                {:else}
                  <span>{t("submit")}</span>
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
                <h3 class="text-sm font-bold text-slate-800">{t("preview_title")}</h3>
                <p class="text-xs text-slate-500">{t("preview_hint")}</p>
              </div>
              <button
                type="button"
                onclick={executeSelectedActions}
                disabled={isLoading}
                class="px-4 py-2 bg-emerald-600 hover:bg-emerald-700 text-white text-xs font-bold rounded-lg shadow transition disabled:opacity-50"
              >
                {t("execute_selected")}
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
            <h3 class="text-xs font-bold text-slate-700">{t("raw_output_title")}</h3>
            <pre class="bg-slate-900 text-slate-100 p-4 rounded-xl text-xs font-mono whitespace-pre-wrap overflow-x-auto max-h-80">{resultOutput}</pre>
          </div>
        {/if}
      </div>
    </div>
  </main>

  <!-- Footer credit -->
  <footer class="max-w-6xl mx-auto px-4 py-6 flex justify-center">
    <a
      href="https://bot-phone.netlify.app/"
      target="_blank"
      rel="noopener noreferrer"
      class="inline-flex items-center gap-2 px-4 py-2 rounded-full bg-white border border-slate-200 shadow-sm text-xs font-medium text-slate-600 hover:text-blue-600 hover:border-blue-300 transition"
    >
      <span class="text-base" aria-hidden="true">🤖</span>
      <span>{t("footer_credit")}</span>
    </a>
  </footer>

  <!-- Knowledge Explorer Modal -->
  {#if showKnowledgeModal}
    <div class="fixed inset-0 bg-black/50 z-50 flex items-center justify-center p-4 backdrop-blur-sm">
      <div class="bg-white rounded-2xl max-w-4xl w-full max-h-[85vh] flex flex-col shadow-2xl overflow-hidden">
        <div class="p-4 border-b flex items-center justify-between bg-slate-50">
          <div class="flex items-center gap-2">
            <span class="text-xl">📚</span>
            <h3 class="font-bold text-sm text-slate-800">{t("knowledge_modal_title", { count: knowledgeFiles.length })}</h3>
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
          <div class="p-3 {rtl ? 'border-l' : 'border-r'} border-slate-200 overflow-y-auto max-h-[70vh]">
            <input
              type="text"
              bind:value={searchQuery}
              placeholder={t("search_knowledge")}
              class="w-full text-xs rounded-lg border border-slate-300 p-2 mb-2 focus:ring-2 focus:ring-blue-500 focus:outline-none"
            />
            <div class="space-y-1">
              {#each knowledgeFiles.filter(f => f.name.includes(searchQuery)) as file}
                <button
                  type="button"
                  onclick={() => openKnowledgeFile(file.name)}
                  class="w-full {rtl ? 'text-right' : 'text-left'} p-2 text-xs rounded-lg hover:bg-blue-50 hover:text-blue-700 transition flex items-center justify-between {selectedFileName === file.name ? 'bg-blue-100 font-bold text-blue-800' : 'text-slate-700'}"
                >
                  <span class="truncate">{file.name.replace('.txt', '')}</span>
                  <span class="text-[10px] text-slate-400">{(file.size / 1024).toFixed(1)}k</span>
                </button>
              {/each}
            </div>
          </div>

          <!-- Content Viewer -->
          <div class="md:col-span-2 flex flex-col overflow-hidden bg-slate-50">
            {#if selectedFileContent}
              <!-- Header Bar of Viewer -->
              <div class="p-3 border-b bg-white flex items-center justify-between gap-2">
                <div class="flex items-center gap-2 overflow-hidden">
                  <span class="text-base">📄</span>
                  <h4 class="text-xs font-bold text-slate-800 truncate" title={selectedFileName}>
                    {selectedFileName.replace('.txt', '')}
                  </h4>
                </div>

                <div class="flex items-center gap-2">
                  <!-- View Mode Toggle -->
                  <div class="flex bg-slate-100 p-0.5 rounded-lg text-[11px]">
                    <button
                      type="button"
                      onclick={() => isRawView = false}
                      class="px-2.5 py-1 rounded-md transition font-medium {!isRawView ? 'bg-white text-blue-700 shadow-sm font-bold' : 'text-slate-600 hover:text-slate-800'}"
                    >
                      {t("view_formatted")}
                    </button>
                    <button
                      type="button"
                      onclick={() => isRawView = true}
                      class="px-2.5 py-1 rounded-md transition font-medium {isRawView ? 'bg-white text-blue-700 shadow-sm font-bold' : 'text-slate-600 hover:text-slate-800'}"
                    >
                      {t("view_raw")}
                    </button>
                  </div>

                  <!-- Copy Button -->
                  <button
                    type="button"
                    onclick={copySelectedContent}
                    class="px-2.5 py-1 bg-slate-100 hover:bg-slate-200 text-slate-700 text-[11px] rounded-lg transition flex items-center gap-1 font-medium"
                    title={t("copy_file")}
                  >
                    {#if copyFeedback}
                      <span class="text-emerald-600 font-bold">✓ {t("file_copied")}</span>
                    {:else}
                      <span>📋 {t("copy_file")}</span>
                    {/if}
                  </button>
                </div>
              </div>

              <!-- Content Scrollable Body -->
              <div
                bind:this={contentContainerRef}
                class="flex-1 p-5 overflow-y-auto max-h-[64vh] bg-white scroll-smooth"
              >
                {#if isRawView}
                  <pre class="text-xs text-slate-700 font-sans whitespace-pre-wrap leading-relaxed">{selectedFileContent}</pre>
                {:else}
                  <!-- svelte-ignore a11y_click_events_have_key_events -->
                  <!-- svelte-ignore a11y_no_static_element_interactions -->
                  <div
                    class="knowledge-prose text-xs leading-relaxed"
                    onclick={handleContentClick}
                  >
                    {@html renderedMarkdownHtml}
                  </div>
                {/if}
              </div>
            {:else}
              <div class="h-full flex flex-col items-center justify-center text-slate-400 text-xs p-6 gap-2">
                <span class="text-3xl">📖</span>
                <span>{t("select_file_hint")}</span>
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
            <span>{t("mfa_modal_title")}</span>
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
          {t("mfa_modal_desc")}
        </p>

        <div class="flex gap-4 text-xs text-slate-700">
          <label class="flex items-center gap-1.5 cursor-pointer">
            <input type="radio" bind:group={mfaMethod} value="call" class="text-blue-600">
            <span>{t("mfa_call")}</span>
          </label>
          <label class="flex items-center gap-1.5 cursor-pointer">
            <input type="radio" bind:group={mfaMethod} value="sms" class="text-blue-600">
            <span>{t("mfa_sms")}</span>
          </label>
        </div>

        <button
          type="button"
          onclick={requestMfa}
          class="w-full py-2 bg-slate-100 hover:bg-slate-200 text-slate-800 text-xs font-semibold rounded-lg transition"
        >
          {t("mfa_send_code")}
        </button>

        <div class="pt-2">
          <label for="mfa-code-input" class="block text-xs font-semibold text-slate-600 mb-1">{t("mfa_code_label")}</label>
          <input
            id="mfa-code-input"
            type="text"
            bind:value={mfaCode}
            placeholder={t("mfa_code_placeholder")}
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
            {t("cancel")}
          </button>
          <button
            type="button"
            onclick={verifyMfa}
            class="px-5 py-2 bg-blue-600 hover:bg-blue-700 text-white text-xs font-bold rounded-lg shadow transition"
          >
            {t("mfa_verify")}
          </button>
        </div>
      </div>
    </div>
  {/if}

  <!-- Login (create token) Modal -->
  {#if showLoginModal}
    <div class="fixed inset-0 bg-black/50 z-50 flex items-center justify-center p-4 backdrop-blur-sm">
      <div class="bg-white rounded-2xl max-w-md w-full p-6 shadow-2xl space-y-4">
        <div class="flex items-center justify-between border-b pb-3">
          <h3 class="font-bold text-sm text-slate-800 flex items-center gap-2">
            <span>🔑</span>
            <span>{t("login_title")}</span>
          </h3>
          <button
            type="button"
            onclick={closeLoginModal}
            class="text-slate-400 hover:text-slate-600"
          >
            ✕
          </button>
        </div>

        {#if loginStep === "credentials"}
          <div>
            <label for="login-username" class="block text-xs font-semibold text-slate-600 mb-1">{t("system_number")}</label>
            <input
              id="login-username"
              type="text"
              dir="ltr"
              bind:value={loginUsername}
              placeholder={t("system_number_placeholder")}
              class="w-full text-xs rounded-lg border border-slate-300 p-2 focus:ring-2 focus:ring-blue-500 focus:outline-none"
            />
          </div>

          <div>
            <div class="flex items-center justify-between mb-1">
              <label for="login-password" class="text-xs font-semibold text-slate-600">{t("password")}</label>
              <button
                type="button"
                onclick={() => loginShowPassword = !loginShowPassword}
                class="text-[11px] text-blue-600 hover:underline"
              >
                {loginShowPassword ? t("hide") : t("show")}
              </button>
            </div>
            <input
              id="login-password"
              type={loginShowPassword ? "text" : "password"}
              dir="ltr"
              bind:value={loginPassword}
              placeholder={t("password_placeholder")}
              class="w-full text-xs rounded-lg border border-slate-300 p-2 focus:ring-2 focus:ring-blue-500 focus:outline-none"
            />
          </div>

          <button
            type="button"
            onclick={handleLogin}
            disabled={loginLoading}
            class="w-full py-2 bg-blue-600 hover:bg-blue-700 text-white text-xs font-bold rounded-lg shadow transition disabled:opacity-50"
          >
            {t("login_btn")}
          </button>
        {:else}
          <div>
            <label for="login-mfa-method" class="block text-xs font-semibold text-slate-600 mb-1">{t("mfa_methods")}</label>
            <select
              id="login-mfa-method"
              bind:value={loginMethodId}
              onchange={(e) => {
                const m = loginMethods.find((mm) => mm.id === e.target.value);
                loginSendType = m && m.send_types && m.send_types[0] ? m.send_types[0] : "";
                loginCodeSent = false;
              }}
              class="w-full text-xs rounded-lg border border-slate-300 p-2 focus:ring-2 focus:ring-blue-500 focus:outline-none bg-white"
            >
              {#each loginMethods as m}
                <option value={m.id}>{m.label}</option>
              {/each}
            </select>
          </div>

          <button
            type="button"
            onclick={handleSendLoginCode}
            disabled={loginLoading}
            class="w-full py-2 bg-slate-100 hover:bg-slate-200 text-slate-800 text-xs font-semibold rounded-lg transition disabled:opacity-50"
          >
            📞 {t("send_code")}
          </button>

          {#if loginCodeSent}
            <div class="pt-2">
              <label for="login-mfa-code" class="block text-xs font-semibold text-slate-600 mb-1">{t("code_sent")}</label>
              <input
                id="login-mfa-code"
                type="text"
                dir="ltr"
                bind:value={loginCode}
                placeholder={t("mfa_code_placeholder")}
                class="w-full text-center text-sm font-mono tracking-widest rounded-lg border border-slate-300 p-2 focus:ring-2 focus:ring-blue-500 focus:outline-none"
              />
            </div>

            <button
              type="button"
              onclick={handleValidateLoginCode}
              disabled={loginLoading}
              class="w-full py-2 bg-blue-600 hover:bg-blue-700 text-white text-xs font-bold rounded-lg shadow transition disabled:opacity-50"
            >
              {t("validate_code")}
            </button>
          {/if}
        {/if}

        {#if loginStatus}
          <p class="text-xs text-blue-700 font-medium">{loginStatus}</p>
        {/if}
        {#if loginError}
          <p class="text-xs text-rose-600 font-medium">❌ {loginError}</p>
        {/if}
      </div>
    </div>
  {/if}
</div>
