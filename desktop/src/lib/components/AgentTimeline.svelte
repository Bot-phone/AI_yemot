<script>
  /**
   * Run-progress panel for a direct-mode agent run. Presentation only — every
   * piece of state lives in +page.svelte and arrives here as a prop.
   */
  import { t } from "$lib/i18n.svelte.js";
  import { ltrify, renderAgentMarkdown, handleCopyPreClick } from "$lib/markdown.js";

  let {
    /** @type {any[]} */
    timeline = [],
    turn = 0,
    maxTurns = 0,
    running = false,
    cancelling = false,
    /** @type {any} */
    retryNotice = null,
    /** @type {any} */
    error = null,
    /** @type {any} */
    finish = null,
    /** "טוקנים: … · מטמון … · n סבבים · t שנ׳" — measured facts. */
    statsLine = "",
    /** "עלות משוערת: $… לפי מחירון …", or the "unknown model" line. */
    costLine = "",
    elapsedLabel = "0.0",
    elapsedMs = 0,
    awaitingText = false,
    canRetry = false,
    busy = false,
    /** @type {(stop: string) => string} */
    stopLabel,
    /** @type {() => void} */
    onCancel,
    /** @type {() => void} */
    onRetry,
    /** @type {(el: HTMLElement | null) => void} */
    onTimelineElement,
    /** @type {() => void} */
    onTimelineScroll,
    /**
     * Link interception for the rendered agent markdown — without it a link in
     * the model's answer navigates the webview itself away from the app.
     * @type {((e: MouseEvent) => void) | undefined}
     */
    onLinkClick = undefined
  } = $props();

  /**
   * Both handlers have to run on every click: one owns the copy buttons, the
   * other owns the links.
   * @param {MouseEvent} e
   */
  function handleTimelineClick(e) {
    void handleCopyPreClick(e);
    onLinkClick?.(e);
  }

  /** @type {HTMLElement | null} */
  let timelineRef = $state(null);

  /** The cost disclaimer is one click away, not a permanent wall of text. */
  let showCostNote = $state(false);

  // Hand the scroll container to the page, which owns the follow-the-bottom
  // logic; clear it again when this panel goes away.
  $effect(() => {
    onTimelineElement?.(timelineRef);
    return () => onTimelineElement?.(null);
  });
</script>

<div class="bg-white rounded-2xl border border-slate-200 p-5 shadow-sm space-y-3">
  <div class="flex items-center justify-between border-b pb-3">
    <div class="flex items-center gap-2 flex-wrap">
      <h3 class="text-sm font-bold text-slate-800">{t("agent_panel_title")}</h3>
      {#if maxTurns > 0}
        <span class="text-xs bg-blue-50 text-blue-700 border border-blue-200 px-2 py-0.5 rounded-full font-semibold">
          {t("agent_turn_header", { turn, max: maxTurns })}
        </span>
      {/if}
      {#if running || elapsedMs > 0}
        <span class="text-xs text-slate-500 font-medium tabular-nums" dir="ltr">
          ⏱ {t("agent_elapsed", { secs: elapsedLabel })}
        </span>
      {/if}
    </div>
    {#if running}
      <button
        type="button"
        onclick={onCancel}
        disabled={cancelling}
        class="px-3 py-1.5 bg-rose-50 hover:bg-rose-100 border border-rose-200 text-rose-700 text-xs font-bold rounded-lg transition disabled:opacity-50"
      >
        {cancelling ? t("agent_cancelling") : `⏹ ${t("agent_cancel")}`}
      </button>
    {/if}
  </div>

  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    bind:this={timelineRef}
    onscroll={onTimelineScroll}
    onclick={handleTimelineClick}
    class="space-y-2 max-h-[28rem] overflow-y-auto"
  >
    {#each timeline as item, idx (idx)}
      {#if item.type === "turn"}
        <div class="flex items-center gap-2 pt-2">
          <span class="text-xs font-bold text-slate-500">
            {t("agent_turn_header", { turn: item.turn, max: item.max })}
          </span>
          <span class="flex-1 h-px bg-slate-200"></span>
        </div>
      {:else if item.type === "text"}
        {#if item.streaming}
          <!-- While streaming the text is not valid markdown yet: show it as
               escaped plain text and only render it once finalized. -->
          <div class="text-xs text-slate-700 bg-slate-50 border border-slate-100 rounded-xl p-3 leading-relaxed whitespace-pre-wrap">{item.text}</div>
        {:else}
          <div class="agent-md text-xs text-slate-700 bg-slate-50 border border-slate-100 rounded-xl p-3 leading-relaxed">
            {@html renderAgentMarkdown(item.text)}
          </div>
        {/if}
      {:else if item.type === "retry"}
        <div class="text-xs text-amber-800 bg-amber-50 border border-amber-200 rounded-lg px-3 py-2">
          🔁 {t("agent_retry_notice", { attempt: item.attempt, max: item.max })}
        </div>
      {:else if item.type === "tool"}
        <div class="flex items-start gap-2 text-xs px-2 py-1.5 rounded-lg hover:bg-slate-50">
          <span class="mt-0.5 {item.status === 'running' ? 'animate-spin' : ''}">
            {item.status === "ok" ? "✓" : item.status === "failed" ? "✗" : "⏳"}
          </span>
          <div class="flex-1 min-w-0">
            <div class="font-medium {item.status === 'failed' ? 'text-rose-700' : 'text-slate-800'}">
              {@html ltrify(item.label)}
            </div>
            {#if item.summary}
              <div class="text-xs text-slate-500 mt-0.5 break-words">{@html ltrify(item.summary)}</div>
            {:else if item.status === "running"}
              <div class="text-xs text-slate-500 mt-0.5">{t("agent_tool_running")}</div>
            {/if}
          </div>
          {#if item.ms !== null && item.ms !== undefined}
            <span class="text-xs text-slate-500 shrink-0" dir="ltr">{t("agent_ms", { ms: item.ms })}</span>
          {/if}
        </div>
      {/if}
    {/each}

    <!-- Waiting for the first assistant text of a turn -->
    {#if awaitingText}
      <div class="space-y-2 px-1 py-2" aria-hidden="true">
        <div class="h-2.5 rounded-full bg-slate-200 animate-pulse w-3/4"></div>
        <div class="h-2.5 rounded-full bg-slate-200 animate-pulse w-1/2"></div>
      </div>
    {/if}

    {#if running && timeline.length === 0}
      <div class="flex items-center gap-2 text-xs text-slate-500 py-3">
        <span class="animate-spin">⏳</span>
        <span>{t("agent_thinking")}</span>
      </div>
    {/if}
  </div>

  {#if retryNotice}
    <div class="text-xs text-amber-800 bg-amber-50 border border-amber-200 rounded-lg px-3 py-2">
      🔁 {t("agent_retry_notice", { attempt: retryNotice.attempt, max: retryNotice.max })}
    </div>
  {/if}

  {#if error}
    <div class="text-xs text-rose-700 bg-rose-50 border border-rose-200 rounded-xl p-3 space-y-2">
      <div class="font-bold">⚠️ {t("agent_error_title")}</div>
      <div class="text-xs">
        {error.code === "session_expired" ? t("agent_session_expired") : error.message}
      </div>
      {#if canRetry && !busy}
        <button
          type="button"
          onclick={onRetry}
          class="px-2.5 py-1 rounded-lg bg-white border border-rose-300 text-rose-700 text-xs font-bold hover:bg-rose-100 transition"
        >
          🔁 {t("retry_run")}
        </button>
      {/if}
    </div>
  {/if}

  {#if finish}
    <div class="border-t pt-3 space-y-1">
      <div class="text-xs font-bold {finish.ok ? 'text-emerald-700' : 'text-slate-700'}">
        {finish.ok ? "✅" : "⚠️"} {stopLabel(finish.stop)}
      </div>
      {#if statsLine}
        <div class="text-xs text-slate-500 font-medium">
          <bdi>{statsLine}</bdi>
        </div>
      {/if}
      {#if costLine}
        <div class="text-xs text-slate-500 font-medium flex items-center gap-1.5 flex-wrap">
          <bdi>{costLine}</bdi>
          <button
            type="button"
            onclick={() => (showCostNote = !showCostNote)}
            title={t("cost_note")}
            aria-label={t("cost_note_toggle")}
            aria-expanded={showCostNote}
            class="w-4 h-4 shrink-0 rounded-full border border-slate-300 text-slate-500 text-xs leading-none hover:bg-slate-100 focus-visible:outline-2 focus-visible:outline-blue-600"
          >
            ?
          </button>
        </div>
        {#if showCostNote}
          <p class="text-xs text-slate-500 leading-relaxed bg-slate-50 border border-slate-200 rounded-lg px-2.5 py-1.5">
            {t("cost_note")}
          </p>
        {/if}
      {/if}
    </div>
  {/if}
</div>
