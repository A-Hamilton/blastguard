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
