//! In-memory per-server session state used for test-failure attribution and
//! the `blastguard://status` resource.
//!
//! Resets on server restart, matching per-task evaluation environments like
//! SWE-bench's containerised runs (SPEC §4).

use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::graph::types::SymbolId;
use crate::runner::TestResults;

/// Session state shared across tool handlers. Guarded by `tokio::sync::Mutex`
/// at the wiring layer.
#[derive(Debug, Default)]
pub struct SessionState {
    modified_files: Vec<(PathBuf, Instant)>,
    modified_symbols: Vec<(SymbolId, Instant)>,
    last_test_results: Option<TestResults>,
    session_start: Option<Instant>,
}

impl SessionState {
    #[must_use]
    pub fn new() -> Self {
        Self {
            session_start: Some(Instant::now()),
            ..Self::default()
        }
    }

    pub fn record_file_edit(&mut self, path: &Path) {
        let now = Instant::now();
        self.modified_files.push((path.to_path_buf(), now));
    }

    pub fn record_symbol_edit(&mut self, id: SymbolId) {
        self.modified_symbols.push((id, Instant::now()));
    }

    pub fn record_test_results(&mut self, results: TestResults) {
        self.last_test_results = Some(results);
    }

    #[must_use]
    pub fn modified_symbols(&self) -> &[(SymbolId, Instant)] {
        &self.modified_symbols
    }

    #[must_use]
    pub fn last_test_results(&self) -> Option<&TestResults> {
        self.last_test_results.as_ref()
    }

    /// Elapsed milliseconds since the session was created, or `None` for a
    /// default-constructed session (used by `blastguard://status` resource,
    /// Phase 1.8).
    #[must_use]
    pub fn elapsed_ms(&self) -> Option<u128> {
        self.session_start.map(|t| t.elapsed().as_millis())
    }

    /// Returns how many `apply_change` calls ago the given symbol was modified,
    /// or `None` if the symbol has not been edited in this session.
    ///
    /// `0` means "just edited"; `N` means "N edits happened after this one".
    #[must_use]
    pub fn edits_ago(&self, id: &SymbolId) -> Option<usize> {
        self.modified_symbols
            .iter()
            .rev()
            .position(|(sym, _)| sym == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::types::SymbolKind;

    fn sym(name: &str) -> SymbolId {
        SymbolId {
            file: PathBuf::from("a.ts"),
            name: name.to_string(),
            kind: SymbolKind::Function,
        }
    }

    #[test]
    fn records_edits_in_order() {
        let mut s = SessionState::new();
        s.record_symbol_edit(sym("a"));
        s.record_symbol_edit(sym("b"));
        s.record_symbol_edit(sym("a"));
        assert_eq!(s.modified_symbols().len(), 3);
    }

    #[test]
    fn edits_ago_returns_zero_for_latest() {
        let mut s = SessionState::new();
        s.record_symbol_edit(sym("a"));
        s.record_symbol_edit(sym("b"));
        assert_eq!(s.edits_ago(&sym("b")), Some(0));
    }
}
