//! Failure attribution — append "YOU MODIFIED X (N edits ago)" hints to
//! test failure messages when the stack trace or `file:line` mentions a
//! symbol in [`crate::session::SessionState::modified_symbols`].

use std::collections::HashSet;

use crate::graph::types::{CodeGraph, SymbolId};
use crate::runner::TestFailure;
use crate::session::SessionState;

/// Append attribution hints to each failure's `message`. Non-destructive
/// for failures whose stack/file matches nothing in the session.
#[must_use]
pub fn annotate_failures(
    graph: &CodeGraph,
    session: &SessionState,
    failures: Vec<TestFailure>,
) -> Vec<TestFailure> {
    let modified_index: HashSet<&SymbolId> =
        session.modified_symbols().iter().map(|(id, _)| id).collect();

    failures
        .into_iter()
        .map(|mut f| {
            let mut hits: Vec<String> = Vec::new();
            for (stack_file, stack_line) in std::iter::once((f.file.clone(), f.line))
                .chain(f.stack.iter().cloned())
            {
                if let Some(sym_ids) = graph.file_symbols.get(&stack_file) {
                    for id in sym_ids {
                        if !modified_index.contains(id) {
                            continue;
                        }
                        let Some(sym) = graph.symbols.get(id) else { continue };
                        if stack_line >= sym.line_start && stack_line <= sym.line_end {
                            let n = session.edits_ago(id).unwrap_or(0);
                            hits.push(format!(
                                "YOU MODIFIED {} in {}:{} ({} edits ago)",
                                id.name,
                                id.file.display(),
                                sym.line_start,
                                n
                            ));
                        }
                    }
                }
            }
            hits.sort();
            hits.dedup();
            if !hits.is_empty() {
                f.message.push_str(". ");
                f.message.push_str(&hits.join(". "));
            }
            f
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::types::{CodeGraph, Symbol, SymbolId, SymbolKind, Visibility};
    use std::path::PathBuf;

    fn sym(name: &str, file: &str, line_start: u32, line_end: u32) -> Symbol {
        Symbol {
            id: SymbolId {
                file: PathBuf::from(file),
                name: name.to_string(),
                kind: SymbolKind::Function,
            },
            line_start,
            line_end,
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
    fn failure_pointing_at_modified_symbol_gets_annotation() {
        let mut g = CodeGraph::new();
        let s = sym("processRequest", "src/handler.ts", 5, 20);
        g.insert_symbol(s.clone());

        let mut session = SessionState::new();
        session.record_symbol_edit(s.id.clone());

        let failures = vec![TestFailure {
            test_name: "test_proc".to_string(),
            file: PathBuf::from("tests/a.ts"),
            line: 10,
            message: "AssertionError".to_string(),
            stack: vec![(PathBuf::from("src/handler.ts"), 12)],
        }];

        let annotated = annotate_failures(&g, &session, failures);
        assert!(
            annotated[0].message.contains("YOU MODIFIED processRequest"),
            "got: {}",
            annotated[0].message
        );
        assert!(annotated[0].message.contains("0 edits ago"));
    }

    #[test]
    fn failure_not_matching_any_edit_is_unchanged() {
        let mut g = CodeGraph::new();
        g.insert_symbol(sym("lonely", "src/l.ts", 1, 5));

        let session = SessionState::new();

        let failures = vec![TestFailure {
            test_name: "t".to_string(),
            file: PathBuf::from("tests/b.ts"),
            line: 1,
            message: "Error".to_string(),
            stack: vec![],
        }];

        let annotated = annotate_failures(&g, &session, failures);
        assert_eq!(annotated[0].message, "Error");
    }

    #[test]
    fn annotation_includes_edits_ago_count() {
        let mut g = CodeGraph::new();
        let a = sym("a", "src/a.ts", 1, 10);
        let b = sym("b", "src/b.ts", 1, 10);
        g.insert_symbol(a.clone());
        g.insert_symbol(b.clone());

        let mut session = SessionState::new();
        session.record_symbol_edit(a.id.clone());
        session.record_symbol_edit(b.id.clone());

        let failures = vec![TestFailure {
            test_name: "t".to_string(),
            file: PathBuf::from("tests/x.ts"),
            line: 1,
            message: "E".to_string(),
            stack: vec![(PathBuf::from("src/a.ts"), 5)],
        }];

        let annotated = annotate_failures(&g, &session, failures);
        assert!(annotated[0].message.contains("1 edits ago"),
            "got: {}", annotated[0].message);
    }
}
