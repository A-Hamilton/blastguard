//! Bundled context for `apply_change` responses — callers + tests of the
//! symbols affected by an edit (SPEC §3.2 "Context bundle eliminates
//! follow-up searches").

use std::path::Path;

use crate::edit::request::BundledContext;
use crate::graph::types::{CodeGraph, Symbol};
use crate::search::structural::{callers_of_id, tests_for};

/// Build the bundled context for a set of changed symbols in `file`.
/// - `callers`: up to 5 per changed symbol, capped at 10 total.
/// - `tests`: test-path importers of `file`, deduplicated.
#[must_use]
pub fn build(graph: &CodeGraph, file: &Path, changed: &[Symbol]) -> BundledContext {
    let mut callers: Vec<String> = Vec::new();
    let per_symbol_cap: usize = 5;
    let total_cap: usize = 10;

    'outer: for sym in changed {
        let hits = callers_of_id(graph, &sym.id, per_symbol_cap);
        for hit in hits {
            let line_str = match hit.signature.as_deref() {
                Some(sig) => format!("{}:{} — {}", hit.file.display(), hit.line, sig),
                None => format!("{}:{}", hit.file.display(), hit.line),
            };
            callers.push(line_str);
            if callers.len() >= total_cap {
                break 'outer;
            }
        }
    }

    let tests: Vec<String> = tests_for(graph, &file.to_string_lossy())
        .into_iter()
        .map(|h| h.file.to_string_lossy().to_string())
        .collect::<std::collections::BTreeSet<String>>()
        .into_iter()
        .collect();

    BundledContext { callers, tests }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::types::{
        Confidence, Edge, EdgeKind, Symbol, SymbolId, SymbolKind, Visibility,
    };
    use std::path::PathBuf;

    fn sym(name: &str, file: &str) -> Symbol {
        Symbol {
            id: SymbolId {
                file: PathBuf::from(file),
                name: name.to_string(),
                kind: SymbolKind::Function,
            },
            line_start: 10,
            line_end: 20,
            signature: format!("fn {name}(x: i32)"),
            params: vec![],
            return_type: None,
            visibility: Visibility::Export,
            body_hash: 0,
            is_async: false,
            embedding_id: None,
        }
    }

    #[test]
    fn build_returns_callers_and_skips_absent_tests() {
        let mut g = CodeGraph::new();
        let target = sym("processRequest", "src/handler.ts");
        let caller = sym("api", "src/api.ts");
        g.insert_symbol(target.clone());
        g.insert_symbol(caller.clone());
        g.insert_edge(Edge {
            from: caller.id.clone(),
            to: target.id.clone(),
            kind: EdgeKind::Calls,
            line: 5,
            confidence: Confidence::Certain,
        });

        let ctx = build(&g, Path::new("src/handler.ts"), &[target]);
        assert_eq!(ctx.callers.len(), 1);
        assert!(ctx.callers[0].contains("api.ts"));
        assert!(ctx.tests.is_empty());
    }

    #[test]
    fn build_deduplicates_test_files() {
        let mut g = CodeGraph::new();
        let target_id = SymbolId {
            file: PathBuf::from("src/handler.ts"),
            name: "handler".to_string(),
            kind: SymbolKind::Module,
        };
        let test_id = SymbolId {
            file: PathBuf::from("tests/handler.test.ts"),
            name: "t".to_string(),
            kind: SymbolKind::Module,
        };
        for id in [&target_id, &test_id] {
            let mut s = sym(&id.name, &id.file.to_string_lossy());
            s.id = id.clone();
            g.insert_symbol(s);
        }
        g.insert_edge(Edge {
            from: test_id.clone(),
            to: target_id.clone(),
            kind: EdgeKind::Imports,
            line: 1,
            confidence: Confidence::Certain,
        });

        let target = sym("handler", "src/handler.ts");
        let ctx = build(&g, Path::new("src/handler.ts"), &[target]);
        assert_eq!(ctx.tests.len(), 1);
        assert!(ctx.tests[0].contains(".test."));
    }
}
