/**
 * Helpers shared by the line-editor workspace (tree, inspector, change log,
 * attachments, presets). Pure functions only — no Svelte state lives here.
 */

/**
 * Turn an unknown rejection value into a readable string.
 * @param {unknown} e
 * @returns {string}
 */
export function errorText(e) {
  if (e === null || e === undefined) return "";
  if (e instanceof Error) return e.message;
  if (typeof e === "string") return e;
  try {
    return JSON.stringify(e);
  } catch (_) {
    return String(e);
  }
}

/**
 * A Tauri command that this build does not register rejects with a
 * "not found / not allowed" style message. That is not a real failure — the
 * feature simply is not in this binary — so the UI degrades instead of
 * shouting. Mirrors the check `undoAction` in +page.svelte already uses.
 * @param {unknown} e
 */
export function isMissingCommand(e) {
  const text = errorText(e);
  return /not (found|allowed)|unknown command|command .* not|Command .* not found/i.test(
    text
  );
}

/**
 * The Yemot session died (token expired / MFA needed again). Rust renders these
 * through `yemot::render_error`, so the marker travels inside the error string
 * exactly like the `session_expired` code an agent run reports.
 * @param {unknown} e
 */
export function isSessionExpired(e) {
  const text = errorText(e);
  return /session_expired|session expired|פג תוקף|החיבור לימות|התחבר מחדש|אימות דו-שלבי/i.test(
    text
  );
}

/**
 * Audio extensions the Rust side accepts for `upload_audio_file` (contract R2).
 * Keep in sync with the dialog filter below.
 */
export const AUDIO_EXTENSIONS = ["wav", "mp3", "m4a", "ogg", "wma", "aac"];

/** @type {Record<string, string>} */
const AUDIO_MIME = {
  wav: "audio/wav",
  mp3: "audio/mpeg",
  m4a: "audio/mp4",
  ogg: "audio/ogg",
  wma: "audio/x-ms-wma",
  aac: "audio/aac"
};

/**
 * The MIME type implied by a file name's extension, or "" when unknown. The
 * Rust side validates it again — this is a hint, not a security boundary.
 * @param {string} name
 */
export function mimeFromName(name) {
  const ext = (name.split(".").pop() ?? "").toLowerCase();
  return AUDIO_MIME[ext] ?? "";
}

/**
 * The last path segment of a native file path (Windows or POSIX separators).
 * @param {string} p
 */
export function baseName(p) {
  const parts = String(p ?? "").split(/[\\/]/);
  return parts[parts.length - 1] || String(p ?? "");
}

/**
 * Human-readable byte size. `null`/`undefined`/0-with-unknown renders as "".
 * @param {number | null | undefined} bytes
 */
export function formatSize(bytes) {
  if (bytes === null || bytes === undefined || !Number.isFinite(bytes)) return "";
  if (bytes < 1024) return `${bytes} B`;
  const kb = bytes / 1024;
  if (kb < 1024) return `${kb.toFixed(kb < 10 ? 1 : 0)} KB`;
  return `${(kb / 1024).toFixed(1)} MB`;
}

/**
 * Local HH:MM for a millisecond timestamp.
 * @param {number} ms
 */
export function formatTime(ms) {
  if (!Number.isFinite(ms) || ms <= 0) return "";
  const d = new Date(ms);
  const hh = String(d.getHours()).padStart(2, "0");
  const mm = String(d.getMinutes()).padStart(2, "0");
  return `${hh}:${mm}`;
}

/**
 * Change-log timestamp: "HH:MM" for something that happened today, and
 * "DD/MM HH:MM" for anything older — the log spans runs from earlier days, and
 * a bare "14:32" there reads as if the change had just been made.
 * @param {number} ms
 * @param {Date} [now] injected in tests
 */
export function formatLogTime(ms, now = new Date()) {
  const time = formatTime(ms);
  if (!time) return "";
  const d = new Date(ms);
  const sameDay =
    d.getFullYear() === now.getFullYear() &&
    d.getMonth() === now.getMonth() &&
    d.getDate() === now.getDate();
  if (sameDay) return time;
  const dd = String(d.getDate()).padStart(2, "0");
  const mo = String(d.getMonth() + 1).padStart(2, "0");
  return `${dd}/${mo} ${time}`;
}

/** Icon per `ExtensionDetail.files[].kind` (contract R1). */
/** @type {Record<string, string>} */
export const FILE_KIND_ICON = {
  audio: "🔊",
  text: "📄",
  ini: "⚙️",
  other: "📦"
};

// ---------------------------------------------------------------------------
//  Presets — user-editable, persisted under `ai_yemot_presets_v1`
// ---------------------------------------------------------------------------

export const PRESETS_KEY = "ai_yemot_presets_v1";

/**
 * The four presets the app shipped with. `text` is an i18n key resolved at
 * seed time, so the seeded copy is plain text the user can edit afterwards.
 * @type {{id: string, titleKey: string, textKey: string}[]}
 */
export const DEFAULT_PRESETS = [
  { id: "p_menu", titleKey: "preset_menu", textKey: "preset_menu_prompt" },
  { id: "p_play", titleKey: "preset_play", textKey: "preset_play_prompt" },
  { id: "p_record", titleKey: "preset_record", textKey: "preset_record_prompt" },
  { id: "p_human", titleKey: "preset_human", textKey: "preset_human_prompt" }
];

/**
 * Seed the default presets in the current language.
 * @param {(key: string) => string} translate
 * @returns {{id: string, title: string, text: string}[]}
 */
export function seedPresets(translate) {
  return DEFAULT_PRESETS.map((p) => ({
    id: p.id,
    title: translate(p.titleKey),
    text: translate(p.textKey)
  }));
}

/**
 * Read the stored presets, seeding (and persisting) the defaults on first run.
 * A corrupt or non-array value is replaced rather than crashing the page.
 *
 * Ids are de-duplicated on the way out: `PresetBar` keys its `{#each}` by id, so
 * a stored list that repeats one (hand-edited storage, or a restore-defaults
 * that landed next to an untouched seeded copy) would otherwise crash the page
 * with `each_key_duplicate` instead of merely looking odd.
 * @param {(key: string) => string} translate
 * @returns {{id: string, title: string, text: string}[]}
 */
export function loadPresets(translate) {
  try {
    const raw = localStorage.getItem(PRESETS_KEY);
    if (raw) {
      const parsed = JSON.parse(raw);
      if (Array.isArray(parsed)) {
        const seen = new Set();
        return parsed
          .filter((p) => p && typeof p === "object")
          .map((p, i) => {
            let id = String(p.id ?? `p_${i}`);
            while (seen.has(id)) id = `${id}_${i}`;
            seen.add(id);
            return {
              id,
              title: String(p.title ?? ""),
              text: String(p.text ?? "")
            };
          });
      }
    }
  } catch (_) {}
  const seeded = seedPresets(translate);
  savePresets(seeded);
  return seeded;
}

/** @param {{id: string, title: string, text: string}[]} presets */
export function savePresets(presets) {
  try {
    localStorage.setItem(PRESETS_KEY, JSON.stringify(presets));
  } catch (_) {}
}

/** A short unique id for a new preset. */
export function newPresetId() {
  return `p_${Date.now().toString(36)}${Math.floor(Math.random() * 1e4).toString(36)}`;
}
