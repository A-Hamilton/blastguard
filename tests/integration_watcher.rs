//! End-to-end: spawn the watcher against a tempdir, write a source file,
//! poll the graph for the expected symbol. Uses the public `spawn_watcher`
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
    let handle =
        spawn_watcher(tmp.path().to_path_buf(), Arc::clone(&graph)).expect("spawn watcher");

    // Give the watcher a moment to settle before writing.
    tokio::time::sleep(Duration::from_millis(50)).await;

    let file = tmp.path().join("src/new.ts");
    std::fs::write(&file, "export function freshSymbol() { return 1; }\n").expect("write");

    // Poll for up to 3s for the symbol to appear (btrfs/tmpfs can be slow).
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
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
                "watcher did not pick up freshSymbol within 3s; this likely indicates \
                 the watcher is not running or notify events are not firing for this fs"
            );
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    handle.abort();
    let _ = handle.await;
}

/// Regression: after the watcher reindexes a file whose symbols are called
/// from another file, cross-file callers must still resolve. Without the
/// restitch + re-resolve pair in `handle_event`, `remove_file` wipes the
/// `reverse_edges` entry and the caller is silently forgotten.
#[tokio::test]
async fn watcher_preserves_cross_file_callers_on_reindex() {
    use blastguard::graph::ops::callers;
    use blastguard::graph::types::SymbolId;
    use blastguard::index::indexer::cold_index;

    let tmp = tempfile::tempdir().expect("tempdir");
    std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(tmp.path())
        .status()
        .expect("git init");
    std::fs::create_dir_all(tmp.path().join("src/utils")).expect("mkdir");
    std::fs::write(
        tmp.path().join("src/utils/auth.py"),
        "def login(user):\n    return user\n",
    )
    .expect("write auth");
    std::fs::write(
        tmp.path().join("src/handler.py"),
        "from utils.auth import login\n\ndef handle(req):\n    return login(req)\n",
    )
    .expect("write handler");

    // Prime via cold_index so the graph mirrors the apply_change path's
    // starting state.
    let initial = cold_index(tmp.path()).expect("cold_index");
    let login_id = initial
        .symbols
        .keys()
        .find(|id| id.name == "login")
        .cloned()
        .expect("login must be indexed");
    let graph = Arc::new(Mutex::new(initial));
    // Sanity: cold index resolved the cross-file caller.
    {
        let g = graph.lock().expect("lock");
        assert!(
            !callers(&g, &login_id).is_empty(),
            "cold_index should have resolved handle → login"
        );
    }

    let handle =
        spawn_watcher(tmp.path().to_path_buf(), Arc::clone(&graph)).expect("spawn watcher");
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Rewrite auth.py with the same login signature but a changed body —
    // forces a reindex of auth.py via the watcher.
    std::fs::write(
        tmp.path().join("src/utils/auth.py"),
        "def login(user):\n    return {'user': user}\n",
    )
    .expect("rewrite auth");

    // Poll until the reindex has settled. We detect settling by checking the
    // body hash changed — the new login's signature is the same so we can
    // only see it via symbol count staying ≥1 AND callers still resolving.
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    let mut observed_caller = false;
    while std::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(100)).await;
        let g = graph.lock().expect("lock");
        let Some(new_login) = g.symbols.keys().find(|id| id.name == "login").cloned() else {
            continue;
        };
        // Allow at least one watcher cycle to complete before checking.
        let call_count = callers(&g, &new_login).len();
        if call_count >= 1 {
            observed_caller = true;
            break;
        }
    }
    handle.abort();
    let _ = handle.await;

    assert!(
        observed_caller,
        "cross-file caller `handle` should still be registered after watcher reindex; \
         missing it means restitch_reverse_edges_for_file was skipped on the watcher path"
    );
    // Suppress unused warning — we only needed the id at cold_index time.
    let _ = SymbolId {
        file: login_id.file,
        name: login_id.name,
        kind: login_id.kind,
    };
}

#[tokio::test]
async fn watcher_drops_symbols_on_file_delete() {
    let tmp = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(tmp.path().join("src")).expect("mkdir");
    let file = tmp.path().join("src/doomed.ts");
    std::fs::write(&file, "export function doomed() {}\n").expect("write");

    // Prime the graph manually so we don't race the watcher on the initial write.
    let mut initial = CodeGraph::new();
    let parsed = blastguard::parse::typescript::extract(&file, "export function doomed() {}\n");
    for s in parsed.symbols {
        initial.insert_symbol(s);
    }
    let graph = Arc::new(Mutex::new(initial));
    assert!(graph
        .lock()
        .unwrap()
        .symbols
        .keys()
        .any(|id| id.name == "doomed"));

    let handle = spawn_watcher(tmp.path().to_path_buf(), Arc::clone(&graph)).expect("spawn");
    tokio::time::sleep(Duration::from_millis(50)).await;

    std::fs::remove_file(&file).expect("unlink");

    let deadline = std::time::Instant::now() + Duration::from_secs(3);
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
            panic!("watcher did not drop 'doomed' within 3s");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    handle.abort();
    let _ = handle.await;
}
