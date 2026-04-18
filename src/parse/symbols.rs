//! Shared symbol-extraction helpers across language drivers.
//!
//! Phase 1.2 holds this thin. Each language module in [`super`] is responsible
//! for its own tree-sitter queries and emits [`super::ParseOutput`]. Helpers
//! here avoid duplicating boilerplate across drivers.

// TODO(phase-1.2): signature formatter that joins param tokens with commas
// and a trailing `-> Ret` when present. Must be language-agnostic enough to
// render inline in search results.
