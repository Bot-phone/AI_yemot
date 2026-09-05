/**
 * Markdown rendering for everything that reaches `{@html}`.
 *
 * Every path here ends in DOMPurify: the model's answers and the embedded
 * knowledge files are untrusted input as far as the webview is concerned, and
 * the app runs under a real CSP (see src-tauri/tauri.conf.json).
 */

import { marked } from "marked";
import DOMPurify from "dompurify";
import { t } from "./i18n.svelte.js";

marked.setOptions({
  gfm: true,
  breaks: true
});

const SANITIZE_CONFIG = {
  ALLOWED_TAGS: [
    "a", "b", "blockquote", "br", "code", "del", "em", "h1", "h2", "h3", "h4",
    "h5", "h6", "hr", "i", "img", "li", "ol", "p", "pre", "s", "span", "strong",
    "sub", "sup", "table", "tbody", "td", "tfoot", "th", "thead", "tr", "ul",
    "bdi", "bdo", "div", "input"
  ],
  ALLOWED_ATTR: [
    "href", "title", "target", "rel", "name", "id", "dir", "align", "colspan",
    "rowspan", "src", "alt", "class", "type", "checked", "disabled", "start"
  ],
  ALLOW_DATA_ATTR: false
};

// Markdown links open in the system browser (openUrl), but a stray
// target="_blank" must never carry an opener reference.
if (typeof window !== "undefined") {
  DOMPurify.addHook("afterSanitizeAttributes", (node) => {
    if (node instanceof Element && node.tagName === "A" && node.hasAttribute("target")) {
      node.setAttribute("rel", "noopener noreferrer");
    }
  });
}

/**
 * @param {string} html
 * @returns {string}
 */
export function sanitizeHtml(html) {
  return DOMPurify.sanitize(html, SANITIZE_CONFIG);
}

/**
 * @param {any} value
 * @returns {string}
 */
export function escapeHtml(value) {
  return String(value ?? "")
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;");
}

// Technical tokens (ivr2 paths, /1/2 paths, file names, key=value) must stay LTR
// even inside a right-to-left sentence — wrap each one in <bdi dir="ltr">.
//
// The bare `/1/2` alternative needs boundaries on both sides, or it eats a slice
// out of the middle of ordinary text and splits it into separate bidi runs:
//   "24/7"            → the left lookbehind (a preceding \w) stops it
//   "05/09/2026"      → same: "05" precedes the first slash
//   "a.co/1/2"        → the preceding "." / ":" / "/" stops it
//   "/3/1x"           → the right lookahead stops it; `/` is in the lookahead
//                       too, so it cannot backtrack down to a bare "/3" either
//   "בשלוחה /3/1 יש"  → still matched, which is the whole point
export const TECHNICAL_TOKEN_RE =
  /(ivr2:\/[^\s,;]*|(?<![\w./:])\/\d+(?:\/\d+)*(?![\w/])|[A-Za-z0-9_.-]+\.(?:wav|txt|ini|mp3|json)|[A-Za-z_][A-Za-z0-9_]{1,}=[^\s,;]+)/g;

/**
 * Escape a technical string and isolate its LTR tokens for RTL layouts.
 * @param {any} text
 * @returns {string}
 */
export function ltrify(text) {
  return escapeHtml(text).replace(
    TECHNICAL_TOKEN_RE,
    (m) => `<bdi dir="ltr">${m}</bdi>`
  );
}

/**
 * Wrap bare link targets that contain spaces in <> so `marked` keeps them whole.
 * @param {string | null} content
 * @returns {string}
 */
export function preprocessMarkdown(content) {
  if (!content) return "";
  return content.replace(/\[([^\]]+)\]\(([^)\n]+)\)/g, (match, text, href) => {
    const trimmed = href.trim();
    if (trimmed.includes(" ") && !trimmed.startsWith("<") && !trimmed.endsWith(">")) {
      return `[${text}](<${trimmed}>)`;
    }
    return match;
  });
}

/**
 * Render a knowledge file: markdown → sanitized HTML.
 * @param {string | null} content
 * @returns {string}
 */
export function renderKnowledgeMarkdown(content) {
  if (!content) return "";
  try {
    return sanitizeHtml(/** @type {string} */ (marked.parse(preprocessMarkdown(content))));
  } catch (e) {
    console.error("Markdown parse error:", e);
    return escapeHtml(content);
  }
}

/**
 * Post-process sanitized markdown: isolate technical tokens inside text nodes so
 * `ivr2:/3/1` keeps reading left-to-right inside a Hebrew sentence, and add a
 * copy button to every code block.
 * @param {string} html
 * @returns {string}
 */
function enhanceAgentHtml(html) {
  if (typeof DOMParser === "undefined") return html;
  const doc = new DOMParser().parseFromString(`<div>${html}</div>`, "text/html");
  const root = doc.body.firstElementChild;
  if (!root) return html;

  // 1. bidi isolation of technical tokens in plain text (code/pre already carry
  //    `unicode-bidi: isolate` from the stylesheet).
  const walker = doc.createTreeWalker(root, NodeFilter.SHOW_TEXT);
  /** @type {Text[]} */
  const textNodes = [];
  while (walker.nextNode()) {
    textNodes.push(/** @type {Text} */ (walker.currentNode));
  }
  for (const node of textNodes) {
    const parent = node.parentElement;
    // `a` too: a link's text is already an isolated, LTR-ish unit and slicing it
    // into <bdi> runs breaks both its rendering and the click target.
    if (!parent || parent.closest("code, pre, bdi, a")) continue;
    const text = node.nodeValue ?? "";
    const matches = [...text.matchAll(TECHNICAL_TOKEN_RE)];
    if (matches.length === 0) continue;
    const frag = doc.createDocumentFragment();
    let cursor = 0;
    for (const m of matches) {
      const start = m.index ?? 0;
      if (start > cursor) frag.append(text.slice(cursor, start));
      const bdi = doc.createElement("bdi");
      bdi.setAttribute("dir", "ltr");
      bdi.textContent = m[0];
      frag.append(bdi);
      cursor = start + m[0].length;
    }
    if (cursor < text.length) frag.append(text.slice(cursor));
    node.replaceWith(frag);
  }

  // 2. copy button per code block (the markup is ours, added after sanitizing).
  for (const pre of Array.from(root.querySelectorAll("pre"))) {
    const wrap = doc.createElement("div");
    wrap.className = "md-pre-wrap";
    pre.replaceWith(wrap);
    wrap.append(pre);
    const btn = doc.createElement("button");
    btn.setAttribute("type", "button");
    btn.setAttribute("data-copy-pre", "");
    btn.className = "md-copy-btn";
    btn.textContent = t("copy_code");
    wrap.append(btn);
  }

  return root.innerHTML;
}

/**
 * Render an assistant text block: markdown → sanitized, bidi-isolated HTML.
 * @param {string} text
 * @returns {string}
 */
export function renderAgentMarkdown(text) {
  if (!text) return "";
  try {
    const html = sanitizeHtml(
      /** @type {string} */ (marked.parse(preprocessMarkdown(text)))
    );
    return enhanceAgentHtml(html);
  } catch (e) {
    console.error("Markdown parse error:", e);
    return escapeHtml(text);
  }
}

/**
 * Copy-button delegation for the code blocks injected by `enhanceAgentHtml`.
 * @param {MouseEvent} e
 */
export async function handleCopyPreClick(e) {
  const target = /** @type {HTMLElement} */ (e.target);
  const btn = target.closest("[data-copy-pre]");
  if (!btn) return;
  const pre = btn.parentElement?.querySelector("pre");
  if (!pre) return;
  try {
    await navigator.clipboard.writeText(pre.textContent ?? "");
    btn.textContent = t("copied");
    setTimeout(() => {
      btn.textContent = t("copy_code");
    }, 1600);
  } catch (err) {
    console.error("Failed to copy:", err);
  }
}
