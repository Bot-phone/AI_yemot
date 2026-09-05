<script>
  /**
   * Approval list for the ProposedActions of a run. The rows themselves are the
   * `$state` objects owned by +page.svelte, so `bind:checked` on `action.selected`
   * still writes straight back into the page's state.
   */
  import { t } from "$lib/i18n.svelte.js";
  import { ltrify } from "$lib/markdown.js";

  let {
    /** @type {any[]} */
    actions = [],
    /** @type {Record<string, any>} */
    results = {},
    /** @type {Record<string, string>} */
    undoState = {},
    /** @type {Record<string, string>} */
    undoMessages = {},
    selectedCount = 0,
    /** @type {{settings: number, paths: number, overwrites: number}} */
    riskSummary = { settings: 0, paths: 0, overwrites: 0 },
    confirmRisky = false,
    notice = "",
    busy = false,
    rtl = true,
    /** @type {(action: any) => boolean} */
    isApplied,
    /** @type {() => void} */
    onSelectAll,
    /** @type {() => void} */
    onClearAll,
    /** @type {() => void} */
    onApprove,
    /** @type {() => void} */
    onCancelConfirm,
    /** @type {(actionId: string) => void} */
    onUndo,
    /**
     * A single checkbox changed. The armed risky-change confirmation describes a
     * selection that no longer exists, so the page disarms it.
     * @type {(() => void) | undefined}
     */
    onToggleAction = undefined
  } = $props();

  /**
   * `res.undo` is an UndoRecord (src-tauri/src/agent/runner.rs): `{ action_id,
   * kind, path, params: [key, previousValue | null][], contents: string | null }`.
   * Rendering it straight into the markup gives "[object Object]", so pull the
   * two shapes it can carry apart and ignore anything unrecognised.
   * @param {any} undo
   */
  function undoParams(undo) {
    const rows = undo && Array.isArray(undo.params) ? undo.params : [];
    return rows
      .filter((/** @type {any} */ r) => Array.isArray(r) && r.length >= 1)
      .map((/** @type {any} */ r) => ({
        key: String(r[0]),
        value: r[1] == null ? t("value_missing") : r[1] === "" ? t("value_empty") : String(r[1])
      }));
  }

  /** @param {any} undo */
  function undoContents(undo) {
    return undo && typeof undo.contents === "string" ? undo.contents : "";
  }
</script>

<div class="bg-white rounded-2xl border border-slate-200 p-6 shadow-sm space-y-4">
  <div class="flex items-start justify-between border-b pb-3 gap-3 flex-wrap">
    <div>
      <h3 class="text-sm font-bold text-slate-800">{t("agent_actions_title")}</h3>
      <p class="text-xs text-slate-500">{t("agent_actions_hint")}</p>
    </div>
    <div class="flex items-center gap-2 flex-wrap">
      <button
        type="button"
        onclick={onSelectAll}
        class="px-2.5 py-1 rounded-lg border border-slate-300 text-xs font-medium text-slate-700 hover:bg-slate-100 transition"
      >
        {t("select_all")}
      </button>
      <button
        type="button"
        onclick={onClearAll}
        class="px-2.5 py-1 rounded-lg border border-slate-300 text-xs font-medium text-slate-700 hover:bg-slate-100 transition"
      >
        {t("clear_all")}
      </button>
      <button
        type="button"
        onclick={onApprove}
        disabled={busy || selectedCount === 0}
        class="px-4 py-2 bg-emerald-600 hover:bg-emerald-700 text-white text-xs font-bold rounded-lg shadow transition disabled:opacity-50 disabled:cursor-not-allowed shrink-0"
      >
        {t("approve_selected_count", { count: selectedCount })}
      </button>
    </div>
  </div>

  {#if notice}
    <div class="text-xs text-amber-800 bg-amber-50 border border-amber-200 rounded-lg px-3 py-2">
      {notice}
    </div>
  {/if}

  {#if confirmRisky}
    <div class="text-xs bg-amber-50 border border-amber-300 rounded-xl p-3 space-y-2">
      <div class="font-bold text-amber-900">⚠️ {t("confirm_risky_title")}</div>
      <p class="text-amber-900 leading-relaxed">
        {t("confirm_risky_line", {
          settings: riskSummary.settings,
          paths: riskSummary.paths,
          overwrites: riskSummary.overwrites
        })}
      </p>
      <div class="flex items-center gap-2">
        <button
          type="button"
          onclick={onApprove}
          disabled={busy}
          class="px-3 py-1.5 bg-amber-600 hover:bg-amber-700 text-white text-xs font-bold rounded-lg transition disabled:opacity-50"
        >
          {t("confirm_risky_approve")}
        </button>
        <button
          type="button"
          onclick={onCancelConfirm}
          class="px-3 py-1.5 border border-slate-300 text-slate-700 text-xs font-medium rounded-lg hover:bg-white transition"
        >
          {t("cancel")}
        </button>
      </div>
    </div>
  {/if}

  <div class="divide-y divide-slate-100 max-h-[30rem] overflow-y-auto">
    {#each actions as action (action.id)}
      {@const applied = isApplied(action)}
      <div class="py-3 space-y-2 {applied ? 'opacity-70 bg-emerald-50/40 rounded-lg px-2' : ''}">
        <div class="flex items-start gap-3">
          <input
            type="checkbox"
            bind:checked={action.selected}
            onchange={() => onToggleAction?.()}
            disabled={applied}
            aria-label={action.path}
            class="mt-1 rounded text-blue-600 focus:ring-blue-500 disabled:opacity-50"
          />
          <div class="flex-1 min-w-0 text-xs space-y-1.5">
            <div class="flex items-center gap-2 flex-wrap">
              <span class="bg-slate-100 text-slate-800 px-1.5 py-0.5 rounded font-mono font-bold">
                <bdi dir="ltr">{action.path}</bdi>
              </span>
              <span class="text-xs text-slate-500">
                {action.kind === "upload_text_file" ? t("kind_upload_file") : t("kind_set_params")}
              </span>
              {#if applied}
                <span class="text-xs bg-emerald-100 text-emerald-800 border border-emerald-300 px-1.5 py-0.5 rounded-full font-semibold">
                  ✓ {t("action_already_applied")}
                </span>
              {/if}
              {#if !action.exists}
                <span class="text-xs bg-sky-50 text-sky-700 border border-sky-200 px-1.5 py-0.5 rounded-full font-semibold">
                  ✨ {t("will_be_created")}
                </span>
              {/if}
              <span
                class="text-xs px-1.5 py-0.5 rounded-full font-semibold border
                  {action.risk === 'destructive'
                    ? 'bg-rose-50 text-rose-700 border-rose-200'
                    : action.risk === 'overwrite'
                      ? 'bg-amber-50 text-amber-800 border-amber-200'
                      : 'bg-emerald-50 text-emerald-700 border-emerald-200'}"
              >
                {action.risk === "destructive"
                  ? t("risk_destructive")
                  : action.risk === "overwrite"
                    ? t("risk_overwrite")
                    : t("risk_low")}
              </span>
            </div>

            {#if action.reason}
              <p class="text-slate-600 leading-relaxed">{action.reason}</p>
            {/if}

            {#if action.warnings.length > 0}
              <div class="text-xs text-amber-800 bg-amber-50 border border-amber-200 rounded-lg px-2.5 py-1.5">
                <span class="font-bold">{t("warnings_title")}:</span>
                <ul class="list-disc {rtl ? 'mr-4' : 'ml-4'} mt-0.5 space-y-0.5">
                  {#each action.warnings as w}
                    <li>{@html ltrify(w)}</li>
                  {/each}
                </ul>
              </div>
            {/if}

            <button
              type="button"
              onclick={() => (action.expanded = !action.expanded)}
              class="text-xs text-blue-600 hover:underline"
            >
              {action.expanded ? `▲ ${t("hide_diff")}` : `▼ ${t("show_diff")}`}
            </button>

            {#if action.expanded}
              {#if action.previous}
                <!-- The full previous file content: long, so keep it collapsed. -->
                <details class="text-xs text-slate-600 bg-slate-50 border border-slate-200 rounded-lg px-2.5 py-1.5">
                  <summary class="cursor-pointer font-bold">{t("undo_previous_content")}</summary>
                  <pre class="mt-1 max-h-48 overflow-auto whitespace-pre-wrap break-all rounded-md bg-white p-2 font-mono text-xs" dir="ltr">{action.previous}</pre>
                </details>
              {/if}
              {#if action.diff.length === 0}
                <p class="text-xs text-slate-500">{t("no_diff")}</p>
              {:else}
                <div class="overflow-x-auto border border-slate-200 rounded-lg">
                  <table class="w-full text-xs">
                    <thead class="bg-slate-50 text-slate-500">
                      <tr>
                        <th class="p-1.5 {rtl ? 'text-right' : 'text-left'} font-semibold">{t("diff_key")}</th>
                        <th class="p-1.5 {rtl ? 'text-right' : 'text-left'} font-semibold">{t("diff_before")}</th>
                        <th class="p-1.5 {rtl ? 'text-right' : 'text-left'} font-semibold">{t("diff_after")}</th>
                      </tr>
                    </thead>
                    <tbody class="divide-y divide-slate-100">
                      {#each action.diff as d}
                        <tr
                          class={d.kind === "new"
                            ? "bg-sky-50/60"
                            : d.kind === "changed"
                              ? "bg-amber-50/60"
                              : ""}
                        >
                          <td class="p-1.5 font-mono font-bold text-slate-700">
                            <bdi dir="ltr">{d.key}</bdi>
                          </td>
                          <td class="p-1.5 font-mono text-slate-500">
                            {#if d.before}
                              <bdi dir="ltr">{d.before}</bdi>
                            {:else}
                              <span class="text-slate-500">{t("diff_empty")}</span>
                            {/if}
                          </td>
                          <td
                            class="p-1.5 font-mono font-semibold
                              {d.kind === 'new'
                                ? 'text-sky-700'
                                : d.kind === 'changed'
                                  ? 'text-amber-800'
                                  : 'text-slate-500'}"
                          >
                            {#if d.after}
                              <bdi dir="ltr">{d.after}</bdi>
                            {:else}
                              <span class="text-slate-500">{t("diff_empty")}</span>
                            {/if}
                          </td>
                        </tr>
                      {/each}
                    </tbody>
                  </table>
                </div>
              {/if}
            {/if}

            {#if results[action.id]}
              {@const res = results[action.id]}
              <div
                class="text-xs rounded-lg px-2.5 py-1.5 border
                  {res.ok
                    ? 'bg-emerald-50 border-emerald-200 text-emerald-800'
                    : 'bg-rose-50 border-rose-200 text-rose-800'}"
              >
                <div class="font-bold">
                  {res.ok ? `✓ ${t("action_ok")}` : `✗ ${t("action_failed")}`}
                  {#if res.message}<span class="font-normal"> — {res.message}</span>{/if}
                </div>
                {#if res.params && res.params.length > 0}
                  <ul class="mt-1 space-y-0.5">
                    {#each res.params as p}
                      <li class="flex items-center gap-1.5 flex-wrap">
                        <span>{p.applied ? "✓" : "✗"}</span>
                        <span class="font-mono"><bdi dir="ltr">{p.key}={p.value}</bdi></span>
                        <span class="text-slate-600">
                          {p.applied ? t("param_applied") : t("param_not_applied")}
                        </span>
                        {#if p.note}<span class="text-slate-600">— {p.note}</span>{/if}
                      </li>
                    {/each}
                  </ul>
                {/if}
                {#if res.undo}
                  {@const prevParams = undoParams(res.undo)}
                  {@const prevContents = undoContents(res.undo)}
                  {#if prevParams.length > 0}
                    <div class="mt-1 text-slate-700">
                      <span class="font-bold">{t("undo_previous_params")}:</span>
                      <ul class="mt-0.5 space-y-0.5">
                        {#each prevParams as p}
                          <li class="font-mono break-all">
                            <bdi dir="ltr">{p.key}</bdi>: <bdi dir="ltr">{p.value}</bdi>
                          </li>
                        {/each}
                      </ul>
                    </div>
                  {:else if prevContents}
                    <details class="mt-1 text-slate-700">
                      <summary class="cursor-pointer font-bold">{t("undo_previous_content")}</summary>
                      <pre class="mt-1 max-h-48 overflow-auto whitespace-pre-wrap break-all rounded-md bg-white/60 p-2 font-mono text-xs" dir="ltr">{prevContents}</pre>
                    </details>
                  {/if}
                {/if}
                {#if res.ok}
                  <div class="mt-1.5 flex items-center gap-2 flex-wrap">
                    {#if undoState[action.id] === "done"}
                      <span class="font-semibold text-slate-700">↩ {t("undo_done")}</span>
                      {#if undoMessages[action.id]}
                        <span class="text-slate-600">— {undoMessages[action.id]}</span>
                      {/if}
                    {:else if undoState[action.id] === "unavailable"}
                      <span class="text-slate-600">{t("undo_unavailable")}</span>
                    {:else if undoState[action.id] === "failed"}
                      <span class="font-semibold text-rose-700">✗ {t("undo_failed")}</span>
                      {#if undoMessages[action.id]}
                        <span class="text-rose-700">— {undoMessages[action.id]}</span>
                      {/if}
                      <button
                        type="button"
                        onclick={() => onUndo(action.id)}
                        class="px-2 py-0.5 rounded-md border border-slate-300 bg-white text-slate-700 text-xs font-medium hover:bg-slate-100 transition"
                      >
                        ↩ {t("undo_action")}
                      </button>
                    {:else}
                      <button
                        type="button"
                        onclick={() => onUndo(action.id)}
                        class="px-2 py-0.5 rounded-md border border-slate-300 bg-white text-slate-700 text-xs font-medium hover:bg-slate-100 transition"
                      >
                        ↩ {t("undo_action")}
                      </button>
                    {/if}
                  </div>
                {/if}
              </div>
            {/if}
          </div>
        </div>
      </div>
    {/each}
  </div>
</div>
