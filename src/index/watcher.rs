//! Incremental file watcher — SPEC §11.
//!
//! `notify-debouncer-mini` with a 100ms debounce drives incremental
//! reindexing of the [`CodeGraph`]. Events that fall inside gitignored
//! paths or hit non-source extensions are filtered out before any
//! parse work runs. The watcher owns an `Arc<Mutex<CodeGraph>>` clone
//! and acquires the write lock only long enough to apply each file's
//! remove/reinsert pair.

use std::path::{Path, PathBuf};
use std::sync::mpsc::RecvTimeoutError;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ignore::gitignore::Gitignore;
use notify::RecursiveMode;
use notify_debouncer_mini::{new_debouncer, DebounceEventResult, DebouncedEvent};
use tokio::task::JoinHandle;

use crate::graph::types::CodeGraph;
use crate::parse::{detect_language, Language};

/// Default debounce window for file-change events (SPEC §11).
const DEBOUNCE: Duration = Duration::from_millis(100);

/// Spawn the watcher as a dedicated tokio task. Returns the join handle so
/// callers (currently `mcp::server::run`) can abort it on shutdown.
///
/// # Errors
///
/// Returns any error produced while setting up the `notify` watcher
/// (invalid project root, insufficient permissions).
pub fn spawn_watcher(
    project_root: PathBuf,
    graph: Arc<Mutex<CodeGraph>>,
) -> std::io::Result<JoinHandle<()>> {
    // Bridge: debouncer-mini only impls DebounceEventHandler for
    // std::sync::mpsc::Sender, not for tokio channels. We use a std channel
    // as the debouncer sink and a tokio unbounded channel to carry events into
    // the async task.
    let (std_tx, std_rx) = std::sync::mpsc::channel::<DebounceEventResult>();
    let (tokio_tx, mut tokio_rx) = tokio::sync::mpsc::unbounded_channel::<Vec<DebouncedEvent>>();

    let mut debouncer = new_debouncer(DEBOUNCE, std_tx)
        .map_err(|e| std::io::Error::other(format!("debouncer init: {e}")))?;

    debouncer
        .watcher()
        .watch(&project_root, RecursiveMode::Recursive)
        .map_err(|e| std::io::Error::other(format!("watch {}: {e}", project_root.display())))?;

    // Fix C: load gitignore once here rather than on every handle_event call.
    let gitignore = load_gitignore(&project_root);

    // Relay thread: forwards debounced events from std mpsc → tokio channel.
    // Runs on a std thread (not tokio) because `std_rx.recv()` is blocking.
    // Fix B: poll with a timeout so the thread exits within one tick after the
    // tokio task is aborted (relay_tx.is_closed() becomes true).
    let relay_tx = tokio_tx;
    std::thread::Builder::new()
        .name("blastguard-watcher-relay".to_string())
        .spawn(move || {
            // Keep debouncer alive for the duration of the relay thread.
            let _keep_alive = debouncer;
            loop {
                match std_rx.recv_timeout(Duration::from_millis(200)) {
                    Ok(result) => {
                        match result {
                            Ok(events) if !events.is_empty() => {
                                if relay_tx.send(events).is_err() {
                                    // tokio receiver dropped — shut down cleanly.
                                    break;
                                }
                            }
                            Ok(_) => {}
                            Err(e) => {
                                tracing::warn!(error = %e, "watcher: notify error");
                            }
                        }
                    }
                    Err(RecvTimeoutError::Timeout) => {
                        if relay_tx.is_closed() {
                            break;
                        }
                    }
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            }
        })
        .map_err(|e| std::io::Error::other(format!("relay thread spawn: {e}")))?;

    let handle = tokio::spawn(async move {
        while let Some(events) = tokio_rx.recv().await {
            for event in events {
                handle_event(&event, &project_root, &graph, &gitignore);
            }
        }
    });

    Ok(handle)
}

/// A file event is "relevant" when all three are true:
/// 1. It lives inside `project_root` (not an unrelated absolute path).
/// 2. Its path isn't blocked by the project's gitignore set.
/// 3. `detect_language` recognises its extension.
#[must_use]
pub(crate) fn is_relevant(path: &Path, project_root: &Path, gitignore: &Gitignore) -> bool {
    let Ok(rel) = path.strip_prefix(project_root) else {
        return false;
    };
    if gitignore
        .matched_path_or_any_parents(rel, /* is_dir = */ false)
        .is_ignore()
    {
        return false;
    }
    detect_language(path).is_some()
}

/// Load the `.gitignore` at `project_root`. Returns an empty matcher when
/// no gitignore is present.
pub(crate) fn load_gitignore(project_root: &Path) -> Gitignore {
    let gi_path = project_root.join(".gitignore");
    if !gi_path.exists() {
        return Gitignore::empty();
    }
    let (gi, _err) = Gitignore::new(&gi_path);
    gi
}

fn handle_event(
    event: &DebouncedEvent,
    project_root: &Path,
    graph: &Arc<Mutex<CodeGraph>>,
    gitignore: &Gitignore,
) {
    // Fix C: gitignore is pre-loaded by the caller — no filesystem read here.
    let path = &event.path;
    if !is_relevant(path, project_root, gitignore) {
        return;
    }

    if !path.exists() {
        // File was deleted — drop its entries from the graph.
        // Fix A: degrade gracefully if the lock is poisoned instead of panicking.
        let Ok(mut g) = graph.lock() else {
            tracing::error!(
                path = %path.display(),
                "watcher: graph lock poisoned, skipping reindex"
            );
            return;
        };
        g.remove_file(path);
        tracing::debug!(path = %path.display(), "watcher: dropped deleted file");
        return;
    }

    // Read + reparse.
    let Ok(source) = std::fs::read_to_string(path) else {
        tracing::warn!(path = %path.display(), "watcher: read failed, skipping");
        return;
    };
    let Some(language) = detect_language(path) else {
        return;
    };
    let self_crate_name = crate::index::indexer::read_crate_name(project_root);
    let parsed = match language {
        Language::TypeScript => crate::parse::typescript::extract(path, &source),
        Language::JavaScript => crate::parse::javascript::extract(path, &source),
        Language::Python => crate::parse::python::extract(path, &source),
        Language::Rust => {
            crate::parse::rust::extract_with_crate_name(path, &source, self_crate_name.as_deref())
        }
    };

    // Fix A: degrade gracefully if the lock is poisoned instead of panicking.
    let Ok(mut g) = graph.lock() else {
        tracing::error!(
            path = %path.display(),
            "watcher: graph lock poisoned, skipping reindex"
        );
        return;
    };
    g.remove_file(path);
    for sym in parsed.symbols {
        g.insert_symbol(sym);
    }
    for edge in parsed.edges {
        g.insert_edge(edge);
    }
    g.library_imports.extend(parsed.library_imports);

    // Re-resolve and re-stitch so cross-file callers keep working through
    // incremental updates (same rationale as apply_change's reparse step).
    crate::parse::resolve::resolve_imports(&mut g, project_root);
    crate::parse::resolve::resolve_calls(&mut g);
    g.restitch_reverse_edges_for_file(path);
    tracing::debug!(path = %path.display(), "watcher: reindexed");
}

#[cfg(test)]
mod tests {
    use super::*;
    use notify_debouncer_mini::DebouncedEventKind;

    #[test]
    fn spawn_watcher_on_missing_dir_returns_error() {
        let graph = Arc::new(Mutex::new(CodeGraph::new()));
        let result = spawn_watcher(PathBuf::from("/nonexistent/path/xyz123"), graph);
        assert!(result.is_err());
    }

    #[test]
    fn modify_event_drops_and_reinserts_symbols() {
        let tmp = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(tmp.path().join("src")).expect("mkdir");
        let file = tmp.path().join("src/a.ts");
        std::fs::write(&file, "export function first() {}\n").expect("write v1");

        // Prime the graph via cold_index.
        let initial_graph = crate::index::indexer::cold_index(tmp.path()).expect("cold");
        // Sanity: the initial symbol is in the graph.
        assert!(initial_graph.symbols.keys().any(|id| id.name == "first"));
        let graph = Arc::new(Mutex::new(initial_graph));

        // Mutate the file, then invoke handle_event directly.
        std::fs::write(&file, "export function second() {}\n").expect("rewrite");
        let event = DebouncedEvent {
            path: file.clone(),
            kind: DebouncedEventKind::Any,
        };
        handle_event(&event, tmp.path(), &graph, &Gitignore::empty());

        let g = graph.lock().expect("lock");
        assert!(
            g.symbols.keys().any(|id| id.name == "second"),
            "expected 'second' after reindex; symbols: {:?}",
            g.symbols.keys().map(|k| &k.name).collect::<Vec<_>>()
        );
        assert!(
            !g.symbols.keys().any(|id| id.name == "first"),
            "old symbol 'first' should be gone"
        );
    }

    #[test]
    fn delete_event_removes_file_symbols() {
        let tmp = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(tmp.path().join("src")).expect("mkdir");
        let file = tmp.path().join("src/gone.ts");
        std::fs::write(&file, "export function doomed() {}\n").expect("write");

        let initial_graph = crate::index::indexer::cold_index(tmp.path()).expect("cold");
        let graph = Arc::new(Mutex::new(initial_graph));

        // Delete the file, then fire a synthetic event.
        std::fs::remove_file(&file).expect("unlink");
        let event = DebouncedEvent {
            path: file.clone(),
            kind: DebouncedEventKind::Any,
        };
        handle_event(&event, tmp.path(), &graph, &Gitignore::empty());

        let g = graph.lock().expect("lock");
        assert!(
            !g.symbols.keys().any(|id| id.name == "doomed"),
            "doomed should be gone"
        );
    }

    /// Exercises the relay-thread shutdown path introduced in Fix B.
    ///
    /// After the tokio task is aborted, the relay thread polls `std_rx` with a
    /// 200ms timeout and checks `relay_tx.is_closed()`. Within one poll cycle
    /// (≤200ms + a small margin) it should exit. This test doesn't assert the
    /// OS thread is gone (thread introspection is platform-specific) but it
    /// exercises the code path at runtime and documents the expected behaviour.
    #[tokio::test]
    async fn relay_thread_exits_when_tokio_task_aborted() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let graph = Arc::new(Mutex::new(CodeGraph::new()));
        let handle = spawn_watcher(tmp.path().to_path_buf(), Arc::clone(&graph)).expect("spawn");

        // Abort the task; the relay thread should notice relay_tx is closed
        // within the 200ms poll timeout and exit cleanly.
        handle.abort();
        let _ = handle.await;
        // 300ms > 200ms timeout, so one full poll cycle has elapsed.
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    }
}

#[cfg(test)]
mod filter_tests {
    use super::*;

    #[test]
    fn source_extension_is_relevant() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let gi = Gitignore::empty();
        assert!(is_relevant(&tmp.path().join("src/a.ts"), tmp.path(), &gi));
        assert!(is_relevant(&tmp.path().join("src/b.py"), tmp.path(), &gi));
        assert!(is_relevant(&tmp.path().join("src/c.rs"), tmp.path(), &gi));
    }

    #[test]
    fn non_source_extension_is_filtered() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let gi = Gitignore::empty();
        assert!(!is_relevant(&tmp.path().join("README.md"), tmp.path(), &gi));
        assert!(!is_relevant(
            &tmp.path().join("Cargo.toml"),
            tmp.path(),
            &gi
        ));
    }

    #[test]
    fn gitignored_path_is_filtered() {
        let tmp = tempfile::tempdir().expect("tempdir");
        std::fs::write(tmp.path().join(".gitignore"), "node_modules/\ntarget/\n")
            .expect("gitignore");
        let gi = load_gitignore(tmp.path());
        assert!(!is_relevant(
            &tmp.path().join("node_modules/pkg.ts"),
            tmp.path(),
            &gi
        ));
        assert!(is_relevant(&tmp.path().join("src/a.ts"), tmp.path(), &gi));
    }

    #[test]
    fn path_outside_root_is_filtered() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let gi = Gitignore::empty();
        assert!(!is_relevant(
            Path::new("/tmp/unrelated.ts"),
            tmp.path(),
            &gi
        ));
    }
}
