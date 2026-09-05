<script>
  /**
   * Inspector for one extension — the read-only detail view next to the tree.
   * `detail` is an `ExtensionDetail` from the R1 contract:
   * `{ path, display, exists, ext_type, params, raw, size, mtime, files }`.
   */
  import { t } from "$lib/i18n.svelte.js";
  import { formatSize, FILE_KIND_ICON } from "$lib/lineEditor.js";

  let {
    /** @type {any} */
    detail = null,
    loading = false,
    error = "",
    /** The tree row the user picked, shown while the detail is still loading. */
    /** @type {string | null} */
    selectedDisplay = null,
    /** @type {() => void} */
    onRefresh,
    /** @type {() => void} */
    onUseAsContext,
    rtl = true
  } = $props();

  let showRaw = $state(false);
  /** The param key whose value was just copied, for the inline "✓". */
  let copiedKey = $state("");

  /**
   * @param {string} key
   * @param {string} value
   */
  async function copyValue(key, value) {
    try {
      await navigator.clipboard.writeText(value);
      copiedKey = key;
      setTimeout(() => {
        if (copiedKey === key) copiedKey = "";
      }, 1500);
    } catch (e) {
      console.error("Failed to copy value:", e);
    }
  }

  let params = $derived(Array.isArray(detail?.params) ? detail.params : []);
  let files = $derived(Array.isArray(detail?.files) ? detail.files : []);
</script>

<section
  class="bg-white rounded-2xl border border-slate-200 p-4 shadow-sm space-y-3"
  aria-labelledby="inspector-title"
>
  <div class="flex items-center justify-between gap-2 border-b pb-2 flex-wrap">
    <h2 id="inspector-title" class="text-sm font-bold text-slate-800">
      {t("inspector_title")}
      {#if detail?.display || selectedDisplay}
        <span class="font-mono text-xs text-slate-500">
          <bdi dir="ltr">{detail?.display ?? selectedDisplay}</bdi>
        </span>
      {/if}
    </h2>
    {#if detail || selectedDisplay}
      <div class="flex items-center gap-1.5">
        <button
          type="button"
          onclick={onUseAsContext}
          class="px-2.5 py-1 rounded-lg bg-blue-50 border border-blue-200 text-xs font-bold text-blue-700 hover:bg-blue-100 transition focus-visible:outline-2 focus-visible:outline-blue-600"
        >
          {t("inspector_task_on_ext")}
        </button>
        <button
          type="button"
          onclick={onRefresh}
          disabled={loading}
          class="px-2.5 py-1 rounded-lg border border-slate-300 text-xs font-medium text-slate-700 hover:bg-slate-100 transition disabled:opacity-40 focus-visible:outline-2 focus-visible:outline-blue-600"
        >
          {t("inspector_refresh")}
        </button>
      </div>
    {/if}
  </div>

  {#if loading}
    <div class="space-y-2 py-2" aria-live="polite">
      <p class="text-xs text-slate-500">{t("inspector_loading")}</p>
      <div class="h-2.5 rounded-full bg-slate-200 animate-pulse w-2/3"></div>
      <div class="h-2.5 rounded-full bg-slate-200 animate-pulse w-1/2"></div>
    </div>
  {:else if error}
    <p class="text-xs text-rose-700 bg-rose-50 border border-rose-200 rounded-lg px-2.5 py-2">
      {error}
    </p>
  {:else if !detail}
    <p class="text-xs text-slate-500 py-3">{t("inspector_empty")}</p>
  {:else}
    <dl class="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-xs">
      <dt class="font-semibold text-slate-600">{t("inspector_path")}</dt>
      <dd class="font-mono text-slate-800 break-all">
        <!-- The canonical path is what the tools take; the display path ("/2")
             is what the user sees on the line and in the tree, so the row shows
             both instead of only the internal spelling. -->
        <bdi dir="ltr">{detail.path}</bdi>
        {#if detail.display && detail.display !== detail.path}
          <span class="text-slate-500"><bdi dir="ltr">({detail.display})</bdi></span>
        {/if}
      </dd>
      <dt class="font-semibold text-slate-600">{t("inspector_type")}</dt>
      <dd class="text-slate-800">{detail.ext_type || "—"}</dd>
    </dl>

    {#if detail.exists === false}
      <p class="text-xs text-amber-800 bg-amber-50 border border-amber-200 rounded-lg px-2.5 py-1.5">
        {t("inspector_missing")}
      </p>
    {/if}

    <div>
      <h3 class="text-xs font-bold text-slate-700 mb-1">{t("inspector_params")}</h3>
      {#if params.length === 0}
        <p class="text-xs text-slate-500">{t("inspector_no_params")}</p>
      {:else}
        <div class="overflow-x-auto border border-slate-200 rounded-lg">
          <table class="w-full text-xs">
            <thead class="bg-slate-50 text-slate-500">
              <tr>
                <th class="p-1.5 {rtl ? 'text-right' : 'text-left'} font-semibold">
                  {t("diff_key")}
                </th>
                <!-- The inspector shows what the extension *is*, not what a
                     change would make it: "ערך", never "ערך חדש". -->
                <th class="p-1.5 {rtl ? 'text-right' : 'text-left'} font-semibold">
                  {t("value_label")}
                </th>
                <th class="p-1.5 w-8"><span class="sr-only">{t("copy_value")}</span></th>
              </tr>
            </thead>
            <tbody class="divide-y divide-slate-100">
              {#each params as p (p.key)}
                <tr>
                  <td class="p-1.5 font-mono font-bold text-slate-700 align-top">
                    <bdi dir="ltr">{p.key}</bdi>
                  </td>
                  <td class="p-1.5 font-mono text-slate-600 break-all align-top">
                    {#if p.value}
                      <bdi dir="ltr">{p.value}</bdi>
                    {:else}
                      <span class="text-slate-400">{t("value_empty")}</span>
                    {/if}
                  </td>
                  <td class="p-1.5 align-top">
                    <button
                      type="button"
                      onclick={() => copyValue(p.key, p.value ?? "")}
                      title={t("copy_value")}
                      aria-label={`${t("copy_value")}: ${p.key}`}
                      class="text-slate-400 hover:text-slate-700 rounded focus-visible:outline-2 focus-visible:outline-blue-600"
                    >
                      {copiedKey === p.key ? "✓" : "⧉"}
                    </button>
                  </td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
      {/if}
    </div>

    <div>
      <h3 class="text-xs font-bold text-slate-700 mb-1">{t("inspector_files")}</h3>
      {#if files.length === 0}
        <p class="text-xs text-slate-500">{t("inspector_no_files")}</p>
      {:else}
        <ul class="space-y-0.5 max-h-40 overflow-y-auto">
          {#each files as f (f.name)}
            <li class="flex items-center gap-2 text-xs text-slate-700">
              <span aria-hidden="true">{FILE_KIND_ICON[f.kind] ?? FILE_KIND_ICON.other}</span>
              <span class="font-mono truncate flex-1 min-w-0">
                <bdi dir="ltr">{f.name}</bdi>
              </span>
              {#if formatSize(f.size)}
                <span class="text-slate-500 shrink-0" dir="ltr">{formatSize(f.size)}</span>
              {/if}
            </li>
          {/each}
        </ul>
      {/if}
    </div>

    {#if detail.raw}
      <div>
        <button
          type="button"
          onclick={() => (showRaw = !showRaw)}
          aria-expanded={showRaw}
          class="text-xs text-blue-600 hover:underline rounded focus-visible:outline-2 focus-visible:outline-blue-600"
        >
          {showRaw ? `▲ ${t("inspector_hide_raw")}` : `▼ ${t("inspector_show_raw")}`}
        </button>
        {#if showRaw}
          <pre
            dir="ltr"
            class="mt-1.5 max-h-64 overflow-auto whitespace-pre-wrap break-all rounded-lg bg-slate-900 text-slate-100 p-3 font-mono text-xs">{detail.raw}</pre>
        {/if}
      </div>
    {/if}
  {/if}
</section>
