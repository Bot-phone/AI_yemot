//! Hand-rolled, loss-less parser for Yemot `ext.ini` files.
//!
//! The format is a *flat* ini (no sections): `key=value` lines, `;` / `#`
//! comments and blank lines. Everything is preserved so a parsed file can be
//! rendered back (modulo line endings, which are normalised to `\n`).
#![allow(dead_code)]


/// A single physical line of an ini file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IniLine {
    /// `key=value` — `raw` is the original line (without its line terminator).
    Pair {
        key: String,
        value: String,
        raw: String,
    },
    /// A comment line (`;` or `#`), or any non-empty line without `=`.
    Comment(String),
    /// An empty / whitespace-only line.
    Blank,
}

/// A parsed `ext.ini` file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExtIni {
    pub lines: Vec<IniLine>,
}

/// How a single incoming parameter compares to the current file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffKind {
    New,
    Changed,
    Unchanged,
}

/// One row of a merge preview (what `UpdateExtension` would do).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffRow {
    pub key: String,
    pub before: Option<String>,
    pub after: String,
    pub kind: DiffKind,
}

impl ExtIni {
    /// Parse ini text. Never fails: anything unrecognised is kept as a comment.
    pub fn parse(input: &str) -> ExtIni {
        // A UTF-8 BOM would glue itself to the first key ("\u{feff}type"), so
        // every lookup of that key would silently miss. Drop it on the way in;
        // `to_text` therefore never writes one back.
        let input = input.strip_prefix('\u{feff}').unwrap_or(input);
        let mut lines = Vec::new();
        for raw_line in input.split('\n') {
            // Tolerate CRLF: the `\r` belongs to the terminator, not the value.
            let raw = raw_line.strip_suffix('\r').unwrap_or(raw_line);
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                lines.push(IniLine::Blank);
                continue;
            }
            if trimmed.starts_with(';') || trimmed.starts_with('#') {
                lines.push(IniLine::Comment(raw.to_string()));
                continue;
            }
            // Split on the FIRST '=' only — values may contain '='.
            match raw.find('=') {
                Some(idx) => {
                    let key = raw[..idx].trim().to_string();
                    let value = raw[idx + 1..].trim().to_string();
                    if key.is_empty() {
                        lines.push(IniLine::Comment(raw.to_string()));
                    } else {
                        lines.push(IniLine::Pair {
                            key,
                            value,
                            raw: raw.to_string(),
                        });
                    }
                }
                None => lines.push(IniLine::Comment(raw.to_string())),
            }
        }
        // "a\n" splits into ["a", ""] — drop that phantom trailing blank.
        if input.ends_with('\n') {
            lines.pop();
        }
        ExtIni { lines }
    }

    /// Value of the first occurrence of `key` (first wins).
    pub fn get(&self, key: &str) -> Option<&str> {
        self.lines.iter().find_map(|l| match l {
            IniLine::Pair { key: k, value, .. } if k == key => Some(value.as_str()),
            _ => None,
        })
    }

    /// All values for `key`, in file order.
    pub fn get_all(&self, key: &str) -> Vec<&str> {
        self.lines
            .iter()
            .filter_map(|l| match l {
                IniLine::Pair { key: k, value, .. } if k == key => Some(value.as_str()),
                _ => None,
            })
            .collect()
    }

    /// Every pair, in file order (duplicates included).
    pub fn pairs(&self) -> Vec<(&str, &str)> {
        self.lines
            .iter()
            .filter_map(|l| match l {
                IniLine::Pair { key, value, .. } => Some((key.as_str(), value.as_str())),
                _ => None,
            })
            .collect()
    }

    /// Number of `key=value` lines.
    pub fn key_count(&self) -> usize {
        self.lines
            .iter()
            .filter(|l| matches!(l, IniLine::Pair { .. }))
            .count()
    }

    /// Compact, model-facing rendering: `key=value` lines only, in order,
    /// truncated on a line boundary so the result stays within `max_bytes`.
    /// Returns the text and whether anything was dropped.
    pub fn to_compact(&self, max_bytes: usize) -> (String, bool) {
        let mut out = String::new();
        let mut truncated = false;
        for (key, value) in self.pairs() {
            let line = format!("{}={}\n", key, value);
            if out.len() + line.len() > max_bytes {
                truncated = true;
                break;
            }
            out.push_str(&line);
        }
        (out, truncated)
    }

    /// What would change if `params` were sent to `UpdateExtension`.
    pub fn preview_merge(&self, params: &[(String, String)]) -> Vec<DiffRow> {
        params
            .iter()
            .map(|(key, after)| {
                let before = self.get(key).map(|s| s.to_string());
                let kind = match &before {
                    None => DiffKind::New,
                    Some(b) if b == after => DiffKind::Unchanged,
                    Some(_) => DiffKind::Changed,
                };
                DiffRow {
                    key: key.clone(),
                    before,
                    after: after.clone(),
                    kind,
                }
            })
            .collect()
    }

    /// Render back to text (line endings normalised to `\n`).
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        for (i, l) in self.lines.iter().enumerate() {
            if i > 0 {
                out.push('\n');
            }
            match l {
                IniLine::Pair { raw, .. } => out.push_str(raw),
                IniLine::Comment(raw) => out.push_str(raw),
                IniLine::Blank => {}
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(k: &str, v: &str) -> (String, String) {
        (k.to_string(), v.to_string())
    }

    #[test]
    fn parses_pairs_comments_and_blanks() {
        let ini = ExtIni::parse("type=menu\n\n; a comment\n# another\ntitle=hello\n");
        assert_eq!(ini.lines.len(), 5);
        assert_eq!(ini.get("type"), Some("menu"));
        assert_eq!(ini.get("title"), Some("hello"));
        assert_eq!(ini.key_count(), 2);
        assert!(matches!(ini.lines[1], IniLine::Blank));
        assert!(matches!(ini.lines[2], IniLine::Comment(_)));
    }

    #[test]
    fn duplicate_keys_first_wins_and_get_all_returns_both() {
        let ini = ExtIni::parse("k=1\nk=2\nk=3");
        assert_eq!(ini.get("k"), Some("1"));
        assert_eq!(ini.get_all("k"), vec!["1", "2", "3"]);
    }

    #[test]
    fn splits_on_first_equals_only() {
        let ini = ExtIni::parse("api_url=https://x.co/a?b=1&c=2");
        assert_eq!(ini.get("api_url"), Some("https://x.co/a?b=1&c=2"));
    }

    #[test]
    fn keeps_commas_and_hebrew() {
        let ini = ExtIni::parse("title=שלוחה ראשית\ntype=api,menu,routing_yemot");
        assert_eq!(ini.get("title"), Some("שלוחה ראשית"));
        assert_eq!(ini.get("type"), Some("api,menu,routing_yemot"));
    }

    #[test]
    fn handles_crlf_and_round_trips() {
        let ini = ExtIni::parse("type=menu\r\ntitle=בדיקה\r\n");
        assert_eq!(ini.get("title"), Some("בדיקה"));
        assert_eq!(ini.key_count(), 2);
        assert_eq!(ini.to_text(), "type=menu\ntitle=בדיקה");
    }

    #[test]
    fn trims_whitespace_around_key_and_value() {
        let ini = ExtIni::parse("  type  =  menu  ");
        assert_eq!(ini.get("type"), Some("menu"));
    }

    #[test]
    fn line_without_equals_is_preserved_as_comment() {
        let ini = ExtIni::parse("garbage line\ntype=menu");
        assert_eq!(ini.key_count(), 1);
        assert!(matches!(&ini.lines[0], IniLine::Comment(c) if c == "garbage line"));
    }

    #[test]
    fn to_compact_drops_comments_and_reports_truncation() {
        let ini = ExtIni::parse("; c\ntype=menu\n\ntitle=abc\n");
        let (text, truncated) = ini.to_compact(8192);
        assert_eq!(text, "type=menu\ntitle=abc\n");
        assert!(!truncated);

        let (text, truncated) = ini.to_compact(12);
        assert_eq!(text, "type=menu\n");
        assert!(truncated);
    }

    #[test]
    fn preview_merge_classifies_rows() {
        let ini = ExtIni::parse("type=menu\ntitle=old");
        let rows = ini.preview_merge(&[p("type", "menu"), p("title", "new"), p("enter_id", "yes")]);
        assert_eq!(rows[0].kind, DiffKind::Unchanged);
        assert_eq!(rows[1].kind, DiffKind::Changed);
        assert_eq!(rows[1].before.as_deref(), Some("old"));
        assert_eq!(rows[2].kind, DiffKind::New);
        assert_eq!(rows[2].before, None);
    }

    #[test]
    fn strips_a_leading_bom() {
        let ini = ExtIni::parse("\u{feff}type=menu\ntitle=בדיקה\n");
        assert_eq!(ini.get("type"), Some("menu"));
        assert_eq!(ini.key_count(), 2);
        let text = ini.to_text();
        assert!(!text.contains('\u{feff}'));
        assert_eq!(text, "type=menu\ntitle=בדיקה");
        // a BOM in the middle of the file is content, not an encoding marker
        assert_eq!(ExtIni::parse("a=1\n\u{feff}b=2").get("b"), None);
    }

    #[test]
    fn empty_input_has_no_lines() {
        let ini = ExtIni::parse("");
        assert_eq!(ini.key_count(), 0);
        assert_eq!(ini.to_compact(100), (String::new(), false));
    }
}
