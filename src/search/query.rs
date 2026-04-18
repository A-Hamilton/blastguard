//! Query classifier — parses a search query string into a [`QueryKind`]
//! per SPEC §3.1 dispatcher table.
//!
//! Unknown queries fall through to [`QueryKind::Grep`] — the regex grep
//! fallback. The classifier is the only layer that does string parsing;
//! downstream dispatcher arms consume the enum directly.

use std::path::PathBuf;
use std::sync::OnceLock;

use regex::Regex;

/// Parsed query kind. The dispatcher routes on this enum.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueryKind {
    /// `callers of X` / `what calls X` — reverse-edge lookup.
    Callers(String),
    /// `callees of X` / `what does X call` — forward-edge lookup.
    Callees(String),
    /// `outline of FILE` — all symbols declared in a file.
    Outline(PathBuf),
    /// `chain from X to Y` — BFS shortest path.
    Chain(String, String),
    /// `find X` / `where is X` — centrality-ranked name lookup.
    Find(String),
    /// `tests for FILE` or `tests for X` — importers under test paths.
    TestsFor(String),
    /// `imports of FILE` — forward Imports edges.
    ImportsOf(PathBuf),
    /// `importers of FILE` — reverse Imports edges.
    ImportersOf(PathBuf),
    /// `exports of FILE` — visibility-filtered symbols.
    ExportsOf(PathBuf),
    /// `libraries` — grouped `library_imports` with use counts.
    Libraries,
    /// Fallback: treat the whole query as a regex for the grep backend.
    Grep(String),
}

/// Classify a query string per SPEC §3.1 dispatcher table. Trimmed of
/// surrounding whitespace. Falls through to [`QueryKind::Grep`] when no
/// structural pattern matches.
#[must_use]
pub fn classify(query: &str) -> QueryKind {
    let q = query.trim();

    if let Some(caps) = re(r"^(?:callers of|what calls)\s+(.+)$").captures(q) {
        return QueryKind::Callers(caps[1].trim().to_string());
    }
    if let Some(caps) = re(r"^callees of\s+(.+)$").captures(q) {
        return QueryKind::Callees(caps[1].trim().to_string());
    }
    if let Some(caps) = re(r"^what does\s+(.+?)\s+call$").captures(q) {
        return QueryKind::Callees(caps[1].trim().to_string());
    }
    if let Some(caps) = re(r"^outline of\s+(.+)$").captures(q) {
        return QueryKind::Outline(PathBuf::from(caps[1].trim()));
    }
    if let Some(caps) = re(r"^chain from\s+(.+?)\s+to\s+(.+)$").captures(q) {
        return QueryKind::Chain(caps[1].trim().to_string(), caps[2].trim().to_string());
    }
    if let Some(caps) = re(r"^(?:find|where is)\s+(.+)$").captures(q) {
        return QueryKind::Find(caps[1].trim().to_string());
    }
    if let Some(caps) = re(r"^tests for\s+(.+)$").captures(q) {
        return QueryKind::TestsFor(caps[1].trim().to_string());
    }
    if let Some(caps) = re(r"^imports of\s+(.+)$").captures(q) {
        return QueryKind::ImportsOf(PathBuf::from(caps[1].trim()));
    }
    if let Some(caps) = re(r"^importers of\s+(.+)$").captures(q) {
        return QueryKind::ImportersOf(PathBuf::from(caps[1].trim()));
    }
    if let Some(caps) = re(r"^exports of\s+(.+)$").captures(q) {
        return QueryKind::ExportsOf(PathBuf::from(caps[1].trim()));
    }
    if q == "libraries" {
        return QueryKind::Libraries;
    }

    QueryKind::Grep(q.to_string())
}

/// Cache compiled regexes by pattern string. Each pattern is a compile-time
/// constant; once constructed, the leaked `'static` reference lives for the
/// program duration. The leak is bounded: this function is only called with
/// the fixed pattern set embedded in [`classify`].
fn re(pattern: &'static str) -> &'static Regex {
    static CACHE: OnceLock<
        std::sync::Mutex<std::collections::HashMap<&'static str, &'static Regex>>,
    > = OnceLock::new();
    let map =
        CACHE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()));
    // SAFETY: mutex poisoning only occurs if our own code already panicked,
    // which is an unrecoverable state. `expect` is intentional here.
    #[allow(clippy::significant_drop_in_scrutinee)]
    let mut guard = map.lock().expect("regex cache mutex poisoned");
    if let Some(&r) = guard.get(pattern) {
        return r;
    }
    // Hardcoded patterns are validated at author-time; any compile failure is
    // a source-code bug, so `expect` is intentional.
    let leaked: &'static Regex =
        Box::leak(Box::new(Regex::new(pattern).expect("hardcoded regex compiles")));
    guard.insert(pattern, leaked);
    leaked
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn callers_of_pattern() {
        assert_eq!(
            classify("callers of processRequest"),
            QueryKind::Callers("processRequest".into())
        );
    }

    #[test]
    fn what_calls_alias() {
        assert_eq!(
            classify("what calls handler"),
            QueryKind::Callers("handler".into())
        );
    }

    #[test]
    fn callees_of_pattern() {
        assert_eq!(
            classify("callees of foo"),
            QueryKind::Callees("foo".into())
        );
    }

    #[test]
    fn what_does_x_call_alias() {
        assert_eq!(
            classify("what does foo call"),
            QueryKind::Callees("foo".into())
        );
    }

    #[test]
    fn outline_of_file() {
        assert_eq!(
            classify("outline of src/handler.ts"),
            QueryKind::Outline(PathBuf::from("src/handler.ts"))
        );
    }

    #[test]
    fn chain_from_to() {
        assert_eq!(
            classify("chain from a to b"),
            QueryKind::Chain("a".into(), "b".into())
        );
    }

    #[test]
    fn find_and_where_is() {
        assert_eq!(classify("find x"), QueryKind::Find("x".into()));
        assert_eq!(classify("where is y"), QueryKind::Find("y".into()));
    }

    #[test]
    fn tests_for_file_or_symbol() {
        assert_eq!(
            classify("tests for src/a.ts"),
            QueryKind::TestsFor("src/a.ts".into())
        );
        assert_eq!(
            classify("tests for processRequest"),
            QueryKind::TestsFor("processRequest".into())
        );
    }

    #[test]
    fn imports_and_importers() {
        assert_eq!(
            classify("imports of src/a.ts"),
            QueryKind::ImportsOf(PathBuf::from("src/a.ts"))
        );
        assert_eq!(
            classify("importers of src/a.ts"),
            QueryKind::ImportersOf(PathBuf::from("src/a.ts"))
        );
    }

    #[test]
    fn exports_of_file() {
        assert_eq!(
            classify("exports of src/a.ts"),
            QueryKind::ExportsOf(PathBuf::from("src/a.ts"))
        );
    }

    #[test]
    fn libraries_keyword() {
        assert_eq!(classify("libraries"), QueryKind::Libraries);
    }

    #[test]
    fn unknown_falls_through_to_grep() {
        assert_eq!(
            classify("some random regex pattern [a-z]+"),
            QueryKind::Grep("some random regex pattern [a-z]+".into())
        );
    }

    #[test]
    fn leading_trailing_whitespace_trimmed() {
        assert_eq!(
            classify("  callers of foo  "),
            QueryKind::Callers("foo".into())
        );
    }

    #[test]
    fn callers_of_whitespace_in_arg() {
        // Argument may contain spaces (rare — probably a quoted symbol name in
        // real usage). Captured as-is after trimming.
        assert_eq!(
            classify("callers of some symbol"),
            QueryKind::Callers("some symbol".into())
        );
    }
}
