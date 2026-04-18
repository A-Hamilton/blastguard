//! TypeScript driver — tree-sitter-typescript.
//!
//! Phase 1.2 emits function/async-function/class/method/interface/type-alias
//! symbols, plus `LibraryImport` records for bare-specifier (external) imports.
//! Internal/relative imports produce `Imports` edges; call expressions inside a
//! function/method produce `Calls` edges (intra-file only; Task 13 resolves
//! cross-file calls).

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
const QUERY_SRC: &str = include_str!("../../queries/typescript.scm");

thread_local! {
    /// Parser and compiled query per thread. Both are not `Send` so this is
    /// the correct place to own them for rayon-parallel indexing.
    static TS_STATE: std::cell::RefCell<(Parser, Query)> = std::cell::RefCell::new({
        let mut parser = Parser::new();
        let lang: Language = tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into();
        // Unreachable in a correctly-built binary — the grammar crate is a
        // compile-time constant and the query is validated by `cargo test`.
        parser.set_language(&lang).expect("tree-sitter TypeScript grammar must load");
        let query = Query::new(&lang, QUERY_SRC).expect("typescript.scm must be a valid query");
        (parser, query)
    });
}

/// Parse a TypeScript source file and return symbols, external library
/// imports, internal `Imports` edges (to be resolved by Task 8), and
/// intra-file `Calls` edges (cross-file resolution is Task 13).
///
/// Returns a default `ParseOutput` with `partial_parse = true` when
/// tree-sitter cannot produce a tree at all. When ERROR nodes are present,
/// extraction continues over successfully-parsed regions and
/// `partial_parse` is set.
///
/// # Panics
///
/// Panics if `queries/typescript.scm` contains a syntax error (compile-time
/// bug — the `.scm` file is embedded and validated at start-up, so this is
/// unreachable in a correctly-built binary).
#[must_use = "parsed symbols and imports should be ingested into the graph"]
pub fn extract(path: &Path, source: &str) -> ParseOutput {
    TS_STATE.with(|cell| {
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
                    "method.decl" => emit_simple(node, source, path, &mut out, SymbolKind::Method),
                    "interface.decl" => {
                        emit_simple(node, source, path, &mut out, SymbolKind::Interface);
                    }
                    "type_alias.decl" => {
                        emit_simple(node, source, path, &mut out, SymbolKind::TypeAlias);
                    }
                    "import.source" => {
                        let literal = node.utf8_text(src_bytes).unwrap_or("");
                        emit_import(literal, node, path, &mut out);
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
    let kind = if is_async {
        SymbolKind::AsyncFunction
    } else {
        SymbolKind::Function
    };

    let params_text = node
        .child_by_field_name("parameters")
        .map(|n| n.utf8_text(src_bytes).unwrap_or("").to_owned())
        .unwrap_or_default();

    let return_type = node
        .child_by_field_name("return_type")
        .map(|n| {
            n.utf8_text(src_bytes)
                .unwrap_or("")
                .trim_start_matches(':')
                .trim()
                .to_owned()
        });

    let signature = render_signature(&name, &params_text, return_type.as_deref());
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
        return_type,
        // Visibility refinement (export vs public vs private) is Task 2.
        visibility: Visibility::Export,
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
        visibility: Visibility::Export,
        body_hash: body_hash(body_text),
        is_async: false,
        embedding_id: None,
    });
}

fn emit_import(
    source_specifier: &str,
    node: tree_sitter::Node<'_>,
    path: &Path,
    out: &mut ParseOutput,
) {
    if source_specifier.is_empty() {
        return;
    }
    let line = u32::try_from(node.start_position().row)
        .unwrap_or(u32::MAX)
        .saturating_add(1);

    // Relative (`./foo`, `../bar`) and absolute (`/abs`) paths are internal —
    // emit an Imports edge. Task 8's resolver will rewrite `to.file` from the
    // raw specifier to the canonical on-disk path.
    if source_specifier.starts_with('.') || source_specifier.starts_with('/') {
        let module_name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_owned();
        out.edges.push(Edge {
            from: SymbolId {
                file: path.to_path_buf(),
                name: module_name,
                kind: SymbolKind::Module,
            },
            to: SymbolId {
                file: std::path::PathBuf::from(source_specifier),
                name: String::new(),
                kind: SymbolKind::Module,
            },
            kind: EdgeKind::Imports,
            line,
            confidence: Confidence::Unresolved,
        });
        return;
    }

    // Bare specifiers are external (npm/yarn) library imports.
    // For scoped packages (@scope/pkg or @scope/pkg/subpath) the canonical
    // npm identifier is "@scope/pkg" — keep both the scope and the name.
    // For unscoped packages strip any subpath (lodash/merge → lodash).
    let library = if source_specifier.starts_with('@') {
        let mut parts = source_specifier.splitn(3, '/');
        match (parts.next(), parts.next()) {
            (Some(scope), Some(pkg)) => format!("{scope}/{pkg}"),
            _ => source_specifier.to_owned(),
        }
    } else {
        source_specifier
            .split('/')
            .next()
            .unwrap_or(source_specifier)
            .to_owned()
    };

    out.library_imports.push(LibraryImport {
        library,
        symbol: String::new(),
        file: path.to_path_buf(),
        line,
    });
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

/// Emit a [`Calls`] edge when a call expression's callee lies inside a named
/// function or method. Top-level (module-scope) calls are silently dropped —
/// there is no meaningful `from` symbol to attribute them to.
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

/// Walk up the tree-sitter ancestor chain to find the nearest enclosing named
/// `function_declaration` or `method_definition`. Returns `None` for calls at
/// module scope (no enclosing function).
///
/// **Arrow-function attribution contract:** `arrow_function` nodes are
/// transparent — they are neither matched nor returned. A call inside an arrow
/// body therefore bubbles up to the nearest enclosing `function_declaration` or
/// `method_definition`. A call inside a *module-scope* arrow function (no named
/// ancestor) returns `None` and is dropped, exactly like a bare module-scope
/// call.
fn enclosing_fn(
    mut node: tree_sitter::Node<'_>,
    source: &str,
) -> Option<(String, SymbolKind)> {
    while let Some(parent) = node.parent() {
        match parent.kind() {
            "function_declaration" => {
                let name = parent
                    .child_by_field_name("name")?
                    .utf8_text(source.as_bytes())
                    .ok()?
                    .to_owned();
                let kind = if first_child_text_is(parent, source, "async") {
                    SymbolKind::AsyncFunction
                } else {
                    SymbolKind::Function
                };
                return Some((name, kind));
            }
            "method_definition" => {
                let name = parent
                    .child_by_field_name("name")?
                    .utf8_text(source.as_bytes())
                    .ok()?
                    .to_owned();
                return Some((name, SymbolKind::Method));
            }
            _ => {}
        }
        node = parent;
    }
    None
}

/// Split a parameter list string like `"(req: Request, res: Response)"` into
/// individual parameter strings, stripping surrounding parentheses.
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
    use crate::graph::types::SymbolKind;
    use std::path::PathBuf;

    const SAMPLE: &str = r#"
export async function processRequest(req: Request, res: Response): Promise<Response> {
  return handler(req);
}

export class Handler {
  async handle(req: Request): Promise<void> {
    return processRequest(req, res);
  }
}

export interface Greeter {
  greet(name: string): string;
}

import { get } from "lodash";
import { helper } from "./utils/helper";
"#;

    #[test]
    fn extracts_exported_async_function() {
        let out = extract(&PathBuf::from("src/handler.ts"), SAMPLE);
        let fn_sym = out
            .symbols
            .iter()
            .find(|s| s.id.name == "processRequest")
            .expect("processRequest missing");
        assert_eq!(fn_sym.id.kind, SymbolKind::AsyncFunction);
        assert!(fn_sym.is_async);
        assert!(fn_sym.signature.contains("req: Request"));
        assert_eq!(fn_sym.return_type.as_deref(), Some("Promise<Response>"));
    }

    #[test]
    fn extracts_class_and_method() {
        let out = extract(&PathBuf::from("src/handler.ts"), SAMPLE);
        assert!(out.symbols.iter().any(|s| s.id.name == "Handler"
            && s.id.kind == SymbolKind::Class));
        assert!(out.symbols.iter().any(|s| s.id.name == "handle"
            && s.id.kind == SymbolKind::Method));
    }

    #[test]
    fn extracts_interface() {
        let out = extract(&PathBuf::from("src/handler.ts"), SAMPLE);
        assert!(out.symbols.iter().any(|s| s.id.name == "Greeter"
            && s.id.kind == SymbolKind::Interface));
    }

    #[test]
    fn extracts_library_imports() {
        let out = extract(&PathBuf::from("src/handler.ts"), SAMPLE);
        assert!(out.library_imports.iter().any(|li| li.library == "lodash"));
        // `./utils/helper` is internal — Task 2 handles that path. Task 1 must
        // NOT emit it as a library_import.
        assert!(!out.library_imports.iter().any(|li| li.library.starts_with("./")));
    }

    #[test]
    fn scoped_package_keeps_scope_and_name() {
        let src = r#"import { Button } from "@tanstack/react-query";"#;
        let out = extract(&PathBuf::from("src/a.ts"), src);
        assert!(
            out.library_imports.iter().any(|li| li.library == "@tanstack/react-query"),
            "expected canonical @scope/pkg library name, got {:?}",
            out.library_imports
        );
    }

    #[test]
    fn subpath_import_strips_subpath_but_keeps_package() {
        let src = r#"import { merge } from "lodash/merge";"#;
        let out = extract(&PathBuf::from("src/a.ts"), src);
        assert!(out.library_imports.iter().any(|li| li.library == "lodash"));
    }

    #[test]
    fn scoped_subpath_keeps_full_scope_and_name() {
        let src = r#"import { x } from "@scope/pkg/sub";"#;
        let out = extract(&PathBuf::from("src/a.ts"), src);
        assert!(out.library_imports.iter().any(|li| li.library == "@scope/pkg"));
    }

    #[test]
    fn internal_import_becomes_imports_edge_not_library_import() {
        let out = extract(&PathBuf::from("src/handler.ts"), SAMPLE);
        // Library imports still contain lodash and nothing else that's relative.
        assert!(!out.library_imports.iter().any(|li| li.library.starts_with("./")));

        // An Imports edge exists for ./utils/helper with Unresolved confidence
        // (Task 8 will rewrite to.file to the canonical on-disk path).
        let has_internal = out.edges.iter().any(|e| {
            e.kind == crate::graph::types::EdgeKind::Imports
                && e.confidence == crate::graph::types::Confidence::Unresolved
                && e.to.file == std::path::Path::new("./utils/helper")
        });
        assert!(has_internal, "expected Imports edge for ./utils/helper; edges = {:?}", out.edges);
    }

    #[test]
    fn intra_file_call_produces_calls_edge_with_enclosing_fn() {
        // processRequest() calls handler() — we should see an edge
        // Calls { from: processRequest, to: handler }.
        let out = extract(&PathBuf::from("src/handler.ts"), SAMPLE);
        let has_call = out.edges.iter().any(|e| {
            e.kind == crate::graph::types::EdgeKind::Calls
                && e.from.name == "processRequest"
                && e.to.name == "handler"
        });
        assert!(has_call, "expected processRequest -> handler edge; edges = {:?}", out.edges);
    }

    #[test]
    fn method_call_tracked_with_enclosing_method_as_from() {
        // Handler.handle() calls processRequest() — expect Calls edge from handle.
        let out = extract(&PathBuf::from("src/handler.ts"), SAMPLE);
        let has_call = out.edges.iter().any(|e| {
            e.kind == crate::graph::types::EdgeKind::Calls
                && e.from.name == "handle"
                && e.to.name == "processRequest"
        });
        assert!(has_call, "expected handle -> processRequest edge; edges = {:?}", out.edges);
    }

    #[test]
    fn top_level_call_outside_any_function_is_ignored() {
        let src = "someGlobal();\nexport function wrapper() { helper(); }\n";
        let out = extract(&PathBuf::from("src/a.ts"), src);
        // `someGlobal()` at module scope has no enclosing function — no Calls edge
        // should attribute it. Only `wrapper -> helper` should exist.
        let calls: Vec<_> = out
            .edges
            .iter()
            .filter(|e| e.kind == crate::graph::types::EdgeKind::Calls)
            .collect();
        assert_eq!(calls.len(), 1, "got {calls:?}");
        assert_eq!(calls[0].from.name, "wrapper");
        assert_eq!(calls[0].to.name, "helper");
    }

    #[test]
    fn repeated_calls_to_same_callee_produce_one_edge() {
        let src = "
export function retry() {
    helper();
    helper();
    helper();
}
";
        let out = extract(&PathBuf::from("src/retry.ts"), src);
        let calls: Vec<_> = out
            .edges
            .iter()
            .filter(|e| {
                e.kind == crate::graph::types::EdgeKind::Calls
                    && e.from.name == "retry"
                    && e.to.name == "helper"
            })
            .collect();
        assert_eq!(calls.len(), 1, "expected 1 dedup'd edge, got {calls:?}");
    }

    #[test]
    fn call_inside_arrow_inside_named_fn_attributes_to_named_fn() {
        // Arrow functions are transparent to call attribution: the call bubbles
        // up to the nearest enclosing function_declaration / method_definition.
        // Module-scope arrows drop their calls (same as bare module-scope calls).
        let src = "
export function outer() {
    const retry = () => { helper(); };
    retry();
}
";
        let out = extract(&PathBuf::from("src/a.ts"), src);
        let attributed = out.edges.iter().any(|e| {
            e.kind == crate::graph::types::EdgeKind::Calls
                && e.from.name == "outer"
                && e.to.name == "helper"
        });
        assert!(
            attributed,
            "expected helper() inside arrow to attribute to outer; edges = {:?}",
            out.edges
        );
    }

    #[test]
    fn call_inside_module_scope_arrow_is_dropped() {
        let src = "const cb = () => { helper(); };";
        let out = extract(&PathBuf::from("src/a.ts"), src);
        let any_call = out
            .edges
            .iter()
            .any(|e| e.kind == crate::graph::types::EdgeKind::Calls);
        assert!(
            !any_call,
            "module-scope arrow calls should be dropped; edges = {:?}",
            out.edges
        );
    }

    const BROKEN_TS: &str = r"
export function good() { return 1; }
this is { not valid :: syntax
export function alsoGood() { return 2; }
";

    #[test]
    fn partial_parse_still_extracts_what_parsed() {
        let out = extract(&PathBuf::from("src/broken.ts"), BROKEN_TS);
        assert!(out.partial_parse, "partial flag should be set on broken input");
        assert!(out.symbols.iter().any(|s| s.id.name == "good"),
            "expected to still extract `good`; got {:?}",
            out.symbols.iter().map(|s| &s.id.name).collect::<Vec<_>>());
        assert!(out.symbols.iter().any(|s| s.id.name == "alsoGood"),
            "expected to still extract `alsoGood`; got {:?}",
            out.symbols.iter().map(|s| &s.id.name).collect::<Vec<_>>());
    }
}
