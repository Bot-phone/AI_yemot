<script>
  import { onMount, onDestroy, tick } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { open as openFileDialog } from "@tauri-apps/plugin-dialog";
  import { t, i18n, isRTL, setLocale, availableLocales } from "$lib/i18n.svelte.js";
  import AgentTimeline from "$lib/components/AgentTimeline.svelte";
  import ActionApprovalList from "$lib/components/ActionApprovalList.svelte";
  import LoginModal from "$lib/components/LoginModal.svelte";
  import KnowledgeModal from "$lib/components/KnowledgeModal.svelte";
  import LineTree from "$lib/components/LineTree.svelte";
  import ExtensionInspector from "$lib/components/ExtensionInspector.svelte";
  import ChangeLog from "$lib/components/ChangeLog.svelte";
  import TaskHistory from "$lib/components/TaskHistory.svelte";
  import PresetBar from "$lib/components/PresetBar.svelte";
  import SettingsPanel from "$lib/components/SettingsPanel.svelte";
  import {
    AUDIO_EXTENSIONS,
    baseName,
    errorText,
    formatSize,
    isMissingCommand,
    isSessionExpired,
    loadPresets,
    mimeFromName,
    savePresets,
    seedPresets
  } from "$lib/lineEditor.js";

  // Interface direction (reactive to language changes)
  let rtl = $derived(isRTL());

  // State (Svelte 5 Runes)
  let targetMode = $state("direct"); // "direct" | "script" — default: direct (persisted locally)
  let aiProvider = $state("gemini"); // "claude" | "gemini" | "openai" | "groq" | "custom"
  let modelType = $state("regular"); // "regular" | "pro" (script mode)
  // Direct mode model selection: from a known list, or manually entered ID
  let modelSource = $state("list"); // "list" | "manual"
  let selectedModel = $state("gemini-2.5-flash"); // model ID chosen from the list
  let customModel = $state(""); // model ID entered manually
  let customBaseUrl = $state(""); // custom full API URL (custom provider)
  let promptText = $state("");
  let yemotToken = $state("");
  let showToken = $state(false);
  /** API key per provider — stored in the OS keychain, never in localStorage. */
  /** @type {Record<string, string>} */
  let apiKeys = $state({ claude: "", gemini: "", openai: "", groq: "", custom: "" });
  /** The key of the provider currently selected. */
  let apiKey = $derived(apiKeys[aiProvider] ?? "");
  let showApiKey = $state(false);
  let isPreviewMode = $state(true);
  let logoutOnFinish = $state(false);
  /** Matches `SCRIPT_DISABLED_PREFIX` in src-tauri/src/ai.rs. */
  const SCRIPT_DISABLED_PREFIX = "SCRIPT_DISABLED: ";
  let scriptUrl = $state("https://script.google.com/macros/s/AKfycbz_REPLACE_ME/exec");

  // Known model list per direct provider (first two are the classic Regular/Pro).
  // tag = optional i18n key appended to the label; label = i18n key replacing the raw id.
  /** @type {Record<string, {id: string, tag?: string, label?: string}[]>} */
  const PROVIDER_MODELS = {
    // Claude uses the contract aliases ("regular" / "pro"); Rust maps them to real model ids.
    claude: [
      { id: "regular", label: "model_claude_regular" },
      { id: "pro", label: "model_claude_pro" },
    ],
    gemini: [
      { id: "gemini-2.5-flash", tag: "model_regular" },
      { id: "gemini-2.5-pro", tag: "model_pro" },
      { id: "gemini-2.0-flash" },
      { id: "gemini-2.5-flash-lite" },
      { id: "gemini-1.5-flash" },
      { id: "gemini-1.5-pro" },
    ],
    openai: [
      { id: "gpt-4.1-mini", tag: "model_small_recent" },
      { id: "gpt-4.1", tag: "model_pro" },
      { id: "gpt-4.1-nano" },
      { id: "gpt-4o-mini" },
      { id: "gpt-4o" },
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

  // Agent-run settings (direct mode)
  let autoApply = $state(false); // true = mutating tools run immediately, no approval step
  let includeTree = $state(true); // append the "[מצב נוכחי]" root tree to the first user message
  /** Keep a local record of every finished task, so one can be continued later. */
  let saveHistory = $state(true);

  // Status & Progress
  let isLoading = $state(false);
  let statusMessage = $state("");
  let errorMessage = $state("");
  let resultOutput = $state("");
  /** @type {any[]} */
  let parsedActions = $state([]);
  /** Per-param outcomes of the last script-mode execution, grouped by path. */
  /** @type {any[]} */
  let scriptActionResults = $state([]);

  // ----- Agent run state (direct mode) -----
  /** @type {string | null} */
  let runId = $state(null);
  let agentStarting = $state(false); // a start_agent_run call is in flight (run_id not known yet)
  let agentRunning = $state(false);
  let agentCancelling = $state(false);
  let agentTurn = $state(0);
  let agentMaxTurns = $state(0);
  /** Ordered timeline of the current run. */
  /** @type {any[]} */
  let agentTimeline = $state([]);
  /** @type {any} */
  let agentRetryNotice = $state(null);
  /** @type {any} */
  let agentFinish = $state(null);
  /** @type {any} */
  let agentError = $state(null);
  /** Proposed actions awaiting approval (agent mode). */
  /** @type {any[]} */
  let proposedActions = $state([]);
  /** action_id -> ActionApplyResult */
  /** @type {Record<string, any>} */
  let actionResults = $state({});
  /** Unlisten callbacks for the active agent subscription. */
  /** @type {Array<() => void>} */
  let agentUnlisteners = [];

  // Token verification status
  /** @type {{valid: boolean, message: string} | null} */
  let tokenStatus = $state(null);

  // Login (create token) state
  let showLoginModal = $state(false);
  let loginUsername = $state("");
  let loginPassword = $state("");
  let loginShowPassword = $state(false);
  let loginToken = $state("");
  let loginStep = $state("credentials"); // "credentials" | "mfa"
  /** @type {any[]} */
  let loginMethods = $state([]);
  let loginMethodId = $state("");
  let loginSendType = $state("");
  let loginCode = $state("");
  let loginCodeSent = $state(false);
  let loginStatus = $state("");
  let loginError = $state("");
  let loginLoading = $state(false);

  // Knowledge Explorer
  /** @type {any[]} */
  let knowledgeFiles = $state([]);
  let searchQuery = $state("");
  /** @type {string | null} */
  let selectedFileContent = $state(null);
  let selectedFileName = $state("");
  let showKnowledgeModal = $state(false);
  let isRawView = $state(false);
  let copyFeedback = $state(false);
  /** @type {HTMLElement | null} */
  let contentContainerRef = $state(null);


  // GitHub Update info
  /** @type {any} */
  let updateInfo = $state(null);

  // ----- Approval-panel UX state -----
  /** Inline message shown inside the approval card (replaces alert()). */
  let approvalNotice = $state("");
  /** Same, for the legacy script-mode preview list. */
  let scriptPreviewNotice = $state("");
  /** The risky-change confirmation step is armed and waiting for a click. */
  let confirmRisky = $state(false);
  /** action_id -> "" | "done" | "failed" | "unavailable" for the undo button. */
  /** @type {Record<string, string>} */
  let undoState = $state({});
  /** action_id -> message returned by the last undo attempt. */
  /** @type {Record<string, string>} */
  let undoMessages = $state({});

  // ----- Run-feedback state -----
  /** Prompt of the last submitted run, so "נסה שוב" can repeat it. */
  let lastRunPrompt = $state("");
  /**
   * Whether that run was a fresh task or a continuation ("דייק את המשימה").
   * Retry and the post-re-login auto-rerun have to reproduce the same kind:
   * re-dispatching a refinement as a fresh run drops the parent's transcript and
   * silently turns "also do X" into a task that only says "also do X".
   * @type {"fresh" | "continue"}
   */
  let lastRunKind = $state("fresh");
  /** Parent run id of the last continuation, "" when the last run was fresh. */
  let lastRunParentId = $state("");
  /** Set when a run died with session_expired: re-run once the user logs back in. */
  let pendingRerun = $state(false);
  /** Wall-clock start of the run, for the running seconds counter. */
  let runStartedAt = $state(0);
  /** Ticks once a second while a run is active. */
  let runElapsedMs = $state(0);
  /** @type {ReturnType<typeof setInterval> | null} */
  let runTimer = null;
  /** @type {HTMLElement | null} */
  let timelineRef = $state(null);
  /** The user has not scrolled away from the bottom of the timeline. */
  let timelineAtBottom = true;

  // ----- Modal focus management -----
  /** @type {HTMLElement | null} */
  let lastFocusedBeforeModal = null;

  // ----- Knowledge search (debounced, backed by search_knowledge_files) -----
  /** @type {any[]} */
  let knowledgeMatches = $state([]);
  /** @type {ReturnType<typeof setTimeout> | null} */
  let searchDebounce = null;

  // ============================================================
  //  Line editor workspace — tree, inspector, context, change log
  // ============================================================

  /** The settings drawer (everything that used to be the left-hand card). */
  let showSettings = $state(false);
  /** System number of the line we are signed in to, for the connection chip. */
  let connectedSystem = $state("");

  // ----- מבנה הקו (extension tree) -----
  /** @type {any[]} ExtTreeNode[] from `get_extension_tree`. */
  let treeNodes = $state([]);
  let treeLoading = $state(false);
  let treeError = $state("");
  /** A tree actually came back from the line — proof the token still works. */
  let treeOk = $state(false);
  /** path -> open? */
  /** @type {Record<string, boolean>} */
  let treeExpanded = $state({});

  // ----- Inspector -----
  /** @type {string | null} */
  let selectedExtPath = $state(null);
  /** @type {string | null} */
  let selectedExtDisplay = $state(null);
  /** @type {any} ExtensionDetail from `read_extension`. */
  let extDetail = $state(null);
  let extLoading = $state(false);
  let extError = $state("");

  // ----- Task composer -----
  /** Extensions pinned onto the task as context chips. */
  /** @type {{path: string, display: string}[]} */
  let contextExts = $state([]);
  /** Audio files picked for upload: `Attachment` rows of the R2 contract. */
  /** @type {{id: string, name: string, local_path: string, size: number, mime: string}[]} */
  let attachments = $state([]);
  let attachError = $state("");
  /** User-editable ready-made tasks (`ai_yemot_presets_v1`). */
  /** @type {{id: string, title: string, text: string}[]} */
  let presets = $state([]);
  /** The resolved model id of the last run, reused by a continuation. */
  let lastPayloadModel = $state("");

  // ----- Task continuation ("דייק את המשימה") -----
  let refineText = $state("");
  let refineBusy = $state(false);
  let refineError = $state("");
  /** Every instruction of the current task chain, oldest first. */
  /** @type {string[]} */
  let taskChain = $state([]);

  // ----- יומן שינויים (applied change log) -----
  /** @type {any[]} AppliedChange[] from `list_applied_changes`. */
  let appliedChanges = $state([]);
  let changeLogLoading = $state(false);
  let changeLogError = $state("");
  /**
   * "<run_id>:<action_id>" -> "" | "done" | "failed" | "unavailable".
   * Action ids are per-run (`a_1` in every run) while the log spans runs, so the
   * run id has to be part of the key — both here and in the `{#each}` above it.
   */
  /** @type {Record<string, string>} */
  let changeLogUndoState = $state({});

  // ----- היסטוריית משימות (local task history) -----
  /** @type {any[]} TaskSummary[] from `list_task_history`. */
  let taskHistory = $state([]);
  let historyLoading = $state(false);
  let historyError = $state("");
  /**
   * The task_id of a stored task loaded into the workspace, "" while the panel
   * is showing a live run. Its proposals are read-only: the run that made them
   * is gone, so there is no handle left to approve or undo them through.
   */
  let loadedTaskId = $state("");
  let historyReadOnly = $derived(loadedTaskId !== "");
  /** A run (or a start/continue call) is in flight. */
  let runBusy = $derived(agentRunning || agentStarting || refineBusy);

  onMount(async () => {
    // Load local storage if previously saved
    try {
      const savedScriptUrl = localStorage.getItem("ai_yemot_script_url");
      if (savedScriptUrl) scriptUrl = savedScriptUrl;

      const savedTargetMode = localStorage.getItem("ai_yemot_target_mode");
      if (savedTargetMode === "script" || savedTargetMode === "direct") {
        targetMode = savedTargetMode;
      }

      const savedProvider = localStorage.getItem("ai_yemot_provider");
      if (savedProvider && ["claude", "gemini", "openai", "groq", "custom"].includes(savedProvider)) {
        aiProvider = savedProvider;
      }

      // Agent-run settings (contract keys: autoApply / includeTree)
      const savedAutoApply = localStorage.getItem("autoApply");
      if (savedAutoApply !== null) autoApply = savedAutoApply === "true";

      const savedIncludeTree = localStorage.getItem("includeTree");
      if (savedIncludeTree !== null) includeTree = savedIncludeTree === "true";

      // Default on: the history is what makes "המשך משימה" work after a restart.
      const savedSaveHistory = localStorage.getItem("ai_yemot_save_history");
      if (savedSaveHistory !== null) saveHistory = savedSaveHistory === "true";

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

      const savedSystem = localStorage.getItem("ai_yemot_system");
      if (savedSystem) connectedSystem = savedSystem;

      normalizeModelSelection();
    } catch (_) {}

    // Ready-made tasks are seeded on first run and editable afterwards.
    presets = loadPresets(t);

    // Secrets live in the OS keychain, not in localStorage (migrated on first run).
    await loadSecrets();

    // The workspace needs the line itself: both calls degrade quietly when the
    // backend does not provide them yet.
    void loadTree();
    void loadChangeLog();
    void loadTaskHistory();

    // Load embedded knowledge files count
    try {
      knowledgeFiles = await invoke("get_knowledge_files");
      knowledgeMatches = knowledgeFiles;
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

  // ============================================================
  //  Secrets — OS keychain via the `secret_*` Tauri commands
  // ============================================================

  /** Secret names the backend accepts, mirrored from src-tauri/src/secrets.rs. */
  const PROVIDER_NAMES = ["claude", "gemini", "openai", "groq", "custom"];

  /**
   * @param {string} name
   * @returns {Promise<string>}
   */
  async function secretGet(name) {
    try {
      const v = await invoke("secret_get", { name });
      return typeof v === "string" ? v : "";
    } catch (e) {
      console.error("secret_get failed:", name, e);
      return "";
    }
  }

  /**
   * @param {string} name
   * @param {string} value
   * @returns {Promise<boolean>} true when the keychain actually took the value.
   */
  async function secretSet(name, value) {
    try {
      await invoke("secret_set", { name, value });
      return true;
    } catch (e) {
      console.error("secret_set failed:", name, e);
      return false;
    }
  }

  /** @param {string} name */
  async function secretDelete(name) {
    try {
      await invoke("secret_delete", { name });
    } catch (e) {
      console.error("secret_delete failed:", name, e);
    }
  }

  /**
   * Read every secret out of the keychain, migrating the plaintext localStorage
   * values written by earlier versions on the way (and deleting them there).
   */
  async function loadSecrets() {
    // A failed keychain write must never cost the user their credentials: the
    // localStorage copy is deleted only once the secret is safely stored, and
    // the value stays in memory for this session either way.
    let migrationFailed = false;
    let fallbackToken = "";
    let fallbackKey = "";
    let fallbackUrl = "";
    try {
      const legacyToken = localStorage.getItem("ai_yemot_token");
      if (legacyToken) {
        if (await secretSet("yemot_token", legacyToken)) {
          localStorage.removeItem("ai_yemot_token");
        } else {
          migrationFailed = true;
          fallbackToken = legacyToken;
        }
      }
      const legacyKey = localStorage.getItem("ai_yemot_api_key");
      if (legacyKey) {
        // The old build kept a single key with no provider attached — it belongs
        // to whichever provider was selected when it was saved.
        if (await secretSet(`api_key_${aiProvider}`, legacyKey)) {
          localStorage.removeItem("ai_yemot_api_key");
        } else {
          migrationFailed = true;
          fallbackKey = legacyKey;
        }
      }
      const legacyUrl = localStorage.getItem("ai_yemot_custom_base_url");
      if (legacyUrl) {
        if (await secretSet("custom_base_url", legacyUrl)) {
          localStorage.removeItem("ai_yemot_custom_base_url");
        } else {
          migrationFailed = true;
          fallbackUrl = legacyUrl;
        }
      }
    } catch (_) {}

    yemotToken = (await secretGet("yemot_token")) || fallbackToken;
    customBaseUrl = (await secretGet("custom_base_url")) || fallbackUrl;
    for (const p of PROVIDER_NAMES) {
      apiKeys[p] = await secretGet(`api_key_${p}`);
    }
    if (fallbackKey && !apiKeys[aiProvider]) apiKeys[aiProvider] = fallbackKey;

    if (migrationFailed) {
      errorMessage = t("keychain_write_failed");
    }
  }

  /** Persist the API key of the provider currently selected. */
  async function saveApiKey() {
    await secretSet(`api_key_${aiProvider}`, (apiKeys[aiProvider] ?? "").trim());
  }

  async function saveCustomBaseUrl() {
    await secretSet("custom_base_url", customBaseUrl.trim());
  }

  async function saveYemotToken() {
    await secretSet("yemot_token", yemotToken.trim());
  }

  async function saveSettings() {
    await saveYemotToken();
    await saveApiKey();
    await saveCustomBaseUrl();
    try {
      localStorage.setItem("ai_yemot_script_url", scriptUrl);
      localStorage.setItem("ai_yemot_target_mode", targetMode);
      localStorage.setItem("ai_yemot_provider", aiProvider);
      localStorage.setItem("ai_yemot_model_source", modelSource);
      localStorage.setItem("ai_yemot_selected_model", selectedModel);
      localStorage.setItem("ai_yemot_custom_model", customModel);
      localStorage.setItem("autoApply", String(autoApply));
      localStorage.setItem("includeTree", String(includeTree));
    } catch (_) {}
  }

  // Persist the agent toggles as soon as they change (they live outside the submit flow).
  function saveAgentToggles() {
    try {
      localStorage.setItem("autoApply", String(autoApply));
      localStorage.setItem("includeTree", String(includeTree));
      localStorage.setItem("ai_yemot_save_history", String(saveHistory));
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
   * @returns {{id: string, tag?: string, label?: string}[]}
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
    // Persist right away: `saveSettings` only runs on submit, so a provider the
    // user picked and never submitted was lost on restart.
    try {
      localStorage.setItem("ai_yemot_provider", aiProvider);
      localStorage.setItem("ai_yemot_model_source", modelSource);
      localStorage.setItem("ai_yemot_selected_model", selectedModel);
    } catch (_) {}
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
        await saveYemotToken();
        void loadTree();
      } else if (res.mfa_required) {
        // One MFA surface only: reuse the login modal's MFA step with the token
        // we already hold, instead of a second, near-identical modal.
        tokenStatus = { valid: false, message: t("mfa_required") };
        openLoginModal();
        loginToken = yemotToken.trim();
        loginStatus = t("mfa_required");
        await loadLoginMethods();
      } else {
        tokenStatus = { valid: false, message: res.message };
      }
    } catch (e) {
      tokenStatus = { valid: false, message: t("comm_error", { error: e }) };
    } finally {
      statusMessage = "";
    }
  }

  // ============================================================
  //  Line editor workspace
  // ============================================================

  /** Signed in *and* proven so — either by "בדוק" or by a tree that loaded. */
  let connected = $derived.by(() => {
    // The cast keeps TypeScript from narrowing `tokenStatus` to its `null`
    // initializer at this point in the module body.
    const status = /** @type {{valid: boolean, message: string} | null} */ (tokenStatus);
    return !!yemotToken.trim() && (status?.valid === true || treeOk);
  });

  let connectionLabel = $derived(
    connected
      ? connectedSystem
        ? t("connected_to", { system: connectedSystem })
        : t("connected")
      : t("not_connected")
  );

  /**
   * Turn a rejected workspace `invoke` into the string its panel should show.
   * A command this build does not register is "not available", not a failure;
   * an expired Yemot session opens the login modal exactly like a dead run does.
   * @param {unknown} e
   * @param {string} unavailableKey
   * @param {string} errorKey
   * @returns {string}
   */
  function describeLineError(e, unavailableKey, errorKey) {
    // Session-expiry is checked first: Yemot's "SESSION_EXPIRED: …" text can
    // itself contain a "not found"-ish phrase, and mistaking a dead session for
    // an unregistered command hides the login prompt the user actually needs.
    if (isSessionExpired(e)) {
      openLoginModal();
      loginError = t("agent_session_expired");
      return t("agent_session_expired");
    }
    if (isMissingCommand(e)) return t(unavailableKey);
    return t(errorKey, { error: errorText(e) });
  }

  /** Load "מבנה הקו" for the whole line (root ""), three levels deep. */
  async function loadTree() {
    const token = yemotToken.trim();
    if (!token) {
      treeNodes = [];
      treeOk = false;
      treeError = "";
      return;
    }
    treeLoading = true;
    treeError = "";
    try {
      const nodes = await invoke("get_extension_tree", { token, root: "", depth: 3 });
      treeNodes = Array.isArray(nodes) ? nodes : [];
      treeOk = true;
    } catch (e) {
      treeNodes = [];
      treeOk = false;
      treeError = describeLineError(e, "tree_unavailable", "tree_error");
    } finally {
      treeLoading = false;
    }
  }

  /**
   * Depth-first lookup of a node by canonical path.
   * @param {any[]} list
   * @param {string} path
   * @returns {any}
   */
  function findTreeNode(list, path) {
    for (const n of list) {
      if (n.path === path) return n;
      if (Array.isArray(n.children) && n.children.length > 0) {
        const hit = findTreeNode(n.children, path);
        if (hit) return hit;
      }
    }
    return null;
  }

  /** @param {string} path */
  function toggleTreeNode(path) {
    treeExpanded = { ...treeExpanded, [path]: !treeExpanded[path] };
  }

  /** @param {string} path */
  async function selectExtension(path) {
    selectedExtPath = path;
    const node = findTreeNode(treeNodes, path);
    selectedExtDisplay = node?.display ?? path;
    await loadExtension(path);
  }

  /** @param {string} path */
  async function loadExtension(path) {
    const token = yemotToken.trim();
    if (!token || !path) return;
    extLoading = true;
    extError = "";
    try {
      extDetail = await invoke("read_extension", { token, path });
    } catch (e) {
      extDetail = null;
      extError = describeLineError(e, "inspector_unavailable", "inspector_error");
    } finally {
      extLoading = false;
    }
  }

  function refreshExtension() {
    if (selectedExtPath) void loadExtension(selectedExtPath);
  }

  // ----- Context chips -----

  /** Pin the inspected extension onto the task. */
  function addInspectedToContext() {
    const path = extDetail?.path ?? selectedExtPath;
    if (!path) return;
    if (contextExts.some((c) => c.path === path)) return;
    const display = extDetail?.display ?? selectedExtDisplay ?? path;
    contextExts = [...contextExts, { path, display }];
  }

  /** @param {string} path */
  function removeContextExtension(path) {
    contextExts = contextExts.filter((c) => c.path !== path);
  }

  /**
   * The text actually sent to the model: the context chips are prepended to the
   * instruction, never written into the textarea the user is editing.
   * @param {string} text
   */
  function buildTaskPrompt(text) {
    const base = text.trim();
    if (contextExts.length === 0) return base;
    const paths = contextExts.map((c) => c.display).join(", ");
    return `${t("context_prefix", { paths })}\n\n${base}`;
  }

  // ----- Attachments (audio files for `upload_audio_file`) -----

  /**
   * Attachment ids are positional and are baked into the running prompt's
   * `[קבצים מצורפים]` block, so adding or removing one mid-run would renumber
   * the list out from under the model — `a2` would stop meaning what the
   * transcript says it means. The list is frozen (not cleared) while a run is
   * in flight, and stays available for the next task afterwards.
   */
  let attachmentsLocked = $derived(agentRunning || agentStarting);

  /**
   * The OS file picker. There is no command that stats a local file, so `size`
   * is sent as 0 and the MIME type is inferred from the extension — Rust
   * validates both against the real file before the run starts.
   */
  async function pickAttachments() {
    if (attachmentsLocked) return;
    attachError = "";
    try {
      const picked = await openFileDialog({
        multiple: true,
        filters: [{ name: "Audio", extensions: AUDIO_EXTENSIONS }]
      });
      const list = Array.isArray(picked) ? picked : picked ? [picked] : [];
      const next = [...attachments];
      for (const entry of list) {
        const localPath = String(entry);
        if (!localPath || next.some((a) => a.local_path === localPath)) continue;
        const name = baseName(localPath);
        next.push({ id: "", name, local_path: localPath, size: 0, mime: mimeFromName(name) });
      }
      attachments = renumberAttachments(next);
    } catch (e) {
      attachError = isMissingCommand(e)
        ? t("attach_unavailable")
        : t("attach_failed", { error: errorText(e) });
    }
  }

  /**
   * Ids are positional (`a1`, `a2`, …) and must stay dense: the model may only
   * upload ids listed in the run's own `[קבצים מצורפים]` block.
   * @param {{id: string, name: string, local_path: string, size: number, mime: string}[]} list
   */
  function renumberAttachments(list) {
    return list.map((a, i) => ({ ...a, id: `a${i + 1}` }));
  }

  /** @param {string} id */
  function removeAttachment(id) {
    if (attachmentsLocked) return;
    attachments = renumberAttachments(attachments.filter((a) => a.id !== id));
  }

  // ----- Ready-made tasks (presets) -----

  /** @param {{id: string, title: string, text: string}[]} list */
  function updatePresets(list) {
    presets = list;
    savePresets(list);
  }

  function restoreDefaultPresets() {
    updatePresets(seedPresets(t));
  }

  // ----- יומן שינויים -----

  /** Read the applied-change log back from Rust (survives a page reload). */
  async function loadChangeLog() {
    changeLogLoading = true;
    changeLogError = "";
    try {
      const list = await invoke("list_applied_changes");
      appliedChanges = Array.isArray(list) ? list : [];
    } catch (e) {
      appliedChanges = [];
      changeLogError = isMissingCommand(e)
        ? t("changelog_unavailable")
        : t("changelog_error", { error: errorText(e) });
    } finally {
      changeLogLoading = false;
    }
  }

  /**
   * Undo from the change log. It targets the run that made the change, which is
   * not necessarily the run on screen — hence the run id travelling with the row.
   * @param {string} logRunId
   * @param {string} actionId
   */
  async function undoLoggedChange(logRunId, actionId) {
    const key = `${logRunId}:${actionId}`;
    try {
      const r = /** @type {any} */ (
        await invoke("undo_action", { runId: logRunId, actionId })
      );
      const outcome = r?.ok ? "done" : "failed";
      changeLogUndoState = { ...changeLogUndoState, [key]: outcome };
      // Keep the approval panel in step when it is showing the same action.
      if (runId === logRunId) {
        undoState = { ...undoState, [actionId]: outcome };
        undoMessages = { ...undoMessages, [actionId]: r?.message ?? "" };
      }
    } catch (e) {
      console.error("undo_action failed:", e);
      // Routed through `describeLineError` for its side effect: a dead Yemot
      // session reopens the login modal instead of failing silently here.
      describeLineError(e, "undo_unavailable", "comm_error");
      changeLogUndoState = {
        ...changeLogUndoState,
        [key]: isMissingCommand(e) ? "unavailable" : "failed"
      };
    }
    await loadChangeLog();
  }

  // ----- היסטוריית משימות -----

  /**
   * Read the stored task list back from Rust. Like the change log, this is a
   * command an older binary does not register — then the card says so instead
   * of showing a failure.
   */
  async function loadTaskHistory() {
    historyLoading = true;
    historyError = "";
    try {
      const list = await invoke("list_task_history");
      taskHistory = Array.isArray(list) ? list : [];
    } catch (e) {
      taskHistory = [];
      historyError = isMissingCommand(e)
        ? t("history_unavailable")
        : t("history_error", { error: errorText(e) });
    } finally {
      historyLoading = false;
    }
  }

  /** Drop one stored task. An id the store does not know is not an error. */
  /** @param {string} taskId */
  async function deleteTaskHistory(taskId) {
    if (!taskId) return;
    try {
      await invoke("delete_task_history", { taskId });
      if (loadedTaskId === taskId) loadedTaskId = "";
    } catch (e) {
      historyError = isMissingCommand(e)
        ? t("history_unavailable")
        : t("history_error", { error: errorText(e) });
    }
    await loadTaskHistory();
  }

  /** Two-step confirmed in `TaskHistory`; this is the second step. */
  async function clearTaskHistory() {
    try {
      await invoke("clear_task_history");
      loadedTaskId = "";
    } catch (e) {
      historyError = isMissingCommand(e)
        ? t("history_unavailable")
        : t("history_error", { error: errorText(e) });
    }
    await loadTaskHistory();
  }

  /**
   * "המשך משימה" — load a stored task into the workspace as the current one.
   *
   * The record arrives without its transcript (display only); the transcript
   * that the continuation replays stays in Rust. What lands on screen is the
   * chain of instructions, the last run's final answer, and its proposals —
   * read-only, because the run that could apply them no longer exists. The only
   * way forward from here is "דייק את המשימה", which continues the chain
   * through its `task_id`.
   * @param {string} taskId
   */
  async function continueFromHistory(taskId) {
    if (!taskId || runBusy || isLoading) return;
    historyError = "";
    /** @type {any} */
    let record;
    try {
      record = await invoke("get_task_history", { taskId });
    } catch (e) {
      historyError = isMissingCommand(e)
        ? t("history_unavailable")
        : t("history_load_failed", { error: errorText(e) });
      return;
    }
    if (!record) {
      historyError = t("history_load_failed", { error: taskId });
      return;
    }

    // Clears the live panel — and `loadedTaskId`, which is set again below.
    resetAgentRun();

    const instructions = Array.isArray(record.instructions)
      ? record.instructions.map((/** @type {any} */ x) => String(x))
      : [];
    const finalText = typeof record.final_text === "string" ? record.final_text : "";

    taskChain = instructions;
    loadedTaskId = String(record.task_id ?? taskId);
    agentFinish = {
      ok: record.ok !== false,
      stop: typeof record.stop === "string" ? record.stop : "end_turn",
      final_text: finalText,
      // `TaskUsage` carries no elapsed time and no cache ratio, and the stats /
      // cost lines are stated as measured facts — so a stored task shows none
      // rather than made-up zeroes.
      usage: null
    };
    if (finalText) {
      pushTimeline({ type: "text", text: finalText, streaming: false });
    }
    resultOutput = finalText;

    mergeProposedActions(Array.isArray(record.actions) ? record.actions : []);
    const appliedIds = Array.isArray(record.applied_action_ids)
      ? record.applied_action_ids
      : [];
    /** @type {Record<string, any>} */
    const applied = {};
    for (const id of appliedIds) {
      applied[String(id)] = { action_id: String(id), ok: true, message: "" };
    }
    actionResults = applied;

    // The refine box continues the chain, not a live run: its parent is the
    // task id, and a retry after a failed continuation has to reproduce that.
    lastRunKind = "continue";
    lastRunParentId = loadedTaskId;
    lastRunPrompt = instructions[instructions.length - 1] ?? "";
    // Reuse the model the task ran on only when it belongs to the provider that
    // is selected now — otherwise the continuation would hand one provider
    // another provider's model id.
    lastPayloadModel =
      record.provider === aiProvider && record.model
        ? String(record.model)
        : modelSource === "manual"
          ? customModel.trim()
          : selectedModel;

    refineText = "";
    refineError = "";
    errorMessage = "";
    statusMessage = "";
  }

  // ============================================================
  //  Agent run (direct mode) — see docs/agent-contract.md
  // ============================================================

  /** Copy the raw final answer shown under the run panel. */
  let resultCopied = $state(false);
  async function copyResultOutput() {
    try {
      await navigator.clipboard.writeText(resultOutput);
      resultCopied = true;
      setTimeout(() => (resultCopied = false), 1600);
    } catch (e) {
      console.error("Failed to copy:", e);
    }
  }

  /**
   * Accept an event only if it belongs to the run currently displayed.
   * Adopts the run id of the first event when start_agent_run has not returned yet.
   * @param {any} payload
   */
  function isCurrentRun(payload) {
    if (!payload || !payload.run_id) return false;
    if (runId) return payload.run_id === runId;
    if (agentStarting) {
      runId = payload.run_id;
      return true;
    }
    return false;
  }

  /** Drop every active agent:* subscription. */
  function teardownAgentListeners() {
    for (const un of agentUnlisteners) {
      try {
        un();
      } catch (_) {}
    }
    agentUnlisteners = [];
  }

  // ----- Run feedback helpers (timer, streaming, auto-scroll) -----

  function startRunTimer() {
    stopRunTimer();
    runStartedAt = Date.now();
    runElapsedMs = 0;
    runTimer = setInterval(() => {
      runElapsedMs = Date.now() - runStartedAt;
    }, 250);
  }

  function stopRunTimer() {
    if (runTimer !== null) {
      clearInterval(runTimer);
      runTimer = null;
    }
  }

  /** Turn any still-streaming text block into a finalized (markdown) block. */
  function finalizeStreamingText() {
    let changed = false;
    for (const item of agentTimeline) {
      if (item.type === "text" && item.streaming) {
        item.streaming = false;
        changed = true;
      }
    }
    if (changed) agentTimeline = [...agentTimeline];
  }

  /** Elapsed seconds of the current run, one decimal. */
  let runElapsedLabel = $derived((runElapsedMs / 1000).toFixed(1));

  /** True while we are waiting for the first assistant text of a turn. */
  let awaitingTurnText = $derived.by(() => {
    if (!agentRunning) return false;
    const last = agentTimeline[agentTimeline.length - 1];
    return !!last && last.type === "turn";
  });

  function handleTimelineScroll() {
    if (!timelineRef) return;
    const distance =
      timelineRef.scrollHeight - timelineRef.scrollTop - timelineRef.clientHeight;
    timelineAtBottom = distance < 48;
  }

  // Follow the timeline only while the user is already parked at the bottom.
  $effect(() => {
    agentTimeline.length;
    const el = timelineRef;
    if (!el || !timelineAtBottom) return;
    tick().then(() => {
      if (timelineRef && timelineAtBottom) {
        timelineRef.scrollTop = timelineRef.scrollHeight;
      }
    });
  });

  /** Clear the panel state before a new run. */
  function resetAgentRun() {
    teardownAgentListeners();
    stopRunTimer();
    runElapsedMs = 0;
    approvalNotice = "";
    confirmRisky = false;
    undoState = {};
    undoMessages = {};
    refineError = "";
    timelineAtBottom = true;
    loadedTaskId = "";
    runId = null;
    agentRunning = false;
    agentCancelling = false;
    agentTurn = 0;
    agentMaxTurns = 0;
    agentTimeline = [];
    agentRetryNotice = null;
    agentFinish = null;
    agentError = null;
    proposedActions = [];
    actionResults = {};
  }

  /**
   * @param {any} item
   */
  function pushTimeline(item) {
    agentTimeline = [...agentTimeline, item];
  }

  /**
   * Map a ProposedAction from the contract onto a selectable UI row.
   * @param {any} action
   */
  function toActionRow(action) {
    return {
      id: action.id,
      tool_use_id: action.tool_use_id ?? null,
      kind: action.kind ?? "set_extension_params",
      path: action.path ?? "",
      params: Array.isArray(action.params) ? action.params : [],
      reason: action.reason ?? "",
      risk: action.risk ?? "low",
      exists: action.exists !== false,
      diff: Array.isArray(action.diff) ? action.diff : [],
      warnings: Array.isArray(action.warnings) ? action.warnings : [],
      // Optional: the backend may attach the previous state so a change can be
      // undone. Absent on older builds — the UI simply hides the row.
      previous: action.previous ?? null,
      selected: true,
      expanded: false
    };
  }

  /**
   * Merge a fresh ProposedAction list into the panel, keeping the user's
   * checkbox and expand state for rows that are already on screen.
   * @param {any[]} list
   */
  function mergeProposedActions(list) {
    const previousRows = new Map(proposedActions.map((a) => [a.id, a]));
    proposedActions = list.map((action) => {
      const row = toActionRow(action);
      const old = previousRows.get(row.id);
      if (old) {
        row.selected = old.selected;
        row.expanded = old.expanded;
      }
      return row;
    });
  }

  /** Register every agent:* listener for the run about to start. */
  async function setupAgentListeners() {
    teardownAgentListeners();

    /** @type {Array<Promise<() => void>>} */
    const subs = [];

    subs.push(
      listen("agent:started", (event) => {
        const p = /** @type {any} */ (event.payload);
        if (!isCurrentRun(p)) return;
        agentRunning = true;
        statusMessage = t("agent_started_with", {
          provider: p.provider ?? "",
          model: p.model ?? ""
        });
      })
    );

    subs.push(
      listen("agent:turn_start", (event) => {
        const p = /** @type {any} */ (event.payload);
        if (!isCurrentRun(p)) return;
        agentTurn = p.turn ?? 0;
        agentMaxTurns = p.max_turns ?? 0;
        agentRetryNotice = null;
        pushTimeline({ type: "turn", turn: p.turn, max: p.max_turns });
      })
    );

    subs.push(
      listen("agent:assistant_text", (event) => {
        const p = /** @type {any} */ (event.payload);
        if (!isCurrentRun(p)) return;
        if (!p.text) return;
        // If deltas already built this block, finalize it instead of duplicating.
        const last = agentTimeline[agentTimeline.length - 1];
        if (last && last.type === "text" && last.streaming) {
          last.text = p.text;
          last.streaming = false;
          agentTimeline = [...agentTimeline];
          return;
        }
        pushTimeline({ type: "text", turn: p.turn, text: p.text, streaming: false });
      })
    );

    subs.push(
      listen("agent:text_delta", (event) => {
        const p = /** @type {any} */ (event.payload);
        if (!isCurrentRun(p)) return;
        if (p.reset) {
          // The attempt that streamed this block failed; the retry starts over,
          // so drop the partial text instead of appending onto it.
          // The retry row may already sit after it, so search backwards.
          const i = agentTimeline.findLastIndex((it) => it.type === "text" && it.streaming);
          if (i >= 0) {
            agentTimeline = [...agentTimeline.slice(0, i), ...agentTimeline.slice(i + 1)];
          }
          return;
        }
        if (!p.delta) return;
        const last = agentTimeline[agentTimeline.length - 1];
        if (last && last.type === "text" && last.streaming) {
          last.text += p.delta;
          agentTimeline = [...agentTimeline];
        } else {
          pushTimeline({ type: "text", turn: agentTurn, text: p.delta, streaming: true });
        }
      })
    );

    subs.push(
      listen("agent:tool_started", (event) => {
        const p = /** @type {any} */ (event.payload);
        if (!isCurrentRun(p)) return;
        pushTimeline({
          type: "tool",
          tool_use_id: p.tool_use_id,
          name: p.name ?? "",
          label: p.label ?? p.name ?? "",
          status: "running",
          summary: "",
          ms: null
        });
      })
    );

    subs.push(
      listen("agent:tool_finished", (event) => {
        const p = /** @type {any} */ (event.payload);
        if (!isCurrentRun(p)) return;
        const row = agentTimeline.find(
          (item) => item.type === "tool" && item.tool_use_id === p.tool_use_id
        );
        if (row) {
          row.status = p.ok ? "ok" : "failed";
          row.summary = p.summary ?? "";
          row.ms = p.ms ?? null;
          agentTimeline = [...agentTimeline];
        }
      })
    );

    subs.push(
      listen("agent:action_proposed", (event) => {
        const p = /** @type {any} */ (event.payload);
        if (!isCurrentRun(p) || !p.action) return;
        if (proposedActions.some((a) => a.id === p.action.id)) return;
        proposedActions = [...proposedActions, toActionRow(p.action)];
      })
    );

    subs.push(
      listen("agent:actions_proposed", (event) => {
        const p = /** @type {any} */ (event.payload);
        if (!isCurrentRun(p)) return;
        const list = Array.isArray(p.actions) ? p.actions : [];
        mergeProposedActions(list);
      })
    );

    subs.push(
      listen("agent:action_applied", (event) => {
        const p = /** @type {any} */ (event.payload);
        if (!isCurrentRun(p) || !p.action_id) return;
        // Merge, never replace: the event and the `approve_actions` result carry
        // overlapping halves of the same row (only the invoke result has the
        // `undo` record), and either one can land second.
        actionResults = {
          ...actionResults,
          [p.action_id]: {
            ...(actionResults[p.action_id] ?? {}),
            action_id: p.action_id,
            ok: !!p.ok,
            message: p.message ?? "",
            params: Array.isArray(p.params) ? p.params : []
          }
        };
      })
    );

    subs.push(
      listen("agent:retry", (event) => {
        const p = /** @type {any} */ (event.payload);
        if (!isCurrentRun(p)) return;
        agentRetryNotice = {
          attempt: p.attempt ?? 1,
          max: p.max ?? 1,
          reason: p.reason ?? "",
          wait_ms: p.wait_ms ?? 0
        };
        pushTimeline({ type: "retry", attempt: p.attempt, max: p.max, reason: p.reason });
      })
    );

    subs.push(
      listen("agent:finished", (event) => {
        const p = /** @type {any} */ (event.payload);
        if (!isCurrentRun(p)) return;
        agentRunning = false;
        agentCancelling = false;
        agentRetryNotice = null;
        isLoading = false;
        stopRunTimer();
        finalizeStreamingText();
        agentFinish = {
          ok: !!p.ok,
          stop: p.stop ?? "end_turn",
          final_text: p.final_text ?? "",
          usage: p.usage ?? null
        };
        if (p.final_text) resultOutput = p.final_text;
        statusMessage = p.ok ? t("request_success") : agentStopLabel(p.stop);
        // The run just rewrote its task's record (unless history is off).
        void loadTaskHistory();
      })
    );

    subs.push(
      listen("agent:error", (event) => {
        const p = /** @type {any} */ (event.payload);
        if (!isCurrentRun(p)) return;
        agentRunning = false;
        agentCancelling = false;
        isLoading = false;
        statusMessage = "";
        stopRunTimer();
        finalizeStreamingText();
        agentError = { code: p.code ?? "internal", message: p.message ?? "" };
        if (p.code === "session_expired") {
          // The Yemot session died mid-run: re-authenticate, then re-run
          // automatically once the new token is in place.
          errorMessage = t("agent_session_expired");
          openLoginModal(true);
          pendingRerun = true;
          loginError = t("agent_session_expired");
        } else {
          errorMessage = p.message || t("request_failed");
        }
      })
    );

    // Append rather than assign: `listen()` resolves after several ticks, and an
    // assignment here would drop (and leak) any subscription registered in the
    // meantime instead of letting `teardownAgentListeners` reach it.
    agentUnlisteners = [...agentUnlisteners, ...(await Promise.all(subs))];
  }

  /**
   * @param {string} stop
   */
  function agentStopLabel(stop) {
    switch (stop) {
      case "end_turn":
        return t("agent_stop_end_turn");
      case "max_turns":
        return t("agent_stop_max_turns");
      case "cancelled":
        return t("agent_stop_cancelled");
      case "truncated":
        return t("agent_stop_truncated");
      case "refusal":
        return t("agent_stop_refusal");
      default:
        return t("agent_stop_error");
    }
  }

  /**
   * Measured facts about the run — token counts, cache hit rate, turns, time.
   * Nothing here is an estimate, so it carries no disclaimer.
   */
  let runStatsLine = $derived.by(() => {
    const usage = agentFinish?.usage;
    if (!usage) return "";
    const secs = ((usage.elapsed_ms ?? 0) / 1000).toFixed(1);
    const rawPct = usage.cache_hit_pct ?? 0;
    const cache = Math.round(rawPct <= 1 ? rawPct * 100 : rawPct);
    return t("run_stats_line", {
      in: usage.input_tokens ?? 0,
      out: usage.output_tokens ?? 0,
      cache,
      turns: usage.turns ?? 0,
      secs
    });
  });

  /**
   * The cost line. `cost_usd` is null whenever the model is not in the built-in
   * price table, and `price_list_date` says which list the number came from —
   * the app cannot see free tiers, discounts or a provider's price change, so
   * this is stated as an estimate, never as a charge.
   */
  let runCostLine = $derived.by(() => {
    const usage = agentFinish?.usage;
    if (!usage) return "";
    if (usage.cost_usd === null || usage.cost_usd === undefined) {
      return t("run_cost_unknown");
    }
    const cost = Number(usage.cost_usd).toFixed(4).replace(/0+$/, "").replace(/\.$/, "");
    const date = usage.price_list_date;
    return date
      ? t("run_cost_line", { cost, price_list_date: date })
      : t("run_cost_line_no_date", { cost });
  });

  /**
   * Build an `AgentRunPayload`. Attachments only ride along on a fresh run — a
   * continuation inherits the parent's list from the transcript.
   * @param {string} payloadModel
   * @param {string} prompt
   * @param {boolean} withAttachments
   */
  function buildRunPayload(payloadModel, prompt, withAttachments) {
    return {
      provider: aiProvider,
      model: payloadModel,
      prompt,
      api_key: apiKey.trim(),
      base_url: customBaseUrl.trim(),
      yemot_token: yemotToken.trim(),
      auto_apply: autoApply,
      include_tree: includeTree,
      save_history: saveHistory,
      attachments: withAttachments
        ? attachments.map((a) => ({
            id: a.id,
            name: a.name,
            local_path: a.local_path,
            size: a.size,
            mime: a.mime
          }))
        : []
    };
  }

  /** Start the agentic loop in Rust and subscribe to its events. */
  /** @param {string} payloadModel */
  async function startAgentRun(payloadModel) {
    resetAgentRun();
    await setupAgentListeners();

    lastRunKind = "fresh";
    lastRunParentId = "";
    lastPayloadModel = payloadModel;
    agentStarting = true;
    agentRunning = true;
    isLoading = true;
    statusMessage = t("agent_starting");
    startRunTimer();

    try {
      const payload = buildRunPayload(payloadModel, buildTaskPrompt(promptText), true);
      const started = await invoke("start_agent_run", { payload });
      const id = /** @type {any} */ (started)?.run_id;
      if (id && !runId) runId = id;
      taskChain = [promptText.trim()];
    } catch (e) {
      teardownAgentListeners();
      stopRunTimer();
      agentRunning = false;
      isLoading = false;
      statusMessage = "";
      errorMessage = t("comm_error", { error: e });
    } finally {
      agentStarting = false;
    }
  }

  /**
   * Everything `resetAgentRun` clears, so a continuation that Rust refuses can
   * put the panel back exactly as the user left it — unapproved proposals
   * included. Wiping them on a rejected continuation loses work that was never
   * written to the line and cannot be re-derived without another model call.
   */
  function snapshotAgentRun() {
    return {
      approvalNotice,
      confirmRisky,
      undoState,
      undoMessages,
      refineError,
      timelineAtBottom,
      loadedTaskId,
      runId,
      agentRunning,
      agentCancelling,
      agentTurn,
      agentMaxTurns,
      agentTimeline,
      agentRetryNotice,
      agentFinish,
      agentError,
      proposedActions,
      actionResults
    };
  }

  /** @param {ReturnType<typeof snapshotAgentRun>} s */
  function restoreAgentRun(s) {
    approvalNotice = s.approvalNotice;
    confirmRisky = s.confirmRisky;
    undoState = s.undoState;
    undoMessages = s.undoMessages;
    refineError = s.refineError;
    timelineAtBottom = s.timelineAtBottom;
    loadedTaskId = s.loadedTaskId;
    runId = s.runId;
    agentRunning = s.agentRunning;
    agentCancelling = s.agentCancelling;
    agentTurn = s.agentTurn;
    agentMaxTurns = s.agentMaxTurns;
    agentTimeline = s.agentTimeline;
    agentRetryNotice = s.agentRetryNotice;
    agentFinish = s.agentFinish;
    agentError = s.agentError;
    proposedActions = s.proposedActions;
    actionResults = s.actionResults;
  }

  /**
   * "דייק את המשימה" — continue the finished run with one more instruction.
   * Rust replays the parent's transcript (so the cached prefix still hits) and
   * hands back a brand-new run id, which the panel treats like any other run.
   *
   * The panel is cleared *before* the invoke on purpose: `isCurrentRun` can only
   * adopt the new run's id while `runId` is null and `agentStarting` is true, so
   * clearing afterwards would drop every event the run emits in the meantime.
   * A rejection is covered by the snapshot instead — nothing is lost either way.
   *
   * @param {string} [parentIdArg] parent to continue; defaults to the run on screen
   * @param {string} [textArg] instruction; defaults to the refine input
   * @returns {Promise<boolean>} true when Rust accepted the continuation
   */
  async function continueAgentRun(parentIdArg, textArg) {
    const text = (textArg ?? refineText).trim();
    // A task loaded from the history has no live run id: its chain is continued
    // through the task_id, which `continue_agent_run` resolves from the store.
    const parentRunId = parentIdArg ?? runId ?? loadedTaskId;
    if (!text || !parentRunId || refineBusy) return false;

    const payload = buildRunPayload(lastPayloadModel || selectedModel, text, false);
    const chain = [...taskChain];
    const snapshot = snapshotAgentRun();

    refineBusy = true;
    refineError = "";
    errorMessage = "";
    // The continuation is a new run: its timeline and its proposals start empty.
    resetAgentRun();
    await setupAgentListeners();

    agentStarting = true;
    agentRunning = true;
    isLoading = true;
    statusMessage = t("agent_starting");
    startRunTimer();

    try {
      const started = await invoke("continue_agent_run", { payload, parentRunId });
      const id = /** @type {any} */ (started)?.run_id;
      if (id && !runId) runId = id;
      taskChain = [...chain, text];
      lastRunPrompt = text;
      lastRunKind = "continue";
      lastRunParentId = parentRunId;
      refineText = "";
      return true;
    } catch (e) {
      teardownAgentListeners();
      stopRunTimer();
      restoreAgentRun(snapshot);
      isLoading = false;
      statusMessage = "";
      taskChain = chain;
      refineError = isMissingCommand(e)
        ? t("refine_unavailable")
        : t("comm_error", { error: errorText(e) });
      return false;
    } finally {
      agentStarting = false;
      refineBusy = false;
    }
  }

  async function cancelAgentRun() {
    if (!runId) return;
    agentCancelling = true;
    statusMessage = t("agent_cancelling");
    try {
      await invoke("cancel_agent_run", { runId });
    } catch (e) {
      console.error("cancel_agent_run failed:", e);
      agentCancelling = false;
    }
  }

  // ----- Approval panel -----

  /** An action already applied successfully is locked (checkbox disabled). */
  /** @param {any} action */
  function isActionApplied(action) {
    return actionResults[action.id]?.ok === true;
  }

  let selectableActions = $derived(proposedActions.filter((a) => !isActionApplied(a)));
  let selectedActions = $derived(selectableActions.filter((a) => a.selected));
  let selectedActionCount = $derived(selectedActions.length);

  /** Any selected action that is not plain "low" risk needs a confirmation step. */
  let hasRiskySelection = $derived(selectedActions.some((a) => a.risk !== "low"));

  /** "N הגדרות ב-M שלוחות, מתוכן K דריסות" — computed from the selection. */
  let riskSummary = $derived.by(() => {
    const paths = new Set();
    let settings = 0;
    let overwrites = 0;
    for (const a of selectedActions) {
      paths.add(a.path);
      settings += a.params.length;
      overwrites += a.diff.filter((/** @type {any} */ d) => d.kind === "changed").length;
    }
    return { settings, paths: paths.size, overwrites };
  });

  function selectAllActions() {
    for (const a of proposedActions) {
      if (!isActionApplied(a)) a.selected = true;
    }
    proposedActions = [...proposedActions];
    approvalNotice = "";
    confirmRisky = false;
  }

  /**
   * Any change to the individual selection invalidates the armed confirmation
   * (and the risk summary it quotes, which re-derives from the selection).
   */
  function handleActionToggled() {
    confirmRisky = false;
    approvalNotice = "";
  }

  function clearAllActions() {
    for (const a of proposedActions) a.selected = false;
    proposedActions = [...proposedActions];
    approvalNotice = "";
    confirmRisky = false;
  }

  /** Approve and apply the checked ProposedActions. */
  async function approveSelectedActions() {
    const selected = selectedActions;
    if (selected.length === 0) {
      approvalNotice = t("no_actions_selected");
      return;
    }
    if (!runId) {
      approvalNotice = t("no_run_id");
      return;
    }
    // Risky changes get one explicit confirmation click before anything runs.
    if (hasRiskySelection && !confirmRisky) {
      confirmRisky = true;
      approvalNotice = "";
      return;
    }
    confirmRisky = false;
    approvalNotice = "";

    isLoading = true;
    statusMessage = t("applying_actions");
    try {
      const results = await invoke("approve_actions", {
        runId,
        actionIds: selected.map((a) => a.id)
      });
      const list = Array.isArray(results) ? results : [];
      /** @type {Record<string, any>} */
      const merged = { ...actionResults };
      for (const r of list) {
        // Same merge as the `agent:action_applied` handler, from the other side.
        if (r && r.action_id) merged[r.action_id] = { ...(merged[r.action_id] ?? {}), ...r };
      }
      actionResults = merged;
      const done = list.filter((r) => r && r.ok).length;
      statusMessage = t("actions_done", { done, total: selected.length });
      // The change log is the durable record of what reached the line.
      await loadChangeLog();
      if (selectedExtPath) void loadExtension(selectedExtPath);
    } catch (e) {
      // A `SESSION_EXPIRED:` rejection is the model-facing wording; the user
      // gets the Hebrew copy and the login modal instead.
      approvalNotice = describeLineError(e, "request_failed", "comm_error");
      statusMessage = "";
    } finally {
      isLoading = false;
      if (logoutOnFinish) {
        await localLogout();
      }
    }
  }

  /**
   * Roll one applied action back through `undo_action`. The command is optional
   * (older backends do not register it), so a missing command is not an error —
   * the button just reports that undo is unavailable.
   * @param {string} actionId
   */
  async function undoAction(actionId) {
    if (!runId) return;
    try {
      const r = /** @type {any} */ (await invoke("undo_action", { runId, actionId }));
      undoState = { ...undoState, [actionId]: r?.ok ? "done" : "failed" };
      undoMessages = { ...undoMessages, [actionId]: r?.message ?? "" };
      await loadChangeLog();
      if (selectedExtPath) void loadExtension(selectedExtPath);
    } catch (e) {
      console.error("undo_action failed:", e);
      // Only a missing command means "undo is unavailable in this build"; every
      // other rejection is a real failure and its text has to reach the user —
      // localized, and with a dead session reopening the login modal.
      const missing = isMissingCommand(e);
      const message = describeLineError(e, "undo_unavailable", "comm_error");
      undoState = { ...undoState, [actionId]: missing ? "unavailable" : "failed" };
      undoMessages = { ...undoMessages, [actionId]: missing ? "" : message };
    }
  }

  onDestroy(() => {
    teardownAgentListeners();
    stopRunTimer();
    if (searchDebounce !== null) clearTimeout(searchDebounce);
  });

  /**
   * Submit entry point. `isLoading` is raised here — before any `await` — so a
   * second click cannot slip through while `saveSettings` / `setupAgentListeners`
   * are still running and start a second (ghost) run with orphaned listeners.
   * @param {SubmitEvent} [event]
   */
  async function handleSubmit(event) {
    event?.preventDefault();
    if (isLoading) return;
    isLoading = true;
    try {
      await runSubmitFlow();
    } finally {
      // A direct-mode run that is actually under way keeps the spinner up until
      // `agent:finished` / `agent:error` clears it; every other path ends here.
      if (!agentRunning && !agentStarting) isLoading = false;
    }
  }

  async function runSubmitFlow() {
    if (!promptText.trim()) {
      errorMessage = t("enter_task");
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

    await saveSettings();
    lastRunPrompt = promptText;
    errorMessage = "";
    resultOutput = "";
    parsedActions = [];
    scriptActionResults = [];

    // Direct mode is the agentic loop (Rust holds the token); script mode stays legacy.
    if (targetMode === "direct") {
      await startAgentRun(payloadModel);
      return;
    }

    resetAgentRun();
    isLoading = true;
    statusMessage = t("sending_to_script");

    try {
      const payload = {
        target_mode: targetMode,
        provider: aiProvider,
        model: payloadModel,
        prompt: promptText.trim(),
        api_key: apiKey.trim(),
        is_preview: isPreviewMode,
        logout: logoutOnFinish,
        script_url: scriptUrl.trim(),
        base_url: ""
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
      const msg = String(e);
      if (msg.startsWith(SCRIPT_DISABLED_PREFIX)) {
        // The agreed kill-switch signal from the script: a dedicated notice,
        // not a generic communication error.
        const reason = msg.slice(SCRIPT_DISABLED_PREFIX.length).trim();
        statusMessage = "";
        errorMessage = reason
          ? t("script_disabled_reason", { reason })
          : t("script_disabled");
      } else {
        errorMessage = t("comm_error", { error: e });
      }
    } finally {
      isLoading = false;
      // Logout is performed locally — the token never reaches the script/AI.
      if (logoutOnFinish) {
        await localLogout();
      }
    }
  }

  /** @param {string} raw */
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
      scriptPreviewNotice = t("no_actions_selected");
      return;
    }
    scriptPreviewNotice = "";

    isLoading = true;
    statusMessage = t("executing_actions", { count: selected.length });
    scriptActionResults = [];

    // One UpdateExtension call per extension: group the parsed key/value pairs by path.
    /** @type {Map<string, {key: string, value: string}[]>} */
    const groups = new Map();
    for (const act of selected) {
      const path = act.path || "";
      const bucket = groups.get(path);
      if (bucket) bucket.push({ key: act.key, value: act.value });
      else groups.set(path, [{ key: act.key, value: act.value }]);
    }

    let successCount = 0;
    /** @type {any[]} */
    const results = [];

    for (const [path, params] of groups) {
      try {
        const res = /** @type {any} */ (
          await invoke("execute_yemot_actions", {
            token: yemotToken.trim(),
            path,
            params
          })
        );
        const ok = res?.success ?? res?.ok ?? false;
        const outcomes = Array.isArray(res?.params)
          ? res.params
          : Array.isArray(res?.results)
            ? res.results
            : [];
        if (ok) successCount += params.length;
        results.push({ path, ok, message: res?.message ?? "", params: outcomes });
      } catch (e) {
        console.error("Action failed:", e);
        results.push({ path, ok: false, message: String(e), params: [] });
      }
    }

    scriptActionResults = results;
    isLoading = false;
    statusMessage = t("actions_done", { done: successCount, total: selected.length });
    // Logout is performed locally — never via the script.
    if (logoutOnFinish) {
      await localLogout();
    }
  }

  /**
   * @param {string} fileName
   * @param {string | null} [targetAnchor]
   */
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

  /** @param {string | null} targetId */
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
      const targetEl = /** @type {HTMLElement} */ (
        el.tagName === "A" && (el.textContent ?? "").trim() === "" && el.nextElementSibling
          ? el.nextElementSibling
          : el
      );

      targetEl.scrollIntoView({ behavior: "smooth", block: "start" });
      targetEl.classList.remove("target-highlight");
      void targetEl.offsetWidth;
      targetEl.classList.add("target-highlight");
    }
  }

  /**
   * Link interception for the agent markdown. The webview must never navigate
   * itself: external links are handed to the OS browser, and everything else is
   * simply inert (agent output is model-generated and not a trusted navigation
   * source). Same rules as `handleContentClick`, minus the knowledge-file jumps.
   * @param {MouseEvent} e
   */
  async function handleAgentContentClick(e) {
    const anchor = /** @type {HTMLElement} */ (e.target).closest("a");
    if (!anchor) return;

    e.preventDefault();

    const href = anchor.getAttribute("href");
    if (!href) return;
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
    }
  }

  /** @param {MouseEvent} e */
  async function handleContentClick(e) {
    const anchor = /** @type {HTMLElement} */ (e.target).closest("a");
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
        knowledgeMatches = knowledgeFiles;
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

  // ----- Token management (stored locally, deletable) -----

  async function clearToken() {
    yemotToken = "";
    tokenStatus = null;
    // Without a token there is no line to show.
    treeNodes = [];
    treeOk = false;
    treeError = "";
    selectedExtPath = null;
    selectedExtDisplay = null;
    extDetail = null;
    extError = "";
    await secretDelete("yemot_token");
  }

  // ----- Knowledge search (debounced, via the Rust index) -----

  /** Ask Rust for the matching knowledge files, 200 ms after the last keystroke. */
  function scheduleKnowledgeSearch() {
    if (searchDebounce !== null) clearTimeout(searchDebounce);
    searchDebounce = setTimeout(runKnowledgeSearch, 200);
  }

  async function runKnowledgeSearch() {
    const query = searchQuery.trim();
    if (!query) {
      knowledgeMatches = knowledgeFiles;
      return;
    }
    try {
      knowledgeMatches = await invoke("search_knowledge_files", { query });
    } catch (e) {
      console.error("search_knowledge_files failed:", e);
      // Fall back to a case-insensitive match on the file names.
      const lowered = query.toLowerCase();
      knowledgeMatches = knowledgeFiles.filter((f) =>
        f.name.toLowerCase().includes(lowered)
      );
    }
  }

  // ----- Modal accessibility (Escape, focus trap entry/return) -----

  /** Which modal is on top right now, or null. */
  let topModal = $derived(
    showLoginModal ? "login" : showKnowledgeModal ? "knowledge" : showSettings ? "settings" : null
  );

  function openSettings() {
    rememberOpener();
    showSettings = true;
  }

  function closeSettings() {
    showSettings = false;
    restoreOpenerFocus();
  }

  function rememberOpener() {
    if (typeof document === "undefined") return;
    lastFocusedBeforeModal = /** @type {HTMLElement | null} */ (document.activeElement);
  }

  function restoreOpenerFocus() {
    const el = lastFocusedBeforeModal;
    lastFocusedBeforeModal = null;
    if (el && typeof el.focus === "function") {
      tick().then(() => el.focus());
    }
  }

  function closeKnowledgeModal() {
    showKnowledgeModal = false;
    selectedFileContent = null;
    restoreOpenerFocus();
  }

  /** @param {KeyboardEvent} e */
  function handleWindowKeydown(e) {
    if (e.key !== "Escape") return;
    if (topModal === "login") {
      closeLoginModal();
    } else if (topModal === "knowledge") {
      closeKnowledgeModal();
    } else if (topModal === "settings") {
      closeSettings();
    }
  }

  /** Ctrl/Cmd+Enter in the prompt box submits the form. */
  /** @param {KeyboardEvent} e */
  function handlePromptKeydown(e) {
    if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
      e.preventDefault();
      if (!isLoading) handleSubmit();
    }
  }

  /**
   * Re-run the last request after a failure — as the same *kind* of run it was.
   * A refinement re-dispatched as a fresh task would silently lose the parent's
   * transcript, so a continuation goes back through `continue_agent_run`; if
   * Rust refuses it (the parent run is gone), the user is told so and gets the
   * text back in the refine box rather than a restarted, context-free run.
   */
  async function retryRun() {
    if (lastRunKind === "continue" && lastRunParentId && lastRunPrompt && !refineBusy) {
      const text = lastRunPrompt;
      const ok = await continueAgentRun(lastRunParentId, text);
      if (!ok) {
        refineText = text;
        refineError = t("refine_parent_gone");
        errorMessage = t("refine_parent_gone");
      }
      return;
    }
    if (lastRunPrompt) promptText = lastRunPrompt;
    await handleSubmit();
  }

  /** True until both the Yemot token and the provider key are in place. */
  let onboardingNeeded = $derived(
    !yemotToken.trim() || (targetMode === "direct" && !apiKey.trim())
  );

  // ----- Login (create token) flow: system number + password + MFA -----

  /**
   * @param {boolean} [keepPendingRerun] true only for the `session_expired`
   *   path, which opens the modal precisely in order to re-run afterwards.
   */
  function openLoginModal(keepPendingRerun = false) {
    // Otherwise a stale flag from an earlier expired run would make an unrelated
    // login silently fire off the previous prompt again.
    if (!keepPendingRerun) pendingRerun = false;
    rememberOpener();
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
    // Closing the modal cancels the automatic re-run of an expired session.
    pendingRerun = false;
    showLoginModal = false;
    restoreOpenerFocus();
  }

  /**
   * The MFA method picker changed: adopt its first send type and drop any code
   * that was already sent for the previous method.
   * @param {string} id
   */
  function handleLoginMethodChange(id) {
    const m = loginMethods.find((mm) => mm.id === id);
    loginSendType = m && m.send_types && m.send_types[0] ? m.send_types[0] : "";
    loginCodeSent = false;
  }

  /** Step back from the MFA step to the credentials form. */
  function backToCredentials() {
    loginStep = "credentials";
    loginCodeSent = false;
    loginCode = "";
    loginError = "";
    loginStatus = "";
  }

  /** @param {string} token */
  async function applyLoginToken(token) {
    yemotToken = token;
    await secretSet("yemot_token", token.trim());
    tokenStatus = { valid: true, message: t("login_success") };
    // The system number is only known here; the connection chip needs it.
    if (loginUsername.trim()) {
      connectedSystem = loginUsername.trim();
      try {
        localStorage.setItem("ai_yemot_system", connectedSystem);
      } catch (_) {}
    }
    // Read the flag before closing — `closeLoginModal` clears it.
    const rerun = pendingRerun;
    closeLoginModal();
    // A fresh session means a fresh view of the line.
    void loadTree();
    if (selectedExtPath) void loadExtension(selectedExtPath);
    // A run that died on `session_expired` picks up again by itself — through
    // `retryRun`, so a continuation resumes as a continuation instead of being
    // demoted to a fresh task that has forgotten everything before it.
    if (rerun) {
      await retryRun();
    }
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
          await applyLoginToken(res.token);
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
    // Yemot sends a numeric one-time code, normally 6 digits; short or
    // non-numeric input is caught here instead of burning an attempt
    // (the server locks after too many tries).
    const code = loginCode.replace(/\D/g, "");
    if (code.length < 4 || code.length > 6) {
      loginError = t("enter_mfa_code");
      return;
    }
    loginCode = code;
    loginLoading = true;
    loginStatus = t("verifying_code");
    try {
      const res = await invoke("validate_mfa_code", {
        token: loginToken,
        code: loginCode.trim()
      });
      if (res.success) {
        await applyLoginToken(loginToken);
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

<svelte:window onkeydown={handleWindowKeydown} />

<div class="min-h-screen bg-slate-50 text-slate-800 pb-12" dir={rtl ? "rtl" : "ltr"}>
  <!-- Top Navigation Bar -->
  <header class="bg-white border-b border-slate-200 sticky top-0 z-30 shadow-sm">
    <div class="max-w-6xl mx-auto px-4 py-3 flex items-center justify-between">
      <div class="flex items-center gap-3 min-w-0">
        <div class="w-10 h-10 shrink-0 rounded-xl bg-gradient-to-tr from-blue-600 to-indigo-500 flex items-center justify-center text-white text-xl shadow-md">
          🤖
        </div>
        <div class="min-w-0">
          <h1 class="text-lg font-bold text-slate-900 leading-tight">AI yemot</h1>
          <p class="text-xs text-slate-500 leading-tight truncate">{t("app_subtitle")}</p>
        </div>

        <!-- Connection chip: signed in, and proven so -->
        <span
          class="hidden sm:inline-flex items-center gap-1.5 px-2.5 py-1 rounded-full text-xs font-semibold border
            {connected
              ? 'bg-emerald-50 text-emerald-800 border-emerald-200'
              : 'bg-slate-100 text-slate-600 border-slate-300'}"
        >
          <span
            class="w-1.5 h-1.5 rounded-full {connected ? 'bg-emerald-500' : 'bg-slate-400'}"
            aria-hidden="true"
          ></span>
          <bdi>{connectionLabel}</bdi>
        </span>
      </div>

      <div class="flex items-center gap-3">
        <!-- Language selector -->
        <select
          value={i18n.locale}
          onchange={(e) => setLocale(/** @type {HTMLSelectElement} */ (e.currentTarget).value)}
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
          onclick={() => { rememberOpener(); showKnowledgeModal = true; }}
          class="px-3 py-1.5 rounded-lg border border-slate-200 text-xs font-medium text-slate-700 bg-slate-50 hover:bg-slate-100 flex items-center gap-1.5 transition"
        >
          <span>{t("knowledge_btn")}</span>
          <span class="bg-blue-100 text-blue-800 text-xs px-1.5 py-0.5 rounded-full font-bold">{knowledgeFiles.length}</span>
        </button>

        {#if updateInfo && updateInfo.has_update}
          <button
            type="button"
            onclick={() => openUrl(updateInfo.release_url)}
            class="px-3 py-1.5 rounded-lg bg-emerald-50 border border-emerald-300 text-xs font-semibold text-emerald-700 flex items-center gap-1 animate-pulse"
          >
            <span>{t("update_available", { version: updateInfo.latest_version })}</span>
          </button>
        {/if}

        <button
          type="button"
          onclick={openSettings}
          aria-label={t("open_settings")}
          title={t("open_settings")}
          class="px-3 py-1.5 rounded-lg border border-slate-200 text-xs font-medium text-slate-700 bg-slate-50 hover:bg-slate-100 transition focus-visible:outline-2 focus-visible:outline-blue-600"
        >
          ⚙️ {t("open_settings")}
        </button>
      </div>
    </div>
  </header>

  <!-- Main Container -->
  <main class="max-w-6xl mx-auto px-4 py-6">
    <!-- Empty state: nothing works until there is a token and a provider key -->
    {#if onboardingNeeded}
      <div class="mb-6 bg-white border border-blue-200 rounded-2xl p-5 shadow-sm">
        <h2 class="text-sm font-bold text-slate-900 mb-3">🚀 {t("onboarding_title")}</h2>
        <ol class="space-y-2 text-xs text-slate-700">
          <li class="flex items-center gap-2 flex-wrap">
            <span class="w-5 h-5 shrink-0 rounded-full flex items-center justify-center font-bold text-white text-xs {yemotToken.trim() ? 'bg-emerald-600' : 'bg-blue-600'}">
              {yemotToken.trim() ? "✓" : "1"}
            </span>
            <span>{t("onboarding_step_token")}</span>
            {#if !yemotToken.trim()}
              <button
                type="button"
                onclick={() => openLoginModal()}
                class="px-2.5 py-1 rounded-lg bg-blue-600 hover:bg-blue-700 text-white text-xs font-bold transition"
              >
                {t("onboarding_open_login")}
              </button>
            {/if}
          </li>
          <li class="flex items-center gap-2 flex-wrap">
            <span class="w-5 h-5 shrink-0 rounded-full flex items-center justify-center font-bold text-white text-xs {apiKey.trim() ? 'bg-emerald-600' : 'bg-blue-600'}">
              {apiKey.trim() ? "✓" : "2"}
            </span>
            <span>{t("onboarding_step_api_key")}</span>
            {#if !apiKey.trim()}
              <button
                type="button"
                onclick={openSettings}
                class="px-2.5 py-1 rounded-lg bg-blue-600 hover:bg-blue-700 text-white text-xs font-bold transition focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-blue-700"
              >
                {t("open_settings")}
              </button>
            {/if}
          </li>
        </ol>
      </div>
    {/if}

    <div class="grid grid-cols-1 lg:grid-cols-3 gap-6">

      <!-- Sidebar: the line itself (inline-start; RTL puts it on the right) -->
      <aside class="lg:col-span-1 space-y-6">
        <LineTree
          nodes={treeNodes}
          loading={treeLoading}
          error={treeError}
          hasToken={!!yemotToken.trim()}
          selectedPath={selectedExtPath}
          expanded={treeExpanded}
          onSelect={selectExtension}
          onToggle={toggleTreeNode}
          onRefresh={loadTree}
        />

        <ExtensionInspector
          detail={extDetail}
          loading={extLoading}
          error={extError}
          selectedDisplay={selectedExtDisplay}
          onRefresh={refreshExtension}
          onUseAsContext={addInspectedToContext}
          {rtl}
        />
      </aside>

      <!-- Right / Main Action Column -->
      <div class="lg:col-span-2 space-y-6">
        <!-- Input Form Card -->
        <div class="bg-white rounded-2xl border border-slate-200 p-6 shadow-sm">
          <form onsubmit={handleSubmit} class="space-y-4">
            <!-- Context chips: extensions pinned onto the task -->
            {#if contextExts.length > 0}
              <div class="rounded-xl border border-blue-200 bg-blue-50/60 p-3 space-y-1.5">
                <div class="flex items-baseline gap-2 flex-wrap">
                  <span class="text-xs font-bold text-slate-700">{t("context_title")}</span>
                  <span class="text-xs text-slate-500">{t("context_hint")}</span>
                </div>
                <ul class="flex flex-wrap gap-1.5">
                  {#each contextExts as c (c.path)}
                    <li>
                      <span class="inline-flex items-center gap-1 rounded-lg bg-white border border-blue-200 px-2 py-0.5 text-xs">
                        <span class="font-mono font-bold text-blue-800">
                          <bdi dir="ltr">{c.display}</bdi>
                        </span>
                        <button
                          type="button"
                          onclick={() => removeContextExtension(c.path)}
                          aria-label={`${t("context_remove")}: ${c.display}`}
                          title={t("context_remove")}
                          class="text-slate-400 hover:text-rose-700 rounded focus-visible:outline-2 focus-visible:outline-blue-600"
                        >
                          ✕
                        </button>
                      </span>
                    </li>
                  {/each}
                </ul>
              </div>
            {/if}

            <div>
              <div class="flex items-center justify-between mb-2">
                <label for="prompt-textarea" class="text-sm font-bold text-slate-800">{t("task_label")}</label>
                <span class="text-xs text-slate-500">{t("task_hint")}</span>
              </div>
              <textarea
                id="prompt-textarea"
                rows="5"
                bind:value={promptText}
                onkeydown={handlePromptKeydown}
                placeholder={t("task_placeholder")}
                class="w-full rounded-xl border border-slate-300 p-3.5 text-sm focus:ring-2 focus:ring-blue-500 focus:outline-none leading-relaxed resize-y"
              ></textarea>
              <p class="text-xs text-slate-500 mt-1.5">⌨️ {t("prompt_submit_hint")}</p>
            </div>

            <!-- Attachments: audio files the model may upload to the line -->
            <div class="space-y-1.5">
              <div class="flex items-center gap-2 flex-wrap">
                <button
                  type="button"
                  onclick={pickAttachments}
                  disabled={attachmentsLocked}
                  class="text-xs px-2.5 py-1 rounded-lg border border-slate-300 text-slate-700 hover:bg-slate-100 transition disabled:opacity-40 disabled:cursor-not-allowed focus-visible:outline-2 focus-visible:outline-blue-600"
                >
                  🎵 {t("attach_audio")}
                </button>
                {#if attachments.length > 0}
                  <span class="text-xs text-slate-500">{t("attachments_title")}</span>
                {/if}
              </div>

              {#if attachments.length > 0}
                <ul class="flex flex-wrap gap-1.5">
                  {#each attachments as a (a.local_path)}
                    <li>
                      <span class="inline-flex items-center gap-1.5 rounded-lg bg-slate-100 px-2 py-0.5 text-xs">
                        <span class="font-mono text-slate-500">{a.id}</span>
                        <span class="text-slate-700 max-w-[14rem] truncate">
                          <bdi dir="ltr">{a.name}</bdi>
                        </span>
                        <!-- The picker cannot stat a local file, so `size` is 0
                             until Rust reads it: "0 B" would be a wrong fact,
                             where no size at all is merely a missing one. -->
                        {#if a.size > 0}
                          <span class="text-slate-500" dir="ltr">{formatSize(a.size)}</span>
                        {/if}
                        <button
                          type="button"
                          onclick={() => removeAttachment(a.id)}
                          disabled={attachmentsLocked}
                          aria-label={`${t("attachment_remove")}: ${a.name}`}
                          title={t("attachment_remove")}
                          class="text-slate-400 hover:text-rose-700 rounded disabled:opacity-40 disabled:cursor-not-allowed focus-visible:outline-2 focus-visible:outline-blue-600"
                        >
                          ✕
                        </button>
                      </span>
                    </li>
                  {/each}
                </ul>
              {/if}

              {#if attachError}
                <p class="text-xs text-amber-800 bg-amber-50 border border-amber-200 rounded-lg px-2.5 py-1.5">
                  {attachError}
                </p>
              {/if}
            </div>

            <!-- Ready-made tasks (editable, persisted locally) -->
            <PresetBar
              {presets}
              onUse={(/** @type {string} */ text) => (promptText = text)}
              onChange={updatePresets}
              onRestoreDefaults={restoreDefaultPresets}
            />

            {#if errorMessage}
              <div class="p-3 bg-rose-50 border border-rose-200 text-rose-700 text-xs rounded-xl flex items-center gap-2 flex-wrap">
                <span>⚠️</span>
                <span class="flex-1 min-w-0">{errorMessage}</span>
                {#if lastRunPrompt && !isLoading}
                  <button
                    type="button"
                    onclick={retryRun}
                    class="px-2.5 py-1 rounded-lg bg-white border border-rose-300 text-rose-700 text-xs font-bold hover:bg-rose-100 transition shrink-0"
                  >
                    🔁 {t("retry_run")}
                  </button>
                {/if}
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
                class="w-full sm:w-auto px-7 py-3 bg-gradient-to-r from-blue-600 to-indigo-600 hover:from-blue-700 hover:to-indigo-700 text-white font-bold text-sm rounded-xl shadow-md transition disabled:opacity-50 flex items-center justify-center gap-2 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-blue-700"
              >
                {#if isLoading}
                  <span class="animate-spin">⏳</span>
                  <span>{t("proposing_changes")}</span>
                {:else}
                  <span>{t("propose_changes")}</span>
                {/if}
              </button>
            </div>
          </form>
        </div>

        <!-- A continued task states what it is continuing from -->
        {#if taskChain.length > 1}
          <div class="bg-white rounded-2xl border border-slate-200 px-5 py-3 shadow-sm">
            <p class="text-xs font-bold text-slate-700">
              {t("refine_parent", { previous: taskChain[taskChain.length - 2] })}
            </p>
            <ol class="mt-1.5 space-y-0.5 text-xs text-slate-500 list-decimal list-inside">
              {#each taskChain as instruction, i (i)}
                <li class="truncate" title={instruction}>{instruction}</li>
              {/each}
            </ol>
          </div>
        {/if}

        <!-- A task loaded from the history says so before anything it shows -->
        {#if historyReadOnly}
          <p
            class="text-xs text-blue-900 bg-blue-50 border border-blue-200 rounded-2xl px-4 py-2.5"
            role="status"
          >
            {t("history_loaded_notice")}
          </p>
        {/if}

        <!-- Agent Progress Panel (direct mode) -->
        {#if agentTimeline.length > 0 || agentRunning || agentFinish || agentError}
          <AgentTimeline
            timeline={agentTimeline}
            turn={agentTurn}
            maxTurns={agentMaxTurns}
            running={agentRunning}
            cancelling={agentCancelling}
            retryNotice={agentRetryNotice}
            error={agentError}
            finish={agentFinish}
            statsLine={runStatsLine}
            costLine={runCostLine}
            elapsedLabel={runElapsedLabel}
            elapsedMs={runElapsedMs}
            awaitingText={awaitingTurnText}
            canRetry={!!lastRunPrompt}
            busy={isLoading}
            stopLabel={agentStopLabel}
            onCancel={cancelAgentRun}
            onRetry={retryRun}
            onTimelineElement={(/** @type {HTMLElement | null} */ el) => (timelineRef = el)}
            onTimelineScroll={handleTimelineScroll}
            onLinkClick={handleAgentContentClick}
          />
        {/if}

        <!-- Agent Approval List (ProposedAction rows with diff) -->
        {#if proposedActions.length > 0}
          <ActionApprovalList
            actions={proposedActions}
            results={actionResults}
            {undoState}
            {undoMessages}
            selectedCount={selectedActionCount}
            {riskSummary}
            {confirmRisky}
            notice={approvalNotice}
            busy={isLoading}
            {rtl}
            isApplied={isActionApplied}
            onSelectAll={selectAllActions}
            onClearAll={clearAllActions}
            onApprove={approveSelectedActions}
            onCancelConfirm={() => (confirmRisky = false)}
            onToggleAction={handleActionToggled}
            onUndo={undoAction}
            readOnly={historyReadOnly}
          />
        {/if}

        <!-- "דייק את המשימה" — one more instruction, continuing this run -->
        {#if agentFinish && (runId || loadedTaskId) && !agentRunning}
          <div class="bg-white rounded-2xl border border-slate-200 p-5 shadow-sm space-y-2">
            <label for="refine-input" class="block text-sm font-bold text-slate-800">
              {t("refine_title")}
            </label>
            <div class="flex items-center gap-2 flex-wrap">
              <input
                id="refine-input"
                type="text"
                bind:value={refineText}
                onkeydown={(e) => {
                  if (e.key === "Enter") {
                    e.preventDefault();
                    void continueAgentRun();
                  }
                }}
                placeholder={t("refine_placeholder")}
                class="flex-1 min-w-[12rem] text-sm rounded-xl border border-slate-300 p-2.5 focus:ring-2 focus:ring-blue-500 focus:outline-none"
              />
              <button
                type="button"
                onclick={() => void continueAgentRun()}
                disabled={refineBusy || isLoading || !refineText.trim()}
                class="px-4 py-2.5 rounded-xl bg-blue-600 hover:bg-blue-700 text-white text-xs font-bold transition disabled:opacity-50 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-blue-700"
              >
                {t("refine_send")}
              </button>
            </div>
            {#if refineError}
              <p class="text-xs text-amber-800 bg-amber-50 border border-amber-200 rounded-lg px-2.5 py-1.5">
                {refineError}
              </p>
            {/if}
          </div>
        {/if}

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

            {#if scriptPreviewNotice}
              <div class="text-xs text-amber-800 bg-amber-50 border border-amber-200 rounded-lg px-3 py-2">
                {scriptPreviewNotice}
              </div>
            {/if}

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
                        <span class="text-slate-500">=</span>
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

        <!-- Script-mode execution results (per extension, per parameter) -->
        {#if scriptActionResults.length > 0}
          <div class="bg-white rounded-2xl border border-slate-200 p-5 shadow-sm space-y-3">
            <h3 class="text-xs font-bold text-slate-700 border-b pb-2">{t("results_title")}</h3>
            <div class="space-y-2">
              {#each scriptActionResults as res}
                <div
                  class="text-xs rounded-lg px-2.5 py-2 border
                    {res.ok
                      ? 'bg-emerald-50 border-emerald-200 text-emerald-800'
                      : 'bg-rose-50 border-rose-200 text-rose-800'}"
                >
                  <div class="font-bold flex items-center gap-1.5 flex-wrap">
                    <span>{res.ok ? "✓" : "✗"}</span>
                    <span class="font-mono"><bdi dir="ltr">{res.path}</bdi></span>
                    <span>{res.ok ? t("action_ok") : t("action_failed")}</span>
                    {#if res.message}<span class="font-normal">— {res.message}</span>{/if}
                  </div>
                  {#if res.params && res.params.length > 0}
                    <ul class="mt-1 space-y-0.5">
                      {#each res.params as p}
                        <li class="flex items-center gap-1.5 flex-wrap">
                          <span>{p.applied ? "✓" : "✗"}</span>
                          <span class="font-mono"><bdi dir="ltr">{p.key}={p.value}</bdi></span>
                          <span class="text-slate-500">{p.applied ? t("param_applied") : t("param_not_applied")}</span>
                          {#if p.note}<span class="text-slate-500">— {p.note}</span>{/if}
                        </li>
                      {/each}
                    </ul>
                  {/if}
                </div>
              {/each}
            </div>
          </div>
        {/if}

        <!-- Raw Result Output — skipped in direct mode, where the same text is
             already rendered as markdown in the run timeline. -->
        {#if resultOutput && !agentFinish}
          <div class="bg-white rounded-2xl border border-slate-200 p-5 shadow-sm space-y-3">
            <div class="flex items-center justify-between gap-2">
              <h3 class="text-xs font-bold text-slate-700">{t("final_answer_title")}</h3>
              <button
                type="button"
                onclick={copyResultOutput}
                class="px-2.5 py-1 rounded-lg border border-slate-300 text-xs font-medium text-slate-700 hover:bg-slate-100 transition"
              >
                {resultCopied ? `✓ ${t("copied")}` : `📋 ${t("copy_output")}`}
              </button>
            </div>
            <pre class="bg-slate-900 text-slate-100 p-4 rounded-xl text-xs font-mono whitespace-pre-wrap overflow-x-auto max-h-80">{resultOutput}</pre>
          </div>
        {/if}

        <!-- יומן שינויים — what actually reached the line, across runs -->
        <ChangeLog
          changes={appliedChanges}
          loading={changeLogLoading}
          error={changeLogError}
          undoState={changeLogUndoState}
          onRefresh={loadChangeLog}
          onUndo={undoLoggedChange}
        />

        <!-- היסטוריית משימות — the tasks kept on this computer -->
        <TaskHistory
          items={taskHistory}
          loading={historyLoading}
          error={historyError}
          runActive={runBusy}
          {loadedTaskId}
          onRefresh={loadTaskHistory}
          onContinue={continueFromHistory}
          onDelete={deleteTaskHistory}
          onClearAll={clearTaskHistory}
        />
      </div>
    </div>
  </main>

  <!-- Footer credit -->
  <footer class="max-w-6xl mx-auto px-4 py-6 flex justify-center">
    <button
      type="button"
      onclick={() => openUrl("https://bot-phone.netlify.app/")}
      class="inline-flex items-center gap-2 px-4 py-2 rounded-full bg-white border border-slate-200 shadow-sm text-xs font-medium text-slate-600 hover:text-blue-600 hover:border-blue-300 transition"
    >
      <span class="text-base" aria-hidden="true">🤖</span>
      <span>{t("footer_credit")}</span>
    </button>
  </footer>

  <!-- Knowledge Explorer Modal -->
  {#if showKnowledgeModal}
    <KnowledgeModal
      totalCount={knowledgeFiles.length}
      matches={knowledgeMatches}
      bind:searchQuery
      content={selectedFileContent}
      fileName={selectedFileName}
      bind:rawView={isRawView}
      {copyFeedback}
      {rtl}
      onClose={closeKnowledgeModal}
      onSearchInput={scheduleKnowledgeSearch}
      onOpenFile={(/** @type {string} */ name) => openKnowledgeFile(name)}
      onCopy={copySelectedContent}
      onContentClick={handleContentClick}
      onContentElement={(/** @type {HTMLElement | null} */ el) => (contentContainerRef = el)}
    />
  {/if}

  <!-- Settings drawer (everything that used to be the left-hand card) -->
  {#if showSettings}
    <SettingsPanel
      {targetMode}
      onTargetMode={setTargetMode}
      bind:aiProvider
      onProviderChange={handleProviderChange}
      bind:modelType
      bind:modelSource
      bind:selectedModel
      bind:customModel
      bind:customBaseUrl
      onSaveCustomBaseUrl={saveCustomBaseUrl}
      {getProviderModels}
      bind:yemotToken
      bind:showToken
      {tokenStatus}
      onSaveYemotToken={saveYemotToken}
      onCheckToken={checkToken}
      onClearToken={clearToken}
      onOpenLogin={() => {
        showSettings = false;
        openLoginModal();
      }}
      {apiKeys}
      bind:showApiKey
      onSaveApiKey={saveApiKey}
      bind:autoApply
      bind:includeTree
      bind:saveHistory
      onSaveAgentToggles={saveAgentToggles}
      bind:logoutOnFinish
      bind:isPreviewMode
      bind:scriptUrl
      {rtl}
      onClose={closeSettings}
    />
  {/if}

  <!-- Login (create token) Modal -->
  {#if showLoginModal}
    <LoginModal
      step={loginStep}
      bind:username={loginUsername}
      bind:password={loginPassword}
      bind:showPassword={loginShowPassword}
      methods={loginMethods}
      bind:methodId={loginMethodId}
      bind:code={loginCode}
      codeSent={loginCodeSent}
      status={loginStatus}
      error={loginError}
      loading={loginLoading}
      {rtl}
      onClose={closeLoginModal}
      onLogin={handleLogin}
      onSendCode={handleSendLoginCode}
      onValidateCode={handleValidateLoginCode}
      onBack={backToCredentials}
      onMethodChange={handleLoginMethodChange}
    />
  {/if}
</div>

<style>
  /* Assistant markdown inside the agent progress panel (rendered via {@html}). */
  :global(.agent-md > *:first-child) {
    margin-top: 0;
  }
  :global(.agent-md > *:last-child) {
    margin-bottom: 0;
  }
  :global(.agent-md p) {
    margin: 0 0 0.45rem;
  }
  :global(.agent-md ul),
  :global(.agent-md ol) {
    margin: 0 0 0.45rem;
    padding-inline-start: 1.15rem;
    list-style: revert;
  }
  :global(.agent-md li) {
    margin-bottom: 0.15rem;
  }
  :global(.agent-md strong) {
    font-weight: 700;
    color: #0f172a;
  }
  :global(.agent-md h1),
  :global(.agent-md h2),
  :global(.agent-md h3),
  :global(.agent-md h4) {
    font-weight: 700;
    color: #0f172a;
    margin: 0.5rem 0 0.3rem;
    font-size: 0.8rem;
  }
  :global(.agent-md code) {
    direction: ltr;
    unicode-bidi: isolate;
    background: #e2e8f0;
    border-radius: 0.25rem;
    padding: 0.05rem 0.25rem;
    font-size: 0.72rem;
  }
  /* Copy button injected into every code block by `enhanceAgentHtml`. */
  :global(.agent-md .md-pre-wrap) {
    position: relative;
  }
  :global(.agent-md .md-copy-btn) {
    position: absolute;
    top: 0.35rem;
    inset-inline-end: 0.35rem;
    padding: 0.1rem 0.4rem;
    font-size: 0.7rem;
    font-weight: 600;
    color: #cbd5e1;
    background: #1e293b;
    border: 1px solid #475569;
    border-radius: 0.35rem;
    cursor: pointer;
    opacity: 0;
    transition: opacity 0.15s;
  }
  :global(.agent-md .md-pre-wrap:hover .md-copy-btn),
  :global(.agent-md .md-copy-btn:focus-visible) {
    opacity: 1;
  }
  :global(.agent-md pre) {
    direction: ltr;
    unicode-bidi: isolate;
    text-align: left;
    background: #0f172a;
    color: #e2e8f0;
    border-radius: 0.5rem;
    padding: 0.6rem;
    overflow-x: auto;
    margin: 0 0 0.45rem;
  }
  :global(.agent-md pre code) {
    background: transparent;
    padding: 0;
    color: inherit;
  }
  :global(.agent-md a) {
    color: #2563eb;
    text-decoration: underline;
  }
  :global(.agent-md table) {
    display: block;
    overflow-x: auto;
    border-collapse: collapse;
    margin: 0 0 0.45rem;
  }
  :global(.agent-md th),
  :global(.agent-md td) {
    border: 1px solid #e2e8f0;
    padding: 0.2rem 0.4rem;
  }
</style>
