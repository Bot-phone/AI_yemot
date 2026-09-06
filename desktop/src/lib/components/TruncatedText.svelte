<script>
  /**
   * Long error text collapsed to its first characters with a "show more"
   * toggle. Presentation only — the full text stays in the DOM only once
   * expanded, so a multi-kilobyte provider error never floods the panel.
   */
  import { t } from "$lib/i18n.svelte.js";

  let {
    text = "",
    /** Characters kept visible while collapsed. */
    limit = 200
  } = $props();

  let expanded = $state(false);

  // A message just over the limit is shown whole: a toggle that reveals a
  // handful of characters is worse than the characters themselves.
  const isLong = $derived(text.length > limit + 40);
  const shown = $derived(!isLong || expanded ? text : text.slice(0, limit).trimEnd() + "…");

  // A new message starts collapsed again.
  $effect(() => {
    void text;
    expanded = false;
  });
</script>

<span class="break-words whitespace-pre-wrap">{shown}</span>
{#if isLong}
  <button
    type="button"
    class="ms-1 underline font-bold hover:no-underline"
    onclick={() => (expanded = !expanded)}
  >
    {expanded ? t("show_less") : t("show_more")}
  </button>
{/if}
