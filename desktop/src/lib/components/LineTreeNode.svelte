<script>
  /**
   * One row of the line tree, plus its open children. Recursive: the component
   * imports itself (Svelte 5's replacement for `<svelte:self>`).
   *
   * `node` is an `ExtTreeNode` from the R1 contract:
   * `{ path, display, ext_type, title, children }`.
   */
  import { t } from "$lib/i18n.svelte.js";
  import Self from "./LineTreeNode.svelte";

  let {
    /** @type {any} */
    node,
    depth = 0,
    /** @type {string | null} */
    selectedPath = null,
    /** @type {Record<string, boolean>} */
    expanded = {},
    /** @type {(path: string) => void} */
    onSelect,
    /** @type {(path: string) => void} */
    onToggle
  } = $props();

  let children = $derived(Array.isArray(node?.children) ? node.children : []);
  let hasChildren = $derived(children.length > 0);
  let isOpen = $derived(expanded[node?.path] === true);
  let isSelected = $derived(selectedPath === node?.path);
</script>

<div class="flex items-stretch gap-0.5" style="padding-inline-start: {depth * 0.85}rem">
  {#if hasChildren}
    <!-- Decorative twisty: the row itself is the treeitem and carries the
         expanded state, and the tree's arrow keys already toggle it, so this
         button is kept out of the tab order and out of the a11y tree rather
         than announcing a second, competing control for the same row. -->
    <button
      type="button"
      onclick={() => onToggle(node.path)}
      tabindex="-1"
      aria-hidden="true"
      title={isOpen ? t("tree_collapse") : t("tree_expand")}
      class="w-5 shrink-0 text-xs text-slate-500 hover:text-slate-800 rounded focus-visible:outline-2 focus-visible:outline-blue-600"
    >
      <span class="inline-block tree-caret {isOpen ? 'tree-caret-open' : ''}">▸</span>
    </button>
  {:else}
    <span class="w-5 shrink-0" aria-hidden="true"></span>
  {/if}

  <button
    type="button"
    role="treeitem"
    data-tree-row="1"
    data-path={node.path}
    data-has-children={hasChildren ? "true" : "false"}
    onclick={() => onSelect(node.path)}
    aria-expanded={hasChildren ? isOpen : undefined}
    aria-selected={isSelected}
    aria-level={depth + 1}
    class="flex-1 min-w-0 text-start px-2 py-1 rounded-lg text-xs transition
      focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-blue-600
      {isSelected
        ? 'bg-blue-50 text-blue-900 font-bold border border-blue-200'
        : 'hover:bg-slate-100 text-slate-700 border border-transparent'}"
  >
    <span class="font-mono font-semibold"><bdi dir="ltr">{node.display}</bdi></span>
    {#if node.ext_type}
      <span class="mx-1 text-xs font-normal text-slate-500">· {node.ext_type}</span>
    {/if}
    {#if node.title}
      <span class="block truncate text-xs font-normal text-slate-500">{node.title}</span>
    {/if}
  </button>
</div>

{#if hasChildren && isOpen}
  <!-- `role="group"` is what makes the rows below belong to the row above them:
       without it every treeitem looks like a sibling at the top level. -->
  <div role="group">
    {#each children as child (child.path)}
      <Self node={child} depth={depth + 1} {selectedPath} {expanded} {onSelect} {onToggle} />
    {/each}
  </div>
{/if}

<style>
  .tree-caret {
    transition: transform 0.15s ease;
  }
  .tree-caret-open {
    transform: rotate(90deg);
  }
  @media (prefers-reduced-motion: reduce) {
    .tree-caret {
      transition: none;
    }
  }
</style>
