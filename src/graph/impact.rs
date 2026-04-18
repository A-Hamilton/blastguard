//! Cascade impact analysis — SPEC §5.
//!
//! Phase 1 ships four warnings: `SIGNATURE`, `ASYNC_CHANGE`, `ORPHAN`, and
//! `INTERFACE_BREAK`. Each fires from a single structural check; the aim is
//! high signal, not exhaustiveness (SPEC §5.2: agents ignore noisy output).

use serde::{Deserialize, Serialize};

use crate::graph::types::SymbolId;

/// Cascade warning kind. Serialised as an uppercase tag in the MCP response.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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

// TODO(phase-1.6): implement diff_signatures, detect_async_flip, detect_orphan,
// detect_interface_break. Each consumes an `old: &CodeGraph` and a
// `new_symbol: &Symbol` (with the graph mutation not yet applied).
