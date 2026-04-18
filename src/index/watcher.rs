//! File watcher — SPEC §11.
//!
//! `notify` 8 + `notify-debouncer-mini` 0.7 at 100 ms debounce. Runs in a
//! dedicated tokio task and updates the shared graph under a write lock.

// TODO(phase-1.9): spawn_watcher(project_root, graph: Arc<RwLock<CodeGraph>>)
// that debounces raw events and dispatches incremental reindex work.
