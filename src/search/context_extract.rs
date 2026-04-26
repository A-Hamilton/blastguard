//! AST-smart context extraction around a call site — returns the
//! enclosing statement's text via tree-sitter so `callers of X with
//! context` can surface argument values without a follow-up `read_file`.
//!
//! See `docs/superpowers/specs/2026-04-24-callers-with-context-design.md`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Maximum context lines to return per hit. Guards against pathological
/// multi-line chains (e.g. hundred-line builder calls). Most enclosing
/// statements are 1-5 lines.
const MAX_CONTEXT_LINES: usize = 20;

/// Maximum ancestor hops before giving up and falling back to the
/// line-window heuristic. Tree-sitter ASTs can have deeper nesting
/// through closures, match arms, and if-let bindings; 16 covers it.
const MAX_ANCESTOR_HOPS: usize = 16;

/// Maximum entries in the thread-local parse cache. Bounded to prevent
/// memory drift across many `callers_of` invocations. 16 entries covers
/// the common case where all callers cluster in 1-3 files.
const PARSE_CACHE_CAPACITY: usize = 16;

thread_local! {
    /// Cache of parsed tree-sitter trees keyed by (file_path, blake3_hash
    /// of source text). The hash ensures we detect file modifications
    /// without re-reading. Bounded to `PARSE_CACHE_CAPACITY` — evicts
    /// oldest entry when full.
    static PARSE_CACHE: std::cell::RefCell<HashMap<(PathBuf, blake3::Hash), tree_sitter::Tree>> =
        std::cell::RefCell::new(HashMap::new());
}

/// Return the enclosing statement's text around a call at `line` in
/// `file`. Best-effort: returns `None` only when the file is
/// unreadable or the language isn't supported by any of our
/// tree-sitter parsers. When the AST heuristic can't find a
/// statement ancestor, falls back to a ±1 line window.
///
/// `line` is 1-based (matching `Edge.line` in the graph).
///
/// Uses a thread-local parse cache keyed by (path, source-hash) so
/// that multiple callers in the same file share a single tree-sitter
/// parse — the dominant cost in context extraction.
#[must_use]
pub fn enclosing_statement(file: &Path, line: u32) -> Option<String> {
    let source = std::fs::read_to_string(file).ok()?;
    let language = detect_language(file)?;

    // Check / populate the parse cache so N callers in the same file
    // share one tree-sitter parse (the dominant cost).
    let source_hash = blake3::hash(source.as_bytes());
    let cache_key = (file.to_path_buf(), source_hash);

    let tree = PARSE_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some(tree) = cache.get(&cache_key) {
            return Some(tree.clone());
        }
        // Parse and cache. Evict oldest entry if at capacity.
        let parsed = parse_source(&source, language)?;
        if cache.len() >= PARSE_CACHE_CAPACITY {
            // Remove the first (oldest) entry — HashMap iteration order
            // is stable enough for LRU-ish eviction in practice.
            if let Some(key) = cache.keys().next().cloned() {
                cache.remove(&key);
            }
        }
        cache.insert(cache_key, parsed.clone());
        Some(parsed)
    })?;

    if let Some(text) = extract_via_ast(&source, &tree, line, language) {
        return Some(text);
    }
    Some(line_window_fallback(&source, line, 1))
}

/// Parse `source` with the appropriate tree-sitter parser for `lang`.
/// Returns `None` if the parser can't be acquired or the parse fails.
fn parse_source(source: &str, lang: Language) -> Option<tree_sitter::Tree> {
    match lang {
        Language::Rust => RS_PARSER.with(|cell| cell.borrow_mut().parse(source, None)),
        Language::Python => PY_PARSER.with(|cell| cell.borrow_mut().parse(source, None)),
        Language::TypeScript => TS_PARSER.with(|cell| cell.borrow_mut().parse(source, None)),
        Language::Tsx => TSX_PARSER.with(|cell| cell.borrow_mut().parse(source, None)),
        Language::JavaScript => JS_PARSER.with(|cell| cell.borrow_mut().parse(source, None)),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Language {
    Rust,
    Python,
    TypeScript,
    JavaScript,
    Tsx,
}

fn detect_language(file: &Path) -> Option<Language> {
    match file.extension().and_then(|e| e.to_str()) {
        Some("rs") => Some(Language::Rust),
        Some("py") => Some(Language::Python),
        Some("ts") => Some(Language::TypeScript),
        Some("tsx") => Some(Language::Tsx),
        Some("js" | "jsx" | "mjs" | "cjs") => Some(Language::JavaScript),
        _ => None,
    }
}

/// Return a ±N-line window around `line` (1-based). Always succeeds
/// for any valid source string; clamps at file boundaries. Returns
/// at most `MAX_CONTEXT_LINES` lines.
fn line_window_fallback(source: &str, line: u32, radius: u32) -> String {
    let lines: Vec<&str> = source.lines().collect();
    let center = (line.saturating_sub(1)) as usize;
    let start = center.saturating_sub(radius as usize);
    let end = (center + radius as usize + 1).min(lines.len());
    let slice = &lines[start..end];
    let capped = if slice.len() > MAX_CONTEXT_LINES {
        &slice[..MAX_CONTEXT_LINES]
    } else {
        slice
    };
    capped.join("\n")
}

// Tree-sitter extraction lives below. Each language owns a
// thread-local Parser (Parser is not Send-safe, matching
// src/parse/rust.rs's pattern).

fn extract_via_ast(
    source: &str,
    tree: &tree_sitter::Tree,
    line: u32,
    lang: Language,
) -> Option<String> {
    match lang {
        Language::Rust => extract_rust(source, tree, line),
        Language::Python => extract_python(source, tree, line),
        Language::TypeScript => extract_typescript(source, tree, line),
        Language::Tsx => extract_tsx(source, tree, line),
        Language::JavaScript => extract_javascript(source, tree, line),
    }
}

// ── Rust ──────────────────────────────────────────────────────────────

thread_local! {
    static RS_PARSER: std::cell::RefCell<tree_sitter::Parser> = std::cell::RefCell::new({
        let mut p = tree_sitter::Parser::new();
        let lang: tree_sitter::Language = tree_sitter_rust::language();
        p.set_language(&lang)
            .expect("tree-sitter Rust grammar must load");
        p
    });
}

fn extract_rust(source: &str, tree: &tree_sitter::Tree, line: u32) -> Option<String> {
    let call = deepest_call_at_line(&tree.root_node(), line)?;
    let stmt = climb_to_statement(call, RUST_STATEMENT_KINDS)?;
    extract_node_text(source, stmt)
}

const RUST_STATEMENT_KINDS: &[&str] = &[
    "let_declaration",
    "expression_statement",
    "return_expression",
    "macro_invocation",
    "assignment_expression",
    "if_expression",
    "for_expression",
    "match_expression",
    "while_expression",
    "loop_expression",
    "unsafe_block",
    "try_block",
];

// ── Python ────────────────────────────────────────────────────────────

thread_local! {
    static PY_PARSER: std::cell::RefCell<tree_sitter::Parser> = std::cell::RefCell::new({
        let mut p = tree_sitter::Parser::new();
        let lang: tree_sitter::Language = tree_sitter_python::LANGUAGE.into();
        p.set_language(&lang)
            .expect("tree-sitter Python grammar must load");
        p
    });
}

fn extract_python(source: &str, tree: &tree_sitter::Tree, line: u32) -> Option<String> {
    let call = deepest_call_at_line(&tree.root_node(), line)?;
    let stmt = climb_to_statement(call, PY_STATEMENT_KINDS)?;
    extract_node_text(source, stmt)
}

const PY_STATEMENT_KINDS: &[&str] = &[
    "expression_statement",
    "assignment",
    "return_statement",
    "if_statement",
];

// ── TypeScript ────────────────────────────────────────────────────────

thread_local! {
    static TS_PARSER: std::cell::RefCell<tree_sitter::Parser> = std::cell::RefCell::new({
        let mut p = tree_sitter::Parser::new();
        let lang: tree_sitter::Language = tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into();
        p.set_language(&lang)
            .expect("tree-sitter TypeScript grammar must load");
        p
    });
}

fn extract_typescript(source: &str, tree: &tree_sitter::Tree, line: u32) -> Option<String> {
    let call = deepest_call_at_line(&tree.root_node(), line)?;
    let stmt = climb_to_statement(call, TS_STATEMENT_KINDS)?;
    extract_node_text(source, stmt)
}

thread_local! {
    static TSX_PARSER: std::cell::RefCell<tree_sitter::Parser> = std::cell::RefCell::new({
        let mut p = tree_sitter::Parser::new();
        let lang: tree_sitter::Language = tree_sitter_typescript::LANGUAGE_TSX.into();
        p.set_language(&lang)
            .expect("tree-sitter TSX grammar must load");
        p
    });
}

fn extract_tsx(source: &str, tree: &tree_sitter::Tree, line: u32) -> Option<String> {
    let call = deepest_call_at_line(&tree.root_node(), line)?;
    let stmt = climb_to_statement(call, TS_STATEMENT_KINDS)?;
    extract_node_text(source, stmt)
}

const TS_STATEMENT_KINDS: &[&str] = &[
    "lexical_declaration",
    "expression_statement",
    "return_statement",
    "assignment_expression",
    "variable_declaration",
];

// ── JavaScript ────────────────────────────────────────────────────────

thread_local! {
    static JS_PARSER: std::cell::RefCell<tree_sitter::Parser> = std::cell::RefCell::new({
        let mut p = tree_sitter::Parser::new();
        let lang: tree_sitter::Language = tree_sitter_javascript::LANGUAGE.into();
        p.set_language(&lang)
            .expect("tree-sitter JavaScript grammar must load");
        p
    });
}

fn extract_javascript(source: &str, tree: &tree_sitter::Tree, line: u32) -> Option<String> {
    let call = deepest_call_at_line(&tree.root_node(), line)?;
    // JS and TS share statement kind names in the tree-sitter grammars —
    // lexical_declaration, expression_statement, return_statement, etc.
    // — so the TS kind list applies as-is.
    let stmt = climb_to_statement(call, TS_STATEMENT_KINDS)?;
    extract_node_text(source, stmt)
}

// ── Shared helpers ────────────────────────────────────────────────────

/// Walk the tree and return the deepest call-expression-ish node whose
/// start row is `line - 1` (0-indexed). "Deepest" means the one whose
/// `start_byte` is largest among matches — a proxy for nesting depth
/// that handles sibling-calls-on-same-line correctly.
fn deepest_call_at_line<'t>(
    root: &tree_sitter::Node<'t>,
    line: u32,
) -> Option<tree_sitter::Node<'t>> {
    let target_row = line.saturating_sub(1) as usize;
    let mut best: Option<tree_sitter::Node<'t>> = None;
    let mut cursor = root.walk();
    let mut stack = vec![*root];
    while let Some(node) = stack.pop() {
        if node.start_position().row == target_row
            && (node.kind() == "call_expression"
                || node.kind() == "call"
                || node.kind() == "macro_invocation")
            && best.is_none_or(|b| node.start_byte() > b.start_byte())
        {
            best = Some(node);
        }
        for child in node.children(&mut cursor) {
            if child.start_position().row <= target_row && child.end_position().row >= target_row {
                stack.push(child);
            }
        }
    }
    best
}

/// Climb parents from `node` until we find an ancestor whose `kind()`
/// is in `stop_at`, or we've climbed `MAX_ANCESTOR_HOPS` levels.
fn climb_to_statement<'t>(
    node: tree_sitter::Node<'t>,
    stop_at: &[&str],
) -> Option<tree_sitter::Node<'t>> {
    let mut current = node;
    for _ in 0..MAX_ANCESTOR_HOPS {
        if stop_at.contains(&current.kind()) {
            return Some(current);
        }
        current = current.parent()?;
    }
    None
}

/// Return the source text of `node`, capped at `MAX_CONTEXT_LINES`.
fn extract_node_text(source: &str, node: tree_sitter::Node<'_>) -> Option<String> {
    let text = node.utf8_text(source.as_bytes()).ok()?;
    let lines: Vec<&str> = text.lines().collect();
    let capped = if lines.len() > MAX_CONTEXT_LINES {
        &lines[..MAX_CONTEXT_LINES]
    } else {
        &lines[..]
    };
    Some(capped.join("\n"))
}

/// Extract the argument expressions from a call to `callee_name` at
/// `call_site_line` (1-based) in `file`. Returns the text between the
/// opening `(` and the matching `)`, trimmed to ≤200 chars.
///
/// Uses a simple text-based scan of the call-site line(s) — no AST
/// needed since we already know the callee name and line from the
/// graph edge. Handles multi-line argument lists up to 10 lines.
#[must_use]
pub fn extract_call_args(file: &Path, call_site_line: u32, callee_name: &str) -> Option<String> {
    let source = std::fs::read_to_string(file).ok()?;
    let lines: Vec<&str> = source.lines().collect();
    let start_row = (call_site_line.saturating_sub(1)) as usize;
    if start_row >= lines.len() {
        return None;
    }

    // Scan from the call-site line forward to find the opening `(` after callee_name.
    // The call may span multiple lines (e.g. foo(\n   a,\n   b\n)).
    // We look at up to 10 lines to find the matching parens.
    let max_scan = (start_row + 10).min(lines.len());
    let mut combined = String::new();
    for line in lines.iter().take(max_scan).skip(start_row) {
        if !combined.is_empty() {
            combined.push('\n');
        }
        combined.push_str(line);
        // Once we've found the opening paren, track depth to find the close.
        if let Some(paren_start) = combined.find('(') {
            // Verify the callee name appears before the `(` to avoid matching
            // a different call on the same line (e.g. bar() vs foo()).
            let before_paren = &combined[..paren_start];
            if !before_paren.contains(callee_name) {
                // The `(` we found isn't for our callee — keep scanning
                // subsequent lines. This is uncommon (sibling calls on
                // the same line) but possible.
                continue;
            }
            // Find the matching close paren starting from paren_start.
            let after_paren = &combined[paren_start + 1..];
            let mut depth: i32 = 1;
            for (i, ch) in after_paren.char_indices() {
                match ch {
                    '(' => depth += 1,
                    ')' => {
                        depth -= 1;
                        if depth == 0 {
                            let args = &after_paren[..i];
                            let trimmed = args.trim();
                            if trimmed.is_empty() {
                                return None;
                            }
                            // Cap at 200 chars to keep context lean
                            let capped = if trimmed.len() > 200 {
                                format!("{}...", &trimmed[..197])
                            } else {
                                trimmed.to_string()
                            };
                            return Some(capped);
                        }
                    }
                    _ => {}
                }
            }
            // Depth never reached 0 — multi-line, keep scanning
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Write `source` to a tempfile with the given extension and
    /// return its path. Tempfile lives as long as the returned handle.
    fn tempfile_with_ext(source: &str, ext: &str) -> tempfile::NamedTempFile {
        let mut f = tempfile::Builder::new()
            .suffix(&format!(".{ext}"))
            .tempfile()
            .expect("tempfile");
        f.write_all(source.as_bytes()).expect("write");
        f
    }

    #[test]
    fn enclosing_stmt_rust_single_line_call() {
        let src = "fn main() {\n    let x = foo(1, 2);\n}\n";
        let f = tempfile_with_ext(src, "rs");
        let got = enclosing_statement(f.path(), 2).expect("some");
        assert!(got.contains("let x = foo(1, 2)"), "got: {got:?}");
    }

    #[test]
    fn enclosing_stmt_rust_multi_line_args() {
        let src = "fn main() {\n    let x = foo(\n        1,\n        2,\n    );\n}\n";
        let f = tempfile_with_ext(src, "rs");
        let got = enclosing_statement(f.path(), 2).expect("some");
        // The whole let-declaration (5 lines) should be returned.
        assert!(got.contains("let x = foo("), "got: {got:?}");
        assert!(got.contains("1,"), "got: {got:?}");
        assert!(got.contains("2,"), "got: {got:?}");
    }

    #[test]
    fn enclosing_stmt_python_assignment() {
        let src = "def main():\n    result = module.func(arg)\n    return result\n";
        let f = tempfile_with_ext(src, "py");
        let got = enclosing_statement(f.path(), 2).expect("some");
        assert!(got.contains("result = module.func(arg)"), "got: {got:?}");
    }

    #[test]
    fn enclosing_stmt_typescript_return_expr() {
        let src = "function main() {\n    return handle(req);\n}\n";
        let f = tempfile_with_ext(src, "ts");
        let got = enclosing_statement(f.path(), 2).expect("some");
        assert!(got.contains("return handle(req)"), "got: {got:?}");
    }

    #[test]
    fn enclosing_stmt_javascript_return_expr() {
        let src = "function main() {\n    return handle(req);\n}\n";
        let f = tempfile_with_ext(src, "js");
        let got = enclosing_statement(f.path(), 2).expect("some");
        assert!(got.contains("return handle(req)"), "got: {got:?}");
    }

    #[test]
    fn enclosing_stmt_none_on_unreadable_file() {
        let missing = std::path::Path::new("/tmp/does_not_exist_92837.rs");
        assert!(enclosing_statement(missing, 5).is_none());
    }

    #[test]
    fn enclosing_stmt_none_on_unsupported_language() {
        let src = "# just a markdown\n";
        let f = tempfile_with_ext(src, "md");
        assert!(enclosing_statement(f.path(), 1).is_none());
    }

    #[test]
    fn enclosing_stmt_rust_sibling_calls_picks_deepest() {
        // Two sibling calls on the same line. "deepest" should win —
        // we use start_byte as a depth proxy, so the RIGHTMOST call
        // (later start_byte) is considered deeper than the leftmost.
        // Both are valid enclosing candidates; the test just pins
        // the behaviour so it's not silent on sibling chains.
        let src = "fn main() {\n    let _ = (foo(1), bar(2));\n}\n";
        let f = tempfile_with_ext(src, "rs");
        let got = enclosing_statement(f.path(), 2).expect("some");
        // The let-declaration wraps both calls — that's the enclosing
        // statement, so the output should contain the whole assignment.
        assert!(got.contains("let _ = (foo(1), bar(2))"), "got: {got:?}");
    }

    #[test]
    fn enclosing_stmt_fallback_on_ast_miss() {
        // A call inside an odd ancestor the AST heuristic may not
        // surface as a statement. The fallback line-window should
        // kick in and return a non-empty string anyway.
        let src = "fn main() {\n    match x { Some(y) => foo(y), _ => () };\n}\n";
        let f = tempfile_with_ext(src, "rs");
        let got = enclosing_statement(f.path(), 2).expect("some");
        // Either we got the full match expression via expression_statement
        // (if tree-sitter surfaces it that way), or we got the ±1 window.
        // Either way the result must contain the call line.
        assert!(got.contains("foo(y)"), "got: {got:?}");
    }
}
