//! Symbol-table diff between a file's pre-edit and post-edit state.
//!
//! `diff(old, new)` classifies each symbol into one of four buckets by
//! [`SymbolId`]: added, removed, `modified_sig` (signature/params/return/
//! async-ness differs), `modified_body` (signature identical, `body_hash`
//! differs). Used by the `apply_change` orchestrator to drive cascade
//! detectors: `modified_sig` feeds `SIGNATURE` / `ASYNC_CHANGE` /
//! `INTERFACE_BREAK`; `removed` feeds `ORPHAN`.

use std::collections::HashMap;

use crate::graph::types::{Symbol, SymbolId};

/// Classification of changes between two symbol sets for the same file.
#[derive(Debug, Default)]
pub struct SymbolDiff {
    pub added: Vec<Symbol>,
    pub removed: Vec<Symbol>,
    pub modified_sig: Vec<(Symbol, Symbol)>,
    pub modified_body: Vec<(Symbol, Symbol)>,
}

impl SymbolDiff {
    /// `true` when every category is empty — whitespace- or comments-only edit.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.added.is_empty()
            && self.removed.is_empty()
            && self.modified_sig.is_empty()
            && self.modified_body.is_empty()
    }
}

/// Diff two symbol lists by [`SymbolId`]. Signature equality is determined
/// by `signature` + `return_type` + `is_async` + `params`; body equality
/// by `body_hash` (only checked when signatures match).
#[must_use]
pub fn diff(old: &[Symbol], new: &[Symbol]) -> SymbolDiff {
    let old_by_id: HashMap<&SymbolId, &Symbol> = old.iter().map(|s| (&s.id, s)).collect();
    let new_by_id: HashMap<&SymbolId, &Symbol> = new.iter().map(|s| (&s.id, s)).collect();

    let mut out = SymbolDiff::default();

    for (id, s) in &new_by_id {
        match old_by_id.get(id) {
            None => out.added.push((*s).clone()),
            Some(old_sym) => {
                let sig_changed = old_sym.signature != s.signature
                    || old_sym.return_type != s.return_type
                    || old_sym.is_async != s.is_async
                    || old_sym.params != s.params;
                if sig_changed {
                    out.modified_sig.push(((*old_sym).clone(), (*s).clone()));
                } else if old_sym.body_hash != s.body_hash {
                    out.modified_body.push(((*old_sym).clone(), (*s).clone()));
                }
            }
        }
    }
    for (id, s) in &old_by_id {
        if !new_by_id.contains_key(id) {
            out.removed.push((*s).clone());
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::types::{SymbolKind, Visibility};
    use std::path::PathBuf;

    fn sym(name: &str, sig: &str, body_hash: u64, is_async: bool) -> Symbol {
        Symbol {
            id: SymbolId {
                file: PathBuf::from("x.ts"),
                name: name.to_string(),
                kind: SymbolKind::Function,
            },
            line_start: 1,
            line_end: 5,
            signature: sig.to_string(),
            params: vec![],
            return_type: None,
            visibility: Visibility::Export,
            body_hash,
            is_async,
            embedding_id: None,
        }
    }

    #[test]
    fn diff_detects_added() {
        let old = vec![sym("foo", "foo()", 0, false)];
        let new = vec![
            sym("foo", "foo()", 0, false),
            sym("bar", "bar()", 0, false),
        ];
        let d = diff(&old, &new);
        assert_eq!(d.added.len(), 1);
        assert_eq!(d.added[0].id.name, "bar");
        assert!(d.removed.is_empty());
        assert!(d.modified_sig.is_empty());
        assert!(d.modified_body.is_empty());
    }

    #[test]
    fn diff_detects_removed() {
        let old = vec![
            sym("foo", "foo()", 0, false),
            sym("bar", "bar()", 0, false),
        ];
        let new = vec![sym("foo", "foo()", 0, false)];
        let d = diff(&old, &new);
        assert_eq!(d.removed.len(), 1);
        assert_eq!(d.removed[0].id.name, "bar");
        assert!(d.added.is_empty());
    }

    #[test]
    fn diff_detects_modified_sig() {
        let old = vec![sym("foo", "foo()", 0, false)];
        let new = vec![sym("foo", "foo(x: i32)", 0, false)];
        let d = diff(&old, &new);
        assert_eq!(d.modified_sig.len(), 1);
        assert!(d.modified_body.is_empty());
        assert_eq!(d.modified_sig[0].0.signature, "foo()");
        assert_eq!(d.modified_sig[0].1.signature, "foo(x: i32)");
    }

    #[test]
    fn diff_detects_modified_body_only() {
        let old = vec![sym("foo", "foo()", 1, false)];
        let new = vec![sym("foo", "foo()", 2, false)];
        let d = diff(&old, &new);
        assert!(d.modified_sig.is_empty());
        assert_eq!(d.modified_body.len(), 1);
    }

    #[test]
    fn diff_detects_async_flip_as_modified_sig() {
        let old = vec![sym("foo", "foo()", 0, false)];
        let new = vec![sym("foo", "foo()", 0, true)];
        let d = diff(&old, &new);
        assert_eq!(d.modified_sig.len(), 1);
        assert!(d.modified_body.is_empty());
    }

    #[test]
    fn diff_empty_when_nothing_changed() {
        let old = vec![sym("foo", "foo()", 1, false)];
        let new = vec![sym("foo", "foo()", 1, false)];
        let d = diff(&old, &new);
        assert!(d.is_empty());
    }
}
