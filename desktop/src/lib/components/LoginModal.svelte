<script>
  /**
   * Single sign-in surface: system number + password, then the MFA step. The
   * token-check flow lands directly on the MFA step, so there is no second,
   * near-identical modal anywhere in the app.
   */
  import { t } from "$lib/i18n.svelte.js";

  let {
    step = "credentials",
    username = $bindable(""),
    password = $bindable(""),
    showPassword = $bindable(false),
    /** @type {any[]} */
    methods = [],
    methodId = $bindable(""),
    code = $bindable(""),
    codeSent = false,
    status = "",
    error = "",
    loading = false,
    rtl = true,
    /** @type {() => void} */
    onClose,
    /** @type {() => void} */
    onLogin,
    /** @type {() => void} */
    onSendCode,
    /** @type {() => void} */
    onValidateCode,
    /** @type {() => void} */
    onBack,
    /** @type {(id: string) => void} */
    onMethodChange
  } = $props();

  /** @type {HTMLElement | null} */
  let dialogRef = $state(null);

  /** Focusable controls inside the dialog, in DOM order. */
  function focusables() {
    if (!dialogRef) return /** @type {HTMLElement[]} */ ([]);
    return /** @type {HTMLElement[]} */ (
      Array.from(
        dialogRef.querySelectorAll(
          'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])'
        )
      )
    );
  }

  // The credentials step autofocuses its own first input. The MFA step is
  // reached directly from `checkToken` too, where nothing in the dialog would
  // otherwise hold focus at all — so move focus to its first real control.
  $effect(() => {
    if (step !== "mfa" || !dialogRef) return;
    const first = focusables().find(
      (el) => el.id === "login-mfa-method" || el.id === "login-mfa-code"
    );
    (first ?? focusables()[0])?.focus();
  });

  /**
   * Minimal focus trap: Tab / Shift+Tab wrap around inside the dialog instead of
   * walking off into the page behind the overlay.
   * @param {KeyboardEvent} e
   */
  function handleKeydown(e) {
    if (e.key !== "Tab") return;
    const items = focusables();
    if (items.length === 0) return;
    const first = items[0];
    const last = items[items.length - 1];
    const active = /** @type {HTMLElement | null} */ (document.activeElement);
    if (e.shiftKey) {
      if (active === first || !dialogRef?.contains(active)) {
        e.preventDefault();
        last.focus();
      }
    } else if (active === last || !dialogRef?.contains(active)) {
      e.preventDefault();
      first.focus();
    }
  }
</script>

<div class="fixed inset-0 bg-black/50 z-50 flex items-center justify-center p-4 backdrop-blur-sm">
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <div
    bind:this={dialogRef}
    onkeydown={handleKeydown}
    class="bg-white rounded-2xl max-w-md w-full p-6 shadow-2xl space-y-4"
    role="dialog"
    aria-modal="true"
    aria-labelledby="login-modal-title"
    tabindex="-1"
  >
    <div class="flex items-center justify-between border-b pb-3">
      <h3 id="login-modal-title" class="font-bold text-sm text-slate-800 flex items-center gap-2">
        <span aria-hidden="true">{step === "credentials" ? "🔑" : "🔐"}</span>
        <span>{step === "credentials" ? t("login_title") : t("mfa_modal_title")}</span>
      </h3>
      <button
        type="button"
        onclick={onClose}
        aria-label={t("close")}
        title={t("close")}
        class="text-slate-500 hover:text-slate-800"
      >
        ✕
      </button>
    </div>

    {#if step === "credentials"}
      <div>
        <label for="login-username" class="block text-xs font-semibold text-slate-600 mb-1">{t("system_number")}</label>
        <!-- svelte-ignore a11y_autofocus -->
        <input
          id="login-username"
          type="text"
          dir="ltr"
          autofocus
          bind:value={username}
          placeholder={t("system_number_placeholder")}
          class="w-full text-xs rounded-lg border border-slate-300 p-2 focus:ring-2 focus:ring-blue-500 focus:outline-none"
        />
      </div>

      <div>
        <div class="flex items-center justify-between mb-1">
          <label for="login-password" class="text-xs font-semibold text-slate-600">{t("password")}</label>
          <button
            type="button"
            onclick={() => (showPassword = !showPassword)}
            class="text-xs text-blue-600 hover:underline"
          >
            {showPassword ? t("hide") : t("show")}
          </button>
        </div>
        <input
          id="login-password"
          type={showPassword ? "text" : "password"}
          dir="ltr"
          bind:value={password}
          placeholder={t("password_placeholder")}
          class="w-full text-xs rounded-lg border border-slate-300 p-2 focus:ring-2 focus:ring-blue-500 focus:outline-none"
        />
      </div>

      <button
        type="button"
        onclick={onLogin}
        disabled={loading}
        class="w-full py-2 bg-blue-600 hover:bg-blue-700 text-white text-xs font-bold rounded-lg shadow transition disabled:opacity-50"
      >
        {t("login_btn")}
      </button>
    {:else}
      <p class="text-xs text-slate-600 leading-relaxed">{t("mfa_modal_desc")}</p>

      <div>
        <label for="login-mfa-method" class="block text-xs font-semibold text-slate-600 mb-1">{t("mfa_methods")}</label>
        <select
          id="login-mfa-method"
          bind:value={methodId}
          onchange={(e) => onMethodChange(/** @type {HTMLSelectElement} */ (e.currentTarget).value)}
          class="w-full text-xs rounded-lg border border-slate-300 p-2 focus:ring-2 focus:ring-blue-500 focus:outline-none bg-white"
        >
          {#each methods as m}
            <option value={m.id}>{m.label}</option>
          {/each}
        </select>
      </div>

      <button
        type="button"
        onclick={onSendCode}
        disabled={loading}
        class="w-full py-2 bg-slate-100 hover:bg-slate-200 text-slate-800 text-xs font-semibold rounded-lg transition disabled:opacity-50"
      >
        📞 {t("send_code")}
      </button>

      {#if codeSent}
        <div class="pt-2">
          <label for="login-mfa-code" class="block text-xs font-semibold text-slate-600 mb-1">{t("code_sent")}</label>
          <!-- svelte-ignore a11y_autofocus -->
          <input
            id="login-mfa-code"
            type="text"
            dir="ltr"
            autofocus
            inputmode="numeric"
            autocomplete="one-time-code"
            pattern="[0-9]*"
            maxlength="6"
            bind:value={code}
            oninput={() => { code = code.replace(/\D/g, "").slice(0, 6); }}
            placeholder={t("mfa_code_placeholder")}
            class="w-full text-center text-sm font-mono tracking-widest rounded-lg border border-slate-300 p-2 focus:ring-2 focus:ring-blue-500 focus:outline-none"
          />
        </div>

        <button
          type="button"
          onclick={onValidateCode}
          disabled={loading}
          class="w-full py-2 bg-blue-600 hover:bg-blue-700 text-white text-xs font-bold rounded-lg shadow transition disabled:opacity-50"
        >
          {t("validate_code")}
        </button>
      {/if}

      <button
        type="button"
        onclick={onBack}
        class="w-full py-2 border border-slate-300 text-slate-700 text-xs font-medium rounded-lg hover:bg-slate-50 transition"
      >
        {rtl ? "→" : "←"} {t("back")}
      </button>
    {/if}

    {#if status}
      <p class="text-xs text-blue-700 font-medium">{status}</p>
    {/if}
    {#if error}
      <p class="text-xs text-rose-600 font-medium">❌ {error}</p>
    {/if}
  </div>
</div>
