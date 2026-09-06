<script>
  /** Browser for the knowledge base embedded in the binary. */
  import { t } from "$lib/i18n.svelte.js";
  import { renderKnowledgeMarkdown, handleCopyPreClick } from "$lib/markdown.js";

  let {
    totalCount = 0,
    /** @type {any[]} */
    matches = [],
    searchQuery = $bindable(""),
    /** @type {string | null} */
    content = null,
    fileName = "",
    rawView = $bindable(false),
    copyFeedback = false,
    rtl = true,
    /** @type {() => void} */
    onClose,
    /** @type {() => void} */
    onSearchInput,
    /** @type {(name: string, heading: string | null) => void} */
    onOpenFile,
    /** @type {() => void} */
    onCopy,
    /** @type {(e: MouseEvent) => void} */
    onContentClick,
    /** @type {(el: HTMLElement | null) => void} */
    onContentElement
  } = $props();

  /** @type {HTMLElement | null} */
  let contentRef = $state(null);

  // The page scrolls this container when following an in-document anchor.
  $effect(() => {
    onContentElement?.(contentRef);
    return () => onContentElement?.(null);
  });

  let renderedHtml = $derived(renderKnowledgeMarkdown(content));

  /** @param {MouseEvent} e */
  function handleProseClick(e) {
    void handleCopyPreClick(e);
    onContentClick?.(e);
  }
</script>

<div class="fixed inset-0 bg-black/50 z-50 flex items-center justify-center p-4 backdrop-blur-sm">
  <div
    class="bg-white rounded-2xl max-w-4xl w-full max-h-[85vh] flex flex-col shadow-2xl overflow-hidden"
    role="dialog"
    aria-modal="true"
    aria-labelledby="knowledge-modal-title"
  >
    <div class="p-4 border-b flex items-center justify-between bg-slate-50">
      <div class="flex items-center gap-2">
        <span class="text-xl" aria-hidden="true">📚</span>
        <h3 id="knowledge-modal-title" class="font-bold text-sm text-slate-800">
          {t("knowledge_modal_title", { count: totalCount })}
        </h3>
      </div>
      <button
        type="button"
        onclick={onClose}
        aria-label={t("close")}
        title={t("close")}
        class="text-slate-500 hover:text-slate-800 text-lg font-bold"
      >
        ✕
      </button>
    </div>

    <div class="grid grid-cols-1 md:grid-cols-3 flex-1 overflow-hidden">
      <!-- File List -->
      <div class="p-3 {rtl ? 'border-l' : 'border-r'} border-slate-200 overflow-y-auto max-h-[70vh]">
        <!-- svelte-ignore a11y_autofocus -->
        <input
          type="text"
          autofocus
          bind:value={searchQuery}
          oninput={onSearchInput}
          aria-label={t("search_knowledge")}
          placeholder={t("search_knowledge")}
          class="w-full text-xs rounded-lg border border-slate-300 p-2 mb-2 focus:ring-2 focus:ring-blue-500 focus:outline-none"
        />
        <div class="space-y-1">
          {#each matches as file (file.name)}
            <button
              type="button"
              onclick={() => onOpenFile(file.name, file.heading || null)}
              class="w-full {rtl ? 'text-right' : 'text-left'} p-2 text-xs rounded-lg hover:bg-blue-50 hover:text-blue-700 transition {fileName === file.name ? 'bg-blue-100 text-blue-800' : 'text-slate-700'}"
            >
              <span class="flex items-center justify-between gap-2 {fileName === file.name ? 'font-bold' : ''}">
                <span class="truncate">{file.name.replace('.txt', '')}</span>
                <span class="text-xs text-slate-500 shrink-0">{(file.size / 1024).toFixed(1)}k</span>
              </span>
              {#if file.heading}
                <span class="block mt-0.5 truncate text-[11px] text-blue-700">› {file.heading}</span>
              {/if}
              {#if file.snippet}
                <span class="block mt-0.5 text-[11px] leading-snug text-slate-500 line-clamp-2 whitespace-pre-line">{file.snippet}</span>
              {/if}
            </button>
          {:else}
            <div class="p-2 text-xs text-slate-500">{t("no_knowledge_results")}</div>
          {/each}
        </div>
      </div>

      <!-- Content Viewer -->
      <div class="md:col-span-2 flex flex-col overflow-hidden bg-slate-50">
        {#if content}
          <div class="p-3 border-b bg-white flex items-center justify-between gap-2">
            <div class="flex items-center gap-2 overflow-hidden">
              <span class="text-base" aria-hidden="true">📄</span>
              <h4 class="text-xs font-bold text-slate-800 truncate" title={fileName}>
                {fileName.replace('.txt', '')}
              </h4>
            </div>

            <div class="flex items-center gap-2">
              <div class="flex bg-slate-100 p-0.5 rounded-lg text-xs">
                <button
                  type="button"
                  onclick={() => (rawView = false)}
                  class="px-2.5 py-1 rounded-md transition font-medium {!rawView ? 'bg-white text-blue-700 shadow-sm font-bold' : 'text-slate-600 hover:text-slate-800'}"
                >
                  {t("view_formatted")}
                </button>
                <button
                  type="button"
                  onclick={() => (rawView = true)}
                  class="px-2.5 py-1 rounded-md transition font-medium {rawView ? 'bg-white text-blue-700 shadow-sm font-bold' : 'text-slate-600 hover:text-slate-800'}"
                >
                  {t("view_raw")}
                </button>
              </div>

              <button
                type="button"
                onclick={onCopy}
                class="px-2.5 py-1 bg-slate-100 hover:bg-slate-200 text-slate-700 text-xs rounded-lg transition flex items-center gap-1 font-medium"
                title={t("copy_file")}
              >
                {#if copyFeedback}
                  <span class="text-emerald-700 font-bold">✓ {t("file_copied")}</span>
                {:else}
                  <span>📋 {t("copy_file")}</span>
                {/if}
              </button>
            </div>
          </div>

          <div
            bind:this={contentRef}
            class="flex-1 p-5 overflow-y-auto max-h-[64vh] bg-white scroll-smooth"
          >
            {#if rawView}
              <pre class="text-xs text-slate-700 font-sans whitespace-pre-wrap leading-relaxed">{content}</pre>
            {:else}
              <!-- svelte-ignore a11y_click_events_have_key_events -->
              <!-- svelte-ignore a11y_no_static_element_interactions -->
              <div class="knowledge-prose text-xs leading-relaxed" onclick={handleProseClick}>
                {@html renderedHtml}
              </div>
            {/if}
          </div>
        {:else}
          <div class="h-full flex flex-col items-center justify-center text-slate-500 text-xs p-6 gap-2">
            <span class="text-3xl" aria-hidden="true">📖</span>
            <span>{t("select_file_hint")}</span>
          </div>
        {/if}
      </div>
    </div>
  </div>
</div>
