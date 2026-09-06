<script>
  /**
   * Settings drawer — everything that used to sit in the left-hand settings
   * card, moved off the workspace so the page can be a line editor.
   *
   * Every value is still owned by +page.svelte (and persisted by it, under the
   * same localStorage / keychain names as before); this component only binds to
   * them and calls the page's save handlers. The legacy GAS "script" mode is
   * kept, demoted into the collapsed "מתקדם" section.
   */
  import { DEFAULT_SCRIPT_URL } from "$lib/lineEditor.js";
  import { t } from "$lib/i18n.svelte.js";

  let {
    // --- target / provider / model -------------------------------------
    targetMode = "direct",
    /** @type {(mode: "direct" | "script") => void} */
    onTargetMode,
    aiProvider = $bindable("gemini"),
    /** @type {() => void} */
    onProviderChange,
    modelType = $bindable("regular"),
    modelSource = $bindable("list"),
    selectedModel = $bindable(""),
    customModel = $bindable(""),
    customBaseUrl = $bindable(""),
    /** @type {() => void} */
    onSaveCustomBaseUrl,
    /** @type {(provider: string) => {id: string, tag?: string, label?: string}[]} */
    getProviderModels,

    // --- credentials ----------------------------------------------------
    yemotToken = $bindable(""),
    showToken = $bindable(false),
    /** @type {{valid: boolean, message: string} | null} */
    tokenStatus = null,
    /** @type {() => void} */
    onSaveYemotToken,
    /** @type {() => void} */
    onCheckToken,
    /** @type {() => void} */
    onClearToken,
    /** @type {() => void} */
    onOpenLogin,
    /** The page's `$state` map of provider → key; mutated in place. */
    /** @type {Record<string, string>} */
    apiKeys = {},
    showApiKey = $bindable(false),
    /** @type {() => void} */
    onSaveApiKey,

    // --- run behaviour ---------------------------------------------------
    autoApply = $bindable(false),
    includeTree = $bindable(true),
    saveHistory = $bindable(true),
    /** @type {() => void} */
    onSaveAgentToggles,
    logoutOnFinish = $bindable(false),

    // --- legacy script mode ---------------------------------------------
    isPreviewMode = $bindable(true),
    scriptUrl = $bindable(""),

    rtl = true,
    /** @type {() => void} */
    onClose
  } = $props();

  /** @type {HTMLElement | null} */
  let panelRef = $state(null);

  // Move focus into the drawer when it opens, so keyboard users are not left
  // behind on the page underneath.
  $effect(() => {
    panelRef?.focus();
  });
</script>

<div class="fixed inset-0 z-50 flex" dir={rtl ? "rtl" : "ltr"}>
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="absolute inset-0 bg-slate-900/40" onclick={onClose}></div>

  <div
    bind:this={panelRef}
    tabindex="-1"
    role="dialog"
    aria-modal="true"
    aria-labelledby="settings-drawer-title"
    class="relative ms-auto h-full w-full max-w-md bg-white shadow-2xl overflow-y-auto focus:outline-none"
  >
    <div
      class="sticky top-0 bg-white border-b border-slate-200 px-5 py-3 flex items-center justify-between z-10"
    >
      <h2 id="settings-drawer-title" class="text-sm font-bold text-slate-900">
        {t("settings_drawer_title")}
      </h2>
      <button
        type="button"
        onclick={onClose}
        aria-label={t("settings_close")}
        class="w-8 h-8 rounded-lg text-slate-500 hover:bg-slate-100 hover:text-slate-800 transition focus-visible:outline-2 focus-visible:outline-blue-600"
      >
        ✕
      </button>
    </div>

    <div class="p-5 space-y-5">
      <!-- AI source: direct provider vs. relay server -->
      <div>
        <span id="target-mode-label" class="block text-xs font-semibold text-slate-600 mb-2">
          {t("target_mode")}
        </span>
        <div
          class="grid grid-cols-2 gap-2 p-1 bg-slate-100 rounded-xl"
          role="group"
          aria-labelledby="target-mode-label"
        >
          <button
            type="button"
            onclick={() => onTargetMode("direct")}
            aria-pressed={targetMode === "direct"}
            class="py-2 px-3 text-xs font-medium rounded-lg transition {targetMode === 'direct'
              ? 'bg-white text-blue-700 shadow-sm font-bold'
              : 'text-slate-600 hover:text-slate-900'}"
          >
            {t("mode_direct")}
          </button>
          <button
            type="button"
            onclick={() => onTargetMode("script")}
            aria-pressed={targetMode === "script"}
            class="py-2 px-3 text-xs font-medium rounded-lg transition {targetMode === 'script'
              ? 'bg-white text-blue-700 shadow-sm font-bold'
              : 'text-slate-600 hover:text-slate-900'}"
          >
            {t("mode_script")}
          </button>
        </div>
        <p class="text-xs text-slate-500 mt-1.5 leading-relaxed">
          {targetMode === "direct" ? t("mode_direct_hint") : t("mode_script_hint")}
        </p>
      </div>

      {#if targetMode === "script"}
        <div class="rounded-xl border border-slate-200 bg-slate-50 p-3 space-y-2 text-xs text-slate-700">
          <p class="font-semibold text-slate-800">{t("relay_sent_title")}</p>
          <ul class="list-disc ps-5 space-y-0.5">
            <li>{t("relay_sent_task")}</li>
            <li>{t("relay_sent_model")}</li>
            <li>{t("relay_sent_preview")}</li>
            <li>{t("relay_sent_key")}</li>
          </ul>
          <p class="font-semibold text-slate-800 pt-1">{t("relay_not_sent_title")}</p>
          <ul class="list-disc ps-5 space-y-0.5">
            <li>{t("relay_not_sent_token")}</li>
            <li>{t("relay_not_sent_line")}</li>
          </ul>
        </div>

        <div
          role="note"
          class="rounded-xl border border-amber-300 bg-amber-50 p-3 text-xs text-amber-900 leading-relaxed"
        >
          <span class="font-bold">⚠️ {t("relay_key_warning_title")}</span>
          {t("relay_key_warning")}
        </div>

        <div>
          <span id="model-type-label" class="block text-xs font-semibold text-slate-600 mb-1.5">
            {t("model_label")}
          </span>
          <div class="space-y-1.5" role="radiogroup" aria-labelledby="model-type-label">
            <label class="flex items-center gap-2 text-xs text-slate-700 cursor-pointer">
              <input
                type="radio"
                bind:group={modelType}
                value="regular"
                class="text-blue-600 focus:ring-blue-500"
              />
              <span>{t("model_regular")}</span>
            </label>
            <label class="flex items-center gap-2 text-xs text-slate-700 cursor-pointer">
              <input
                type="radio"
                bind:group={modelType}
                value="pro"
                class="text-blue-600 focus:ring-blue-500"
              />
              <span>{t("model_pro")}</span>
            </label>
          </div>
        </div>

        <label class="flex items-center gap-2 text-xs text-slate-700 cursor-pointer">
          <input
            type="checkbox"
            bind:checked={isPreviewMode}
            class="rounded text-blue-600 focus:ring-blue-500"
          />
          <span class="font-medium">{t("preview_mode")}</span>
        </label>

        <div>
          <label for="script-url-input" class="block text-xs font-semibold text-slate-600 mb-1">
            {t("script_url_label")}
          </label>
          <input
            id="script-url-input"
            type="text"
            dir="ltr"
            bind:value={scriptUrl}
            placeholder={DEFAULT_SCRIPT_URL}
            class="w-full text-xs rounded-lg border border-slate-300 p-2 focus:ring-2 focus:ring-blue-500 focus:outline-none text-slate-600"
          />
          <p class="text-xs text-slate-500 mt-1 leading-relaxed">
            {scriptUrl.trim() && scriptUrl.trim() !== DEFAULT_SCRIPT_URL
              ? t("script_url_custom_hint")
              : t("script_url_default_hint")}
            {#if scriptUrl.trim() !== DEFAULT_SCRIPT_URL}
              <button
                type="button"
                onclick={() => (scriptUrl = DEFAULT_SCRIPT_URL)}
                class="text-blue-600 hover:underline focus-visible:outline-2 focus-visible:outline-blue-600"
              >
                {t("script_url_reset")}
              </button>
            {/if}
          </p>
        </div>
      {/if}

      <!-- Provider (direct mode) -->
      {#if targetMode === "direct"}
        <div>
          <label for="provider-select" class="block text-xs font-semibold text-slate-600 mb-1.5">
            {t("provider_label")}
          </label>
          <select
            id="provider-select"
            bind:value={aiProvider}
            onchange={onProviderChange}
            class="w-full text-xs rounded-lg border border-slate-300 p-2 focus:ring-2 focus:ring-blue-500 focus:outline-none bg-white"
          >
            <option value="claude">{t("provider_claude")}</option>
            <option value="gemini">{t("provider_gemini")}</option>
            <option value="openai">{t("provider_openai")}</option>
            <option value="groq">{t("provider_groq")}</option>
            <option value="custom">{t("provider_custom")}</option>
          </select>
        </div>

        <div>
          <label for="model-select" class="block text-xs font-semibold text-slate-600 mb-1.5">
            {t("model_label")}
          </label>
          <div class="grid grid-cols-2 gap-2 p-1 bg-slate-100 rounded-xl mb-2">
            <button
              type="button"
              onclick={() => (modelSource = "list")}
              disabled={aiProvider === "custom"}
              class="py-1.5 px-3 text-xs font-medium rounded-lg transition disabled:opacity-40 {modelSource ===
              'list'
                ? 'bg-white text-blue-700 shadow-sm font-bold'
                : 'text-slate-600 hover:text-slate-900'}"
            >
              {t("model_from_list")}
            </button>
            <button
              type="button"
              onclick={() => (modelSource = "manual")}
              class="py-1.5 px-3 text-xs font-medium rounded-lg transition {modelSource === 'manual'
                ? 'bg-white text-blue-700 shadow-sm font-bold'
                : 'text-slate-600 hover:text-slate-900'}"
            >
              {t("model_manual")}
            </button>
          </div>

          {#if aiProvider !== "custom" && modelSource === "list"}
            <select
              id="model-select"
              bind:value={selectedModel}
              class="w-full text-xs rounded-lg border border-slate-300 p-2 focus:ring-2 focus:ring-blue-500 focus:outline-none bg-white"
            >
              {#each getProviderModels(aiProvider) as m}
                <option value={m.id}>
                  {m.label ? t(m.label) : m.id}{m.tag ? ` — ${t(m.tag)}` : ""}
                </option>
              {/each}
            </select>
          {:else}
            <input
              id="model-select"
              type="text"
              dir="ltr"
              bind:value={customModel}
              placeholder={t("model_manual_placeholder")}
              class="w-full text-xs rounded-lg border border-slate-300 p-2 focus:ring-2 focus:ring-blue-500 focus:outline-none text-slate-600"
            />
          {/if}
        </div>

        {#if aiProvider === "custom"}
          <div>
            <label for="custom-api-url" class="block text-xs font-semibold text-slate-600 mb-1.5">
              {t("custom_url_label")}
            </label>
            <input
              id="custom-api-url"
              type="text"
              dir="ltr"
              bind:value={customBaseUrl}
              onchange={onSaveCustomBaseUrl}
              placeholder={t("custom_url_placeholder")}
              class="w-full text-xs rounded-lg border border-slate-300 p-2 focus:ring-2 focus:ring-blue-500 focus:outline-none text-slate-600"
            />
            <p class="text-xs text-slate-500 mt-1.5 leading-relaxed">{t("custom_url_hint")}</p>
          </div>
        {/if}
      {/if}

      <!-- Yemot token -->
      <div class="border-t pt-4">
        <div class="flex items-center justify-between mb-1.5 flex-wrap gap-1">
          <label for="yemot-token-input" class="text-xs font-semibold text-slate-600">
            {t("token_label")}
          </label>
          <div class="flex items-center gap-2">
            <button type="button" onclick={onOpenLogin} class="text-xs text-blue-600 hover:underline">
              {t("get_token")}
            </button>
            {#if yemotToken}
              <button
                type="button"
                onclick={onClearToken}
                title={t("clear_token")}
                aria-label={t("clear_token")}
                class="text-xs text-rose-600 hover:text-rose-700"
              >
                🗑 {t("clear_token")}
              </button>
            {/if}
            <button
              type="button"
              onclick={() => (showToken = !showToken)}
              class="text-xs text-blue-600 hover:underline"
            >
              {showToken ? t("hide") : t("show")}
            </button>
          </div>
        </div>
        <div class="relative">
          <input
            id="yemot-token-input"
            type={showToken ? "text" : "password"}
            bind:value={yemotToken}
            onchange={onSaveYemotToken}
            placeholder={t("token_placeholder")}
            class="w-full text-xs rounded-lg border border-slate-300 p-2 {rtl
              ? 'pr-2 pl-14'
              : 'pl-2 pr-14'} focus:ring-2 focus:ring-blue-500 focus:outline-none"
          />
          <button
            type="button"
            onclick={onCheckToken}
            class="absolute {rtl
              ? 'left-1'
              : 'right-1'} top-1 bottom-1 px-2.5 bg-slate-100 hover:bg-slate-200 text-slate-700 text-xs font-medium rounded-md transition"
          >
            {t("check")}
          </button>
        </div>
        {#if tokenStatus}
          <p
            class="text-xs mt-1.5 {tokenStatus.valid
              ? 'text-emerald-700'
              : 'text-rose-700'} font-medium"
          >
            {tokenStatus.valid ? "✅ " : "❌ "}{tokenStatus.message}
          </p>
        {/if}
        <p class="text-xs text-slate-500 mt-1.5 leading-relaxed">🔒 {t("local_note")}</p>
      </div>

      <!-- Personal API key -->
      <div class="border-t pt-4">
        <div class="flex items-center justify-between mb-1.5">
          <label for="api-key-input" class="text-xs font-semibold text-slate-600">
            {t("api_key_label")}
          </label>
          <button
            type="button"
            onclick={() => (showApiKey = !showApiKey)}
            class="text-xs text-blue-600 hover:underline"
          >
            {showApiKey ? t("hide") : t("show")}
          </button>
        </div>
        <input
          id="api-key-input"
          type={showApiKey ? "text" : "password"}
          bind:value={apiKeys[aiProvider]}
          onchange={onSaveApiKey}
          placeholder={t("api_key_placeholder")}
          class="w-full text-xs rounded-lg border border-slate-300 p-2 focus:ring-2 focus:ring-blue-500 focus:outline-none"
        />
        <p class="text-xs text-slate-500 mt-1.5">
          {targetMode === "direct" ? t("api_key_hint_direct") : t("api_key_hint_script")}
        </p>
        <p class="text-xs text-slate-500 mt-1 leading-relaxed">🔒 {t("secrets_note")}</p>
      </div>

      <!-- Run behaviour -->
      {#if targetMode === "direct"}
        <div class="border-t pt-4 space-y-3">
          <div>
            <label class="flex items-center gap-2 text-xs text-slate-700 cursor-pointer">
              <input
                type="checkbox"
                bind:checked={autoApply}
                onchange={onSaveAgentToggles}
                class="rounded text-amber-600 focus:ring-amber-500"
              />
              <span class="font-medium">{t("auto_apply_label")}</span>
            </label>
            {#if autoApply}
              <p
                class="text-xs text-amber-800 bg-amber-50 border border-amber-200 rounded-lg p-2 mt-1.5 leading-relaxed"
              >
                ⚠️ {t("auto_apply_warning")}
              </p>
            {:else}
              <p class="text-xs text-slate-500 mt-1 leading-relaxed">{t("auto_apply_warning")}</p>
            {/if}
          </div>

          <div>
            <label class="flex items-center gap-2 text-xs text-slate-700 cursor-pointer">
              <input
                type="checkbox"
                bind:checked={includeTree}
                onchange={onSaveAgentToggles}
                class="rounded text-blue-600 focus:ring-blue-500"
              />
              <span class="font-medium">{t("include_tree_label")}</span>
            </label>
            <p class="text-xs text-slate-500 mt-1 leading-relaxed">{t("include_tree_hint")}</p>
          </div>

          <div>
            <label class="flex items-center gap-2 text-xs text-slate-700 cursor-pointer">
              <input
                type="checkbox"
                bind:checked={saveHistory}
                onchange={onSaveAgentToggles}
                class="rounded text-blue-600 focus:ring-blue-500"
              />
              <span class="font-medium">{t("save_history_label")}</span>
            </label>
            <p class="text-xs text-slate-500 mt-1 leading-relaxed">{t("save_history_hint")}</p>
          </div>
        </div>
      {/if}

      <label class="flex items-center gap-2 text-xs text-slate-700 cursor-pointer border-t pt-4">
        <input
          type="checkbox"
          bind:checked={logoutOnFinish}
          class="rounded text-blue-600 focus:ring-blue-500"
        />
        <span>{t("logout_on_finish")}</span>
      </label>

      <p class="text-xs text-slate-500 leading-relaxed border-t pt-4">⚠️ {t("disclaimer")}</p>
    </div>
  </div>
</div>

<style>
  @media (prefers-reduced-motion: reduce) {
    .transition {
      transition: none !important;
    }
  }
</style>
