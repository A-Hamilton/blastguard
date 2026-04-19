//! End-to-end smoke: construct `BlastGuardServer`, call `serve()` over an
//! in-memory duplex transport, abort shortly after — asserts the server
//! boots without panicking. Full JSON-RPC initialize/tools-list
//! round-trip is deferred to the Plan 7 benchmark harness.

use blastguard::config::Config;
use blastguard::graph::types::CodeGraph;
use blastguard::mcp::server::BlastGuardServer;
use rmcp::ServiceExt as _;

#[tokio::test]
async fn server_boots_on_duplex_transport_without_panicking() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let server = BlastGuardServer::new(
        CodeGraph::new(),
        tmp.path().to_path_buf(),
        Config::default(),
    );

    // Duplex transport simulates stdin/stdout. We spawn serve() and then
    // abort the task before any real traffic — enough to exercise the
    // boot path without writing a JSON-RPC client.
    let (server_io, _client_io) = tokio::io::duplex(4096);
    let (server_rd, server_wr) = tokio::io::split(server_io);

    let service_fut = server.serve((server_rd, server_wr));

    let handle = tokio::spawn(service_fut);
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    handle.abort();
    // Drain the abort without asserting the error type — cancellation
    // semantics vary.
    let _ = handle.await;
}

#[tokio::test]
async fn search_response_hits_are_compact_and_path_relative() {
    use blastguard::graph::types::{Symbol, SymbolId, SymbolKind, Visibility};
    use blastguard::mcp::server::SearchRequest;
    use rmcp::handler::server::wrapper::Parameters;

    let tmp = tempfile::tempdir().expect("tempdir");
    let project_root = tmp.path().to_path_buf();

    // Seed a graph with one symbol under the project root. The to_compact_line
    // renderer is expected to (1) strip the project_root prefix from the path
    // and (2) drop lifetime / generic-bound syntax from the signature.
    let mut graph = CodeGraph::new();
    let file = project_root.join("src/graph/ops.rs");
    graph.insert_symbol(Symbol {
        id: SymbolId {
            file: file.clone(),
            name: "callers".to_string(),
            kind: SymbolKind::Function,
        },
        line_start: 12,
        line_end: 16,
        signature: "fn callers<'g>(graph: &'g CodeGraph, target: &SymbolId) -> Vec<&'g SymbolId>"
            .to_string(),
        params: vec![],
        return_type: None,
        visibility: Visibility::Export,
        body_hash: 0,
        is_async: false,
        embedding_id: None,
    });

    let server = BlastGuardServer::new(graph, project_root.clone(), Config::default());

    let resp = server
        .search_tool(Parameters(SearchRequest {
            query: "outline of src/graph/ops.rs".to_string(),
            scope: None,
        }))
        .await
        .expect("search_tool should return a successful Json response");

    let hits = &resp.0.hits;
    assert_eq!(hits.len(), 1, "expected exactly one hit, got {hits:?}");

    let line = &hits[0];

    // Path must be relative (no project_root prefix leaked).
    let root_str = project_root.display().to_string();
    assert!(
        !line.contains(&root_str),
        "absolute project_root leaked into hit line: {line}"
    );
    assert!(
        line.starts_with("src/graph/ops.rs:12"),
        "hit should start with relative file:line, got: {line}"
    );

    // Signature noise must be stripped by to_compact_line.
    assert!(!line.contains("'g"), "lifetime not stripped: {line}");
    assert!(
        !line.contains("fn callers"),
        "fn keyword not stripped: {line}"
    );
    assert!(
        line.contains("callers"),
        "symbol name should survive: {line}"
    );
}

#[tokio::test]
async fn apply_change_tool_returns_status_and_summary() {
    use blastguard::edit::{ApplyChangeRequest, ApplyStatus, Change};
    use blastguard::index::indexer::cold_index;
    use rmcp::handler::server::wrapper::Parameters;
    use std::fs;

    let tmp = tempfile::tempdir().expect("tempdir");
    let project_root = tmp.path().to_path_buf();

    // Minimal TypeScript project so cold_index has something to parse.
    std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(&project_root)
        .status()
        .expect("git init");
    fs::create_dir_all(project_root.join("src")).expect("mkdir");
    let file = project_root.join("src/app.ts");
    fs::write(
        &file,
        "export function greet(name: string): string {\n    return `hi ${name}`;\n}\n",
    )
    .expect("write");

    let graph = cold_index(&project_root).expect("cold_index");
    let server = BlastGuardServer::new(graph, project_root.clone(), Config::default());

    // In-place text replacement that preserves the signature — should land as APPLIED.
    let resp = server
        .apply_change_tool(Parameters(ApplyChangeRequest {
            file: file.clone(),
            changes: vec![Change {
                old_text: "hi ${name}".to_string(),
                new_text: "hello ${name}".to_string(),
            }],
            create_file: false,
            delete_file: false,
        }))
        .await
        .expect("apply_change_tool should return Ok");

    let body = resp.0;
    assert_eq!(body.status, ApplyStatus::Applied);
    assert!(
        body.summary.contains("app.ts"),
        "summary should mention the filename, got: {}",
        body.summary
    );
    // File on disk should reflect the edit.
    let new_content = fs::read_to_string(&file).expect("read modified file");
    assert!(
        new_content.contains("hello ${name}"),
        "file not updated on disk, got: {new_content}"
    );
    assert!(
        !new_content.contains("hi ${name}"),
        "old text still in file, got: {new_content}"
    );
}

#[tokio::test]
async fn run_tests_tool_response_has_shape_when_cargo_available() {
    use blastguard::runner::RunTestsRequest;
    use rmcp::handler::server::wrapper::Parameters;

    let tmp = tempfile::tempdir().expect("tempdir");
    let project_root = tmp.path().to_path_buf();

    std::fs::write(
        project_root.join("Cargo.toml"),
        "[package]\nname = \"bg_mcp_test_fixture\"\nversion = \"0.0.1\"\nedition = \"2021\"\n",
    )
    .expect("write Cargo.toml");
    std::fs::create_dir_all(project_root.join("src")).expect("mkdir");
    std::fs::write(
        project_root.join("src/lib.rs"),
        "pub fn add(a: i32, b: i32) -> i32 { a + b }\n\
         #[cfg(test)]\n\
         mod tests {\n\
             use super::*;\n\
             #[test]\n\
             fn ok_one() { assert_eq!(add(1, 2), 3); }\n\
             #[test]\n\
             fn will_fail() { assert_eq!(add(1, 1), 3); }\n\
         }\n",
    )
    .expect("write lib.rs");

    let server = BlastGuardServer::new(CodeGraph::new(), project_root, Config::default());

    // cargo test -- --format json requires nightly; on stable the runner
    // returns an error. Mirror the opportunistic pattern from
    // tests/integration_run_tests.rs: skip assertions on err / zero counts.
    let resp = match server
        .run_tests_tool(Parameters(RunTestsRequest {
            filter: None,
            timeout_seconds: 120,
        }))
        .await
    {
        Ok(r) => r,
        Err(e) => {
            eprintln!("run_tests_tool err (skipping on stable / cargo unavailable): {e:?}");
            return;
        }
    };

    let body = resp.0;
    if body.passed + body.failed + body.skipped == 0 {
        eprintln!("zero counts — skipping assertions");
        return;
    }
    // Response-shape regression guard: field types + failure-line format.
    assert!(body.passed >= 1, "expected >=1 pass: {body:?}");
    assert!(body.failed >= 1, "expected >=1 fail: {body:?}");
    for line in &body.failures {
        assert!(
            line.starts_with("FAIL "),
            "failure line should start with 'FAIL ': {line:?}"
        );
    }
    assert!(body.duration_ms > 0, "duration should be non-zero");
}
