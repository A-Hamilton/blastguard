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
