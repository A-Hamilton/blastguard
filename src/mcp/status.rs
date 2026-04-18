//! `blastguard://status` resource renderer — compact project overview.
//!
//! Renders a single plain-text block that MCP clients can attach as context.
//! The block reports graph node/edge counts, indexed languages, recently
//! modified files/symbols, and a summary of the last test run.

use std::path::Path;
use std::sync::Mutex;

use crate::graph::types::CodeGraph;
use crate::session::SessionState;

/// Render a compact status block for the `blastguard://status` resource.
///
/// All data is read under the relevant locks and formatted into a single
/// UTF-8 string. The function holds each lock only for the duration of the
/// read; it does not hold both simultaneously.
///
/// # Panics
///
/// Panics if the `graph` or `session` `Mutex` is poisoned by a prior thread
/// panic — an unrecoverable condition in this server.
#[must_use]
pub fn render(
    graph: &Mutex<CodeGraph>,
    session: &Mutex<SessionState>,
    project_root: &Path,
) -> String {
    let (symbol_count, edge_count, languages) = {
        let g = graph.lock().expect("graph lock poisoned");
        let sym_count = g.symbols.len();
        let edge_count = g.forward_edges.values().map(Vec::len).sum::<usize>();
        // Derive indexed languages from file extensions in the symbol table.
        let mut exts: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
        for id in g.symbols.keys() {
            if let Some(ext) = id.file.extension().and_then(|e| e.to_str()) {
                exts.insert(ext);
            }
        }
        let langs = exts.into_iter().collect::<Vec<_>>().join(", ");
        (sym_count, edge_count, langs)
    };

    let (edit_count, last_test_summary) = {
        let s = session.lock().expect("session lock poisoned");
        let edits = s.modified_symbols().len();
        let test_summary = s.last_test_results().map_or_else(
            || "no test run yet".to_string(),
            |r| {
                format!(
                    "{} passed, {} failed, {} skipped ({} ms)",
                    r.passed, r.failed, r.skipped, r.duration_ms
                )
            },
        );
        (edits, test_summary)
    };

    let lang_line = if languages.is_empty() {
        "languages: (none — index empty)".to_string()
    } else {
        format!("languages: {languages}")
    };

    format!(
        "BlastGuard status\n\
         project: {root}\n\
         symbols: {symbol_count}  edges: {edge_count}\n\
         {lang_line}\n\
         session edits: {edit_count}\n\
         last test run: {last_test_summary}\n",
        root = project_root.display(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::types::{Symbol, SymbolId, SymbolKind, Visibility};
    use std::path::PathBuf;
    use std::sync::Mutex;

    fn make_sym(name: &str, file: &str) -> Symbol {
        Symbol {
            id: SymbolId {
                file: PathBuf::from(file),
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
    fn render_reports_counts_and_languages() {
        let mut g = CodeGraph::new();
        g.insert_symbol(make_sym("foo", "src/a.ts"));
        g.insert_symbol(make_sym("bar", "src/b.rs"));
        let graph = Mutex::new(g);
        let session = Mutex::new(SessionState::new());
        let root = PathBuf::from("/proj");

        let out = render(&graph, &session, &root);

        assert!(out.contains("symbols: 2"), "got: {out}");
        assert!(out.contains("ts"), "expected ts in languages; got: {out}");
        assert!(out.contains("rs"), "expected rs in languages; got: {out}");
        assert!(out.contains("session edits: 0"), "got: {out}");
        assert!(out.contains("no test run yet"), "got: {out}");
    }

    #[test]
    fn render_shows_session_edits() {
        use crate::graph::types::SymbolId;

        let graph = Mutex::new(CodeGraph::new());
        let mut s = SessionState::new();
        s.record_symbol_edit(SymbolId {
            file: PathBuf::from("src/a.ts"),
            name: "doThing".to_string(),
            kind: SymbolKind::Function,
        });
        s.record_symbol_edit(SymbolId {
            file: PathBuf::from("src/b.rs"),
            name: "handle".to_string(),
            kind: SymbolKind::Function,
        });
        let session = Mutex::new(s);
        let root = PathBuf::from("/proj");

        let out = render(&graph, &session, &root);

        assert!(out.contains("session edits: 2"), "got: {out}");
    }
}
