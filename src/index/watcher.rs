//! Incremental file watcher — SPEC §11.
//!
//! `notify-debouncer-mini` with a 100ms debounce drives incremental
//! reindexing of the [`CodeGraph`]. Events that fall inside gitignored
//! paths or hit non-source extensions are filtered out before any
//! parse work runs. The watcher owns an `Arc<Mutex<CodeGraph>>` clone
//! and acquires the write lock only long enough to apply each file's
//! remove/reinsert pair.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use notify::RecursiveMode;
use notify_debouncer_mini::{new_debouncer, DebouncedEvent, DebounceEventResult};
use tokio::task::JoinHandle;

use crate::graph::types::CodeGraph;
use crate::parse::detect_language;

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
    let (tokio_tx, mut tokio_rx) =
        tokio::sync::mpsc::unbounded_channel::<Vec<DebouncedEvent>>();

    let mut debouncer = new_debouncer(DEBOUNCE, std_tx)
        .map_err(|e| std::io::Error::other(format!("debouncer init: {e}")))?;

    debouncer
        .watcher()
        .watch(&project_root, RecursiveMode::Recursive)
        .map_err(|e| std::io::Error::other(format!("watch {}: {e}", project_root.display())))?;

    // Relay thread: forwards debounced events from std mpsc → tokio channel.
    // Runs on a std thread (not tokio) because `std_rx.recv()` is blocking.
    let relay_tx = tokio_tx;
    std::thread::Builder::new()
        .name("blastguard-watcher-relay".to_string())
        .spawn(move || {
            // Keep debouncer alive for the duration of the relay thread.
            let _keep_alive = debouncer;
            for result in std_rx {
                match result {
                    Ok(events) if !events.is_empty() => {
                        // Ignore send errors — tokio task has exited, shutdown in progress.
                        let _ = relay_tx.send(events);
                    }
                    Ok(_) => {}
                    Err(e) => {
                        tracing::warn!(error = %e, "watcher: notify error");
                    }
                }
            }
        })
        .map_err(|e| std::io::Error::other(format!("relay thread spawn: {e}")))?;

    let handle = tokio::spawn(async move {
        while let Some(events) = tokio_rx.recv().await {
            for event in events {
                handle_event(&event, &project_root, &graph);
            }
        }
    });

    Ok(handle)
}

fn handle_event(_event: &DebouncedEvent, _project_root: &Path, _graph: &Arc<Mutex<CodeGraph>>) {
    // Task 3 populates.
}

// Suppress unused import warning during Tasks 1–2 while handle_event is a stub.
#[allow(dead_code)]
fn _detect_language_used(p: &Path) -> bool {
    detect_language(p).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawn_watcher_on_missing_dir_returns_error() {
        let graph = Arc::new(Mutex::new(CodeGraph::new()));
        let result = spawn_watcher(PathBuf::from("/nonexistent/path/xyz123"), graph);
        assert!(result.is_err());
    }
}
