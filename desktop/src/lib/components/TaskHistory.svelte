<script>
  /**
   * "היסטוריית משימות" — the tasks this machine kept on disk, newest first.
   *
   * Rows are `TaskSummary` (contract: local task history): `{ task_id,
   * last_run_id, created_at_ms, updated_at_ms, provider, model, title, steps,
   * ok, stop, cost_usd, resumable }`. Presentation plus the list-local bits the
   * page has no reason to own: the filter box and the two-step "נקה הכל"
   * confirmation. Everything that talks to Rust is a callback.
   */
  import { t } from "$lib/i18n.svelte.js";
  import { formatLogTime } from "$lib/lineEditor.js";

  let {
    /** @type {any[]} */
    items = [],
    loading = false,
    error = "",
    /** A run is in flight — continuing a stored task would fight with it. */
    runActive = false,
    /** The task currently loaded into the workspace, "" when none. */
    loadedTaskId = "",
    /** @type {() => void} */
    onRefresh,
    /** @type {(taskId: string) => void} */
    onContinue,
    /** @type {(taskId: string) => void} */
    onDelete,
    /** @type {() => void} */
    onClearAll
  } = $props();

  let filter = $state("");
  /** The "נקה הכל" confirmation is armed and waiting for a second click. */
  let confirmClear = $state(false);

  let matches = $derived.by(() => {
    const q = filter.trim().toLowerCase();
    if (!q) return items;
    return items.filter((row) =>
      [row?.title, row?.model, row?.provider]
        .filter(Boolean)
        .some((v) => String(v).toLowerCase().includes(q))
    );
  });

  /**
   * "הושלם" / "נכשל" / "בוטל". A cancelled run is not a failure — it is the
   * user's own stop — so the stop reason is read before `ok`.
   * @param {any} row
   */
  function statusOf(row) {
    if (row?.stop === "cancelled") return "cancelled";
    return row?.ok ? "ok" : "failed";
  }

  /** @param {any} row */
  function statusLabel(row) {
    const s = statusOf(row);
    if (s === "cancelled") return t("history_status_cancelled");
    if (s === "ok") return t("history_status_ok");
    return t("history_status_failed");
  }

  /** @param {any} row */
  function statusClass(row) {
    const s = statusOf(row);
    if (s === "cancelled") return "bg-slate-100 text-slate-600 border-slate-300";
    if (s === "ok") return "bg-emerald-50 text-emerald-800 border-emerald-200";
    return "bg-rose-50 text-rose-700 border-rose-200";
  }

  /**
   * The estimated cost of the chain, or "" when the model is not in the price
   * table (`cost_usd: null`) — an absent number is never rendered as "$0".
   * @param {any} row
   */
  function costLabel(row) {
    const c = row?.cost_usd;
    if (c === null || c === undefined || !Number.isFinite(Number(c))) return "";
    const cost = Number(c).toFixed(4).replace(/0+$/, "").replace(/\.$/, "");
    return t("history_cost", { cost });
  }

  function clearClicked() {
    if (!confirmClear) {
      confirmClear = true;
      return;
    }
    confirmClear = false;
    onClearAll();
  }
</script>

<section
  class="bg-white rounded-2xl border border-slate-200 p-5 shadow-sm space-y-3"
  aria-labelledby="task-history-title"
>
  <div class="flex items-center justify-between gap-2 border-b pb-2 flex-wrap">
    <h2 id="task-history-title" class="text-sm font-bold text-slate-800">
      {t("history_title")}
    </h2>
    <div class="flex items-center gap-2">
      <button
        type="button"
        onclick={onRefresh}
        disabled={loading}
        title={t("history_refresh")}
        aria-label={t("history_refresh")}
        class="px-2 py-1 rounded-lg border border-slate-300 text-xs font-medium text-slate-700 hover:bg-slate-100 transition disabled:opacity-40 focus-visible:outline-2 focus-visible:outline-blue-600"
      >
        <span class="inline-block {loading ? 'history-spin' : ''}" aria-hidden="true">⟳</span>
      </button>
      <button
        type="button"
        onclick={clearClicked}
        disabled={loading || items.length === 0}
        class="px-2.5 py-1 rounded-lg border text-xs font-medium transition disabled:opacity-40 focus-visible:outline-2 focus-visible:outline-blue-600
          {confirmClear
          ? 'border-rose-300 bg-rose-600 text-white hover:bg-rose-700'
          : 'border-slate-300 text-slate-700 hover:bg-slate-100'}"
      >
        {confirmClear ? t("history_clear_confirm") : t("history_clear_all")}
      </button>
      {#if confirmClear}
        <button
          type="button"
          onclick={() => (confirmClear = false)}
          class="px-2.5 py-1 rounded-lg border border-slate-300 text-xs font-medium text-slate-700 hover:bg-slate-100 transition focus-visible:outline-2 focus-visible:outline-blue-600"
        >
          {t("cancel")}
        </button>
      {/if}
    </div>
  </div>

  {#if items.length > 0}
    <input
      type="search"
      bind:value={filter}
      placeholder={t("history_search")}
      aria-label={t("history_search")}
      class="w-full text-xs rounded-lg border border-slate-300 p-2 focus:ring-2 focus:ring-blue-500 focus:outline-none"
    />
  {/if}

  {#if error}
    <p class="text-xs text-rose-700 bg-rose-50 border border-rose-200 rounded-lg px-2.5 py-2">
      {error}
    </p>
  {:else if items.length === 0}
    <p class="text-xs text-slate-500 py-2">{t("history_empty")}</p>
  {:else if matches.length === 0}
    <p class="text-xs text-slate-500 py-2">{t("history_no_matches")}</p>
  {:else}
    <ul class="divide-y divide-slate-100 max-h-80 overflow-y-auto">
      {#each matches as row (row.task_id)}
        {@const cost = costLabel(row)}
        {@const isLoaded = row.task_id === loadedTaskId}
        <li
          class="py-2 flex items-start gap-3 {isLoaded ? 'bg-blue-50/60 rounded-lg px-2' : ''}"
        >
          <div class="flex-1 min-w-0 text-xs space-y-1">
            <p class="font-medium text-slate-800 break-words">
              {row.title || t("history_untitled")}
            </p>
            <div class="flex items-center gap-2 flex-wrap text-slate-500">
              <span class="tabular-nums" dir="ltr">{formatLogTime(row.updated_at_ms)}</span>
              <span>{row.steps === 1 ? t("history_one_step") : t("history_steps", { count: row.steps ?? 0 })}</span>
              <span class="bg-slate-100 text-slate-700 px-1.5 py-0.5 rounded font-mono">
                <bdi dir="ltr">{row.model || row.provider || ""}</bdi>
              </span>
              <span
                class="px-1.5 py-0.5 rounded-full border font-semibold {statusClass(row)}"
              >
                {statusLabel(row)}
              </span>
              {#if cost}
                <span class="tabular-nums" dir="ltr">{cost}</span>
              {/if}
              {#if isLoaded}
                <span
                  class="bg-blue-50 text-blue-800 border border-blue-200 px-1.5 py-0.5 rounded-full font-semibold"
                >
                  {t("history_loaded_chip")}
                </span>
              {/if}
            </div>
          </div>

          <div class="shrink-0 flex items-center gap-1.5">
            <button
              type="button"
              onclick={() => onContinue(row.task_id)}
              disabled={runActive || !row.resumable}
              title={row.resumable ? t("history_continue") : t("history_not_resumable")}
              class="px-2 py-0.5 rounded-md border border-slate-300 bg-white text-slate-700 text-xs font-medium hover:bg-slate-100 transition disabled:opacity-40 disabled:cursor-not-allowed focus-visible:outline-2 focus-visible:outline-blue-600"
            >
              {t("history_continue")}
            </button>
            <button
              type="button"
              disabled={runActive}
              onclick={() => onDelete(row.task_id)}
              aria-label={t("history_delete")}
              class="px-2 py-0.5 rounded-md border border-slate-300 bg-white text-slate-600 text-xs font-medium hover:bg-rose-50 hover:text-rose-700 hover:border-rose-300 transition focus-visible:outline-2 focus-visible:outline-blue-600"
            >
              {t("history_delete")}
            </button>
          </div>
        </li>
      {/each}
    </ul>
  {/if}
</section>

<style>
  .history-spin {
    animation: history-rotate 0.9s linear infinite;
  }
  @keyframes history-rotate {
    to {
      transform: rotate(360deg);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .history-spin {
      animation: none;
    }
  }
</style>
