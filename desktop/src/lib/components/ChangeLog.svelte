<script>
  /**
   * "יומן שינויים" — every change that was actually written to the line.
   * Rows are `AppliedChange` (contract R2): `{ run_id, action_id, kind, path,
   * display, applied_at_ms, params, undo_available, undone, label }`, newest
   * first. The list survives a page reload because it is read back from the
   * Rust WRITE_STATE, not from component state.
   */
  import { t } from "$lib/i18n.svelte.js";
  import { formatLogTime } from "$lib/lineEditor.js";

  let {
    /** @type {any[]} */
    changes = [],
    loading = false,
    error = "",
    /**
     * "<run_id>:<action_id>" -> "" | "done" | "failed" | "unavailable".
     * Action ids restart at `a_1` in every run, so they only identify a row
     * together with the run that produced it.
     */
    /** @type {Record<string, string>} */
    undoState = {},
    /** @type {() => void} */
    onRefresh,
    /** @type {(runId: string, actionId: string) => void} */
    onUndo
  } = $props();

  /**
   * The identity of a logged change. `action_id` alone is not unique: it is
   * assigned per run (`a_1`, `a_2`, …), and the log holds rows from every run.
   * @param {any} change
   */
  function changeKey(change) {
    return `${change?.run_id ?? ""}:${change?.action_id ?? ""}`;
  }

  /** A one-line "key=value · key=value" summary of the change's params. */
  /** @param {any} change */
  function paramsSummary(change) {
    const list = Array.isArray(change?.params) ? change.params : [];
    return list
      .map((/** @type {any} */ p) => `${p.key}=${p.value}`)
      .join(" · ");
  }
</script>

<section
  class="bg-white rounded-2xl border border-slate-200 p-5 shadow-sm space-y-3"
  aria-labelledby="changelog-title"
>
  <div class="flex items-center justify-between gap-2 border-b pb-2">
    <h2 id="changelog-title" class="text-sm font-bold text-slate-800">{t("changelog_title")}</h2>
    <button
      type="button"
      onclick={onRefresh}
      disabled={loading}
      class="px-2.5 py-1 rounded-lg border border-slate-300 text-xs font-medium text-slate-700 hover:bg-slate-100 transition disabled:opacity-40 focus-visible:outline-2 focus-visible:outline-blue-600"
    >
      {t("changelog_refresh")}
    </button>
  </div>

  {#if error}
    <p class="text-xs text-rose-700 bg-rose-50 border border-rose-200 rounded-lg px-2.5 py-2">
      {error}
    </p>
  {:else if changes.length === 0}
    <p class="text-xs text-slate-500 py-2">{t("changelog_empty")}</p>
  {:else}
    <ul class="divide-y divide-slate-100 max-h-80 overflow-y-auto">
      {#each changes as c (changeKey(c))}
        {@const summary = paramsSummary(c)}
        {@const key = changeKey(c)}
        <li class="py-2 flex items-start gap-3">
          <div class="flex-1 min-w-0 text-xs space-y-1">
            <div class="flex items-center gap-2 flex-wrap">
              <span class="bg-slate-100 text-slate-800 px-1.5 py-0.5 rounded font-mono font-bold">
                <bdi dir="ltr">{c.display || c.path}</bdi>
              </span>
              <span class="text-slate-700 font-medium">{c.label || c.kind}</span>
              <span class="text-slate-500 tabular-nums" dir="ltr">
                {formatLogTime(c.applied_at_ms)}
              </span>
              {#if c.undone}
                <span
                  class="bg-slate-100 text-slate-600 border border-slate-300 px-1.5 py-0.5 rounded-full font-semibold"
                >
                  ↩ {t("changelog_undone")}
                </span>
              {:else}
                <span
                  class="bg-emerald-50 text-emerald-800 border border-emerald-200 px-1.5 py-0.5 rounded-full font-semibold"
                >
                  ✓ {t("changelog_done")}
                </span>
              {/if}
            </div>
            {#if summary}
              <p class="font-mono text-slate-500 break-all"><bdi dir="ltr">{summary}</bdi></p>
            {/if}
            {#if undoState[key] === "failed"}
              <p class="text-rose-700 font-semibold">✗ {t("undo_failed")}</p>
            {:else if undoState[key] === "unavailable"}
              <p class="text-slate-500">{t("undo_unavailable")}</p>
            {/if}
          </div>

          {#if c.undo_available && !c.undone}
            <button
              type="button"
              onclick={() => onUndo(c.run_id, c.action_id)}
              class="shrink-0 px-2 py-0.5 rounded-md border border-slate-300 bg-white text-slate-700 text-xs font-medium hover:bg-slate-100 transition focus-visible:outline-2 focus-visible:outline-blue-600"
            >
              ↩ {t("undo_action")}
            </button>
          {/if}
        </li>
      {/each}
    </ul>
  {/if}
</section>
