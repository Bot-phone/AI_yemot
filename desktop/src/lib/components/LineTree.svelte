<script>
  /**
   * "מבנה הקו" — the live extension tree of the user's line.
   *
   * Presentation only: the nodes, the loading/error state and the selection all
   * live in +page.svelte and arrive here as props. Nodes are `ExtTreeNode`
   * (contract R1): `{ path, display, ext_type, title, children }`.
   */
  import { t } from "$lib/i18n.svelte.js";
  import LineTreeNode from "./LineTreeNode.svelte";

  let {
    /** @type {any[]} */
    nodes = [],
    loading = false,
    /** @type {string} */
    error = "",
    /** Nothing to load until the user is signed in. */
    hasToken = false,
    /** @type {string | null} */
    selectedPath = null,
    /** @type {Record<string, boolean>} */
    expanded = {},
    /** @type {(path: string) => void} */
    onSelect,
    /** @type {(path: string) => void} */
    onToggle,
    /** @type {() => void} */
    onRefresh
  } = $props();

  let filter = $state("");

  /**
   * Depth-first flattening, used by the filter view.
   * @param {any[]} list
   * @param {any[]} out
   * @returns {any[]}
   */
  function flatten(list, out = []) {
    for (const n of list) {
      out.push(n);
      if (Array.isArray(n.children) && n.children.length > 0) flatten(n.children, out);
    }
    return out;
  }

  /** Filtered rows (flat), or `null` while no filter is typed. */
  let matches = $derived.by(() => {
    const q = filter.trim().toLowerCase();
    if (!q) return null;
    return flatten(nodes)
      .filter((n) =>
        [n.display, n.title, n.ext_type]
          .filter(Boolean)
          .some((v) => String(v).toLowerCase().includes(q))
      )
      .map((n) => ({ ...n, children: [] }));
  });

  /**
   * Roving focus: ArrowUp/ArrowDown walk the rendered rows; the arrow that
   * "opens" a branch toggles it. Enter/Space are the buttons' own behaviour,
   * so the tree is fully usable without arrow keys at all.
   * @param {KeyboardEvent} e
   */
  function handleTreeKeydown(e) {
    const key = e.key;
    if (key !== "ArrowDown" && key !== "ArrowUp" && key !== "ArrowRight" && key !== "ArrowLeft")
      return;
    const container = /** @type {HTMLElement} */ (e.currentTarget);
    const rows = /** @type {HTMLElement[]} */ ([
      ...container.querySelectorAll("[data-tree-row]")
    ]);
    const current = /** @type {HTMLElement | null} */ (document.activeElement);
    const idx = current ? rows.indexOf(current) : -1;
    if (idx < 0) return;

    if (key === "ArrowDown" || key === "ArrowUp") {
      e.preventDefault();
      rows[idx + (key === "ArrowDown" ? 1 : -1)]?.focus();
      return;
    }

    // Direction-independent: whichever horizontal arrow would change the state
    // toggles it, so the same keys work in an RTL and an LTR layout.
    const path = current?.getAttribute("data-path");
    if (!path) return;
    if (current?.getAttribute("data-has-children") !== "true") return;
    e.preventDefault();
    const isOpen = expanded[path] === true;
    if ((key === "ArrowLeft" && !isOpen) || (key === "ArrowRight" && isOpen)) {
      onToggle(path);
    }
  }
</script>

<section
  class="bg-white rounded-2xl border border-slate-200 p-4 shadow-sm space-y-3"
  aria-labelledby="line-tree-title"
>
  <div class="flex items-center justify-between gap-2 border-b pb-2">
    <h2 id="line-tree-title" class="text-sm font-bold text-slate-800">{t("tree_title")}</h2>
    <button
      type="button"
      onclick={onRefresh}
      disabled={loading || !hasToken}
      title={t("tree_refresh")}
      aria-label={t("tree_refresh")}
      class="px-2 py-1 rounded-lg border border-slate-300 text-xs font-medium text-slate-700 hover:bg-slate-100 transition disabled:opacity-40 focus-visible:outline-2 focus-visible:outline-blue-600"
    >
      <span class="inline-block {loading ? 'tree-spin' : ''}" aria-hidden="true">⟳</span>
    </button>
  </div>

  {#if hasToken && nodes.length > 0}
    <input
      type="search"
      bind:value={filter}
      placeholder={t("tree_search")}
      aria-label={t("tree_search")}
      class="w-full text-xs rounded-lg border border-slate-300 p-2 focus:ring-2 focus:ring-blue-500 focus:outline-none"
    />
  {/if}

  {#if !hasToken}
    <p class="text-xs text-slate-500 py-3">{t("tree_needs_token")}</p>
  {:else if loading && nodes.length === 0}
    <div class="space-y-2 py-2" aria-live="polite">
      <p class="text-xs text-slate-500">{t("tree_loading")}</p>
      <div class="h-2.5 rounded-full bg-slate-200 animate-pulse w-3/4"></div>
      <div class="h-2.5 rounded-full bg-slate-200 animate-pulse w-1/2"></div>
      <div class="h-2.5 rounded-full bg-slate-200 animate-pulse w-2/3"></div>
    </div>
  {:else if error}
    <p class="text-xs text-rose-700 bg-rose-50 border border-rose-200 rounded-lg px-2.5 py-2">
      {error}
    </p>
  {:else if nodes.length === 0}
    <p class="text-xs text-slate-500 py-3">{t("tree_empty")}</p>
  {:else}
    <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
    <div
      role="tree"
      aria-label={t("tree_title")}
      tabindex="-1"
      onkeydown={handleTreeKeydown}
      class="space-y-0.5 max-h-[26rem] overflow-y-auto"
    >
      {#if matches !== null}
        {#if matches.length === 0}
          <p class="text-xs text-slate-500 py-2">{t("tree_no_match")}</p>
        {:else}
          {#each matches as node (node.path)}
            <LineTreeNode
              {node}
              depth={0}
              {selectedPath}
              {expanded}
              {onSelect}
              {onToggle}
            />
          {/each}
        {/if}
      {:else}
        {#each nodes as node (node.path)}
          <LineTreeNode {node} depth={0} {selectedPath} {expanded} {onSelect} {onToggle} />
        {/each}
      {/if}
    </div>
  {/if}
</section>

<style>
  .tree-spin {
    animation: tree-rotate 0.9s linear infinite;
  }
  @keyframes tree-rotate {
    to {
      transform: rotate(360deg);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .tree-spin {
      animation: none;
    }
  }
</style>
