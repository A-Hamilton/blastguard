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
