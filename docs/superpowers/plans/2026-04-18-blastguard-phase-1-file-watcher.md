# BlastGuard File Watcher Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement the incremental file watcher described in SPEC §11 — a background task that uses `notify-debouncer-mini` with a 100ms debounce to detect source-file creates/modifies/deletes and keeps the `CodeGraph` live without waiting for the next `apply_change`.

**Architecture:** One `spawn_watcher(project_root, graph)` function that returns a `JoinHandle`. It owns a `notify-debouncer-mini` instance watching the project root recursively. Debounced events are routed through the `ignore` crate's gitignore matcher (so `node_modules/`, `target/`, etc. don't fire re-parses) and filtered by `detect_language`. For each surviving event, the watcher acquires the graph `Mutex` under a write lock and calls one of `remove_file` / parse-and-insert. Spawned from `mcp::server::run` alongside the rmcp service; cleanly aborted on server shutdown.

**Tech Stack:** Rust 1.82+, `notify = "8"`, `notify-debouncer-mini = "0.7"` (both already in Cargo.toml), `ignore = "0.4"` (for the gitignore matcher), `tokio` (for the spawn/abort mechanics).

**Preconditions:**
- Repo at `/home/adam/Documents/blastguard`. Branch: `phase-1-file-watcher` from `main` (HEAD `caabb30`).
- `src/index/watcher.rs` is a TODO-only stub from Plan 1.
- `BlastGuardServer` exposes `pub(crate) graph: Arc<Mutex<CodeGraph>>` — the watcher will clone the Arc.
- `src/parse::{detect_language, Language}` and the four driver `extract` functions exist.
- `src/graph::types::CodeGraph::remove_file` exists and preserves caller forward-edges (Plan 1 Task 0 fix).

**Pre-work — context7 the debouncer API before writing:**

```
mcp__context7__resolve-library-id { libraryName: "notify-debouncer-mini" }
mcp__context7__query-docs { libraryId: <id>,
    query: "notify-debouncer-mini 0.7 new_debouncer API with a Duration + event handler (tokio mpsc::UnboundedSender or std mpsc::Sender). What type does the event callback receive — DebouncedEvent or Vec<DebouncedEvent>? How to call watch() on the returned Debouncer. Example with tokio::sync::mpsc::UnboundedSender and iteration via rx.recv().await." }
mcp__context7__query-docs { libraryId: "/notify-rs/notify", query: "notify 8 EventKind::{Create, Modify, Remove} event matching for distinguishing file vs directory and data-change vs metadata-change." }
```

Pin the exact API shape. If it differs from the sketch below, adjust — do not invent.

**Definition of done:**
- Editing a source file in the project root causes `CodeGraph` to reflect the new symbols within ~200ms (100ms debounce + parse latency).
- Deleting a source file drops its symbols from the graph; caller forward edges are preserved (Plan 1 invariant).
- Files under `.gitignore`d directories don't trigger reparses.
- Files with non-source extensions (`.md`, `Cargo.toml`, etc.) don't trigger reparses.
- Watcher is cleanly aborted when `mcp::server::run` exits.
- `cargo check/test/clippy/build` all green. Test count ≥ 253 (248 baseline + ~6 new).

---

## File Structure

| Path | Responsibility |
|---|---|
| `src/index/watcher.rs` | `spawn_watcher`, `WatcherEvent` enum, reindex_one helper |
| `src/mcp/server.rs` | Spawn watcher on boot, abort on shutdown |
| `tests/integration_watcher.rs` | E2E: edit a file, poll graph for the new symbol |

---

## Task 1: Watcher skeleton + debouncer bring-up

**Files:**
- Modify: `src/index/watcher.rs`

- [ ] **Step 1: Replace the stub file**

```rust
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

use notify::{EventKind, RecursiveMode};
use notify_debouncer_mini::{new_debouncer, DebouncedEvent};
use tokio::task::JoinHandle;

use crate::graph::types::CodeGraph;
use crate::parse::{detect_language, Language};

/// Default debounce window for file-change events (SPEC §11).
const DEBOUNCE: Duration = Duration::from_millis(100);

/// Spawn the watcher as a dedicated tokio task. Returns the join handle so
/// callers (currently `mcp::server::run`) can abort it on shutdown.
///
/// # Errors
/// Returns any error produced while setting up the `notify` watcher
/// (invalid project root, insufficient permissions).
pub fn spawn_watcher(
    project_root: PathBuf,
    graph: Arc<Mutex<CodeGraph>>,
) -> std::io::Result<JoinHandle<()>> {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Vec<DebouncedEvent>>();

    let mut debouncer = new_debouncer(DEBOUNCE, move |res: Result<Vec<DebouncedEvent>, _>| {
        if let Ok(events) = res {
            // Ignore send errors: the watcher task has exited, which is fine.
            let _ = tx.send(events);
        }
    })
    .map_err(|e| std::io::Error::other(format!("debouncer init: {e}")))?;

    debouncer
        .watcher()
        .watch(&project_root, RecursiveMode::Recursive)
        .map_err(|e| std::io::Error::other(format!("watch {}: {e}", project_root.display())))?;

    let handle = tokio::spawn(async move {
        // Keep the debouncer alive for the duration of the task.
        let _keep_alive = debouncer;
        while let Some(events) = rx.recv().await {
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

#[allow(dead_code)]
fn _unused_types_keepalive(_k: EventKind, _l: Language) {}

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
```

Note on the `_unused_types_keepalive` helper: imports that aren't used elsewhere will trigger `unused_import` warnings during development across Tasks 1-5. The keepalive function fn consumes them; Task 5 replaces it with real use.

If context7 showed a different signature for `new_debouncer` (e.g., takes an `EventHandler` trait instead of a closure), adjust. The 0.7 series typically accepts `FnMut(DebounceEventResult)` or `tokio::sync::mpsc::UnboundedSender<DebounceEventResult>` as the handler.

- [ ] **Step 2: Verify + commit**

```bash
cd /home/adam/Documents/blastguard
cargo check --all-targets 2>&1 | tail -5
cargo test -p blastguard index::watcher::tests 2>&1 | tail -5
cargo clippy --all-targets -- -W clippy::pedantic -D warnings 2>&1 | tail -5
git checkout -b phase-1-file-watcher
git add src/index/watcher.rs
git commit -m "phase 1.9: watcher skeleton — debouncer + tokio channel

spawn_watcher constructs a notify-debouncer-mini with a 100ms window,
forwards events through a tokio unbounded channel into a dedicated
task. Event handler is a stub until Task 3. Returns the JoinHandle
so the server can abort on shutdown."
```

---

## Task 2: Path filtering — gitignore + source-extension

**Files:**
- Modify: `src/index/watcher.rs`

- [ ] **Step 1: Add the filter helper + test**

Add above `handle_event`:

```rust
use ignore::gitignore::Gitignore;

/// A file event is "relevant" when all three are true:
/// 1. It lives inside `project_root` (not an unrelated absolute path).
/// 2. Its path isn't blocked by the project's gitignore set.
/// 3. `detect_language` recognises its extension.
#[must_use]
pub(crate) fn is_relevant(path: &Path, project_root: &Path, gitignore: &Gitignore) -> bool {
    if path.strip_prefix(project_root).is_err() {
        return false;
    }
    let rel = path.strip_prefix(project_root).unwrap_or(path);
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
        assert!(!is_relevant(&tmp.path().join("Cargo.toml"), tmp.path(), &gi));
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
        assert!(!is_relevant(Path::new("/tmp/unrelated.ts"), tmp.path(), &gi));
    }
}
```

- [ ] **Step 2: Verify + commit**

```bash
cargo test -p blastguard index::watcher::filter_tests 2>&1 | tail -10
cargo clippy --all-targets -- -W clippy::pedantic -D warnings 2>&1 | tail -3
git add src/index/watcher.rs
git commit -m "phase 1.9: relevance filter — gitignore + detect_language"
```

---

## Task 3: handle_event — reindex one file

**Files:**
- Modify: `src/index/watcher.rs`

- [ ] **Step 1: Test with synthetic event**

Add to the existing `tests` module:

```rust
#[test]
fn modify_event_drops_and_reinserts_symbols() {
    let tmp = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(tmp.path().join("src")).expect("mkdir");
    let file = tmp.path().join("src/a.ts");
    std::fs::write(&file, "export function first() {}\n").expect("write v1");

    // Prime the graph via cold_index.
    let mut initial_graph = crate::index::indexer::cold_index(tmp.path()).expect("cold");
    // Sanity: the initial symbol is in the graph.
    assert!(initial_graph.symbols.keys().any(|id| id.name == "first"));
    let graph = Arc::new(Mutex::new(initial_graph));

    // Mutate the file, then invoke handle_event directly.
    std::fs::write(&file, "export function second() {}\n").expect("rewrite");
    let event = DebouncedEvent {
        path: file.clone(),
        kind: notify_debouncer_mini::DebouncedEventKind::Any,
    };
    handle_event(&event, tmp.path(), &graph);

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
        kind: notify_debouncer_mini::DebouncedEventKind::Any,
    };
    handle_event(&event, tmp.path(), &graph);

    let g = graph.lock().expect("lock");
    assert!(
        !g.symbols.keys().any(|id| id.name == "doomed"),
        "doomed should be gone"
    );
}
```

Note: `DebouncedEvent` and `DebouncedEventKind` may have different shapes in 0.7. Confirm via context7 — the test's event construction must match the actual struct fields. If `kind` isn't `DebouncedEventKind::Any`, use whatever "don't-care" variant exists, or just `Default::default()` if derived.

- [ ] **Step 2: Implement `handle_event`**

Replace the stub:

```rust
fn handle_event(event: &DebouncedEvent, project_root: &Path, graph: &Arc<Mutex<CodeGraph>>) {
    let gi = load_gitignore(project_root);
    let path = &event.path;
    if !is_relevant(path, project_root, &gi) {
        return;
    }

    if !path.exists() {
        // File was deleted — drop its entries from the graph.
        let mut g = graph.lock().expect("graph lock poisoned");
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
    let parsed = match language {
        Language::TypeScript => crate::parse::typescript::extract(path, &source),
        Language::JavaScript => crate::parse::javascript::extract(path, &source),
        Language::Python => crate::parse::python::extract(path, &source),
        Language::Rust => crate::parse::rust::extract(path, &source),
    };

    let mut g = graph.lock().expect("graph lock poisoned");
    g.remove_file(path);
    for sym in parsed.symbols {
        g.insert_symbol(sym);
    }
    for edge in parsed.edges {
        g.insert_edge(edge);
    }
    g.library_imports.extend(parsed.library_imports);
    tracing::debug!(path = %path.display(), "watcher: reindexed");
}
```

Delete the `_unused_types_keepalive` stub — `EventKind` isn't referenced by the final handler, but `Language` is. If clippy complains about unused `EventKind`, remove that import.

- [ ] **Step 3: Verify + commit**

```bash
cargo test -p blastguard index::watcher:: 2>&1 | tail -15
cargo clippy --all-targets -- -W clippy::pedantic -D warnings 2>&1 | tail -5
git add src/index/watcher.rs
git commit -m "phase 1.9: handle_event — remove_file + reparse or drop

Per-event dispatch: load gitignore, filter irrelevant paths, remove
the file's existing symbols from the graph, and if the file still
exists on disk, re-parse it and reinsert symbols + edges +
library_imports. Deletions drop entries (remove_file preserves caller
forward edges for ORPHAN cascade detection)."
```

---

## Task 4: Wire the watcher into `mcp::server::run`

**Files:**
- Modify: `src/mcp/server.rs`

- [ ] **Step 1: Spawn on boot, abort on shutdown**

Inside `run`, after constructing `BlastGuardServer` and before calling `server.serve`, add:

```rust
let watcher_handle = crate::index::watcher::spawn_watcher(
    project_root.to_path_buf(),
    Arc::clone(&server.graph),
)
.context("spawning file watcher")?;
tracing::info!("file watcher active at 100ms debounce");
```

After `service.waiting().await?;`, abort the handle:

```rust
watcher_handle.abort();
let _ = watcher_handle.await;
```

The `let _ = watcher_handle.await;` drains the JoinHandle without panicking on `JoinError::Cancelled`.

- [ ] **Step 2: Verify + commit**

```bash
cargo check --all-targets 2>&1 | tail -3
cargo clippy --all-targets -- -W clippy::pedantic -D warnings 2>&1 | tail -3
git add src/mcp/server.rs
git commit -m "phase 1.9: spawn watcher on boot, abort on shutdown

run() spawns the watcher with an Arc clone of the server's graph, so
edits outside apply_change (e.g. the user saves a file in their IDE)
keep the graph current. Abort + drain on service exit."
```

---

## Task 5: Integration test — real edit → watcher → graph

**Files:**
- Create: `tests/integration_watcher.rs`

- [ ] **Step 1: Test**

```rust
//! End-to-end: spawn the watcher against a tempdir, write a source file,
//! poll the graph for the expected symbol. Uses the public spawn_watcher
//! API so we don't depend on `mcp::server::run`'s full boot.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use blastguard::graph::types::CodeGraph;
use blastguard::index::watcher::spawn_watcher;

#[tokio::test]
async fn watcher_reindexes_on_new_file() {
    let tmp = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(tmp.path().join("src")).expect("mkdir");

    let graph = Arc::new(Mutex::new(CodeGraph::new()));
    let handle = spawn_watcher(tmp.path().to_path_buf(), Arc::clone(&graph))
        .expect("spawn watcher");

    // Give the watcher a moment to settle before writing.
    tokio::time::sleep(Duration::from_millis(50)).await;

    let file = tmp.path().join("src/new.ts");
    std::fs::write(&file, "export function freshSymbol() { return 1; }\n")
        .expect("write");

    // Poll for up to 2s for the symbol to appear.
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    loop {
        {
            let g = graph.lock().expect("lock");
            if g.symbols.keys().any(|id| id.name == "freshSymbol") {
                break;
            }
        }
        if std::time::Instant::now() >= deadline {
            handle.abort();
            let _ = handle.await;
            panic!(
                "watcher did not pick up freshSymbol within 2s; this likely indicates \
                 the watcher is not running or notify events are not firing for this fs"
            );
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    handle.abort();
    let _ = handle.await;
}
```

- [ ] **Step 2: Run**

```bash
cd /home/adam/Documents/blastguard
cargo test --test integration_watcher 2>&1 | tail -10
```

If the test flakes on some filesystems (btrfs on Linux has been flaky with notify historically), give it a retry and widen the 2s deadline. Don't weaken the assertion — the symbol must actually appear.

- [ ] **Step 3: Commit**

```bash
git add tests/integration_watcher.rs
git commit -m "phase 1.9: integration test — real edit flows through watcher"
```

---

## Task 6: Deletion integration test

**Files:**
- Modify: `tests/integration_watcher.rs`

- [ ] **Step 1: Test**

Append to `tests/integration_watcher.rs`:

```rust
#[tokio::test]
async fn watcher_drops_symbols_on_file_delete() {
    let tmp = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(tmp.path().join("src")).expect("mkdir");
    let file = tmp.path().join("src/doomed.ts");
    std::fs::write(&file, "export function doomed() {}\n").expect("write");

    // Prime the graph manually so we don't race the watcher on the initial write.
    let mut initial = CodeGraph::new();
    let parsed = blastguard::parse::typescript::extract(
        &file,
        "export function doomed() {}\n",
    );
    for s in parsed.symbols {
        initial.insert_symbol(s);
    }
    let graph = Arc::new(Mutex::new(initial));
    assert!(graph.lock().unwrap().symbols.keys().any(|id| id.name == "doomed"));

    let handle = spawn_watcher(tmp.path().to_path_buf(), Arc::clone(&graph))
        .expect("spawn");
    tokio::time::sleep(Duration::from_millis(50)).await;

    std::fs::remove_file(&file).expect("unlink");

    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    loop {
        {
            let g = graph.lock().expect("lock");
            if !g.symbols.keys().any(|id| id.name == "doomed") {
                break;
            }
        }
        if std::time::Instant::now() >= deadline {
            handle.abort();
            let _ = handle.await;
            panic!("watcher did not drop 'doomed' within 2s");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    handle.abort();
    let _ = handle.await;
}
```

- [ ] **Step 2: Run + commit**

```bash
cargo test --test integration_watcher 2>&1 | tail -10
git add tests/integration_watcher.rs
git commit -m "phase 1.9: integration test — delete flows through watcher"
```

---

## Task 7: Boot smoke with watcher active

**Files:** read-only — test the real binary

- [ ] **Step 1: Confirm the watcher doesn't crash boot**

```bash
cd /home/adam/Documents/blastguard
cargo build --release 2>&1 | tail -3
rm -rf tests/fixtures/sample_project/.blastguard
BLASTGUARD_LOG=info timeout 3s ./target/release/blastguard tests/fixtures/sample_project < /dev/null 2> /tmp/bg-watcher-boot.log
echo "Exit: $?"
grep -E "file watcher|BlastGuard" /tmp/bg-watcher-boot.log
```

Expected:
- Exit 124 (timeout — server waiting for stdio).
- Log contains `"file watcher active at 100ms debounce"`.
- No panic or error tracing.

- [ ] **Step 2: Commit**

Nothing to commit unless a fix was needed. If the watcher crashed on boot, investigate and commit the fix.

---

## Task 8: Final verification gate

- [ ] **Step 1: Four gates**

```bash
cd /home/adam/Documents/blastguard
cargo check --all-targets 2>&1 | tail -3
cargo test 2>&1 | grep "test result"
cargo clippy --all-targets -- -W clippy::pedantic -D warnings 2>&1 | tail -5
cargo build --release 2>&1 | tail -3
```

Expected: library test count ≥ 248 (+unit watcher tests), integration test count 6 (previous 5 + integration_watcher).

- [ ] **Step 2: Commit marker**

```bash
git commit --allow-empty -m "phase 1.9: verification gate — file watcher complete

notify-debouncer-mini 0.7 at 100ms debounce feeds incremental reindex
calls into the shared CodeGraph. Gitignored paths and non-source
extensions are filtered. Deletes drop file entries (remove_file
preserves caller forward edges). Watcher spawns on mcp::server::run
boot and aborts cleanly on shutdown.

Closes docs/superpowers/plans/2026-04-18-blastguard-phase-1-file-watcher.md.
Next: Plan 7 — benchmark harness + SWE-bench Pro public-set run."
```

---

## Self-Review

**SPEC §11 coverage:**
- `notify` + `notify-debouncer-mini` at 100ms debounce — Task 1 ✓
- Re-parse on change, update symbols/edges — Task 3 ✓
- Remove all symbols/edges on delete — Task 3 ✓
- Parse and add on create — Task 3 (create = modify path, file exists) ✓
- Ignore files matching `.gitignore` — Task 2 ✓
- Dedicated tokio task, write-lock on graph — Task 1 + Task 3 ✓

**Placeholder scan:** No "TBD" / "implement later" markers.

**Type consistency:** `spawn_watcher(PathBuf, Arc<Mutex<CodeGraph>>) -> io::Result<JoinHandle<()>>` stable across Tasks 1, 4, 5. `is_relevant` / `load_gitignore` signatures stable across Tasks 2-3.

**Known flakiness risk:** filesystem event ordering varies across kernels/filesystems (btrfs, NFS, tmpfs). Integration tests allow 2s polling with clear panic messages so flaky cases produce actionable failures rather than silent hangs.

---

## Execution Handoff

Plan complete and saved. Defaulting to subagent-driven execution per session preference.
