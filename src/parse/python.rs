//! Python driver — tree-sitter-python.
//!
//! Phase 1.2 emits function/async-function/class/method symbols, plus
//! `LibraryImport` records for all `import X` and `from X import Y` statements.
//! All imports are treated as library imports here; Task 10 will re-classify
//! internal imports (those with a `library` segment matching a project package)
//! into `Imports` edges by doing a filesystem lookup.
//!
//! Call expressions inside a function or method produce `Calls` edges (intra-file
//! only; cross-file resolution is Task 13).

use std::collections::HashSet;
use std::path::Path;

use streaming_iterator::StreamingIterator as _;
use tree_sitter::{Language, Parser, Query, QueryCursor};

use crate::graph::types::{
    Confidence, Edge, EdgeKind, LibraryImport, Symbol, SymbolId, SymbolKind, Visibility,
};
use crate::parse::body_hash::body_hash;
use crate::parse::symbols::render_signature;
use crate::parse::ParseOutput;

/// Source of the tree-sitter query embedded at compile time.
const QUERY_SRC: &str = include_str!("../../queries/python.scm");

thread_local! {
    /// Parser and compiled query per thread. Both are not `Send` so this is
    /// the correct place to own them for rayon-parallel indexing.
    static PY_STATE: std::cell::RefCell<(Parser, Query)> = std::cell::RefCell::new({
        let mut parser = Parser::new();
        let lang: Language = tree_sitter_python::LANGUAGE.into();
        // Unreachable in a correctly-built binary — the grammar crate is a
        // compile-time constant and the query is validated by `cargo test`.
        parser.set_language(&lang).expect("tree-sitter Python grammar must load");
        let query = Query::new(&lang, QUERY_SRC).expect("python.scm must be a valid query");
        (parser, query)
    });
}

/// Parse a Python source file and return symbols, external library
/// imports, and intra-file `Calls` edges (cross-file resolution is Task 13).
///
/// All import statements (`import X`, `from X import Y`) emit a
/// [`LibraryImport`] with `library` set to the first dotted segment of the
/// module path. Task 10's import resolver will promote those whose first segment
/// matches a project package to `Imports` edges.
///
/// Returns a default `ParseOutput` with `partial_parse = true` when
/// tree-sitter cannot produce a tree at all. When ERROR nodes are present,
/// extraction continues over successfully-parsed regions and
/// `partial_parse` is set.
///
/// # Panics
///
/// Panics if `queries/python.scm` contains a syntax error (compile-time
/// bug — the `.scm` file is embedded and validated at start-up, so this is
/// unreachable in a correctly-built binary).
#[must_use = "parsed symbols and imports should be ingested into the graph"]
pub fn extract(path: &Path, source: &str) -> ParseOutput {
    PY_STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        let (parser, query) = &mut *state;
        let Some(tree) = parser.parse(source, None) else {
            return ParseOutput {
                partial_parse: true,
                ..ParseOutput::default()
            };
        };
        let root = tree.root_node();
        let partial_parse = root.has_error();

        let mut cursor = QueryCursor::new();
        let mut out = ParseOutput {
            partial_parse,
            ..ParseOutput::default()
        };
        // Dedup Calls edges within this single extract() pass. Tracks
        // (from_name, to_name) pairs so a function calling the same callee
        // multiple times produces exactly one edge.
        let mut seen_calls: HashSet<(String, String)> = HashSet::new();

        let src_bytes = source.as_bytes();
        let mut matches = cursor.matches(query, root, src_bytes);
        while let Some(m) = matches.next() {
            for capture in m.captures {
                let capture_name = query.capture_names()[capture.index as usize];
                let node = capture.node;
                match capture_name {
                    "function.decl" => emit_function(node, source, path, &mut out),
                    "class.decl" => emit_simple(node, source, path, &mut out, SymbolKind::Class),
                    "import.module" | "import.from" => {
                        let text = node.utf8_text(src_bytes).unwrap_or("");
                        emit_import(text, node, path, &mut out);
                    }
                    "call.callee" => {
                        emit_call(node, source, path, &mut out, &mut seen_calls);
                    }
                    // All other captures (name helper nodes, call.site, etc.)
                    // are intentionally ignored here.
                    _ => {}
                }
            }
        }
        out
    })
}

fn emit_function(
    node: tree_sitter::Node<'_>,
    source: &str,
    path: &Path,
    out: &mut ParseOutput,
) {
    let src_bytes = source.as_bytes();
    let Some(name_node) = node.child_by_field_name("name") else {
        return;
    };
    let name = name_node.utf8_text(src_bytes).unwrap_or("").to_owned();
    if name.is_empty() {
        return;
    }

    let is_async = first_child_text_is(node, source, "async");

    // Determine if this function_definition is a method (its ancestor chain
    // hits `class_definition` before reaching `module`).
    let method = is_method(node);

    let kind = if method {
        // There is no AsyncMethod variant — async methods use Method + is_async.
        SymbolKind::Method
    } else if is_async {
        SymbolKind::AsyncFunction
    } else {
        SymbolKind::Function
    };

    let visibility = python_visibility(&name);

    let params_text = node
        .child_by_field_name("parameters")
        .map(|n| n.utf8_text(src_bytes).unwrap_or("").to_owned())
        .unwrap_or_default();

    // Python has no return type annotation syntax we parse here.
    let signature = render_signature(&name, &params_text, None);
    let body_text = node.utf8_text(src_bytes).unwrap_or("");
    let line_start = u32::try_from(node.start_position().row)
        .unwrap_or(u32::MAX)
        .saturating_add(1);
    let line_end = u32::try_from(node.end_position().row)
        .unwrap_or(u32::MAX)
        .saturating_add(1);

    out.symbols.push(Symbol {
        id: SymbolId {
            file: path.to_path_buf(),
            name,
            kind,
        },
        line_start,
        line_end,
        signature,
        params: split_params(&params_text),
        return_type: None,
        visibility,
        body_hash: body_hash(body_text),
        is_async,
        embedding_id: None,
    });
}

fn emit_simple(
    node: tree_sitter::Node<'_>,
    source: &str,
    path: &Path,
    out: &mut ParseOutput,
    kind: SymbolKind,
) {
    let src_bytes = source.as_bytes();
    let Some(name_node) = node.child_by_field_name("name") else {
        return;
    };
    let name = name_node.utf8_text(src_bytes).unwrap_or("").to_owned();
    if name.is_empty() {
        return;
    }
    let visibility = python_visibility(&name);
    let body_text = node.utf8_text(src_bytes).unwrap_or("");
    let signature = name.clone();
    let line_start = u32::try_from(node.start_position().row)
        .unwrap_or(u32::MAX)
        .saturating_add(1);
    let line_end = u32::try_from(node.end_position().row)
        .unwrap_or(u32::MAX)
        .saturating_add(1);

    out.symbols.push(Symbol {
        id: SymbolId {
            file: path.to_path_buf(),
            name,
            kind,
        },
        line_start,
        line_end,
        signature,
        params: Vec::new(),
        return_type: None,
        visibility,
        body_hash: body_hash(body_text),
        is_async: false,
        embedding_id: None,
    });
}

/// Emit a [`LibraryImport`] for either an `import X` or `from X import Y`
/// statement. `module_path` is the dotted name or relative import text
/// (e.g. `"utils.auth"`, `"os.path"`, `"."`, `"..utils"`).
///
/// For relative imports (those beginning with `.`), leading dots are stripped
/// before extracting the first segment:
/// - `"."` or `".."` (no module name after the dots) → `library = "."` sentinel.
///   Task 10 will resolve relative to the file's directory.
/// - `"..utils"` → `library = "utils"` (first segment after stripping dots).
///
/// Task 10 re-classifies entries whose first segment resolves to a project
/// package as internal `Imports` edges.
fn emit_import(
    module_path: &str,
    node: tree_sitter::Node<'_>,
    path: &Path,
    out: &mut ParseOutput,
) {
    if module_path.is_empty() {
        return;
    }
    // Strip leading dots for relative imports — the number of dots indicates
    // how many parent packages to walk up. Task 10 resolves the full path.
    let stripped = module_path.trim_start_matches('.');
    let library = if stripped.is_empty() {
        // `from . import foo` or `from .. import foo` — sibling/parent import
        // with no module-name component.
        ".".to_owned()
    } else {
        stripped.split('.').next().unwrap_or(stripped).to_owned()
    };
    let line = u32::try_from(node.start_position().row)
        .unwrap_or(u32::MAX)
        .saturating_add(1);

    out.library_imports.push(LibraryImport {
        library,
        symbol: String::new(),
        file: path.to_path_buf(),
        line,
    });
}

/// Emit a [`Calls`] edge when a call expression's callee lies inside a named
/// function or method. Module-scope calls are silently dropped — there is no
/// meaningful `from` symbol to attribute them to.
///
/// `seen` tracks `(from_name, callee_name)` pairs so repeated calls to the
/// same callee within one `extract()` pass produce exactly one edge.
fn emit_call(
    callee_node: tree_sitter::Node<'_>,
    source: &str,
    path: &Path,
    out: &mut ParseOutput,
    seen: &mut HashSet<(String, String)>,
) {
    let src_bytes = source.as_bytes();
    let Ok(callee_name) = callee_node.utf8_text(src_bytes) else {
        return;
    };
    if callee_name.is_empty() {
        return;
    }
    let Some((from_name, from_kind)) = enclosing_fn(callee_node, source) else {
        // Call at module scope — nothing to attribute it to.
        return;
    };

    // Dedup: skip if we have already emitted this (from, to) pair.
    if !seen.insert((from_name.clone(), callee_name.to_owned())) {
        return;
    }

    let line = u32::try_from(callee_node.start_position().row)
        .unwrap_or(u32::MAX)
        .saturating_add(1);

    out.edges.push(Edge {
        from: SymbolId {
            file: path.to_path_buf(),
            name: from_name,
            kind: from_kind,
        },
        to: SymbolId {
            file: path.to_path_buf(),
            name: callee_name.to_owned(),
            // Callee's real kind is a placeholder until cross-file resolution (Task 13).
            kind: SymbolKind::Function,
        },
        kind: EdgeKind::Calls,
        line,
        confidence: Confidence::Unresolved,
    });
}

/// Walk up the ancestor chain to find the nearest enclosing
/// `function_definition`. Returns `None` for calls at module scope.
///
/// Python has no arrow-function equivalent. Every `function_definition` in the
/// chain is a valid enclosing scope; we return the nearest one. The kind is
/// determined by:
/// - `function_definition` whose ancestor chain hits `class_definition` before
///   `module` → `Method`
/// - Otherwise → `Function` or `AsyncFunction` based on the `async` keyword.
fn enclosing_fn(
    mut node: tree_sitter::Node<'_>,
    source: &str,
) -> Option<(String, SymbolKind)> {
    while let Some(parent) = node.parent() {
        if parent.kind() == "function_definition" {
            let name = parent
                .child_by_field_name("name")?
                .utf8_text(source.as_bytes())
                .ok()?
                .to_owned();
            let kind = if is_method(parent) {
                SymbolKind::Method
            } else if first_child_text_is(parent, source, "async") {
                SymbolKind::AsyncFunction
            } else {
                SymbolKind::Function
            };
            return Some((name, kind));
        }
        node = parent;
    }
    None
}

/// Return `true` if `node` (a `function_definition`) is a method — that is,
/// its ancestor chain hits `class_definition` before `module` or another
/// `function_definition`.
///
/// A `function_definition` encountered on the way up means the current node is
/// a local function nested inside another function, not a class method — stop
/// and return `false`.
fn is_method(node: tree_sitter::Node<'_>) -> bool {
    let mut current = node;
    while let Some(parent) = current.parent() {
        match parent.kind() {
            "class_definition" => return true,
            // `function_definition` means we are nested inside another function,
            // not directly inside a class — stop and report not-a-method.
            // `module` is the top-level sentinel.
            "function_definition" | "module" => return false,
            _ => {}
        }
        current = parent;
    }
    false
}

/// Return `true` if any direct child token's text matches `needle`.
fn first_child_text_is(node: tree_sitter::Node<'_>, source: &str, needle: &str) -> bool {
    let src_bytes = source.as_bytes();
    let mut walker = node.walk();
    for child in node.children(&mut walker) {
        if child.utf8_text(src_bytes).unwrap_or("") == needle {
            return true;
        }
    }
    false
}

/// Compute Python visibility from the symbol name:
/// - `__dunder__` (starts and ends with `__`) → `Export`
/// - starts with `__` but not dunder (name-mangled) → `Private`
/// - starts with `_` → `Private`
/// - otherwise → `Export`
fn python_visibility(name: &str) -> Visibility {
    if name.starts_with("__") && name.ends_with("__") {
        Visibility::Export
    } else if name.starts_with('_') {
        Visibility::Private
    } else {
        Visibility::Export
    }
}

/// Split a parameter list string like `"(self, req)"` into individual
/// parameter strings, stripping surrounding parentheses.
fn split_params(params: &str) -> Vec<String> {
    params
        .trim_start_matches('(')
        .trim_end_matches(')')
        .split(',')
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::types::{EdgeKind, SymbolKind, Visibility};
    use std::path::PathBuf;

    const SAMPLE: &str = r"
import os
from utils.auth import verify

async def process_request(req):
    return handler(req)

class Handler:
    def handle(self, req):
        return process_request(req)

def _private_helper():
    pass
";

    #[test]
    fn extracts_async_def_as_async_function() {
        let out = extract(&PathBuf::from("src/handler.py"), SAMPLE);
        let sym = out.symbols.iter().find(|s| s.id.name == "process_request")
            .expect("process_request missing");
        assert_eq!(sym.id.kind, SymbolKind::AsyncFunction);
        assert!(sym.is_async);
    }

    #[test]
    fn extracts_class_and_method() {
        let out = extract(&PathBuf::from("src/handler.py"), SAMPLE);
        assert!(out.symbols.iter().any(|s| s.id.name == "Handler" && s.id.kind == SymbolKind::Class));
        assert!(out.symbols.iter().any(|s| s.id.name == "handle" && s.id.kind == SymbolKind::Method));
    }

    #[test]
    fn visibility_detected_from_underscore_prefix() {
        let out = extract(&PathBuf::from("src/handler.py"), SAMPLE);
        let priv_sym = out.symbols.iter().find(|s| s.id.name == "_private_helper")
            .expect("_private_helper missing");
        assert_eq!(priv_sym.visibility, Visibility::Private);
        let pub_sym = out.symbols.iter().find(|s| s.id.name == "process_request")
            .expect("process_request missing");
        assert_eq!(pub_sym.visibility, Visibility::Export);
    }

    #[test]
    fn imports_captured_as_library_imports() {
        let out = extract(&PathBuf::from("src/handler.py"), SAMPLE);
        // `import os` and `from utils.auth import verify` both land in library_imports
        // keyed by the first dotted segment. Task 10 will re-classify internals.
        assert!(out.library_imports.iter().any(|li| li.library == "os"));
        assert!(out.library_imports.iter().any(|li| li.library == "utils"));
    }

    #[test]
    fn intra_file_call_produces_calls_edge() {
        let out = extract(&PathBuf::from("src/handler.py"), SAMPLE);
        let has_call = out.edges.iter().any(|e|
            e.kind == EdgeKind::Calls
                && e.from.name == "process_request"
                && e.to.name == "handler"
        );
        assert!(has_call, "expected process_request -> handler edge; edges = {:?}", out.edges);
    }

    #[test]
    fn method_call_has_method_from_kind() {
        let out = extract(&PathBuf::from("src/handler.py"), SAMPLE);
        // Handler.handle calls process_request. from.kind should be Method.
        let has_call = out.edges.iter().any(|e|
            e.kind == EdgeKind::Calls
                && e.from.name == "handle"
                && e.from.kind == SymbolKind::Method
                && e.to.name == "process_request"
        );
        assert!(has_call, "expected handle (Method) -> process_request edge; edges = {:?}", out.edges);
    }

    #[test]
    fn repeated_calls_dedup() {
        let src = r"
def retry():
    helper()
    helper()
    helper()
";
        let out = extract(&PathBuf::from("src/retry.py"), src);
        let calls: Vec<_> = out.edges.iter().filter(|e|
            e.kind == EdgeKind::Calls
                && e.from.name == "retry"
                && e.to.name == "helper"
        ).collect();
        assert_eq!(calls.len(), 1, "expected 1 dedup'd edge, got {calls:?}");
    }

    #[test]
    fn dunder_method_is_export_visibility() {
        let src = r#"
class Foo:
    def __init__(self):
        pass
    def __str__(self):
        return "Foo"
"#;
        let out = extract(&PathBuf::from("src/foo.py"), src);
        for sym in out.symbols.iter().filter(|s| s.id.name.starts_with("__")) {
            assert_eq!(
                sym.visibility,
                Visibility::Export,
                "__dunder__ methods should be Export, got {:?} for {}",
                sym.visibility,
                sym.id.name
            );
        }
    }

    #[test]
    fn name_mangled_private_is_private() {
        let src = r"
class Foo:
    def __secret(self):
        pass
";
        let out = extract(&PathBuf::from("src/foo.py"), src);
        let sym = out.symbols.iter().find(|s| s.id.name == "__secret")
            .expect("__secret missing");
        assert_eq!(sym.visibility, Visibility::Private);
    }

    #[test]
    fn top_level_call_outside_any_function_is_ignored() {
        let src = "some_global()\ndef wrapper():\n    helper()\n";
        let out = extract(&PathBuf::from("src/a.py"), src);
        let calls: Vec<_> = out.edges.iter().filter(|e| e.kind == EdgeKind::Calls).collect();
        assert_eq!(calls.len(), 1, "got {calls:?}");
        assert_eq!(calls[0].from.name, "wrapper");
        assert_eq!(calls[0].to.name, "helper");
    }

    // --- Fix A regression tests ---

    #[test]
    fn nested_function_inside_method_is_not_a_method() {
        let src = "class C:\n    def m(self):\n        def inner():\n            pass\n";
        let out = extract(&PathBuf::from("src/a.py"), src);
        let inner = out.symbols.iter().find(|s| s.id.name == "inner")
            .expect("inner missing");
        assert_eq!(inner.id.kind, SymbolKind::Function,
            "nested local function should be Function, not Method");
        // m itself is still a method
        let m = out.symbols.iter().find(|s| s.id.name == "m")
            .expect("m missing");
        assert_eq!(m.id.kind, SymbolKind::Method);
    }

    #[test]
    fn call_inside_nested_function_attributes_to_inner_not_outer_method() {
        let src = "class C:\n    def m(self):\n        def inner():\n            helper()\n";
        let out = extract(&PathBuf::from("src/a.py"), src);
        let has_call = out.edges.iter().any(|e|
            e.kind == EdgeKind::Calls
                && e.from.name == "inner"
                && e.to.name == "helper"
        );
        assert!(has_call, "call inside inner() should attribute to inner; edges = {:?}", out.edges);
        // Verify the wrong attribution does NOT happen
        let wrong = out.edges.iter().any(|e|
            e.kind == EdgeKind::Calls
                && e.from.name == "m"
                && e.to.name == "helper"
        );
        assert!(!wrong, "call should NOT attribute to outer method m");
    }

    // --- Fix B regression tests ---

    #[test]
    fn dotted_relative_import_captured_with_stripped_library() {
        let src = "from ..utils import verify\n";
        let out = extract(&PathBuf::from("src/a.py"), src);
        assert!(out.library_imports.iter().any(|li| li.library == "utils"),
            "expected library='utils' (dots stripped); got {:?}", out.library_imports);
    }

    #[test]
    fn bare_relative_import_uses_dot_sentinel() {
        let src = "from . import foo\n";
        let out = extract(&PathBuf::from("src/a.py"), src);
        assert!(out.library_imports.iter().any(|li| li.library == "."),
            "expected library='.' sentinel for bare relative import; got {:?}",
            out.library_imports);
    }

    const BROKEN_PY: &str = r"
def good():
    return 1

def broken(:
    not valid
    return 2

def also_good():
    return 3
";

    #[test]
    fn partial_parse_still_extracts_what_parsed() {
        let out = extract(&PathBuf::from("src/broken.py"), BROKEN_PY);
        assert!(out.partial_parse);
        assert!(out.symbols.iter().any(|s| s.id.name == "good"));
        assert!(out.symbols.iter().any(|s| s.id.name == "also_good"));
    }
}
