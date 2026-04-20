//! On-disk file edit primitive.
//!
//! [`apply_edit`] performs one `old_text → new_text` swap in the target
//! file. If `old_text` appears exactly once, the swap succeeds. If it
//! doesn't appear or appears multiple times, returns an error from
//! [`crate::error::BlastGuardError`]; the `apply_change` orchestrator
//! (Task 12) maps those into `CallToolResult { is_error: true, .. }`.
//!
//! Task 3 extends [`BlastGuardError::EditNotFound`] with closest-line
//! hints; Task 4 populates [`BlastGuardError::AmbiguousEdit::lines`].

use std::path::Path;
use std::sync::Mutex;

use crate::edit::context;
use crate::edit::diff;
use crate::edit::request::{ApplyChangeRequest, ApplyChangeResponse, ApplyStatus, BundledContext};
use crate::error::{BlastGuardError, Result};
use crate::graph::impact::{
    detect_async_change, detect_interface_break, detect_orphan, detect_signature, summary_line,
    Warning,
};
use crate::graph::types::{CodeGraph, Symbol};
use crate::parse::{detect_language, Language};
use crate::session::SessionState;

/// Scan `body` for the line with the highest normalised-Levenshtein
/// similarity to `needle`. Returns `(line_number_1_based, similarity_0_to_1, fragment)`.
fn closest_line(body: &str, needle: &str) -> (u32, f32, String) {
    let mut best_line: u32 = 0;
    let mut best_sim: f32 = 0.0;
    let mut best_fragment = String::new();
    for (idx, line) in body.lines().enumerate() {
        let dist = strsim::levenshtein(line, needle);
        let max_len = line.len().max(needle.len()).max(1);
        #[allow(clippy::cast_precision_loss)]
        let sim = 1.0_f32 - (dist as f32 / max_len as f32);
        if sim > best_sim {
            best_sim = sim;
            best_line = u32::try_from(idx).unwrap_or(u32::MAX).saturating_add(1);
            best_fragment = line.to_string();
        }
    }
    (best_line, best_sim, best_fragment)
}

/// Replace the single occurrence of `old_text` with `new_text` in `path`.
///
/// # Errors
/// - [`BlastGuardError::Io`] on read/write failure.
/// - [`BlastGuardError::EditNotFound`] when `old_text` doesn't appear.
/// - [`BlastGuardError::AmbiguousEdit`] when `old_text` appears 2+ times.
pub fn apply_edit(path: &Path, old_text: &str, new_text: &str) -> Result<()> {
    let body = std::fs::read_to_string(path).map_err(|source| BlastGuardError::Io {
        path: path.to_path_buf(),
        source,
    })?;

    let occurrences = body.matches(old_text).count();
    match occurrences {
        0 => {
            let (line, similarity, fragment) = closest_line(&body, old_text);
            Err(BlastGuardError::EditNotFound {
                path: path.to_path_buf(),
                line,
                similarity,
                fragment,
            })
        }
        1 => {
            let updated = body.replacen(old_text, new_text, 1);
            std::fs::write(path, updated).map_err(|source| BlastGuardError::Io {
                path: path.to_path_buf(),
                source,
            })
        }
        n => {
            let lines = find_match_lines(&body, old_text);
            Err(BlastGuardError::AmbiguousEdit {
                path: path.to_path_buf(),
                count: n,
                lines,
            })
        }
    }
}

/// Enumerate 1-based line numbers where `needle` appears in `body`.
/// Multi-line needles count once per starting line.
fn find_match_lines(body: &str, needle: &str) -> Vec<u32> {
    let mut lines = Vec::new();
    let mut cursor = 0usize;
    while let Some(found) = body[cursor..].find(needle) {
        let offset = cursor + found;
        let line = body[..offset].chars().filter(|&c| c == '\n').count();
        let line_1based = u32::try_from(line).unwrap_or(u32::MAX).saturating_add(1);
        lines.push(line_1based);
        cursor = offset + needle.len().max(1);
    }
    lines
}

/// Orchestrate `apply_change` end-to-end: apply → reparse → diff →
/// detect → context → session.
///
/// # Errors
/// Any error from [`apply_edit`] (`EditNotFound` / `AmbiguousEdit` / Io) or
/// from subsequent file I/O bubbles up verbatim. The MCP handler in Plan
/// 4 maps these to `CallToolResult { is_error: true, .. }`.
///
/// # Panics
/// Panics only if the `graph` or `session` mutex has been poisoned by a
/// previous thread panic, which is a fatal condition in this server.
#[allow(clippy::too_many_lines)]
// `project_root` is forwarded to the recursive call for create_file and will
// be used by Phase 2's import-resolver; the recursion-only lint is a false
// positive here.
#[allow(clippy::only_used_in_recursion)]
pub fn orchestrate(
    graph: &Mutex<CodeGraph>,
    session: &Mutex<SessionState>,
    project_root: &Path,
    request: &ApplyChangeRequest,
) -> Result<ApplyChangeResponse> {
    let file = request.file.clone();

    // Fast-path: create_file — write the file fresh and reparse.
    if request.create_file {
        if request.file.exists() {
            return Err(BlastGuardError::Io {
                path: request.file.clone(),
                source: std::io::Error::new(
                    std::io::ErrorKind::AlreadyExists,
                    format!(
                        "create_file=true but {} already exists",
                        request.file.display()
                    ),
                ),
            });
        }
        if let Some(parent) = request.file.parent() {
            std::fs::create_dir_all(parent).map_err(|source| BlastGuardError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        let content = request
            .changes
            .first()
            .map(|c| c.new_text.clone())
            .unwrap_or_default();
        std::fs::write(&request.file, &content).map_err(|source| BlastGuardError::Io {
            path: request.file.clone(),
            source,
        })?;
        // Recurse with create_file=false and empty changes so the normal
        // reparse/diff path runs against the freshly written file. Override
        // status to `Created` on the way out.
        let inner = ApplyChangeRequest {
            file: request.file.clone(),
            changes: Vec::new(),
            create_file: false,
            delete_file: false,
        };
        let mut resp = orchestrate(graph, session, project_root, &inner)?;
        resp.status = ApplyStatus::Created;
        resp.summary = format!(
            "Created {}. {}.",
            request.file.display(),
            summary_line(&resp.warnings)
        );
        return Ok(resp);
    }

    // Fast-path: delete_file — drop from disk and from graph.
    if request.delete_file {
        std::fs::remove_file(&request.file).map_err(|source| BlastGuardError::Io {
            path: request.file.clone(),
            source,
        })?;
        {
            let mut g = graph.lock().expect("graph lock poisoned");
            g.remove_file(&request.file);
        }
        {
            let mut s = session.lock().expect("session lock poisoned");
            s.record_file_edit(&request.file);
        }
        return Ok(ApplyChangeResponse {
            status: ApplyStatus::Deleted,
            summary: format!("Deleted {}", request.file.display()),
            warnings: Vec::new(),
            context: BundledContext::default(),
        });
    }

    // 1. Snapshot pre-edit symbols.
    let pre_edit_symbols: Vec<Symbol> = {
        let g = graph.lock().expect("graph lock poisoned");
        g.file_symbols
            .get(&file)
            .map(|ids| {
                ids.iter()
                    .filter_map(|id| g.symbols.get(id).cloned())
                    .collect()
            })
            .unwrap_or_default()
    };

    // 2. Apply each change to disk.
    // Snapshot the file so we can roll back if any change fails part-way.
    // `create_file: true` short-circuits above, so we know the file exists
    // here (or does not, and the first apply_edit will surface an Io error).
    let rollback_source: Option<String> = std::fs::read_to_string(&file).ok();

    for change in &request.changes {
        if let Err(err) = apply_edit(&file, &change.old_text, &change.new_text) {
            // Roll back any partial writes. If the rollback itself fails
            // (disk full, permission change, etc.) the file is left in a
            // partial state — surface that clearly rather than swallowing it.
            if let Some(ref original) = rollback_source {
                if let Err(rb_err) = std::fs::write(&file, original) {
                    tracing::warn!(
                        path = %file.display(),
                        error = %rb_err,
                        "apply_change: rollback write failed — file may be in a partial state"
                    );
                }
            }
            return Err(err);
        }
    }

    // 3. Reparse — if the language is not supported, edit landed but graph is unaffected.
    let Some(language) = detect_language(&file) else {
        return Ok(ApplyChangeResponse {
            status: ApplyStatus::Applied,
            summary: format!(
                "Edited {} (no graph impact — unsupported language)",
                file.display()
            ),
            warnings: Vec::new(),
            context: BundledContext::default(),
        });
    };
    let source = std::fs::read_to_string(&file).map_err(|source| BlastGuardError::Io {
        path: file.clone(),
        source,
    })?;
    let parse_out = match language {
        Language::TypeScript => crate::parse::typescript::extract(&file, &source),
        Language::JavaScript => crate::parse::javascript::extract(&file, &source),
        Language::Python => crate::parse::python::extract(&file, &source),
        Language::Rust => crate::parse::rust::extract(&file, &source),
    };
    let new_symbols = parse_out.symbols.clone();

    // 4. Update the graph: drop old file entries, insert new, then
    //    re-resolve and re-stitch so cross-file callers survive the
    //    remove/re-insert cycle.
    {
        let mut g = graph.lock().expect("graph lock poisoned");
        g.remove_file(&file);
        for sym in parse_out.symbols {
            g.insert_symbol(sym);
        }
        for edge in parse_out.edges {
            g.insert_edge(edge);
        }
        g.library_imports.extend(parse_out.library_imports);

        // The newly parsed edges are Unresolved; re-run the resolver so
        // they get real file paths and Confidence::Certain / Inferred.
        crate::parse::resolve::resolve_imports(&mut g, project_root);
        crate::parse::resolve::resolve_calls(&mut g);
        // Other files' forward edges pointing at the edited symbols are
        // kept as dangling by remove_file (for ORPHAN detection). Re-attach
        // them to reverse_edges so callers() finds cross-file callers.
        g.restitch_reverse_edges_for_file(&file);
    }

    // 5. Diff old symbols vs new symbols.
    let d = diff::diff(&pre_edit_symbols, &new_symbols);

    // 6. Detectors.
    let mut warnings: Vec<Warning> = Vec::new();
    {
        let g = graph.lock().expect("graph lock poisoned");
        for (old, new) in &d.modified_sig {
            if let Some(w) = detect_signature(&g, old, new) {
                warnings.push(w);
            }
            if let Some(w) = detect_async_change(&g, old, new) {
                warnings.push(w);
            }
            if let Some(w) = detect_interface_break(&g, old, new) {
                warnings.push(w);
            }
        }
        for removed in &d.removed {
            if let Some(w) = detect_orphan(&g, removed) {
                warnings.push(w);
            }
        }
    }

    // 7. Context — callers + tests for modified symbols.
    let changed_for_context: Vec<Symbol> = d
        .modified_sig
        .iter()
        .map(|(_, new)| new.clone())
        .chain(d.modified_body.iter().map(|(_, new)| new.clone()))
        .collect();
    let context_bundle = {
        let g = graph.lock().expect("graph lock poisoned");
        context::build(&g, &file, &changed_for_context, project_root)
    };

    // 8. Session state.
    {
        let mut s = session.lock().expect("session lock poisoned");
        s.record_file_edit(&file);
        for (_, new) in &d.modified_sig {
            s.record_symbol_edit(new.id.clone());
        }
        for (_, new) in &d.modified_body {
            s.record_symbol_edit(new.id.clone());
        }
        for removed in &d.removed {
            s.record_symbol_edit(removed.id.clone());
        }
    }

    // 9. Build response.
    let status = if d.is_empty() {
        ApplyStatus::NoOp
    } else {
        ApplyStatus::Applied
    };
    let status_word = if status == ApplyStatus::NoOp {
        "No-op edit in"
    } else {
        "Modified"
    };
    let summary = format!(
        "{} {}. {}.",
        status_word,
        file.display(),
        summary_line(&warnings)
    );

    Ok(ApplyChangeResponse {
        status,
        summary,
        warnings,
        context: context_bundle,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_edit_exact_single_match_rewrites_file() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("a.ts");
        std::fs::write(&path, "fn foo() { return 1; }").expect("write");

        apply_edit(&path, "return 1", "return 2").expect("apply_edit");

        let after = std::fs::read_to_string(&path).expect("read");
        assert_eq!(after, "fn foo() { return 2; }");
    }

    #[test]
    fn apply_edit_missing_old_text_returns_edit_not_found() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("a.ts");
        std::fs::write(&path, "fn foo() {}").expect("write");
        let err = apply_edit(&path, "NOT_PRESENT", "x").expect_err("should error");
        assert!(
            matches!(err, BlastGuardError::EditNotFound { .. }),
            "got {err:?}"
        );
    }

    #[test]
    fn apply_edit_ambiguous_old_text_returns_ambiguous_edit() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("a.ts");
        std::fs::write(&path, "a = 1\nb = 1\n").expect("write");
        let err = apply_edit(&path, "= 1", "= 2").expect_err("ambiguous");
        match err {
            BlastGuardError::AmbiguousEdit { count, .. } => assert_eq!(count, 2),
            e => panic!("wrong variant: {e:?}"),
        }
    }

    #[test]
    fn apply_edit_missing_file_returns_io_error() {
        let err = apply_edit(std::path::Path::new("/nope/does/not/exist"), "x", "y")
            .expect_err("should error");
        assert!(matches!(err, BlastGuardError::Io { .. }), "got {err:?}");
    }

    #[test]
    fn ambiguous_edit_lists_all_match_line_numbers() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("a.ts");
        std::fs::write(&path, "a = 1\nb = 1\nc = 1\n").expect("write");
        let err = apply_edit(&path, "= 1", "= 2").expect_err("ambiguous");
        match err {
            BlastGuardError::AmbiguousEdit { count, lines, .. } => {
                assert_eq!(count, 3);
                assert_eq!(lines, vec![1, 2, 3]);
            }
            e => panic!("wrong variant: {e:?}"),
        }
    }

    #[test]
    fn edit_not_found_carries_closest_match_and_similarity() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("a.ts");
        std::fs::write(
            &path,
            "function processRequest(req) {\n    return handler(req);\n}\n",
        )
        .expect("write");
        // Caller provided the function header without the parameter.
        let err = apply_edit(&path, "function processRequest() {", "function x() {")
            .expect_err("not found");
        match err {
            BlastGuardError::EditNotFound {
                line,
                similarity,
                fragment,
                ..
            } => {
                assert_eq!(line, 1, "closest line should be the function header");
                assert!(
                    similarity >= 0.7,
                    "similarity {similarity} too low for a near-miss"
                );
                assert!(fragment.contains("processRequest"), "fragment = {fragment}");
            }
            e => panic!("wrong variant: {e:?}"),
        }
    }
}

#[cfg(test)]
mod flag_tests {
    use super::*;
    use crate::edit::request::{ApplyChangeRequest, ApplyStatus, Change};
    use crate::graph::types::{CodeGraph, Symbol, SymbolId, SymbolKind, Visibility};
    use crate::session::SessionState;
    use std::sync::Mutex;

    #[test]
    fn create_file_writes_new_file_with_content() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let file = tmp.path().join("src/new.ts");
        let graph = Mutex::new(CodeGraph::new());
        let session = Mutex::new(SessionState::new());

        let req = ApplyChangeRequest {
            file: file.clone(),
            changes: vec![Change {
                old_text: String::new(),
                new_text: "export function fresh() {}\n".to_string(),
            }],
            create_file: true,
            delete_file: false,
        };

        let resp = orchestrate(&graph, &session, tmp.path(), &req).expect("create");
        assert_eq!(resp.status, ApplyStatus::Created);
        assert!(file.is_file(), "file should exist");
        let content = std::fs::read_to_string(&file).expect("read");
        assert!(content.contains("fresh"));
    }

    #[test]
    fn multi_change_rolls_back_on_later_failure() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let file = tmp.path().join("src/a.ts");
        std::fs::create_dir_all(file.parent().expect("parent")).expect("mkdir");
        std::fs::write(&file, "one\ntwo\nthree\n").expect("write");

        let graph = Mutex::new(CodeGraph::new());
        let session = Mutex::new(SessionState::new());

        let req = ApplyChangeRequest {
            file: file.clone(),
            changes: vec![
                Change {
                    old_text: "one".to_string(),
                    new_text: "ONE".to_string(),
                },
                Change {
                    old_text: "NOT_PRESENT".to_string(),
                    new_text: "x".to_string(),
                },
            ],
            create_file: false,
            delete_file: false,
        };

        let err = orchestrate(&graph, &session, tmp.path(), &req).expect_err("should fail");
        assert!(
            matches!(err, crate::error::BlastGuardError::EditNotFound { .. }),
            "expected EditNotFound, got {err:?}"
        );

        let after = std::fs::read_to_string(&file).expect("read");
        assert_eq!(
            after, "one\ntwo\nthree\n",
            "file must be rolled back to original on partial-change failure"
        );
    }

    #[test]
    fn create_file_refuses_to_overwrite_existing() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let file = tmp.path().join("src/existing.ts");
        std::fs::create_dir_all(file.parent().expect("parent")).expect("mkdir");
        std::fs::write(&file, "pre-existing content").expect("write");

        let graph = Mutex::new(CodeGraph::new());
        let session = Mutex::new(SessionState::new());

        let req = ApplyChangeRequest {
            file: file.clone(),
            changes: vec![Change {
                old_text: String::new(),
                new_text: "overwrite attempt".to_string(),
            }],
            create_file: true,
            delete_file: false,
        };

        let err = orchestrate(&graph, &session, tmp.path(), &req).expect_err("should refuse");
        assert!(
            matches!(err, crate::error::BlastGuardError::Io { .. }),
            "expected Io, got {err:?}"
        );

        // Original content preserved.
        let after = std::fs::read_to_string(&file).expect("read");
        assert_eq!(after, "pre-existing content");
    }

    #[test]
    fn delete_file_removes_disk_and_graph_entries() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let file = tmp.path().join("src/gone.ts");
        std::fs::create_dir_all(file.parent().expect("parent")).expect("mkdir");
        std::fs::write(&file, "export function doomed() {}\n").expect("write");

        let mut g = CodeGraph::new();
        g.insert_symbol(Symbol {
            id: SymbolId {
                file: file.clone(),
                name: "doomed".into(),
                kind: SymbolKind::Function,
            },
            line_start: 1,
            line_end: 1,
            signature: "doomed()".into(),
            params: vec![],
            return_type: None,
            visibility: Visibility::Export,
            body_hash: 0,
            is_async: false,
            embedding_id: None,
        });
        let graph = Mutex::new(g);
        let session = Mutex::new(SessionState::new());

        let req = ApplyChangeRequest {
            file: file.clone(),
            changes: Vec::new(),
            create_file: false,
            delete_file: true,
        };

        let resp = orchestrate(&graph, &session, tmp.path(), &req).expect("delete");
        assert_eq!(resp.status, ApplyStatus::Deleted);
        assert!(!file.exists(), "file should be gone");
        let g = graph.lock().expect("lock");
        assert!(!g.file_symbols.contains_key(&file));
    }
}
