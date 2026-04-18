# BlastGuard Phase 1 — Indexing Pipeline Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Take BlastGuard from a compiling scaffold to a working cold indexer that builds a complete CodeGraph for any TypeScript/JavaScript/Python/Rust project, with a BLAKE3-Merkle warm-start cache under the 500ms target.

**Architecture:** Single-binary Rust MCP server. Tree-sitter drives per-file extraction, `rayon` fans parsing across CPU cores (one parser per worker via `thread_local!`), `ignore` walks the project, and `rmp-serde` + BLAKE3 Merkle hashes give us sub-500ms warm starts. Every module writes to `tracing` on stderr; stdout is reserved for the MCP protocol frames that come online in a later plan.

**Tech Stack:** Rust 1.82+, rmcp 1.5, tree-sitter 0.24, tree-sitter-{typescript,javascript,python,rust}, notify 8, rayon 1.10, ignore 0.4, rmp-serde 1, blake3 1, seahash 4, serde/serde_json, thiserror 2, anyhow 1, tracing 0.1.

**Preconditions assumed by this plan:**
- Repository is at `/home/adam/Documents/blastguard`.
- `Cargo.toml`, `src/lib.rs`, `src/main.rs`, `src/error.rs`, `src/config.rs`, `src/session.rs`, and the module trees under `src/graph/`, `src/parse/`, `src/index/`, `src/search/`, `src/runner/`, `src/mcp/` already exist in stub form (committed by the scaffold task preceding this plan).
- `src/graph/types.rs` and `src/graph/ops.rs` are already implemented with passing unit tests.
- `SPEC.md` is authoritative for data shapes and performance targets. `CLAUDE.md` is authoritative for module layout and Rust quality rules.

**Definition of done for this plan:**
- `cargo check --all-targets` passes with no warnings.
- `cargo test` passes with >60 unit tests.
- `cargo clippy --all-targets -- -W clippy::pedantic -D warnings` is clean.
- `cargo build --release` succeeds.
- Running `cargo run -- tests/fixtures/sample_project` indexes the fixture in under 200ms cold, under 50ms warm, with zero stderr errors.
- `.blastguard/cache.bin` exists after the first run and the second run is measurably faster (criterion bench or timed test).

---

## File Structure

**New or heavily modified files (ordered by responsibility):**

| Path | Responsibility |
|---|---|
| `queries/typescript.scm` | tree-sitter pattern file: function/class/interface/method/type-alias declarations, import statements, call sites |
| `queries/javascript.scm` | Fork of TS without type annotations, interfaces, or type aliases |
| `queries/python.scm` | function_definition, class_definition, async def, import_statement, import_from_statement, call sites |
| `queries/rust.scm` | fn, impl block methods, struct, enum, trait, use path, call sites |
| `src/parse/typescript.rs` | TS extractor using the SCM query — emits `Symbol`/`Edge`/`LibraryImport` records |
| `src/parse/javascript.rs` | JS extractor sharing helpers from `typescript.rs` |
| `src/parse/python.rs` | Python extractor |
| `src/parse/rust.rs` | Rust extractor |
| `src/parse/symbols.rs` | Shared signature-rendering helper used by every language driver |
| `src/parse/resolve.rs` | TS/JS/PY/RS import resolver + `tsconfig.json` path-alias loader |
| `src/parse/body_hash.rs` | Already implemented; no changes expected |
| `src/index/indexer.rs` | `cold_index` + `warm_start` — walks project, hashes files, dispatches to parsers via `rayon`, assembles `CodeGraph` |
| `src/index/cache.rs` | `load`/`save` for `CacheFile`, BLAKE3 Merkle tree hashing helper |
| `src/index/mod.rs` | Re-exports |
| `tests/fixtures/sample_project/` | Multi-file TS+Python+Rust fixture used by integration tests |
| `tests/integration_indexer.rs` | End-to-end: `cold_index(fixture)` → assert graph has expected symbols/edges |
| `benches/indexer_bench.rs` | Criterion benchmark: cold + warm index timing on the fixture |

Every file has one responsibility. Language drivers are separate because TS vs. Python queries, signature rendering, and import resolution diverge in non-trivial ways — a single `parse.rs` would grow into a switch hell.

---

## Task 0: Close the scaffold

**Files:**
- Create: `src/mcp/apply_change.rs`, `src/mcp/run_tests.rs`, `src/mcp/search.rs`, `src/mcp/status.rs`
- Create: `queries/typescript.scm`, `queries/javascript.scm`, `queries/python.scm`, `queries/rust.scm` (empty placeholders — Task 1 fills TS)
- Create: `CHANGELOG.md`, `README.md`
- Modify: `.gitignore`

- [ ] **Step 1: Write the four mcp tool-handler stubs**

Each of these is a module file that will be fleshed out in the "tool surface" plan. For now they only need to exist so `src/mcp/mod.rs` compiles.

`src/mcp/search.rs`:
```rust
//! `search` tool handler — wired in a later plan.

// TODO(plan-2): implement SearchRequest / handle_search.
```

`src/mcp/apply_change.rs`:
```rust
//! `apply_change` tool handler — wired in a later plan.

// TODO(plan-2): implement ApplyChangeRequest / handle_apply_change.
```

`src/mcp/run_tests.rs`:
```rust
//! `run_tests` tool handler — wired in a later plan.

// TODO(plan-2): implement RunTestsRequest / handle_run_tests.
```

`src/mcp/status.rs`:
```rust
//! `blastguard://status` resource — wired in a later plan.

// TODO(plan-2): implement status resource handler.
```

- [ ] **Step 2: Create empty `queries/*.scm` placeholders**

```bash
mkdir -p /home/adam/Documents/blastguard/queries
touch /home/adam/Documents/blastguard/queries/typescript.scm
touch /home/adam/Documents/blastguard/queries/javascript.scm
touch /home/adam/Documents/blastguard/queries/python.scm
touch /home/adam/Documents/blastguard/queries/rust.scm
```

- [ ] **Step 3: Create `CHANGELOG.md`**

```markdown
# Changelog

All notable changes to BlastGuard will be documented in this file.

## [Unreleased]

### Added
- Project scaffolding: Cargo.toml, module tree, error types, session state.
- Code graph data structures and BFS/DFS/centrality ops (Phase 1.1).
- Body hash normaliser (whitespace- and comment-insensitive).
```

- [ ] **Step 4: Create minimal `README.md`**

```markdown
# BlastGuard

Open-source Rust MCP server that lifts AI coding agents via AST graph
retrieval, cascade warnings, and test-failure attribution. MIT licensed.

This is an early build. See `SPEC.md` for the design, `CLAUDE.md` for
contributor conventions, and `docs/superpowers/plans/` for the active
implementation plans.

## Status

Phase 1 (MVP) in progress. Benchmark results against SWE-bench Pro will be
published here once Phase 1 ships.
```

- [ ] **Step 5: Extend `.gitignore`**

Ensure these lines are present (append if missing):
```
/target
/.blastguard
/bench/results
*.log
```

- [ ] **Step 6: Run `cargo check --all-targets`**

```bash
cd /home/adam/Documents/blastguard
cargo check --all-targets 2>&1 | tee /tmp/blastguard-check.log
```
Expected: `Finished` with zero warnings. If `rmcp` crate fetch is the first thing to fail, read the error carefully — the Cargo.toml pins `rmcp = "1.5"` with features `["server","transport-io","macros","schemars"]`. If a feature name was renamed in 1.x, use `context7` to look up the current name, patch Cargo.toml, re-run.

- [ ] **Step 7: Run `cargo test`**

```bash
cargo test 2>&1 | tee /tmp/blastguard-test.log
```
Expected: Phase 1.1 tests pass (`graph::types::tests::*`, `graph::ops::tests::*`, `parse::tests::*`, `session::tests::*`, `config::tests::*`, `runner::detect::tests::*`, `parse::body_hash::tests::*`). Total >25 passing.

- [ ] **Step 8: Run `cargo clippy`**

```bash
cargo clippy --all-targets -- -W clippy::pedantic -D warnings 2>&1 | tee /tmp/blastguard-clippy.log
```
Expected: no warnings. If any fire, fix them before committing — do not `#[allow(...)]` without a written reason.

- [ ] **Step 9: Commit**

```bash
cd /home/adam/Documents/blastguard
git add -A
git commit -m "scaffold Phase 1 modules and graph primitives

Cargo.toml pinned to rmcp 1.5, schemars 0.9, notify 8 / debouncer-mini 0.7.
Implements graph::types (SPEC §7), graph::ops (callers/callees/shortest_path/
find_by_name), body_hash normaliser, runner detection, and config loader.
All other modules are stubbed with TODO(plan-*) markers."
```

---

## Task 1: TypeScript tree-sitter query file

**Files:**
- Modify: `queries/typescript.scm`
- Test: `src/parse/typescript.rs` (inline `#[cfg(test)]`)

- [ ] **Step 1: Write the failing test**

Add to `src/parse/typescript.rs`:
```rust
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
    fn extracts_imports() {
        let out = extract(&PathBuf::from("src/handler.ts"), SAMPLE);
        assert!(out.library_imports.iter().any(|li| li.library == "lodash"));
        // ./utils/helper is an internal import — emitted as an Edge, not a LibraryImport.
        // Task 2 covers that path.
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

```bash
cargo test -p blastguard parse::typescript::tests 2>&1 | head -40
```
Expected: all four tests fail — `extract` currently returns `ParseOutput::default()` so every assertion trips.

- [ ] **Step 3: Write the S-expression query file**

Replace `queries/typescript.scm` with:
```scheme
; Function declarations (exported and not).
(function_declaration
  name: (identifier) @function.name
  parameters: (formal_parameters) @function.params
  return_type: (type_annotation)? @function.return) @function.decl

; Async functions
((function_declaration) @async.decl
  (#match? @async.decl "^async"))

; Class declarations
(class_declaration
  name: (type_identifier) @class.name) @class.decl

; Methods inside a class body
(method_definition
  name: (property_identifier) @method.name
  parameters: (formal_parameters) @method.params
  return_type: (type_annotation)? @method.return) @method.decl

; Interfaces
(interface_declaration
  name: (type_identifier) @interface.name) @interface.decl

; Type aliases
(type_alias_declaration
  name: (type_identifier) @type_alias.name) @type_alias.decl

; Import statements
(import_statement
  source: (string (string_fragment) @import.source)) @import.decl

; Call expressions
(call_expression
  function: [(identifier) @call.callee (member_expression property: (property_identifier) @call.callee)]) @call.site

; Export modifiers — captured separately so extractors can set visibility.
(export_statement) @export
```

- [ ] **Step 4: Implement `extract` in `src/parse/typescript.rs`**

Replace the file body with:
```rust
//! TypeScript driver — tree-sitter-typescript.
//!
//! Phase 1.2 emits function/class/method/interface/type-alias symbols,
//! `Imports` edges for internal paths, and `LibraryImport` records for
//! external packages.

use std::path::{Path, PathBuf};

use tree_sitter::{Language, Parser, Query, QueryCursor};

use crate::graph::types::{
    Edge, EdgeKind, LibraryImport, Symbol, SymbolId, SymbolKind, Visibility,
};
use crate::parse::body_hash::body_hash;
use crate::parse::ParseOutput;

const QUERY_SRC: &str = include_str!("../../queries/typescript.scm");

thread_local! {
    static PARSER: std::cell::RefCell<Parser> = std::cell::RefCell::new({
        let mut p = Parser::new();
        let lang: Language = tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into();
        p.set_language(&lang).expect("TS language load");
        p
    });
}

#[must_use]
pub fn extract(path: &Path, source: &str) -> ParseOutput {
    PARSER.with(|cell| {
        let mut parser = cell.borrow_mut();
        let Some(tree) = parser.parse(source, None) else {
            return ParseOutput {
                partial_parse: true,
                ..ParseOutput::default()
            };
        };
        let root = tree.root_node();
        let partial_parse = root.has_error();

        let lang: Language = tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into();
        let query = Query::new(&lang, QUERY_SRC).expect("TS query compiles");
        let mut cursor = QueryCursor::new();
        let mut out = ParseOutput {
            partial_parse,
            ..ParseOutput::default()
        };

        for m in cursor.matches(&query, root, source.as_bytes()) {
            collect_match(&query, m, path, source, &mut out);
        }
        out
    })
}

fn collect_match(
    query: &Query,
    m: tree_sitter::QueryMatch<'_, '_>,
    path: &Path,
    source: &str,
    out: &mut ParseOutput,
) {
    for capture in m.captures {
        let name = &query.capture_names()[capture.index as usize];
        let node = capture.node;
        let text = node.utf8_text(source.as_bytes()).unwrap_or("");
        match name.as_str() {
            "function.decl" => emit_function(node, source, path, out, false),
            "async.decl" => emit_function(node, source, path, out, true),
            "class.decl" => emit_simple(node, source, path, out, SymbolKind::Class),
            "method.decl" => emit_simple(node, source, path, out, SymbolKind::Method),
            "interface.decl" => emit_simple(node, source, path, out, SymbolKind::Interface),
            "type_alias.decl" => emit_simple(node, source, path, out, SymbolKind::TypeAlias),
            "import.source" => emit_import(text, node, path, out),
            "call.callee" => emit_call(text, node, path, out),
            _ => {}
        }
    }
}

fn emit_function(node: tree_sitter::Node, source: &str, path: &Path, out: &mut ParseOutput, is_async: bool) {
    let Some(name_node) = node.child_by_field_name("name") else {
        return;
    };
    let name = name_node.utf8_text(source.as_bytes()).unwrap_or("").to_string();
    if name.is_empty() {
        return;
    }
    let kind = if is_async { SymbolKind::AsyncFunction } else { SymbolKind::Function };
    let params = node
        .child_by_field_name("parameters")
        .map(|n| n.utf8_text(source.as_bytes()).unwrap_or("").to_string())
        .unwrap_or_default();
    let return_type = node
        .child_by_field_name("return_type")
        .map(|n| n.utf8_text(source.as_bytes()).unwrap_or("").trim_start_matches(':').trim().to_string());
    let body_text = node.utf8_text(source.as_bytes()).unwrap_or("");
    let signature = format!("{}{}", name, params);
    let sig_with_ret = match &return_type {
        Some(ret) => format!("{signature}: {ret}"),
        None => signature.clone(),
    };
    out.symbols.push(Symbol {
        id: SymbolId {
            file: path.to_path_buf(),
            name,
            kind,
        },
        line_start: node.start_position().row as u32 + 1,
        line_end: node.end_position().row as u32 + 1,
        signature: sig_with_ret,
        params: params_to_vec(&params),
        return_type,
        visibility: Visibility::Export, // TS export detection in Task 2
        body_hash: body_hash(body_text),
        is_async,
        embedding_id: None,
    });
}

fn emit_simple(node: tree_sitter::Node, source: &str, path: &Path, out: &mut ParseOutput, kind: SymbolKind) {
    let Some(name_node) = node.child_by_field_name("name") else {
        return;
    };
    let name = name_node.utf8_text(source.as_bytes()).unwrap_or("").to_string();
    if name.is_empty() {
        return;
    }
    let body_text = node.utf8_text(source.as_bytes()).unwrap_or("");
    out.symbols.push(Symbol {
        id: SymbolId {
            file: path.to_path_buf(),
            name: name.clone(),
            kind,
        },
        line_start: node.start_position().row as u32 + 1,
        line_end: node.end_position().row as u32 + 1,
        signature: format!("{kind:?} {name}"),
        params: vec![],
        return_type: None,
        visibility: Visibility::Export,
        body_hash: body_hash(body_text),
        is_async: false,
        embedding_id: None,
    });
}

fn emit_import(source_path: &str, node: tree_sitter::Node, path: &Path, out: &mut ParseOutput) {
    let is_relative = source_path.starts_with('.') || source_path.starts_with('/');
    if is_relative {
        // Task 8 resolves these into internal `Imports` edges. For now, stash
        // the raw path as an Inferred edge target.
        return;
    }
    out.library_imports.push(LibraryImport {
        library: source_path.to_string(),
        symbol: String::new(),
        file: path.to_path_buf(),
        line: node.start_position().row as u32 + 1,
    });
}

fn emit_call(callee: &str, node: tree_sitter::Node, path: &Path, out: &mut ParseOutput) {
    // In Phase 1.2 we only emit intra-file Calls edges. Cross-file resolution
    // is Task 8. Guard on empty / builtin names.
    let _ = (callee, node, path, out);
}

fn params_to_vec(params: &str) -> Vec<String> {
    params
        .trim_start_matches('(')
        .trim_end_matches(')')
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    // Test module inserted in Step 1 lives here; do not re-declare.
}
```

Note: the `Edge` import will be used in a later step; suppress the unused-import lint in this iteration only by NOT importing it yet. Adjust to match the actual used imports.

- [ ] **Step 5: Run the tests and verify they pass**

```bash
cargo test -p blastguard parse::typescript::tests 2>&1 | head -40
```
Expected: all four tests pass. If any test fails on `return_type` or `signature` formatting, inspect with `cargo test -- --nocapture` and adjust `emit_function`.

- [ ] **Step 6: Run clippy on the new code**

```bash
cargo clippy -p blastguard --lib -- -W clippy::pedantic -D warnings
```
Fix any pedantic warnings (likely `must_use_candidate`, `missing_errors_doc`).

- [ ] **Step 7: Commit**

```bash
git add queries/typescript.scm src/parse/typescript.rs
git commit -m "phase 1.2: TypeScript parser — symbols + library imports"
```

---

## Task 2: TypeScript internal imports and call edges

**Files:**
- Modify: `src/parse/typescript.rs`
- Test: same file, expand `tests` module.

- [ ] **Step 1: Write the failing tests**

Extend `tests` in `src/parse/typescript.rs`:
```rust
#[test]
fn internal_import_becomes_unresolved_edge() {
    // Resolution is Phase 1.3; for now an internal import produces an edge
    // with Confidence::Inferred and a placeholder `to.file`.
    let out = extract(&PathBuf::from("src/handler.ts"), SAMPLE);
    assert!(out.edges.iter().any(|e| e.kind == crate::graph::types::EdgeKind::Imports));
}

#[test]
fn calls_inside_function_produce_calls_edges() {
    let out = extract(&PathBuf::from("src/handler.ts"), SAMPLE);
    // processRequest -> handler (call inside body)
    assert!(out.edges.iter().any(|e|
        e.kind == crate::graph::types::EdgeKind::Calls
            && e.from.name == "processRequest"
            && e.to.name == "handler"));
}
```

- [ ] **Step 2: Run the tests to verify they fail**

```bash
cargo test -p blastguard parse::typescript::tests::internal_import_becomes_unresolved_edge -- --exact
cargo test -p blastguard parse::typescript::tests::calls_inside_function_produce_calls_edges -- --exact
```
Expected: both fail.

- [ ] **Step 3: Implement `emit_import` for internal paths**

Replace the `emit_import` body in `src/parse/typescript.rs`:
```rust
fn emit_import(source_path: &str, node: tree_sitter::Node, path: &Path, out: &mut ParseOutput) {
    let is_relative = source_path.starts_with('.') || source_path.starts_with('/');
    if is_relative {
        // Unresolved placeholder. Task 8 will rewrite `to.file` to the actual
        // resolved PathBuf. Until then, use the literal source_path.
        let placeholder_to = PathBuf::from(source_path);
        out.edges.push(Edge {
            from: SymbolId {
                file: path.to_path_buf(),
                name: path.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string(),
                kind: SymbolKind::Module,
            },
            to: SymbolId {
                file: placeholder_to,
                name: String::new(),
                kind: SymbolKind::Module,
            },
            kind: EdgeKind::Imports,
            line: node.start_position().row as u32 + 1,
            confidence: crate::graph::types::Confidence::Inferred,
        });
        return;
    }
    out.library_imports.push(LibraryImport {
        library: source_path.to_string(),
        symbol: String::new(),
        file: path.to_path_buf(),
        line: node.start_position().row as u32 + 1,
    });
}
```
Also add the `Edge` and `Confidence` imports to the top of the file.

- [ ] **Step 4: Implement intra-file call tracking**

Replace `emit_call` and add a containing-function lookup helper:
```rust
fn emit_call(callee: &str, node: tree_sitter::Node, path: &Path, out: &mut ParseOutput) {
    if callee.is_empty() {
        return;
    }
    let Some(container_name) = enclosing_function_name(node) else {
        return;
    };
    out.edges.push(Edge {
        from: SymbolId {
            file: path.to_path_buf(),
            name: container_name,
            kind: SymbolKind::Function,
        },
        to: SymbolId {
            file: path.to_path_buf(),
            name: callee.to_string(),
            kind: SymbolKind::Function,
        },
        kind: EdgeKind::Calls,
        line: node.start_position().row as u32 + 1,
        confidence: crate::graph::types::Confidence::Inferred,
    });
}

fn enclosing_function_name(mut node: tree_sitter::Node) -> Option<String> {
    while let Some(parent) = node.parent() {
        if matches!(
            parent.kind(),
            "function_declaration" | "method_definition"
        ) {
            if let Some(name_node) = parent.child_by_field_name("name") {
                // `utf8_text` needs source bytes — instead, we'd need to thread them
                // through. For Phase 1.2, record the byte range and let the extractor
                // resolve the name later. Keep the algorithm simple: re-parse the
                // name by range at call time is wasteful, so thread `source` through.
                let _ = name_node;
            }
            return parent
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(&[]).ok().map(ToString::to_string));
        }
        node = parent;
    }
    None
}
```
**Note:** the `utf8_text(&[])` call is a placeholder — the correct fix threads `source: &str` through. Update `emit_call`'s signature to `fn emit_call(callee: &str, node: tree_sitter::Node, path: &Path, source: &str, out: &mut ParseOutput)` and in `collect_match` pass `source.as_bytes()` where needed. Same change for `enclosing_function_name(node: tree_sitter::Node, source_bytes: &[u8])`.

- [ ] **Step 5: Run the tests and verify they pass**

```bash
cargo test -p blastguard parse::typescript::tests 2>&1 | head -60
```
Expected: all six tests pass.

- [ ] **Step 6: Clippy**

```bash
cargo clippy -p blastguard --lib -- -W clippy::pedantic -D warnings
```
Expected: clean.

- [ ] **Step 7: Commit**

```bash
git add src/parse/typescript.rs
git commit -m "phase 1.2: TypeScript internal imports + intra-file call edges"
```

---

## Task 3: JavaScript driver

**Files:**
- Modify: `queries/javascript.scm`, `src/parse/javascript.rs`

- [ ] **Step 1: Write the failing tests**

Add to `src/parse/javascript.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::types::SymbolKind;
    use std::path::PathBuf;

    const SAMPLE: &str = r#"
export async function loadUser(id) {
  return db.find(id);
}

export class UserService {
  async get(id) {
    return loadUser(id);
  }
}

import { debounce } from "lodash";
import { helper } from "./utils/helper";
"#;

    #[test]
    fn extracts_async_function_without_types() {
        let out = extract(&PathBuf::from("src/user.js"), SAMPLE);
        let fn_sym = out
            .symbols
            .iter()
            .find(|s| s.id.name == "loadUser")
            .expect("loadUser missing");
        assert_eq!(fn_sym.id.kind, SymbolKind::AsyncFunction);
        assert!(fn_sym.is_async);
        assert!(fn_sym.return_type.is_none());
    }

    #[test]
    fn library_import_captured() {
        let out = extract(&PathBuf::from("src/user.js"), SAMPLE);
        assert!(out.library_imports.iter().any(|li| li.library == "lodash"));
    }
}
```

- [ ] **Step 2: Run to verify failure**

```bash
cargo test -p blastguard parse::javascript::tests 2>&1 | head -40
```
Expected: both tests fail — `extract` is still the stub.

- [ ] **Step 3: Write `queries/javascript.scm`**

```scheme
(function_declaration
  name: (identifier) @function.name
  parameters: (formal_parameters) @function.params) @function.decl

((function_declaration) @async.decl
  (#match? @async.decl "^async"))

(class_declaration
  name: (identifier) @class.name) @class.decl

(method_definition
  name: (property_identifier) @method.name
  parameters: (formal_parameters) @method.params) @method.decl

(import_statement
  source: (string (string_fragment) @import.source)) @import.decl

(call_expression
  function: [(identifier) @call.callee (member_expression property: (property_identifier) @call.callee)]) @call.site

(export_statement) @export
```

- [ ] **Step 4: Implement `extract` in `src/parse/javascript.rs`**

The code mirrors the TS driver but uses `tree_sitter_javascript::LANGUAGE` and has no `return_type` handling. Copy the TS implementation, delete the return-type extraction, and swap the language/query file. Place the `thread_local!` parser and `QUERY_SRC = include_str!("../../queries/javascript.scm")` at the top.

- [ ] **Step 5: Run the tests and verify they pass**

```bash
cargo test -p blastguard parse::javascript::tests
```
Expected: 2 passed.

- [ ] **Step 6: Clippy**

```bash
cargo clippy -p blastguard --lib -- -W clippy::pedantic -D warnings
```

- [ ] **Step 7: Commit**

```bash
git add queries/javascript.scm src/parse/javascript.rs
git commit -m "phase 1.2: JavaScript driver"
```

---

## Task 4: Python driver

**Files:**
- Modify: `queries/python.scm`, `src/parse/python.rs`

- [ ] **Step 1: Write failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::types::SymbolKind;
    use std::path::PathBuf;

    const SAMPLE: &str = r#"
import os
from utils.auth import verify

async def process_request(req):
    return handler(req)

class Handler:
    def handle(self, req):
        return process_request(req)

def _private_helper():
    pass
"#;

    #[test]
    fn extracts_async_def() {
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
    fn library_and_internal_imports_separated() {
        let out = extract(&PathBuf::from("src/handler.py"), SAMPLE);
        assert!(out.library_imports.iter().any(|li| li.library == "os"));
        // `from utils.auth import verify` is internal-looking — for Phase 1.2 we
        // emit it as an unresolved Imports edge because Python resolution needs
        // the project root (Task 10).
        assert!(out.edges.iter().any(|e| e.kind == crate::graph::types::EdgeKind::Imports));
    }
}
```

- [ ] **Step 2: Verify failure**

```bash
cargo test -p blastguard parse::python::tests
```
Expected: all three fail.

- [ ] **Step 3: Write `queries/python.scm`**

```scheme
(function_definition
  name: (identifier) @function.name
  parameters: (parameters) @function.params
  return_type: (type)? @function.return) @function.decl

(class_definition
  name: (identifier) @class.name) @class.decl

(import_statement
  name: (dotted_name) @import.module) @import.simple

(import_from_statement
  module_name: (dotted_name) @import.from) @import.from_decl

(call
  function: [(identifier) @call.callee (attribute attribute: (identifier) @call.callee)]) @call.site
```

- [ ] **Step 4: Implement `extract` in `src/parse/python.rs`**

Mirror the TS driver with Python-specific tweaks:
- Detect `async` by scanning the node's first child for `async` keyword; if present, set `SymbolKind::AsyncFunction`.
- Distinguish methods from top-level functions by checking `node.parent().map(|p| p.kind()) == Some("class_definition")` (actually `block` inside `class_definition` — walk up).
- Visibility: name starting with `_` is `Visibility::Private`, else `Visibility::Export`.
- Library vs. internal imports: treat `import X` and `from X import ...` uniformly in Phase 1.2; Python resolution happens in Task 10.

- [ ] **Step 5: Verify tests pass**

```bash
cargo test -p blastguard parse::python::tests
```

- [ ] **Step 6: Clippy + commit**

```bash
cargo clippy -p blastguard --lib -- -W clippy::pedantic -D warnings
git add queries/python.scm src/parse/python.rs
git commit -m "phase 1.2: Python driver"
```

---

## Task 5: Rust driver

**Files:**
- Modify: `queries/rust.scm`, `src/parse/rust.rs`

- [ ] **Step 1: Write failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::types::{SymbolKind, Visibility};
    use std::path::PathBuf;

    const SAMPLE: &str = r#"
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

fn private_helper() {}
"#;

    #[test]
    fn extracts_async_fn_with_return_type() {
        let out = extract(&PathBuf::from("src/handler.rs"), SAMPLE);
        let sym = out.symbols.iter().find(|s| s.id.name == "process_request").expect("missing");
        assert_eq!(sym.id.kind, SymbolKind::AsyncFunction);
        assert!(sym.is_async);
        assert_eq!(sym.return_type.as_deref(), Some("Result<Response, Error>"));
    }

    #[test]
    fn extracts_struct_and_trait() {
        let out = extract(&PathBuf::from("src/handler.rs"), SAMPLE);
        assert!(out.symbols.iter().any(|s| s.id.name == "Handler" && s.id.kind == SymbolKind::Struct));
        assert!(out.symbols.iter().any(|s| s.id.name == "Service" && s.id.kind == SymbolKind::Trait));
    }

    #[test]
    fn visibility_detected() {
        let out = extract(&PathBuf::from("src/handler.rs"), SAMPLE);
        let priv_sym = out.symbols.iter().find(|s| s.id.name == "private_helper").expect("missing");
        assert_eq!(priv_sym.visibility, Visibility::Private);
        let pub_sym = out.symbols.iter().find(|s| s.id.name == "process_request").expect("missing");
        assert_eq!(pub_sym.visibility, Visibility::Export);
    }

    #[test]
    fn use_statement_captured() {
        let out = extract(&PathBuf::from("src/handler.rs"), SAMPLE);
        assert!(out.library_imports.iter().any(|li| li.library.starts_with("std::")));
        // `use crate::utils::helper;` is internal — Task 11 resolves it. Here
        // we assert it is captured as an import edge (unresolved).
        assert!(out.edges.iter().any(|e| e.kind == crate::graph::types::EdgeKind::Imports));
    }
}
```

- [ ] **Step 2: Verify failure**

```bash
cargo test -p blastguard parse::rust::tests
```

- [ ] **Step 3: Write `queries/rust.scm`**

```scheme
(function_item
  name: (identifier) @function.name
  parameters: (parameters) @function.params
  return_type: (_)? @function.return) @function.decl

(struct_item
  name: (type_identifier) @struct.name) @struct.decl

(enum_item
  name: (type_identifier) @enum.name) @enum.decl

(trait_item
  name: (type_identifier) @trait.name) @trait.decl

(impl_item) @impl.block

(use_declaration
  argument: (_) @use.path) @use.decl

(call_expression
  function: [(identifier) @call.callee (field_expression field: (field_identifier) @call.callee) (scoped_identifier)]) @call.site
```

- [ ] **Step 4: Implement `extract` in `src/parse/rust.rs`**

Mirror the pattern. Rust-specific notes:
- Async: check `function_item` children for `async` keyword.
- Visibility: look at the `visibility_modifier` child. `pub` → `Export`. `pub(crate)` → `Public`. Missing → `Private`.
- Methods: functions inside `impl_item` → `SymbolKind::Method`. Use `enclosing_impl(node)` to tell.
- External vs internal `use`: path starting with `crate::`, `self::`, `super::` is internal (emit Imports edge); everything else is a `LibraryImport` under the crate name (`std`, `tokio`, …). For simplicity split on `::` and take the first segment.

- [ ] **Step 5: Verify tests pass**

- [ ] **Step 6: Clippy + commit**

```bash
cargo clippy -p blastguard --lib -- -W clippy::pedantic -D warnings
git add queries/rust.scm src/parse/rust.rs
git commit -m "phase 1.2: Rust driver with visibility detection"
```

---

## Task 6: Parse-error graceful degradation

**Files:**
- Modify: `src/parse/typescript.rs` (and mirror to `javascript.rs`, `python.rs`, `rust.rs`)

- [ ] **Step 1: Write the failing test**

Add to `src/parse/typescript.rs` tests:
```rust
const BROKEN: &str = r#"
export function good() { return 1; }
this is { not valid :: syntax
export function alsoGood() { return 2; }
"#;

#[test]
fn parse_errors_set_partial_flag_but_still_extract() {
    let out = extract(&PathBuf::from("src/broken.ts"), BROKEN);
    assert!(out.partial_parse, "partial flag should be set");
    assert!(out.symbols.iter().any(|s| s.id.name == "good"));
    assert!(out.symbols.iter().any(|s| s.id.name == "alsoGood"));
}
```

- [ ] **Step 2: Run to verify it fails or passes accidentally**

```bash
cargo test -p blastguard parse::typescript::tests::parse_errors_set_partial_flag_but_still_extract
```
tree-sitter normally recovers, so the second symbol should already be captured; the assertion on `partial_parse = true` might already pass because we set it from `root.has_error()`. If so, mark as green. If not, tune extraction.

- [ ] **Step 3: Mirror the test to the three other language modules**

Paste the equivalent broken-input test into `javascript.rs`, `python.rs`, `rust.rs` and verify each sets `partial_parse = true` while still yielding the valid symbols.

- [ ] **Step 4: Clippy + commit**

```bash
cargo clippy -p blastguard --lib -- -W clippy::pedantic -D warnings
git add src/parse/typescript.rs src/parse/javascript.rs src/parse/python.rs src/parse/rust.rs
git commit -m "phase 1.2: verify graceful partial-parse recovery across drivers"
```

---

## Task 7: Shared signature-rendering helper

**Files:**
- Modify: `src/parse/symbols.rs`
- Modify: `src/parse/typescript.rs`, `javascript.rs`, `python.rs`, `rust.rs` (call the helper)

Signature rendering duplicated across four drivers is a DRY violation. Extract it.

- [ ] **Step 1: Write the test**

Add to `src/parse/symbols.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_signature_formats_params_and_return() {
        assert_eq!(
            render_signature("process", "(req: Request, res: Response)", Some("Promise<void>")),
            "process(req: Request, res: Response): Promise<void>"
        );
    }

    #[test]
    fn render_signature_omits_empty_return() {
        assert_eq!(
            render_signature("helper", "(x)", None),
            "helper(x)"
        );
    }
}
```

- [ ] **Step 2: Implement `render_signature`**

```rust
#[must_use]
pub fn render_signature(name: &str, params: &str, return_type: Option<&str>) -> String {
    match return_type {
        Some(ret) if !ret.is_empty() => format!("{name}{params}: {ret}"),
        _ => format!("{name}{params}"),
    }
}
```

- [ ] **Step 3: Replace inline formatting in each driver**

Search each `parse/{ts,js,py,rs}.rs` for `let signature = format!(...)` in the function-emitting path and replace with `let signature = crate::parse::symbols::render_signature(&name, &params, return_type.as_deref());`.

- [ ] **Step 4: Run the full parse test suite**

```bash
cargo test -p blastguard parse::
```
Expected: every parse test still green.

- [ ] **Step 5: Clippy + commit**

```bash
cargo clippy -p blastguard --lib -- -W clippy::pedantic -D warnings
git add src/parse/
git commit -m "phase 1.2: shared render_signature helper (DRY)"
```

---

## Task 8: TypeScript import resolver (relative paths)

**Files:**
- Modify: `src/parse/resolve.rs`
- Test: same file, inline tests.

- [ ] **Step 1: Write failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn tempdir_with(files: &[(&str, &str)]) -> tempfile::TempDir {
        let tmp = tempfile::tempdir().expect("tempdir");
        for (path, body) in files {
            let full = tmp.path().join(path);
            std::fs::create_dir_all(full.parent().unwrap()).expect("mkdir");
            std::fs::write(&full, body).expect("write");
        }
        tmp
    }

    #[test]
    fn resolves_relative_ts_file() {
        let tmp = tempdir_with(&[
            ("src/handler.ts", ""),
            ("src/utils/auth.ts", ""),
        ]);
        let from = tmp.path().join("src/handler.ts");
        let r = resolve_ts(tmp.path(), &from, "./utils/auth", None);
        assert_eq!(r, ResolveResult::Internal(tmp.path().join("src/utils/auth.ts")));
    }

    #[test]
    fn resolves_index_file() {
        let tmp = tempdir_with(&[
            ("src/handler.ts", ""),
            ("src/utils/index.ts", ""),
        ]);
        let from = tmp.path().join("src/handler.ts");
        let r = resolve_ts(tmp.path(), &from, "./utils", None);
        assert_eq!(r, ResolveResult::Internal(tmp.path().join("src/utils/index.ts")));
    }

    #[test]
    fn bare_specifier_is_external() {
        let tmp = tempdir_with(&[("src/a.ts", "")]);
        let from = tmp.path().join("src/a.ts");
        let r = resolve_ts(tmp.path(), &from, "lodash", None);
        match r {
            ResolveResult::External { library, .. } => assert_eq!(library, "lodash"),
            _ => panic!("expected External"),
        }
    }

    #[test]
    fn unresolved_when_no_match() {
        let tmp = tempdir_with(&[("src/a.ts", "")]);
        let from = tmp.path().join("src/a.ts");
        let r = resolve_ts(tmp.path(), &from, "./missing", None);
        assert_eq!(r, ResolveResult::Unresolved);
    }
}
```

- [ ] **Step 2: Run to verify failure**

```bash
cargo test -p blastguard parse::resolve::tests
```

- [ ] **Step 3: Implement `resolve_ts` and supporting types**

```rust
use std::path::{Path, PathBuf};

#[derive(Debug, Default, Clone)]
pub struct TsConfig {
    pub base_url: Option<PathBuf>,
    pub paths: std::collections::HashMap<String, Vec<String>>,
}

#[must_use]
pub fn resolve_ts(
    project_root: &Path,
    from_file: &Path,
    spec: &str,
    tsconfig: Option<&TsConfig>,
) -> ResolveResult {
    if let Some(tc) = tsconfig {
        if let Some(hit) = resolve_via_tsconfig(project_root, spec, tc) {
            return hit;
        }
    }

    if !spec.starts_with('.') && !spec.starts_with('/') {
        return ResolveResult::External {
            library: spec.split('/').next().unwrap_or(spec).to_string(),
            symbols: vec![],
        };
    }

    let base = from_file.parent().unwrap_or(project_root);
    let joined = base.join(spec);
    try_ts_suffixes(&joined)
        .or_else(|| try_ts_index(&joined))
        .map_or(ResolveResult::Unresolved, ResolveResult::Internal)
}

fn try_ts_suffixes(candidate: &Path) -> Option<PathBuf> {
    for ext in ["ts", "tsx", "js", "jsx", "mts", "cts"] {
        let with_ext = candidate.with_extension(ext);
        if with_ext.is_file() {
            return Some(with_ext);
        }
    }
    if candidate.is_file() {
        return Some(candidate.to_path_buf());
    }
    None
}

fn try_ts_index(dir: &Path) -> Option<PathBuf> {
    for ext in ["ts", "tsx", "js", "jsx"] {
        let index = dir.join(format!("index.{ext}"));
        if index.is_file() {
            return Some(index);
        }
    }
    None
}

fn resolve_via_tsconfig(_project_root: &Path, _spec: &str, _tc: &TsConfig) -> Option<ResolveResult> {
    // Filled in Task 9.
    None
}
```

- [ ] **Step 4: Verify tests pass**

```bash
cargo test -p blastguard parse::resolve::tests
```

- [ ] **Step 5: Clippy + commit**

```bash
cargo clippy -p blastguard --lib -- -W clippy::pedantic -D warnings
git add src/parse/resolve.rs
git commit -m "phase 1.3: TS/JS relative import resolver"
```

---

## Task 9: tsconfig.json path alias resolution

**Files:**
- Modify: `src/parse/resolve.rs`

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn resolves_via_tsconfig_path_alias() {
    let tmp = tempdir_with(&[
        ("src/handler.ts", ""),
        ("src/shared/auth.ts", ""),
    ]);
    let mut tc = TsConfig::default();
    tc.base_url = Some(PathBuf::from("."));
    tc.paths.insert("@shared/*".to_string(), vec!["src/shared/*".to_string()]);
    let from = tmp.path().join("src/handler.ts");
    let r = resolve_ts(tmp.path(), &from, "@shared/auth", Some(&tc));
    assert_eq!(r, ResolveResult::Internal(tmp.path().join("src/shared/auth.ts")));
}

#[test]
fn loads_tsconfig_from_disk() {
    let tmp = tempdir_with(&[
        ("tsconfig.json", r#"{
            "compilerOptions": {
                "baseUrl": ".",
                "paths": { "@shared/*": ["src/shared/*"] }
            }
        }"#),
    ]);
    let tc = load_tsconfig(tmp.path()).expect("load").expect("present");
    assert!(tc.paths.contains_key("@shared/*"));
}
```

- [ ] **Step 2: Run tests — expect failure**

```bash
cargo test -p blastguard parse::resolve::tests::resolves_via_tsconfig_path_alias
cargo test -p blastguard parse::resolve::tests::loads_tsconfig_from_disk
```

- [ ] **Step 3: Implement `load_tsconfig` and fill in `resolve_via_tsconfig`**

```rust
use crate::error::{BlastGuardError, Result};

/// Load and parse `tsconfig.json` from the project root. Returns `None` when
/// no `tsconfig.json` is present.
///
/// # Errors
/// Returns a [`BlastGuardError::Config`] on malformed JSON.
pub fn load_tsconfig(project_root: &Path) -> Result<Option<TsConfig>> {
    let path = project_root.join("tsconfig.json");
    if !path.exists() {
        return Ok(None);
    }
    let body = std::fs::read_to_string(&path).map_err(|source| BlastGuardError::Io {
        path: path.clone(),
        source,
    })?;
    // tsconfig.json sometimes has // comments — strip them crudely.
    let stripped = strip_jsonc_comments(&body);
    let v: serde_json::Value = serde_json::from_str(&stripped)
        .map_err(|e| BlastGuardError::Config(format!("tsconfig.json: {e}")))?;
    let co = v.get("compilerOptions");
    let base_url = co
        .and_then(|c| c.get("baseUrl"))
        .and_then(|b| b.as_str())
        .map(PathBuf::from);
    let mut paths_map = std::collections::HashMap::new();
    if let Some(paths) = co.and_then(|c| c.get("paths")).and_then(|p| p.as_object()) {
        for (k, v) in paths {
            let targets: Vec<String> = v
                .as_array()
                .map(|arr| arr.iter().filter_map(|s| s.as_str().map(ToString::to_string)).collect())
                .unwrap_or_default();
            paths_map.insert(k.clone(), targets);
        }
    }
    Ok(Some(TsConfig { base_url, paths: paths_map }))
}

fn strip_jsonc_comments(src: &str) -> String {
    src.lines()
        .map(|l| match l.find("//") {
            Some(i) => &l[..i],
            None => l,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn resolve_via_tsconfig(project_root: &Path, spec: &str, tc: &TsConfig) -> Option<ResolveResult> {
    for (pattern, targets) in &tc.paths {
        let Some(prefix) = pattern.strip_suffix("*") else {
            if pattern == spec {
                for target in targets {
                    let base = tc.base_url.as_deref().unwrap_or(Path::new("."));
                    let candidate = project_root.join(base).join(target);
                    if let Some(resolved) = try_ts_suffixes(&candidate).or_else(|| try_ts_index(&candidate)) {
                        return Some(ResolveResult::Internal(resolved));
                    }
                }
            }
            continue;
        };
        if let Some(rest) = spec.strip_prefix(prefix) {
            for target in targets {
                let Some(t_prefix) = target.strip_suffix("*") else {
                    continue;
                };
                let base = tc.base_url.as_deref().unwrap_or(Path::new("."));
                let candidate = project_root.join(base).join(format!("{t_prefix}{rest}"));
                if let Some(resolved) = try_ts_suffixes(&candidate).or_else(|| try_ts_index(&candidate)) {
                    return Some(ResolveResult::Internal(resolved));
                }
            }
        }
    }
    None
}
```

- [ ] **Step 4: Run tests**

```bash
cargo test -p blastguard parse::resolve::tests
```

- [ ] **Step 5: Clippy + commit**

```bash
cargo clippy -p blastguard --lib -- -W clippy::pedantic -D warnings
git add src/parse/resolve.rs
git commit -m "phase 1.3: tsconfig.json path alias resolution"
```

---

## Task 10: Python import resolver

**Files:**
- Modify: `src/parse/resolve.rs`

- [ ] **Step 1: Write failing tests**

```rust
#[test]
fn resolves_python_dotted_module() {
    let tmp = tempdir_with(&[
        ("src/handler.py", ""),
        ("src/utils/auth.py", ""),
        ("src/utils/__init__.py", ""),
    ]);
    let from = tmp.path().join("src/handler.py");
    let r = resolve_py(tmp.path(), &from, "utils.auth");
    assert_eq!(r, ResolveResult::Internal(tmp.path().join("src/utils/auth.py")));
}

#[test]
fn resolves_python_package_init() {
    let tmp = tempdir_with(&[
        ("src/handler.py", ""),
        ("src/utils/__init__.py", ""),
    ]);
    let from = tmp.path().join("src/handler.py");
    let r = resolve_py(tmp.path(), &from, "utils");
    assert_eq!(r, ResolveResult::Internal(tmp.path().join("src/utils/__init__.py")));
}

#[test]
fn unresolved_python_falls_back_to_library() {
    let tmp = tempdir_with(&[("src/a.py", "")]);
    let from = tmp.path().join("src/a.py");
    let r = resolve_py(tmp.path(), &from, "numpy");
    match r {
        ResolveResult::External { library, .. } => assert_eq!(library, "numpy"),
        _ => panic!("expected External"),
    }
}
```

- [ ] **Step 2: Run — expect failure**

```bash
cargo test -p blastguard parse::resolve::tests::resolves_python_dotted_module
```

- [ ] **Step 3: Implement `resolve_py`**

```rust
#[must_use]
pub fn resolve_py(project_root: &Path, _from_file: &Path, spec: &str) -> ResolveResult {
    let rel: PathBuf = spec.split('.').collect();
    let candidates = [
        project_root.join("src").join(&rel).with_extension("py"),
        project_root.join("src").join(&rel).join("__init__.py"),
        project_root.join(&rel).with_extension("py"),
        project_root.join(&rel).join("__init__.py"),
    ];
    for c in candidates {
        if c.is_file() {
            return ResolveResult::Internal(c);
        }
    }
    ResolveResult::External {
        library: spec.split('.').next().unwrap_or(spec).to_string(),
        symbols: vec![],
    }
}
```

- [ ] **Step 4: Run tests**

```bash
cargo test -p blastguard parse::resolve::tests
```

- [ ] **Step 5: Commit**

```bash
git add src/parse/resolve.rs
git commit -m "phase 1.3: Python import resolver"
```

---

## Task 11: Rust import resolver

**Files:**
- Modify: `src/parse/resolve.rs`

- [ ] **Step 1: Write failing tests**

```rust
#[test]
fn resolves_rust_crate_path() {
    let tmp = tempdir_with(&[
        ("src/main.rs", ""),
        ("src/utils/auth.rs", ""),
        ("src/utils/mod.rs", ""),
    ]);
    let from = tmp.path().join("src/main.rs");
    let r = resolve_rs(tmp.path(), &from, "crate::utils::auth");
    assert_eq!(r, ResolveResult::Internal(tmp.path().join("src/utils/auth.rs")));
}

#[test]
fn resolves_rust_mod_rs_package() {
    let tmp = tempdir_with(&[
        ("src/main.rs", ""),
        ("src/utils/mod.rs", ""),
    ]);
    let from = tmp.path().join("src/main.rs");
    let r = resolve_rs(tmp.path(), &from, "crate::utils");
    assert_eq!(r, ResolveResult::Internal(tmp.path().join("src/utils/mod.rs")));
}

#[test]
fn external_crate_path() {
    let tmp = tempdir_with(&[("src/main.rs", "")]);
    let from = tmp.path().join("src/main.rs");
    let r = resolve_rs(tmp.path(), &from, "tokio::sync::Mutex");
    match r {
        ResolveResult::External { library, .. } => assert_eq!(library, "tokio"),
        _ => panic!("expected External"),
    }
}
```

- [ ] **Step 2: Run — expect failure**

- [ ] **Step 3: Implement `resolve_rs`**

```rust
#[must_use]
pub fn resolve_rs(project_root: &Path, _from_file: &Path, spec: &str) -> ResolveResult {
    let head = spec.split("::").next().unwrap_or("");
    match head {
        "crate" | "self" | "super" => {
            let rest: Vec<&str> = spec.split("::").skip(1).collect();
            if rest.is_empty() {
                return ResolveResult::Unresolved;
            }
            let rel: PathBuf = rest.iter().collect();
            let candidates = [
                project_root.join("src").join(&rel).with_extension("rs"),
                project_root.join("src").join(&rel).join("mod.rs"),
            ];
            for c in candidates {
                if c.is_file() {
                    return ResolveResult::Internal(c);
                }
            }
            ResolveResult::Unresolved
        }
        _ => ResolveResult::External {
            library: head.to_string(),
            symbols: vec![],
        },
    }
}
```

- [ ] **Step 4: Run tests**

```bash
cargo test -p blastguard parse::resolve::tests
```

- [ ] **Step 5: Commit**

```bash
git add src/parse/resolve.rs
git commit -m "phase 1.3: Rust import resolver"
```

---

## Task 12: File walker via `ignore` crate

**Files:**
- Modify: `src/index/indexer.rs`

- [ ] **Step 1: Write failing test**

```rust
#[cfg(test)]
mod walker_tests {
    use super::*;

    fn mk(dir: &std::path::Path, files: &[&str]) {
        for rel in files {
            let full = dir.join(rel);
            if let Some(p) = full.parent() {
                std::fs::create_dir_all(p).unwrap();
            }
            std::fs::write(&full, "").unwrap();
        }
    }

    #[test]
    fn walks_source_files_and_respects_gitignore() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join(".gitignore"), "node_modules/\n").unwrap();
        mk(tmp.path(), &[
            "src/a.ts",
            "src/b.py",
            "src/c.rs",
            "node_modules/skip.ts",
            "README.md",
        ]);
        let files = walk_project(tmp.path());
        let rels: Vec<_> = files.iter().map(|f| f.strip_prefix(tmp.path()).unwrap().to_path_buf()).collect();
        assert!(rels.iter().any(|p| p.ends_with("a.ts")));
        assert!(rels.iter().any(|p| p.ends_with("b.py")));
        assert!(rels.iter().any(|p| p.ends_with("c.rs")));
        assert!(!rels.iter().any(|p| p.components().any(|c| c.as_os_str() == "node_modules")));
        assert!(!rels.iter().any(|p| p.ends_with("README.md")));
    }
}
```

- [ ] **Step 2: Run — expect failure (compile or test)**

- [ ] **Step 3: Implement `walk_project`**

```rust
use std::path::{Path, PathBuf};

/// Walk the project root respecting `.gitignore`, filtering to files that
/// one of the language drivers can parse.
pub fn walk_project(project_root: &Path) -> Vec<PathBuf> {
    ignore::WalkBuilder::new(project_root)
        .standard_filters(true)
        .hidden(false) // allow .github, .vscode — up to .gitignore to exclude
        .build()
        .filter_map(std::result::Result::ok)
        .filter(|e| e.file_type().map_or(false, |t| t.is_file()))
        .filter(|e| crate::parse::detect_language(e.path()).is_some())
        .map(|e| e.into_path())
        .collect()
}
```

- [ ] **Step 4: Run tests**

```bash
cargo test -p blastguard index::indexer::walker_tests
```

- [ ] **Step 5: Commit**

```bash
git add src/index/indexer.rs
git commit -m "phase 1.4: project walker via `ignore` crate"
```

---

## Task 13: Parallel parsing fan-out via rayon

**Files:**
- Modify: `src/index/indexer.rs`

- [ ] **Step 1: Write failing test**

```rust
#[test]
fn parses_ts_py_rs_files_in_parallel() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join("src")).unwrap();
    std::fs::write(tmp.path().join("src/a.ts"),
        "export function foo() { return 1; }").unwrap();
    std::fs::write(tmp.path().join("src/b.py"),
        "def bar():\n    return 1\n").unwrap();
    std::fs::write(tmp.path().join("src/c.rs"),
        "pub fn baz() -> i32 { 1 }").unwrap();

    let graph = cold_index(tmp.path()).expect("index");
    assert!(graph.symbols.keys().any(|id| id.name == "foo"));
    assert!(graph.symbols.keys().any(|id| id.name == "bar"));
    assert!(graph.symbols.keys().any(|id| id.name == "baz"));
}
```

- [ ] **Step 2: Run — expect failure**

- [ ] **Step 3: Implement `cold_index`**

Replace the stub in `src/index/indexer.rs`:
```rust
use rayon::prelude::*;

use crate::graph::types::CodeGraph;
use crate::parse::{detect_language, Language, ParseOutput};
use crate::Result;

pub fn cold_index(project_root: &Path) -> Result<CodeGraph> {
    let files = walk_project(project_root);
    let parses: Vec<ParseOutput> = files
        .par_iter()
        .filter_map(|path| {
            let source = std::fs::read_to_string(path).ok()?;
            let out = match detect_language(path)? {
                Language::TypeScript => crate::parse::typescript::extract(path, &source),
                Language::JavaScript => crate::parse::javascript::extract(path, &source),
                Language::Python => crate::parse::python::extract(path, &source),
                Language::Rust => crate::parse::rust::extract(path, &source),
            };
            Some(out)
        })
        .collect();

    let mut graph = CodeGraph::new();
    for out in parses {
        for sym in out.symbols {
            graph.insert_symbol(sym);
        }
        for edge in out.edges {
            graph.insert_edge(edge);
        }
        graph.library_imports.extend(out.library_imports);
    }
    Ok(graph)
}
```

- [ ] **Step 4: Run the test**

```bash
cargo test -p blastguard index::indexer::tests::parses_ts_py_rs_files_in_parallel
```

- [ ] **Step 5: Commit**

```bash
git add src/index/indexer.rs
git commit -m "phase 1.4: parallel parse fan-out via rayon"
```

---

## Task 14: BLAKE3 file and Merkle directory hashing

**Files:**
- Modify: `src/index/cache.rs`

- [ ] **Step 1: Write failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn file_hash_is_stable_for_same_content() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("a.ts");
        std::fs::write(&p, b"hello").unwrap();
        let h1 = hash_file(&p).expect("hash");
        let h2 = hash_file(&p).expect("hash");
        assert_eq!(h1, h2);
    }

    #[test]
    fn directory_hash_changes_when_child_changes() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("a.ts"), b"one").unwrap();
        let h1 = hash_directory_tree(tmp.path()).expect("hash");
        std::fs::write(tmp.path().join("a.ts"), b"two").unwrap();
        let h2 = hash_directory_tree(tmp.path()).expect("hash");
        assert_ne!(h1, h2);
    }
}
```

- [ ] **Step 2: Run — expect failure**

- [ ] **Step 3: Implement `hash_file` and `hash_directory_tree`**

```rust
use std::io::Read;
use std::path::Path;

use crate::error::{BlastGuardError, Result};

/// BLAKE3 hash of a file's full content. Streams so we don't OOM on huge files.
pub fn hash_file(path: &Path) -> Result<u64> {
    let mut hasher = blake3::Hasher::new();
    let mut file = std::fs::File::open(path).map_err(|source| BlastGuardError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let mut buf = [0u8; 8192];
    loop {
        let n = file.read(&mut buf).map_err(|source| BlastGuardError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    // Take the first 8 bytes of the 32-byte hash as a u64. 64 bits of entropy
    // is plenty for cache keying; full hashes waste 24 bytes per file.
    let digest = hasher.finalize();
    Ok(u64::from_le_bytes(digest.as_bytes()[..8].try_into().expect("8 bytes")))
}

/// Merkle hash of a directory tree: hash of the sorted concat of child
/// names and their own hashes (files or recursive subdir hashes).
pub fn hash_directory_tree(dir: &Path) -> Result<u64> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .map_err(|source| BlastGuardError::Io { path: dir.to_path_buf(), source })?
        .filter_map(std::result::Result::ok)
        .collect();
    entries.sort_by_key(std::fs::DirEntry::path);

    let mut hasher = blake3::Hasher::new();
    for entry in entries {
        let ft = entry.file_type().map_err(|source| BlastGuardError::Io {
            path: entry.path(),
            source,
        })?;
        let name = entry.file_name();
        hasher.update(name.to_string_lossy().as_bytes());
        let child_hash = if ft.is_dir() {
            hash_directory_tree(&entry.path())?
        } else if ft.is_file() {
            hash_file(&entry.path())?
        } else {
            continue;
        };
        hasher.update(&child_hash.to_le_bytes());
    }
    let digest = hasher.finalize();
    Ok(u64::from_le_bytes(digest.as_bytes()[..8].try_into().expect("8 bytes")))
}
```

- [ ] **Step 4: Run tests**

```bash
cargo test -p blastguard index::cache::tests
```

- [ ] **Step 5: Commit**

```bash
git add src/index/cache.rs
git commit -m "phase 1.4: BLAKE3 file + Merkle directory hashing"
```

---

## Task 15: Cache persistence (rmp-serde)

**Files:**
- Modify: `src/index/cache.rs`

- [ ] **Step 1: Write failing tests**

```rust
#[test]
fn round_trip_cache_file() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join(".blastguard").join("cache.bin");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();

    let original = CacheFile {
        version: CACHE_VERSION,
        ..CacheFile::default()
    };
    save(&path, &original).expect("save");
    let loaded = load(&path).expect("load").expect("present");
    assert_eq!(loaded.version, CACHE_VERSION);
}

#[test]
fn version_mismatch_returns_none() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("cache.bin");
    let stale = CacheFile { version: 0, ..CacheFile::default() };
    save(&path, &stale).expect("save");
    assert!(load(&path).expect("load").is_none());
}
```

- [ ] **Step 2: Run — expect failure**

- [ ] **Step 3: Implement `save` and `load`**

```rust
pub fn save(path: &Path, cache: &CacheFile) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| BlastGuardError::Io {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    let bytes = rmp_serde::to_vec(cache)
        .map_err(|e| BlastGuardError::CacheCorrupt(e.to_string()))?;
    std::fs::write(path, bytes).map_err(|source| BlastGuardError::Io {
        path: path.to_path_buf(),
        source,
    })
}

pub fn load(path: &Path) -> Result<Option<CacheFile>> {
    if !path.exists() {
        return Ok(None);
    }
    let bytes = std::fs::read(path).map_err(|source| BlastGuardError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let cache: CacheFile = rmp_serde::from_slice(&bytes)
        .map_err(|e| BlastGuardError::CacheCorrupt(e.to_string()))?;
    if cache.version != CACHE_VERSION {
        return Ok(None);
    }
    Ok(Some(cache))
}
```

- [ ] **Step 4: Run tests + commit**

```bash
cargo test -p blastguard index::cache::tests
cargo clippy -p blastguard --lib -- -W clippy::pedantic -D warnings
git add src/index/cache.rs
git commit -m "phase 1.4: rmp-serde cache persistence with version gate"
```

---

## Task 16: Warm-start incremental reindex

**Files:**
- Modify: `src/index/indexer.rs`

- [ ] **Step 1: Write failing test**

```rust
#[test]
fn warm_start_is_faster_than_cold_after_first_run() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join("src")).unwrap();
    for i in 0..20 {
        std::fs::write(
            tmp.path().join(format!("src/m{i}.ts")),
            "export function f(){ return 1; }",
        ).unwrap();
    }

    let t0 = std::time::Instant::now();
    let _ = cold_index(tmp.path()).unwrap();
    let cold_elapsed = t0.elapsed();

    // Save cache
    let cache_path = tmp.path().join(".blastguard").join("cache.bin");
    let cache = crate::index::cache::CacheFile {
        version: crate::index::cache::CACHE_VERSION,
        ..Default::default()
    };
    crate::index::cache::save(&cache_path, &cache).unwrap();

    let t1 = std::time::Instant::now();
    let _ = warm_start(tmp.path()).unwrap();
    let warm_elapsed = t1.elapsed();

    assert!(warm_elapsed < cold_elapsed, "warm should beat cold: warm={:?} cold={:?}", warm_elapsed, cold_elapsed);
}
```

- [ ] **Step 2: Run — expect failure**

- [ ] **Step 3: Implement `warm_start`**

```rust
pub fn warm_start(project_root: &Path) -> Result<CodeGraph> {
    let cache_path = project_root.join(".blastguard").join("cache.bin");
    let Some(cache) = crate::index::cache::load(&cache_path)? else {
        return cold_index(project_root);
    };

    // Compute current directory tree hash. If unchanged, the cache is reusable verbatim.
    let current_root = crate::index::cache::hash_directory_tree(project_root)?;
    let cached_root = cache.tree_hashes.get(project_root).copied().unwrap_or(0);
    if current_root == cached_root {
        return Ok(cache.graph);
    }

    // Otherwise, walk + hash files, skip those whose hash matches cache, reparse the rest.
    let files = walk_project(project_root);
    let mut graph = cache.graph;
    let mut changed = Vec::new();
    for path in &files {
        let h = crate::index::cache::hash_file(path)?;
        if cache.file_hashes.get(path).copied() != Some(h) {
            changed.push(path.clone());
        }
    }

    for path in &changed {
        graph.remove_file(path);
    }

    let parses: Vec<_> = changed
        .par_iter()
        .filter_map(|path| {
            let source = std::fs::read_to_string(path).ok()?;
            let out = match detect_language(path)? {
                Language::TypeScript => crate::parse::typescript::extract(path, &source),
                Language::JavaScript => crate::parse::javascript::extract(path, &source),
                Language::Python => crate::parse::python::extract(path, &source),
                Language::Rust => crate::parse::rust::extract(path, &source),
            };
            Some(out)
        })
        .collect();

    for out in parses {
        for sym in out.symbols {
            graph.insert_symbol(sym);
        }
        for edge in out.edges {
            graph.insert_edge(edge);
        }
        graph.library_imports.extend(out.library_imports);
    }

    Ok(graph)
}
```

- [ ] **Step 4: Run tests**

```bash
cargo test -p blastguard index::indexer::tests::warm_start_is_faster_than_cold_after_first_run
```

- [ ] **Step 5: Commit**

```bash
git add src/index/indexer.rs
git commit -m "phase 1.4: warm-start incremental reindex"
```

---

## Task 17: Integration test fixture + end-to-end test

**Files:**
- Create: `tests/fixtures/sample_project/` with TS + Python + Rust files
- Create: `tests/integration_indexer.rs`

- [ ] **Step 1: Create the fixture**

```bash
mkdir -p /home/adam/Documents/blastguard/tests/fixtures/sample_project/src/utils
```

Files:
- `tests/fixtures/sample_project/src/handler.ts`:
  ```ts
  import { helper } from "./utils/helper";
  export async function processRequest(req: Request): Promise<void> {
    return helper(req);
  }
  ```
- `tests/fixtures/sample_project/src/utils/helper.ts`:
  ```ts
  export function helper(req: any): void {}
  ```
- `tests/fixtures/sample_project/src/worker.py`:
  ```python
  from utils.auth import verify
  def run():
      return verify()
  ```
- `tests/fixtures/sample_project/src/utils/auth.py`:
  ```python
  def verify():
      return True
  ```
- `tests/fixtures/sample_project/src/lib.rs`:
  ```rust
  pub fn start() -> i32 { helper() }
  fn helper() -> i32 { 0 }
  ```
- `tests/fixtures/sample_project/tsconfig.json`:
  ```json
  { "compilerOptions": { "baseUrl": "." } }
  ```
- `tests/fixtures/sample_project/.gitignore`:
  ```
  /target
  ```

- [ ] **Step 2: Write `tests/integration_indexer.rs`**

```rust
use blastguard::index::indexer::cold_index;

#[test]
fn indexes_mixed_language_fixture() {
    let root = std::path::Path::new("tests/fixtures/sample_project");
    let graph = cold_index(root).expect("index");
    assert!(graph.symbols.keys().any(|id| id.name == "processRequest"));
    assert!(graph.symbols.keys().any(|id| id.name == "verify"));
    assert!(graph.symbols.keys().any(|id| id.name == "start"));
    assert!(!graph.file_symbols.is_empty());
}
```

- [ ] **Step 3: Run the integration test**

```bash
cargo test --test integration_indexer
```

- [ ] **Step 4: Commit**

```bash
git add tests/
git commit -m "phase 1.4: integration test over TS+Py+Rust fixture"
```

---

## Task 18: Final verification gate

- [ ] **Step 1: Run all checks**

```bash
cd /home/adam/Documents/blastguard
cargo check --all-targets
cargo test
cargo clippy --all-targets -- -W clippy::pedantic -D warnings
cargo build --release
```

All four must pass with zero warnings. If clippy fires, fix the root cause — do not `#[allow]` without a written reason.

- [ ] **Step 2: Measure cold + warm timing**

```bash
cargo run --release -- tests/fixtures/sample_project 2>&1 | tee /tmp/bg-cold.log
cargo run --release -- tests/fixtures/sample_project 2>&1 | tee /tmp/bg-warm.log
```
Expected: second run completes visibly faster than the first.

- [ ] **Step 3: Commit + push**

```bash
git add -A
git commit --allow-empty -m "phase 1.4: verification gate passed (check/test/clippy/build)"
```

Do NOT push unless the user explicitly asks.

---

## Self-Review

**Spec coverage check:**
- §7 data structures — Task 0 (pre-existing work) + `src/graph/types.rs` ✓
- §8 tree-sitter drivers — Tasks 1–7 (TS/JS/PY/RS + signature helper + degradation) ✓
- §6 import resolution — Tasks 8–11 (TS + tsconfig, PY, RS) ✓
- §9 graph cache — Tasks 14–15 ✓
- §10 cold + warm index — Tasks 12–13, 16 ✓
- §11 file watcher — DEFERRED to Plan 3 (1.9) ✓
- §13 graceful degradation — Task 6 ✓

**Placeholder scan:** No "TBD" or "add appropriate error handling" markers in tasks. `TODO(plan-2)` / `TODO(plan-3)` in stub files are explicit forward references with target plan numbers.

**Type consistency:** `ParseOutput { symbols, edges, library_imports, partial_parse }` used identically across all driver tasks. `ResolveResult::{Internal, External, Unresolved}` stable. `CacheFile { version, file_hashes, tree_hashes, graph, tsconfig }` matches SPEC §9.

---

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-04-18-blastguard-phase-1-indexing-pipeline.md`. Two execution options:

**1. Subagent-Driven (recommended)** — I dispatch a fresh subagent per task, review between tasks, fast iteration. Best for tasks with clear test → implement → green → commit loops.

**2. Inline Execution** — I execute tasks in this session using `executing-plans`, batch execution with checkpoints for your review. Best if you want to watch each step live.

Which approach?
