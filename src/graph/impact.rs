//! Cascade impact analysis — SPEC §5.
//!
//! Phase 1 ships four warnings: `SIGNATURE`, `ASYNC_CHANGE`, `ORPHAN`, and
//! `INTERFACE_BREAK`. Each fires from a single structural check; the aim is
//! high signal, not exhaustiveness (SPEC §5.2: agents ignore noisy output).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::graph::ops::callers;
use crate::graph::types::{CodeGraph, EdgeKind, Symbol, SymbolId, SymbolKind};

/// Cascade warning kind. Serialised as an uppercase tag in the MCP response.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum WarningKind {
    Signature,
    AsyncChange,
    Orphan,
    InterfaceBreak,
}

impl WarningKind {
    #[must_use]
    pub const fn tag(self) -> &'static str {
        match self {
            Self::Signature => "SIGNATURE",
            Self::AsyncChange => "ASYNC_CHANGE",
            Self::Orphan => "ORPHAN",
            Self::InterfaceBreak => "INTERFACE_BREAK",
        }
    }
}

/// A single rendered cascade warning. Body is capped at 200 chars by
/// [`Warning::clamp`] to respect the SPEC §5.4 output budget.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Warning {
    pub kind: WarningKind,
    pub symbol: SymbolId,
    pub body: String,
}

impl Warning {
    #[must_use]
    pub fn new(kind: WarningKind, symbol: SymbolId, body: impl Into<String>) -> Self {
        let mut w = Self {
            kind,
            symbol,
            body: body.into(),
        };
        w.clamp();
        w
    }

    fn clamp(&mut self) {
        const MAX: usize = 200;
        if self.body.chars().count() > MAX {
            let truncated: String = self.body.chars().take(MAX - 1).collect();
            self.body = format!("{truncated}…");
        }
    }
}

/// `SIGNATURE` — function params / return-type / async-ness changed.
/// Fires when the modified symbol has ≥ 1 caller; body lists up to 10
/// caller `file:name` pairs.
#[must_use]
pub fn detect_signature(graph: &CodeGraph, _old: &Symbol, new: &Symbol) -> Option<Warning> {
    let caller_ids = find_callers_by_name(graph, new);
    if caller_ids.is_empty() {
        return None;
    }
    let total = caller_ids.len();
    let shown: Vec<String> = caller_ids
        .iter()
        .take(10)
        .map(|id| format!("{}:{}", id.file.display(), id.name))
        .collect();
    let more = if total > 10 {
        format!(" …and {} more ({} total)", total - 10, total)
    } else {
        String::new()
    };
    let body = format!(
        "{}() signature changed. {} callers may break: {}{}",
        new.id.name,
        total,
        shown.join(", "),
        more
    );
    Some(Warning::new(WarningKind::Signature, new.id.clone(), body))
}

/// Resolve callers of `sym` by `(file, name)` rather than by exact `SymbolId`.
///
/// Why: Phase 1.2 language drivers emit call edges with `to.kind = Function`
/// as a placeholder — the real callee's kind may be `Method` /
/// `AsyncFunction`. Cross-file resolution (Task 13) will patch this up.
/// For Phase 1.6 we relax the match to ignore kind so SIGNATURE fires
/// correctly on methods and async functions.
fn find_callers_by_name<'g>(graph: &'g CodeGraph, sym: &Symbol) -> Vec<&'g SymbolId> {
    // First try the exact SymbolId path (fast path).
    let exact: Vec<&SymbolId> = callers(graph, &sym.id);
    if !exact.is_empty() {
        return exact;
    }
    // Fall back to (file, name) match scanning reverse edges for any kind.
    graph
        .reverse_edges
        .iter()
        .filter(|(to_id, _)| to_id.file == sym.id.file && to_id.name == sym.id.name)
        .flat_map(|(_, edges)| edges.iter().map(|e| &e.from))
        .collect()
}

/// `ASYNC_CHANGE` — function flipped between sync and async. Callers that
/// don't `await` will receive a Promise/Future instead of the value.
#[must_use]
pub fn detect_async_change(graph: &CodeGraph, old: &Symbol, new: &Symbol) -> Option<Warning> {
    if old.is_async == new.is_async {
        return None;
    }
    let caller_ids = find_callers_by_name(graph, new);
    let direction = if new.is_async { "sync→async" } else { "async→sync" };
    let total = caller_ids.len();
    let body = format!(
        "{}() {}. {} caller{} need{} update.",
        new.id.name,
        direction,
        total,
        if total == 1 { "" } else { "s" },
        if total == 1 { "s" } else { "" }
    );
    Some(Warning::new(WarningKind::AsyncChange, new.id.clone(), body))
}

/// `ORPHAN` — a symbol was removed but callers' forward edges still point
/// to it. Scans `graph.forward_edges` for any edge whose `to` matches the
/// removed symbol's id. Plan 1's `remove_file` preserves these dangling
/// edges for exactly this detection path.
#[must_use]
pub fn detect_orphan(graph: &CodeGraph, removed: &Symbol) -> Option<Warning> {
    let dangling: Vec<&SymbolId> = graph
        .forward_edges
        .iter()
        .flat_map(|(_, edges)| edges.iter())
        .filter(|e| e.to.file == removed.id.file && e.to.name == removed.id.name)
        .map(|e| &e.from)
        .collect();
    if dangling.is_empty() {
        return None;
    }
    let total = dangling.len();
    let shown: Vec<String> = dangling
        .iter()
        .take(10)
        .map(|id| format!("{}:{}", id.file.display(), id.name))
        .collect();
    let body = format!(
        "{}() removed but {} caller{} still reference it: {}",
        removed.id.name,
        total,
        if total == 1 { "" } else { "s" },
        shown.join(", ")
    );
    Some(Warning::new(WarningKind::Orphan, removed.id.clone(), body))
}

/// Render the one-line summary header for an `apply_change` response.
/// Groups warnings by [`WarningKind::tag`] and produces
/// `"N warnings: X SIGNATURE, Y ASYNC_CHANGE, ..."`. Empty input yields
/// `"0 warnings"`. Tags are sorted alphabetically for stable output.
#[must_use]
pub fn summary_line(warnings: &[Warning]) -> String {
    if warnings.is_empty() {
        return "0 warnings".to_string();
    }
    let mut counts: std::collections::BTreeMap<&'static str, usize> =
        std::collections::BTreeMap::new();
    for w in warnings {
        *counts.entry(w.kind.tag()).or_insert(0) += 1;
    }
    let parts: Vec<String> = counts
        .iter()
        .map(|(tag, n)| format!("{n} {tag}"))
        .collect();
    format!("{} warnings: {}", warnings.len(), parts.join(", "))
}

/// `INTERFACE_BREAK` — a TS interface or Rust trait's signature changed.
/// Lists implementing classes/structs via reverse `Implements` edges.
#[must_use]
pub fn detect_interface_break(graph: &CodeGraph, old: &Symbol, new: &Symbol) -> Option<Warning> {
    if !matches!(new.id.kind, SymbolKind::Interface | SymbolKind::Trait) {
        return None;
    }
    if old.signature == new.signature {
        return None;
    }
    let implementors: Vec<&SymbolId> = graph
        .reverse_edges
        .get(&new.id)
        .into_iter()
        .flatten()
        .filter(|e| e.kind == EdgeKind::Implements)
        .map(|e| &e.from)
        .collect();
    if implementors.is_empty() {
        return None;
    }
    let total = implementors.len();
    let shown: Vec<String> = implementors
        .iter()
        .take(10)
        .map(|id| format!("{}:{}", id.file.display(), id.name))
        .collect();
    let body = format!(
        "{} contract changed. {} impl{} may violate: {}",
        new.id.name,
        total,
        if total == 1 { "" } else { "s" },
        shown.join(", ")
    );
    Some(Warning::new(WarningKind::InterfaceBreak, new.id.clone(), body))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warning_kind_serialises_as_screaming_snake() {
        assert_eq!(serde_json::to_string(&WarningKind::Signature).unwrap(), "\"SIGNATURE\"");
        assert_eq!(serde_json::to_string(&WarningKind::AsyncChange).unwrap(), "\"ASYNC_CHANGE\"");
        assert_eq!(serde_json::to_string(&WarningKind::Orphan).unwrap(), "\"ORPHAN\"");
        assert_eq!(serde_json::to_string(&WarningKind::InterfaceBreak).unwrap(), "\"INTERFACE_BREAK\"");
    }

    #[test]
    fn warning_kind_tag_matches_wire_format() {
        assert_eq!(WarningKind::Signature.tag(), "SIGNATURE");
        assert_eq!(WarningKind::AsyncChange.tag(), "ASYNC_CHANGE");
        assert_eq!(WarningKind::Orphan.tag(), "ORPHAN");
        assert_eq!(WarningKind::InterfaceBreak.tag(), "INTERFACE_BREAK");
    }

    #[test]
    fn summary_line_groups_warnings_by_kind() {
        use crate::graph::types::{SymbolId, SymbolKind};
        let id = SymbolId {
            file: std::path::PathBuf::from("a.ts"),
            name: "x".to_string(),
            kind: SymbolKind::Function,
        };
        let warnings = vec![
            Warning::new(WarningKind::Signature, id.clone(), "sig"),
            Warning::new(WarningKind::Orphan, id.clone(), "orph"),
            Warning::new(WarningKind::Signature, id.clone(), "sig2"),
        ];
        let s = summary_line(&warnings);
        assert!(s.starts_with("3 warnings"));
        assert!(s.contains("2 SIGNATURE"));
        assert!(s.contains("1 ORPHAN"));
    }

    #[test]
    fn summary_line_zero_case() {
        assert_eq!(summary_line(&[]), "0 warnings");
    }

    #[test]
    fn warning_body_clamped_to_200_chars() {
        use crate::graph::types::{SymbolId, SymbolKind};
        let long_body = "x".repeat(500);
        let id = SymbolId {
            file: std::path::PathBuf::from("a.ts"),
            name: "x".to_string(),
            kind: SymbolKind::Function,
        };
        let w = Warning::new(WarningKind::Signature, id, long_body);
        assert!(w.body.chars().count() <= 200);
        assert!(w.body.ends_with('…'));
    }
}

#[cfg(test)]
mod detector_tests {
    use super::*;
    use crate::graph::types::{
        CodeGraph, Confidence, Edge, EdgeKind, Symbol, SymbolId, SymbolKind, Visibility,
    };
    use std::path::PathBuf;

    fn sym(name: &str, file: &str) -> Symbol {
        Symbol {
            id: SymbolId {
                file: PathBuf::from(file),
                name: name.to_string(),
                kind: SymbolKind::Function,
            },
            line_start: 1,
            line_end: 3,
            signature: format!("fn {name}()"),
            params: vec![],
            return_type: None,
            visibility: Visibility::Export,
            body_hash: 0,
            is_async: false,
            embedding_id: None,
        }
    }

    fn connect(g: &mut CodeGraph, from: &Symbol, to: &Symbol) {
        g.insert_edge(Edge {
            from: from.id.clone(),
            to: to.id.clone(),
            kind: EdgeKind::Calls,
            line: 10,
            confidence: Confidence::Certain,
        });
    }

    // --- Task 6: SIGNATURE ---

    #[test]
    fn signature_warning_lists_callers() {
        let mut g = CodeGraph::new();
        let target = sym("processRequest", "h.ts");
        let caller_a = sym("api", "api.ts");
        let caller_b = sym("admin", "admin.ts");
        g.insert_symbol(target.clone());
        g.insert_symbol(caller_a.clone());
        g.insert_symbol(caller_b.clone());
        connect(&mut g, &caller_a, &target);
        connect(&mut g, &caller_b, &target);

        let mut old = target.clone();
        old.signature = "processRequest(req, res)".to_string();
        let mut new = target.clone();
        new.signature = "processRequest(req, res, next)".to_string();

        let warning = detect_signature(&g, &old, &new).expect("should fire");
        assert_eq!(warning.kind, WarningKind::Signature);
        assert!(warning.body.contains("processRequest"), "body={}", warning.body);
        assert!(warning.body.contains("2 callers"), "body={}", warning.body);
        assert!(warning.body.contains("api.ts") || warning.body.contains("admin.ts"));
    }

    #[test]
    fn signature_warning_none_when_no_callers() {
        let mut g = CodeGraph::new();
        let target = sym("lonely", "x.ts");
        g.insert_symbol(target.clone());
        let mut new = target.clone();
        new.signature = "lonely(x)".to_string();
        assert!(detect_signature(&g, &target, &new).is_none());
    }

    // --- Task 7: ASYNC_CHANGE ---

    #[test]
    fn async_change_warning_when_function_becomes_async() {
        let mut g = CodeGraph::new();
        let target = sym("process", "h.ts");
        let caller = sym("api", "api.ts");
        g.insert_symbol(target.clone());
        g.insert_symbol(caller.clone());
        connect(&mut g, &caller, &target);

        let mut old = target.clone();
        old.is_async = false;
        let mut new = target.clone();
        new.is_async = true;

        let warning = detect_async_change(&g, &old, &new).expect("should fire");
        assert_eq!(warning.kind, WarningKind::AsyncChange);
        assert!(warning.body.contains("async"));
        assert!(warning.body.contains("1 caller"));
    }

    #[test]
    fn async_change_warning_none_when_sync_stays_sync() {
        let mut g = CodeGraph::new();
        let target = sym("process", "h.ts");
        g.insert_symbol(target.clone());
        assert!(detect_async_change(&g, &target, &target).is_none());
    }

    // --- Task 8: ORPHAN ---

    #[test]
    fn orphan_warning_when_removed_symbol_has_callers() {
        let mut g = CodeGraph::new();
        let target = sym("gone", "h.ts");
        let caller = sym("api", "api.ts");
        g.insert_symbol(target.clone());
        g.insert_symbol(caller.clone());
        connect(&mut g, &caller, &target);

        // Simulate: target's file reindexed after delete. remove_file keeps
        // the caller's forward edge dangling (Plan 1 Task 0 fix).
        g.remove_file(std::path::Path::new("h.ts"));

        let warning = detect_orphan(&g, &target).expect("should fire");
        assert_eq!(warning.kind, WarningKind::Orphan);
        assert!(warning.body.contains("gone"));
        assert!(warning.body.contains("1 caller"));
    }

    #[test]
    fn orphan_warning_none_when_no_callers_remaining() {
        let mut g = CodeGraph::new();
        let target = sym("gone", "h.ts");
        g.insert_symbol(target.clone());
        g.remove_file(std::path::Path::new("h.ts"));
        assert!(detect_orphan(&g, &target).is_none());
    }

    // --- Task 9: INTERFACE_BREAK ---

    #[test]
    fn interface_break_warning_lists_implementors() {
        let mut g = CodeGraph::new();
        let mut iface = sym("Greeter", "api.ts");
        iface.id.kind = SymbolKind::Interface;
        let impl_a = sym("EnglishGreeter", "english.ts");
        g.insert_symbol(iface.clone());
        g.insert_symbol(impl_a.clone());
        g.insert_edge(Edge {
            from: impl_a.id.clone(),
            to: iface.id.clone(),
            kind: EdgeKind::Implements,
            line: 1,
            confidence: Confidence::Certain,
        });

        let mut old = iface.clone();
        old.signature = "interface Greeter { greet(): string }".to_string();
        let mut new = iface.clone();
        new.signature = "interface Greeter { greet(name: string): string }".to_string();

        let warning = detect_interface_break(&g, &old, &new).expect("should fire");
        assert_eq!(warning.kind, WarningKind::InterfaceBreak);
        assert!(warning.body.contains("Greeter"));
        assert!(warning.body.contains("EnglishGreeter") || warning.body.contains("1 impl"));
    }

    #[test]
    fn interface_break_none_when_not_interface_or_trait() {
        let mut g = CodeGraph::new();
        let f = sym("foo", "f.ts");
        g.insert_symbol(f.clone());
        assert!(detect_interface_break(&g, &f, &f).is_none());
    }
}
