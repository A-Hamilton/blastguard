//! Rust driver — tree-sitter-rust.
//!
//! Phase 1.2 emits function/async-function/method/struct/enum/trait symbols,
//! plus `LibraryImport` records for external `use` declarations and `Imports`
//! edges for `crate::`/`self::`/`super::` paths. Call expressions inside a
//! function or method produce `Calls` edges (intra-file only; Task 13 resolves
//! cross-file calls).
//!
//! Visibility is derived from the `visibility_modifier` child token:
//! - `pub` alone → [`Visibility::Export`]
//! - `pub(crate)`, `pub(super)`, `pub(in path)` → [`Visibility::Public`]
//! - No modifier → [`Visibility::Private`]

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
const QUERY_SRC: &str = include_str!("../../queries/rust.scm");

thread_local! {
    /// Parser and compiled query per thread. Both are not `Send` so this is
    /// the correct place to own them for rayon-parallel indexing.
    static RS_STATE: std::cell::RefCell<(Parser, Query)> = std::cell::RefCell::new({
        let mut parser = Parser::new();
        let lang: Language = tree_sitter_rust::language();
        // Unreachable in a correctly-built binary — the grammar crate is a
        // compile-time constant and the query is validated by `cargo test`.
        parser.set_language(&lang).expect("tree-sitter Rust grammar must load");
        let query = Query::new(&lang, QUERY_SRC).expect("rust.scm must be a valid query");
        (parser, query)
    });
}

/// Parse a Rust source file and return symbols, external library imports
/// (`use std::…`, `use tokio::…`, etc.), internal `Imports` edges for
/// `crate::`/`self::`/`super::` paths, and intra-file `Calls` edges.
///
/// Returns a default `ParseOutput` with `partial_parse = true` when
/// tree-sitter cannot produce a tree at all. When ERROR nodes are present,
/// extraction continues over successfully-parsed regions and
/// `partial_parse` is set.
///
/// # Panics
///
/// Panics if `queries/rust.scm` contains a syntax error (compile-time
/// bug — the `.scm` file is embedded and validated at start-up, so this is
/// unreachable in a correctly-built binary).
#[must_use = "parsed symbols and imports should be ingested into the graph"]
pub fn extract(path: &Path, source: &str) -> ParseOutput {
    RS_STATE.with(|cell| {
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
                    "struct.decl" => emit_simple(node, source, path, &mut out, SymbolKind::Struct),
                    "enum.decl" => emit_simple(node, source, path, &mut out, SymbolKind::Enum),
                    "trait.decl" => emit_simple(node, source, path, &mut out, SymbolKind::Trait),
                    "use.path" => {
                        let text = node.utf8_text(src_bytes).unwrap_or("");
                        emit_use(text, node, path, &mut out);
                    }
                    "call.callee" => {
                        emit_call(node, source, path, &mut out, &mut seen_calls);
                    }
                    // All other captures (name helper nodes, use.decl, call.site, etc.)
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
    let method = is_method(node);

    let kind = if method {
        // Async methods use Method + is_async; no separate AsyncMethod variant.
        SymbolKind::Method
    } else if is_async {
        SymbolKind::AsyncFunction
    } else {
        SymbolKind::Function
    };

    let visibility = extract_visibility(node, src_bytes);

    let params_text = node
        .child_by_field_name("parameters")
        .map(|n| n.utf8_text(src_bytes).unwrap_or("").to_owned())
        .unwrap_or_default();

    let return_type = node
        .child_by_field_name("return_type")
        .map(|n| n.utf8_text(src_bytes).unwrap_or("").trim().to_owned());

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
    let visibility = extract_visibility(node, src_bytes);
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

/// Emit an `Imports` edge (for `crate::`/`self::`/`super::` paths) or a
/// [`LibraryImport`] (for all other paths, e.g. `std::`, `tokio::`).
///
/// The `path_text` is the full text of the `use_declaration` argument, e.g.
/// `"std::collections::HashMap"` or `"crate::utils::helper"`.
/// Task 11's import resolver will rewrite `to.file` on `Imports` edges to the
/// canonical on-disk path.
fn emit_use(
    path_text: &str,
    node: tree_sitter::Node<'_>,
    path: &Path,
    out: &mut ParseOutput,
) {
    if path_text.is_empty() {
        return;
    }
    let head = path_text.split("::").next().unwrap_or("");
    let line = u32::try_from(node.start_position().row)
        .unwrap_or(u32::MAX)
        .saturating_add(1);

    match head {
        "crate" | "self" | "super" => {
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
                    file: std::path::PathBuf::from(path_text),
                    name: String::new(),
                    kind: SymbolKind::Module,
                },
                kind: EdgeKind::Imports,
                line,
                confidence: Confidence::Unresolved,
            });
        }
        _ => {
            out.library_imports.push(LibraryImport {
                library: head.to_owned(),
                symbol: String::new(),
                file: path.to_path_buf(),
                line,
            });
        }
    }
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

/// Walk up the ancestor chain to find the nearest enclosing `function_item`.
/// Returns `None` for calls at module scope.
///
/// The kind is determined as follows:
/// - `function_item` inside an `impl_item` ancestor (before `source_file` or
///   another `function_item`) → [`SymbolKind::Method`]
/// - Otherwise → `AsyncFunction` if `async`, else `Function`.
fn enclosing_fn(
    mut node: tree_sitter::Node<'_>,
    source: &str,
) -> Option<(String, SymbolKind)> {
    while let Some(parent) = node.parent() {
        if parent.kind() == "function_item" {
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

/// Return `true` if `node` (a `function_item`) is a method — that is,
/// its ancestor chain hits `impl_item` before `function_item` (nested fn, not
/// a method) or `source_file` (top-level fn).
fn is_method(node: tree_sitter::Node<'_>) -> bool {
    let mut cur = node;
    while let Some(parent) = cur.parent() {
        match parent.kind() {
            "impl_item" => return true,
            "function_item" | "source_file" => return false,
            _ => {}
        }
        cur = parent;
    }
    false
}

/// Extract visibility from the `visibility_modifier` child of a declaration node.
///
/// - `pub` alone → [`Visibility::Export`]
/// - `pub(crate)`, `pub(super)`, `pub(in path)` → [`Visibility::Public`]
/// - No `visibility_modifier` child → [`Visibility::Private`]
fn extract_visibility(node: tree_sitter::Node<'_>, src_bytes: &[u8]) -> Visibility {
    let mut walker = node.walk();
    for child in node.children(&mut walker) {
        if child.kind() == "visibility_modifier" {
            let txt = child.utf8_text(src_bytes).unwrap_or("");
            if txt == "pub" {
                return Visibility::Export;
            } else if txt.starts_with("pub(") {
                // pub(crate), pub(super), pub(in path) — all lumped as Public
                return Visibility::Public;
            }
        }
    }
    Visibility::Private
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

/// Split a Rust parameter list like `"(self, req: Request)"` into individual
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
    use crate::graph::types::{Confidence, EdgeKind, SymbolKind, Visibility};
    use std::path::PathBuf;

    const SAMPLE: &str = r"
use std::collections::HashMap;
use crate::utils::helper;

pub async fn process_request(req: Request) -> Result<Response, Error> {
    handler(req).await
}

pub(crate) struct Handler {
    state: HashMap<String, u32>,
}

impl Handler {
    pub async fn handle(&self, req: Request) -> Result<(), Error> {
        process_request(req).await
    }
}

pub trait Service {
    fn name(&self) -> &str;
}

pub enum Status {
    Ok,
    Err,
}

fn private_helper() {
    noop();
}
";

    #[test]
    fn extracts_async_fn_as_async_function() {
        let out = extract(&PathBuf::from("src/handler.rs"), SAMPLE);
        let sym = out
            .symbols
            .iter()
            .find(|s| s.id.name == "process_request")
            .expect("missing");
        assert_eq!(sym.id.kind, SymbolKind::AsyncFunction);
        assert!(sym.is_async);
    }

    #[test]
    fn extracts_struct_enum_trait() {
        let out = extract(&PathBuf::from("src/handler.rs"), SAMPLE);
        assert!(out
            .symbols
            .iter()
            .any(|s| s.id.name == "Handler" && s.id.kind == SymbolKind::Struct));
        assert!(out
            .symbols
            .iter()
            .any(|s| s.id.name == "Service" && s.id.kind == SymbolKind::Trait));
        assert!(out
            .symbols
            .iter()
            .any(|s| s.id.name == "Status" && s.id.kind == SymbolKind::Enum));
    }

    #[test]
    fn method_in_impl_block_is_method_kind() {
        let out = extract(&PathBuf::from("src/handler.rs"), SAMPLE);
        let sym = out
            .symbols
            .iter()
            .find(|s| s.id.name == "handle")
            .expect("handle missing");
        assert_eq!(sym.id.kind, SymbolKind::Method);
        assert!(sym.is_async, "async fn handle must set is_async even as Method");
    }

    #[test]
    fn visibility_from_pub_and_pub_crate() {
        let out = extract(&PathBuf::from("src/handler.rs"), SAMPLE);
        let pub_sym = out
            .symbols
            .iter()
            .find(|s| s.id.name == "process_request")
            .expect("missing");
        assert_eq!(pub_sym.visibility, Visibility::Export);
        let pub_crate = out
            .symbols
            .iter()
            .find(|s| s.id.name == "Handler")
            .expect("missing");
        assert_eq!(pub_crate.visibility, Visibility::Public);
        let priv_sym = out
            .symbols
            .iter()
            .find(|s| s.id.name == "private_helper")
            .expect("missing");
        assert_eq!(priv_sym.visibility, Visibility::Private);
    }

    #[test]
    fn external_use_captured_as_library() {
        let out = extract(&PathBuf::from("src/handler.rs"), SAMPLE);
        assert!(out.library_imports.iter().any(|li| li.library == "std"));
    }

    #[test]
    fn crate_use_becomes_unresolved_imports_edge() {
        let out = extract(&PathBuf::from("src/handler.rs"), SAMPLE);
        let has_internal = out.edges.iter().any(|e| {
            e.kind == EdgeKind::Imports
                && e.confidence == Confidence::Unresolved
                && e.to
                    .file
                    .to_string_lossy()
                    .contains("crate::utils::helper")
        });
        assert!(
            has_internal,
            "expected Imports edge for crate::utils::helper; edges = {:?}",
            out.edges
        );
    }

    #[test]
    fn intra_file_call_edge() {
        let out = extract(&PathBuf::from("src/handler.rs"), SAMPLE);
        let has_call = out.edges.iter().any(|e| {
            e.kind == EdgeKind::Calls
                && e.from.name == "handle"
                && e.to.name == "process_request"
        });
        assert!(
            has_call,
            "expected handle -> process_request edge; edges = {:?}",
            out.edges
        );
    }

    #[test]
    fn repeated_calls_dedup() {
        let src = "fn retry() { helper(); helper(); helper(); }";
        let out = extract(&PathBuf::from("src/retry.rs"), src);
        let calls: Vec<_> = out
            .edges
            .iter()
            .filter(|e| {
                e.kind == EdgeKind::Calls
                    && e.from.name == "retry"
                    && e.to.name == "helper"
            })
            .collect();
        assert_eq!(calls.len(), 1, "expected 1 dedup'd edge, got {calls:?}");
    }

    #[test]
    fn scoped_identifier_call_emits_edge_with_final_name() {
        let src = "fn build() -> Vec<u8> { Vec::new() }";
        let out = extract(&PathBuf::from("src/a.rs"), src);
        let has_call = out.edges.iter().any(|e| {
            e.kind == EdgeKind::Calls && e.from.name == "build" && e.to.name == "new"
        });
        assert!(
            has_call,
            "expected build -> new edge for Vec::new(); edges = {:?}",
            out.edges
        );
    }

    #[test]
    fn nested_scoped_identifier_call_captures_final_segment() {
        let src = "fn x() { crate::utils::helper(); }";
        let out = extract(&PathBuf::from("src/a.rs"), src);
        let has_call = out.edges.iter().any(|e| {
            e.kind == EdgeKind::Calls && e.from.name == "x" && e.to.name == "helper"
        });
        assert!(
            has_call,
            "expected x -> helper edge for crate::utils::helper(); edges = {:?}",
            out.edges
        );
    }

    #[test]
    fn nested_fn_inside_impl_is_function_not_method() {
        let src = "impl Foo { fn outer(&self) { fn inner() {} } }";
        let out = extract(&PathBuf::from("src/x.rs"), src);
        assert!(
            out.symbols
                .iter()
                .any(|s| s.id.name == "inner" && s.id.kind == SymbolKind::Function),
            "inner should be Function, got symbols: {:?}",
            out.symbols
                .iter()
                .map(|s| (&s.id.name, s.id.kind))
                .collect::<Vec<_>>()
        );
        assert!(out
            .symbols
            .iter()
            .any(|s| s.id.name == "outer" && s.id.kind == SymbolKind::Method));
    }

    #[test]
    fn pub_super_maps_to_public_visibility() {
        let src = "pub(super) fn helper() {}";
        let out = extract(&PathBuf::from("src/x.rs"), src);
        let sym = out
            .symbols
            .iter()
            .find(|s| s.id.name == "helper")
            .expect("helper missing");
        assert_eq!(sym.visibility, Visibility::Public);
    }
}
