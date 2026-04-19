//! Search result record and formatting helpers.

use std::cmp::Reverse;
use std::path::PathBuf;

use serde::Serialize;

use crate::graph::types::{CodeGraph, Symbol, SymbolId};

/// A single search result. Rendered to an MCP text block by the tool handler.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SearchHit {
    pub file: PathBuf,
    pub line: u32,
    /// Inline signature for structural results (e.g. `processRequest(req: Request): Promise<Response>`).
    /// `None` for grep hits.
    pub signature: Option<String>,
    /// Raw matching line for grep hits. `None` for structural hits.
    pub snippet: Option<String>,
}

impl SearchHit {
    /// Build a structural hit from a parsed symbol. Copies the signature
    /// through so the MCP response renders inline without a follow-up read.
    #[must_use]
    pub fn structural(symbol: &Symbol) -> Self {
        Self {
            file: symbol.id.file.clone(),
            line: symbol.line_start,
            signature: Some(symbol.signature.clone()),
            snippet: None,
        }
    }

    /// Build a grep hit from a raw `file:line` match.
    #[must_use]
    pub fn grep(file: PathBuf, line: u32, snippet: String) -> Self {
        Self {
            file,
            line,
            signature: None,
            snippet: Some(snippet),
        }
    }

    /// Synthetic "no-match" hint. Used when a query returns no hits but there is
    /// useful guidance about where the match might live — e.g. "no same-file
    /// callers; try grep for cross-file".
    #[must_use]
    pub fn empty_hint(message: &str) -> Self {
        Self {
            file: PathBuf::new(),
            line: 0,
            signature: Some(message.to_owned()),
            snippet: None,
        }
    }

    /// Returns `true` if this hit is a synthetic hint rather than a real match.
    /// Hint hits have an empty file path and `line == 0`.
    #[must_use]
    pub fn is_hint(&self) -> bool {
        self.file.as_os_str().is_empty() && self.line == 0 && self.snippet.is_none()
    }

    /// Render the hit as a single compact line suitable for an MCP tool
    /// response. Uses `project_root`-relative paths when possible, and
    /// strips lifetime/generic-bound syntax from the signature — agents
    /// use this as orientation, not as a copy-paste-ready declaration.
    ///
    /// Examples:
    ///
    /// - `src/graph/ops.rs:12 callers(graph, target) -> Vec<&SymbolId>`
    /// - `/other/abs/path.rs:5 bar()`  (path outside `project_root`)
    /// - `src/a.rs:2 let NEEDLE = 1;`  (no signature — uses snippet)
    #[must_use]
    pub fn to_compact_line(&self, project_root: &std::path::Path) -> String {
        let path = self
            .file
            .strip_prefix(project_root)
            .map_or_else(|_| self.file.display().to_string(), |p| p.display().to_string());
        let body = match (self.signature.as_deref(), self.snippet.as_deref()) {
            (Some(sig), _) => compact_signature(sig),
            (None, Some(snippet)) => snippet.trim().to_string(),
            (None, None) => String::new(),
        };
        if body.is_empty() {
            format!("{path}:{}", self.line)
        } else {
            format!("{path}:{} {body}", self.line)
        }
    }
}

/// Strip Rust-specific noise from a signature line that agents don't need for
/// orientation: explicit lifetimes (`'a`, `'g`, `'static`), trait bounds inside
/// generics (`T: Sized`), and the leading `fn ` keyword. Converts the
/// Rust-idiomatic `): Ret` return-type colon to `) -> Ret` only when the
/// original had no `->`.
fn compact_signature(sig: &str) -> String {
    // Pass 1 — strip lifetimes: scan byte-by-byte, track angle-bracket depth.
    // Inside `<...>`, any `'ident` (followed by opt `, `) is dropped.
    // Outside `<...>`, `'ident` is also dropped (e.g. `&'a T` → `&T`).
    let mut out = String::with_capacity(sig.len());
    let bytes = sig.as_bytes();
    let mut depth_angle: i32 = 0;
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i] as char;
        match c {
            '<' => {
                depth_angle += 1;
                out.push(c);
                i += 1;
            }
            '>' => {
                depth_angle -= 1;
                out.push(c);
                i += 1;
            }
            '\'' if is_lifetime_start(bytes, i) => {
                // Drop the lifetime token itself.
                i += 1;
                while i < bytes.len()
                    && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_')
                {
                    i += 1;
                }
                // Inside generics, also swallow a trailing `, ` so we don't
                // leave orphaned commas like `<, T>`.
                if depth_angle > 0 && i < bytes.len() && bytes[i] == b',' {
                    i += 1;
                    while i < bytes.len() && bytes[i] == b' ' {
                        i += 1;
                    }
                }
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }

    // Pass 2 — strip `X: Bound` inside `<...>` angle-bracket sections.
    // We walk character-by-character tracking depth.  When we see `:` at
    // depth > 0 preceded by an identifier character, we skip until the
    // next `,` or `>` at the same depth.
    let mut cleaned = String::with_capacity(out.len());
    let chars: Vec<char> = out.chars().collect();
    let mut j = 0;
    while j < chars.len() {
        if chars[j] == ':'
            && j > 0
            && chars[j - 1].is_ascii_alphanumeric()
            && inside_generics(&chars, j)
        {
            // Skip to the next `,` or `>` at depth 0 relative to where we are.
            let mut depth = 0i32;
            while j < chars.len() {
                match chars[j] {
                    '<' => depth += 1,
                    '>' | ',' if depth == 0 => break,
                    '>' => depth -= 1,
                    _ => {}
                }
                j += 1;
            }
            continue;
        }
        cleaned.push(chars[j]);
        j += 1;
    }

    // Pass 3 — drop the leading `fn ` keyword when present.
    let trimmed = cleaned.strip_prefix("fn ").unwrap_or(&cleaned);

    // Pass 4 — convert `): T` return-type style to `) -> T` when no `->`.
    if !trimmed.contains("->") {
        if let Some(idx) = trimmed.rfind("):") {
            let (head, tail) = trimmed.split_at(idx + 1);
            return format!("{head} ->{}", &tail[1..]);
        }
    }
    trimmed.to_string()
}

/// `'` starts a lifetime when it is followed immediately by an ASCII letter or `_`.
/// We assume signatures don't contain string/char literals.
fn is_lifetime_start(bytes: &[u8], i: usize) -> bool {
    i + 1 < bytes.len() && (bytes[i + 1].is_ascii_alphabetic() || bytes[i + 1] == b'_')
}

/// Returns `true` when `pos` is inside at least one unclosed `<` in `chars`.
fn inside_generics(chars: &[char], pos: usize) -> bool {
    let mut depth = 0i32;
    for &c in &chars[..pos] {
        match c {
            '<' => depth += 1,
            '>' => depth -= 1,
            _ => {}
        }
    }
    depth > 0
}

/// Sort a slice of [`SymbolId`] references by reverse-edge centrality descending.
///
/// Used to rank multiple matches in `find_by_name` / `callers_of` so the
/// highest-dependent symbols come first.
pub fn sort_by_centrality(graph: &CodeGraph, ids: &mut [&SymbolId]) {
    ids.sort_by_key(|id| Reverse(graph.centrality.get(*id).copied().unwrap_or(0)));
}

#[cfg(test)]
mod tests_compact {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn compact_line_uses_relative_path_when_under_project_root() {
        let hit = SearchHit {
            file: PathBuf::from("/proj/root/src/graph/ops.rs"),
            line: 12,
            signature: Some(
                "callers(graph: &'g CodeGraph, target: &SymbolId): Vec<&'g SymbolId>".to_string(),
            ),
            snippet: None,
        };
        let line = hit.to_compact_line(&PathBuf::from("/proj/root"));
        assert!(line.starts_with("src/graph/ops.rs:12"), "got: {line}");
        assert!(!line.contains("/proj/root"), "absolute path leaked: {line}");
        assert!(line.contains("callers"), "signature name should survive: {line}");
    }

    #[test]
    fn compact_line_strips_lifetimes_and_trailing_generics() {
        let hit = SearchHit {
            file: PathBuf::from("/p/src/a.rs"),
            line: 1,
            signature: Some("fn foo<'a, T: Sized>(x: &'a T) -> Vec<&'a T>".to_string()),
            snippet: None,
        };
        let line = hit.to_compact_line(&PathBuf::from("/p"));
        assert!(!line.contains("'a"), "lifetime not stripped: {line}");
        assert!(!line.contains("T: Sized"), "generic bound not stripped: {line}");
        assert!(line.contains("foo"));
    }

    #[test]
    fn compact_line_preserves_absolute_path_when_outside_project_root() {
        let hit = SearchHit {
            file: PathBuf::from("/other/abs/path.rs"),
            line: 5,
            signature: Some("fn bar()".to_string()),
            snippet: None,
        };
        let line = hit.to_compact_line(&PathBuf::from("/proj/root"));
        assert!(line.starts_with("/other/abs/path.rs:5"), "got: {line}");
    }

    #[test]
    fn compact_line_falls_back_to_snippet_when_no_signature() {
        let hit = SearchHit {
            file: PathBuf::from("/p/a.rs"),
            line: 2,
            signature: None,
            snippet: Some("let NEEDLE = 1;".to_string()),
        };
        let line = hit.to_compact_line(&PathBuf::from("/p"));
        assert!(line.contains("NEEDLE"), "got: {line}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::types::{SymbolKind, Visibility};

    fn sym(name: &str) -> Symbol {
        Symbol {
            id: SymbolId {
                file: PathBuf::from("x.ts"),
                name: name.to_string(),
                kind: SymbolKind::Function,
            },
            line_start: 1,
            line_end: 2,
            signature: format!("fn {name}()"),
            params: vec![],
            return_type: None,
            visibility: Visibility::Export,
            body_hash: 0,
            is_async: false,
            embedding_id: None,
        }
    }

    #[test]
    fn structural_hit_copies_signature() {
        let s = sym("foo");
        let hit = SearchHit::structural(&s);
        assert_eq!(hit.signature.as_deref(), Some("fn foo()"));
        assert!(hit.snippet.is_none());
        assert_eq!(hit.file, PathBuf::from("x.ts"));
        assert_eq!(hit.line, 1);
    }

    #[test]
    fn grep_hit_carries_snippet_only() {
        let hit = SearchHit::grep(PathBuf::from("a.ts"), 5, "  const x = foo();".to_string());
        assert!(hit.signature.is_none());
        assert_eq!(hit.snippet.as_deref(), Some("  const x = foo();"));
    }

    #[test]
    fn sort_by_centrality_orders_highest_first() {
        let mut g = CodeGraph::new();
        let low = sym("low");
        let high = sym("high");
        g.insert_symbol(low.clone());
        g.insert_symbol(high.clone());
        g.centrality.insert(low.id.clone(), 1);
        g.centrality.insert(high.id.clone(), 10);
        let mut ids = vec![&low.id, &high.id];
        sort_by_centrality(&g, &mut ids);
        assert_eq!(ids[0], &high.id);
        assert_eq!(ids[1], &low.id);
    }

    #[test]
    fn sort_by_centrality_missing_entries_treated_as_zero() {
        let mut g = CodeGraph::new();
        let only_in_centrality = sym("a");
        let not_in_centrality = sym("b");
        g.insert_symbol(only_in_centrality.clone());
        g.insert_symbol(not_in_centrality.clone());
        g.centrality.insert(only_in_centrality.id.clone(), 5);
        let mut ids = vec![&not_in_centrality.id, &only_in_centrality.id];
        sort_by_centrality(&g, &mut ids);
        // The one with centrality=5 must come before the one missing (treated as 0).
        assert_eq!(ids[0], &only_in_centrality.id);
    }
}
