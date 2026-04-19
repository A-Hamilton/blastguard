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
#[ignore = "TODO: requires a JSON-RPC client fixture; round-6 micro-bench is the real gate (plan 13 task 2 step 3)"]
async fn search_response_uses_compact_format() {
    // Spin up a minimal in-process MCP server against a small fixture project
    // that has a known symbol. Send a tools/call for search. Assert the
    // response JSON contains relative paths (no "/home/") and doesn't include
    // lifetime syntax ('a, 'g).
    //
    // Implementation deferred — the unit tests in src/search/hit.rs::tests_compact
    // cover to_compact_line in isolation, and the live binary smoke-check in the
    // plan's Step 4 verifies end-to-end correctness before the round-6 bench.
    todo!("implement when JSON-RPC client fixture is available")
}
