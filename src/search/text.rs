//! Regex grep fallback via the `ignore` crate and `regex`.
//!
//! Caps at 30 matches (SPEC §3.1 grep row). Honours `.gitignore`. Always
//! returns `<file:line>` plus the matching line as `snippet`.

// TODO(phase-1.5): grep(query, project_root) -> Vec<SearchHit>.
