//! Read-only inspection of a live line, for the editor UI (מבנה הקו).
//!
//! Everything here reuses [`crate::yemot::YemotClient`]; nothing in this module
//! writes, and no call touches the approval / write state. The model-facing
//! renderers in `yemot.rs` stay as they are — the UI gets JSON instead.
//!
//! Errors are rendered with [`crate::yemot::render_error`], so an expired or
//! unverified session reaches the frontend as a string with the same
//! `SESSION_EXPIRED: ` code prefix the agent run reports as `session_expired`.

use std::collections::{BTreeMap, HashMap};

use serde::Serialize;

use crate::yemot::{
    canon_ext, display_path, natural_key, render_error, ExtNode, ExtParam, ExtRead, FileEntry,
    YemotClient, YemotError,
};

/// Hard cap on [`ExtensionDetail::raw`]. A line's `ext.ini` is a few KB at
/// most; anything past this is a pathological file, not something to render.
const MAX_RAW_BYTES: usize = 64 * 1024;
/// Appended to `raw` when it was cut at [`MAX_RAW_BYTES`].
const RAW_TRUNCATED_MARK: &str = "…[קוצץ]";

/// Audio files the phone system plays.
const AUDIO_EXTS: &[&str] = &["wav", "mp3", "m4a", "ogg", "wma", "aac"];
/// Files the user can read (and, for some of them, edit) as text.
const TEXT_EXTS: &[&str] = &["txt", "ini", "tts", "csv", "json", "api"];

// ---------------------------------------------------------------------------
// Wire shapes (serde field names are the contract with the UI)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ExtTreeNode {
    /// Canonical path (`ivr2:/1/2`).
    pub path: String,
    /// Display form (`/1/2`).
    pub display: String,
    pub ext_type: String,
    pub title: String,
    pub children: Vec<ExtTreeNode>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FileInfo {
    pub name: String,
    pub size: Option<u64>,
    pub mtime: Option<String>,
    /// `ini` | `audio` | `text` | `other`
    pub kind: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExtensionDetail {
    pub path: String,
    pub display: String,
    pub exists: bool,
    pub ext_type: Option<String>,
    /// `ext.ini` keys in file order; comments and blank lines are dropped.
    pub params: Vec<ExtParam>,
    /// The `ext.ini` text as read, capped at 64 KiB.
    pub raw: String,
    pub size: Option<u64>,
    pub mtime: Option<String>,
    pub files: Vec<FileInfo>,
}

// ---------------------------------------------------------------------------
// Pure helpers (unit-tested without a network)
// ---------------------------------------------------------------------------

/// `ext.ini` is its own kind; everything else is classified by extension.
fn file_kind(name: &str) -> &'static str {
    if name.eq_ignore_ascii_case("ext.ini") {
        return "ini";
    }
    let ext = name
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    if AUDIO_EXTS.contains(&ext.as_str()) {
        "audio"
    } else if TEXT_EXTS.contains(&ext.as_str()) {
        "text"
    } else {
        "other"
    }
}

/// Cut `text` to [`MAX_RAW_BYTES`] on a character boundary, marking the cut.
fn cap_raw(text: &str) -> String {
    if text.len() <= MAX_RAW_BYTES {
        return text.to_string();
    }
    let mut end = MAX_RAW_BYTES;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    let mut out = String::with_capacity(end + RAW_TRUNCATED_MARK.len());
    out.push_str(&text[..end]);
    out.push_str(RAW_TRUNCATED_MARK);
    out
}

/// Parent of a canonical extension path; `None` for the line root.
fn parent_path(canon: &str) -> Option<String> {
    let body = canon.strip_prefix("ivr2:/")?;
    if body.is_empty() {
        return None;
    }
    body.rsplit_once('/')
        .map(|(head, _)| format!("ivr2:/{}", head))
}

fn sort_paths(paths: &mut [String]) {
    paths.sort_by_key(|p| natural_key(p));
}

/// Nest a flat [`ExtNode`] list into a tree.
///
/// A node whose direct parent is missing from the list is attached to its
/// nearest *present* ancestor, and to the top level when it has none — the
/// listing is a snapshot of what the server returned, and dropping such a node
/// would hide a real extension from the user.
pub fn build_tree(nodes: &[ExtNode]) -> Vec<ExtTreeNode> {
    let mut by_path: BTreeMap<String, ExtNode> = BTreeMap::new();
    for n in nodes {
        by_path.entry(n.path.clone()).or_insert_with(|| n.clone());
    }

    let mut kids: HashMap<String, Vec<String>> = HashMap::new();
    let mut roots: Vec<String> = Vec::new();
    for path in by_path.keys() {
        // Ancestors are strictly shorter than the node itself, so this walk
        // terminates and the result can never contain a cycle.
        let mut ancestor = parent_path(path);
        let mut attached = false;
        while let Some(p) = ancestor {
            if by_path.contains_key(&p) {
                kids.entry(p).or_default().push(path.clone());
                attached = true;
                break;
            }
            ancestor = parent_path(&p);
        }
        if !attached {
            roots.push(path.clone());
        }
    }

    sort_paths(&mut roots);
    roots
        .iter()
        .map(|p| build_node(p, &by_path, &kids))
        .collect()
}

fn build_node(
    path: &str,
    by_path: &BTreeMap<String, ExtNode>,
    kids: &HashMap<String, Vec<String>>,
) -> ExtTreeNode {
    let node = &by_path[path];
    let mut children = kids.get(path).cloned().unwrap_or_default();
    sort_paths(&mut children);
    ExtTreeNode {
        path: node.path.clone(),
        display: display_path(&node.path),
        ext_type: node.ext_type.clone(),
        title: node.title.clone(),
        children: children
            .iter()
            .map(|c| build_node(c, by_path, kids))
            .collect(),
    }
}

fn detail_from(canon: &str, read: &ExtRead, files: &[FileEntry]) -> ExtensionDetail {
    ExtensionDetail {
        path: canon.to_string(),
        display: display_path(canon),
        exists: read.exists,
        ext_type: read
            .ini
            .get("type")
            .map(|t| t.trim())
            .filter(|t| !t.is_empty())
            .map(|t| t.to_string()),
        params: read
            .ini
            .pairs()
            .into_iter()
            .map(|(key, value)| ExtParam {
                key: key.to_string(),
                value: value.to_string(),
            })
            .collect(),
        raw: if read.exists {
            cap_raw(&read.ini.to_text())
        } else {
            String::new()
        },
        size: read.size,
        mtime: read.mtime.clone(),
        files: files
            .iter()
            .map(|f| FileInfo {
                name: f.name.clone(),
                size: f.size,
                mtime: f.mtime.clone(),
                kind: file_kind(&f.name).to_string(),
            })
            .collect(),
    }
}

/// No token, no request: an unauthenticated read fails closed, and it fails
/// with the session code the UI already knows how to react to.
fn require_client(token: &str) -> Result<YemotClient, String> {
    let token = token.trim();
    if token.is_empty() {
        return Err(render_error(&YemotError::SessionExpired(
            "אין חיבור פעיל לימות המשיח.".to_string(),
        )));
    }
    Ok(YemotClient::new(token))
}

// ---------------------------------------------------------------------------
// Tauri commands (read-only)
// ---------------------------------------------------------------------------

/// The extension tree under `root` (`""` / `"/"` = the whole line), nested.
/// `depth` is clamped to 1..=2, which is everything `list_extensions` can
/// deliver: it fetches the children of `root` and, at `depth >= 2`, one further
/// level. A 3 or a 4 promised a depth the client never expanded.
#[tauri::command]
pub async fn get_extension_tree(
    token: String,
    root: String,
    depth: u8,
) -> Result<Vec<ExtTreeNode>, String> {
    let client = require_client(&token)?;
    let root = canon_ext(&root).map_err(|e| render_error(&e))?;
    let nodes = client
        .list_extensions(&root, depth.clamp(1, 2))
        .await
        .map_err(|e| render_error(&e))?;
    Ok(build_tree(&nodes))
}

/// One extension: its `ext.ini` (parsed + raw) and the files in its folder.
#[tauri::command]
pub async fn read_extension(token: String, path: String) -> Result<ExtensionDetail, String> {
    let client = require_client(&token)?;
    let canon = canon_ext(&path).map_err(|e| render_error(&e))?;
    let read = client
        .get_ext_ini_fresh(&canon)
        .await
        .map_err(|e| render_error(&e))?;
    let files = client
        .list_files(&canon)
        .await
        .map_err(|e| render_error(&e))?;
    Ok(detail_from(&canon, &read, &files))
}

// ---------------------------------------------------------------------------
// Tests (no network)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::yemot::ExtSource;
    use crate::yemot_ini::ExtIni;

    fn node(path: &str) -> ExtNode {
        ExtNode {
            path: path.to_string(),
            ext_type: "menu".to_string(),
            title: format!("t{}", path),
        }
    }

    #[test]
    fn tree_nests_a_flat_list_by_path() {
        let flat = vec![node("ivr2:/2"), node("ivr2:/1/1"), node("ivr2:/1"), node("ivr2:/1/2")];
        let tree = build_tree(&flat);
        assert_eq!(tree.len(), 2);
        assert_eq!(tree[0].path, "ivr2:/1");
        assert_eq!(tree[0].display, "/1");
        assert_eq!(tree[1].path, "ivr2:/2");
        let kids: Vec<&str> = tree[0].children.iter().map(|c| c.path.as_str()).collect();
        assert_eq!(kids, vec!["ivr2:/1/1", "ivr2:/1/2"]);
        assert!(tree[0].children[0].children.is_empty());
        // duplicates in the flat list collapse into one node
        let dup = build_tree(&[node("ivr2:/1"), node("ivr2:/1")]);
        assert_eq!(dup.len(), 1);
        assert!(build_tree(&[]).is_empty());
    }

    #[test]
    fn tree_children_sort_numerically_not_lexically() {
        let flat = vec![node("ivr2:/10"), node("ivr2:/2"), node("ivr2:/1")];
        let tree = build_tree(&flat);
        let order: Vec<&str> = tree.iter().map(|n| n.display.as_str()).collect();
        assert_eq!(order, vec!["/1", "/2", "/10"]);
    }

    #[test]
    fn orphan_attaches_to_the_nearest_existing_ancestor_else_root() {
        // /1/2 is missing: /1/2/3 must land under /1, not disappear.
        let tree = build_tree(&[node("ivr2:/1"), node("ivr2:/1/2/3")]);
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].path, "ivr2:/1");
        assert_eq!(tree[0].children.len(), 1);
        assert_eq!(tree[0].children[0].path, "ivr2:/1/2/3");

        // no ancestor at all: it becomes a top-level node.
        let tree = build_tree(&[node("ivr2:/4/5")]);
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].path, "ivr2:/4/5");
        assert!(tree[0].children.is_empty());
    }

    #[test]
    fn file_kind_classification() {
        assert_eq!(file_kind("ext.ini"), "ini");
        assert_eq!(file_kind("EXT.INI"), "ini");
        for audio in ["000.wav", "a.MP3", "b.m4a", "c.ogg", "d.wma", "e.aac"] {
            assert_eq!(file_kind(audio), "audio", "{}", audio);
        }
        for text in ["a.txt", "other.ini", "1.tts", "x.csv", "y.json", "z.api"] {
            assert_eq!(file_kind(text), "text", "{}", text);
        }
        for other in ["a.zip", "noext", "b.pdf", "c."] {
            assert_eq!(file_kind(other), "other", "{}", other);
        }
    }

    #[test]
    fn raw_is_capped_with_a_marker_on_a_char_boundary() {
        let small = ExtRead {
            exists: true,
            ini: ExtIni::parse("type=menu\ntitle=בדיקה\n"),
            size: Some(30),
            mtime: Some("01/01/2026 10:00".to_string()),
            source: ExtSource::File,
        };
        let d = detail_from("ivr2:/1/2", &small, &[]);
        assert_eq!(d.display, "/1/2");
        assert!(d.exists);
        assert_eq!(d.ext_type.as_deref(), Some("menu"));
        assert_eq!(d.params.len(), 2);
        assert_eq!(d.params[0].key, "type");
        assert!(!d.raw.contains(RAW_TRUNCATED_MARK));

        // 2-byte Hebrew values guarantee the cut lands mid-character if the
        // cap is applied naively.
        let long = ExtRead {
            exists: true,
            ini: ExtIni::parse(&"title=אאאאאאאאאא\n".repeat(4000)),
            size: None,
            mtime: None,
            source: ExtSource::File,
        };
        let d = detail_from("ivr2:/1", &long, &[]);
        assert!(d.raw.ends_with(RAW_TRUNCATED_MARK));
        assert!(d.raw.len() <= MAX_RAW_BYTES + RAW_TRUNCATED_MARK.len());
        assert!(d.raw.chars().count() > 0);

        let missing = ExtRead {
            exists: false,
            ini: ExtIni::default(),
            size: None,
            mtime: None,
            source: ExtSource::File,
        };
        let d = detail_from("ivr2:/9", &missing, &[FileEntry {
            name: "000.wav".to_string(),
            size: Some(12),
            mtime: None,
        }]);
        assert!(!d.exists);
        assert_eq!(d.raw, "");
        assert_eq!(d.ext_type, None);
        assert_eq!(d.files[0].kind, "audio");
    }

    #[tokio::test]
    async fn an_empty_token_fails_closed_before_any_request() {
        for token in ["", "   "] {
            let e = get_extension_tree(token.to_string(), "/".to_string(), 2)
                .await
                .unwrap_err();
            assert!(e.starts_with("SESSION_EXPIRED: "), "{}", e);
            let e = read_extension(token.to_string(), "/1".to_string())
                .await
                .unwrap_err();
            assert!(e.starts_with("SESSION_EXPIRED: "), "{}", e);
        }
    }

    #[tokio::test]
    async fn a_bad_path_is_rejected_before_any_request() {
        let e = read_extension("t".to_string(), "/1/a".to_string())
            .await
            .unwrap_err();
        assert!(e.starts_with("BAD_REQUEST: "), "{}", e);
    }
}
