//! TypeScript driver — tree-sitter-typescript.
//!
//! Phase 1.2 emits function/async-function/class/method/interface/type-alias
//! symbols, plus `LibraryImport` records for bare-specifier (external) imports.
//! Internal/relative imports and call edges are Task 2.

use std::path::Path;

use streaming_iterator::StreamingIterator as _;
use tree_sitter::{Language, Parser, Query, QueryCursor};

use crate::graph::types::{LibraryImport, Symbol, SymbolId, SymbolKind, Visibility};
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

/// Parse a TypeScript source file and return symbols plus external library
/// imports. Internal imports and call edges are populated in Task 2.
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
                    // All other captures (name helper nodes, call.callee/site
                    // for Task 2, etc.) are intentionally ignored here.
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
    // Relative and absolute paths are internal — Task 2 turns those into
    // graph Imports edges. Nothing to do here.
    if source_specifier.starts_with('.') || source_specifier.starts_with('/') {
        return;
    }
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
}
