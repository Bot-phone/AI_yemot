<script>
  /**
   * Editable "ready-made tasks". The list itself lives in +page.svelte (which
   * persists it under `ai_yemot_presets_v1`); this component owns only the
   * inline editing UI and reports the new list back through `onChange`.
   *
   * @typedef {{id: string, title: string, text: string}} Preset
   */
  import { t } from "$lib/i18n.svelte.js";
  import { newPresetId } from "$lib/lineEditor.js";

  let {
    /** @type {Preset[]} */
    presets = [],
    /** @type {(text: string) => void} */
    onUse,
    /** @type {(presets: Preset[]) => void} */
    onChange,
    /** @type {() => void} */
    onRestoreDefaults
  } = $props();

  /** Edit mode reveals the edit/delete controls on every chip. */
  let editing = $state(false);
  /** id of the preset open in the inline editor, "" when none. */
  let editingId = $state("");
  let draftTitle = $state("");
  let draftText = $state("");

  /** @param {Preset} p */
  function startEdit(p) {
    editingId = p.id;
    draftTitle = p.title;
    draftText = p.text;
  }

  function startAdd() {
    editing = true;
    editingId = "__new__";
    draftTitle = "";
    draftText = "";
  }

  function cancelEdit() {
    editingId = "";
    draftTitle = "";
    draftText = "";
  }

  function saveEdit() {
    const title = draftTitle.trim();
    const text = draftText.trim();
    if (!title && !text) {
      cancelEdit();
      return;
    }
    if (editingId === "__new__") {
      onChange([...presets, { id: newPresetId(), title: title || text.slice(0, 24), text }]);
    } else {
      onChange(
        presets.map((p) =>
          p.id === editingId ? { ...p, title: title || p.title, text } : p
        )
      );
    }
    cancelEdit();
  }

  /** @param {string} id */
  function remove(id) {
    onChange(presets.filter((p) => p.id !== id));
    if (editingId === id) cancelEdit();
  }
</script>

<div class="space-y-2">
  <div class="flex flex-wrap items-center gap-2">
    <span class="text-xs text-slate-500">{t("quick_presets")}</span>

    {#each presets as p (p.id)}
      <span class="inline-flex items-center rounded-lg bg-slate-100 overflow-hidden">
        <button
          type="button"
          onclick={() => onUse(p.text)}
          title={p.text}
          class="text-xs text-slate-700 px-2.5 py-1 hover:bg-slate-200 transition focus-visible:outline-2 focus-visible:outline-offset-[-2px] focus-visible:outline-blue-600"
        >
          {p.title}
        </button>
        {#if editing}
          <button
            type="button"
            onclick={() => startEdit(p)}
            aria-label={`${t("preset_edit_btn")}: ${p.title}`}
            title={t("preset_edit_btn")}
            class="text-xs text-slate-500 hover:text-blue-700 px-1.5 py-1 focus-visible:outline-2 focus-visible:outline-offset-[-2px] focus-visible:outline-blue-600"
          >
            ✎
          </button>
          <button
            type="button"
            onclick={() => remove(p.id)}
            aria-label={`${t("preset_delete")}: ${p.title}`}
            title={t("preset_delete")}
            class="text-xs text-slate-500 hover:text-rose-700 px-1.5 py-1 focus-visible:outline-2 focus-visible:outline-offset-[-2px] focus-visible:outline-blue-600"
          >
            🗑
          </button>
        {/if}
      </span>
    {/each}

    <button
      type="button"
      onclick={() => {
        editing = !editing;
        if (!editing) cancelEdit();
      }}
      aria-pressed={editing}
      class="text-xs px-2 py-1 rounded-lg border border-slate-300 text-slate-600 hover:bg-slate-100 transition focus-visible:outline-2 focus-visible:outline-blue-600"
    >
      {editing ? t("preset_done") : t("presets_edit")}
    </button>

    {#if editing}
      <button
        type="button"
        onclick={startAdd}
        class="text-xs px-2 py-1 rounded-lg border border-blue-300 text-blue-700 hover:bg-blue-50 transition focus-visible:outline-2 focus-visible:outline-blue-600"
      >
        + {t("preset_add")}
      </button>
      <button
        type="button"
        onclick={onRestoreDefaults}
        class="text-xs px-2 py-1 rounded-lg border border-slate-300 text-slate-600 hover:bg-slate-100 transition focus-visible:outline-2 focus-visible:outline-blue-600"
      >
        {t("preset_restore")}
      </button>
    {/if}
  </div>

  {#if editingId}
    <div class="rounded-xl border border-slate-200 bg-slate-50 p-3 space-y-2">
      <input
        type="text"
        bind:value={draftTitle}
        placeholder={t("preset_title_placeholder")}
        aria-label={t("preset_title_placeholder")}
        class="w-full text-xs rounded-lg border border-slate-300 p-2 focus:ring-2 focus:ring-blue-500 focus:outline-none"
      />
      <textarea
        rows="3"
        bind:value={draftText}
        placeholder={t("preset_text_placeholder")}
        aria-label={t("preset_text_placeholder")}
        class="w-full text-xs rounded-lg border border-slate-300 p-2 focus:ring-2 focus:ring-blue-500 focus:outline-none resize-y"
      ></textarea>
      <div class="flex items-center gap-2">
        <button
          type="button"
          onclick={saveEdit}
          class="px-3 py-1 rounded-lg bg-blue-600 hover:bg-blue-700 text-white text-xs font-bold transition focus-visible:outline-2 focus-visible:outline-blue-600"
        >
          {t("preset_save")}
        </button>
        <button
          type="button"
          onclick={cancelEdit}
          class="px-3 py-1 rounded-lg border border-slate-300 text-slate-700 text-xs font-medium hover:bg-white transition focus-visible:outline-2 focus-visible:outline-blue-600"
        >
          {t("preset_cancel")}
        </button>
      </div>
    </div>
  {/if}
</div>
