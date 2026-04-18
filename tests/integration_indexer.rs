use blastguard::index::indexer::cold_index;

#[test]
fn indexes_mixed_language_fixture() {
    let root = std::path::Path::new("tests/fixtures/sample_project");
    let graph = cold_index(root).expect("cold_index");

    // TS symbol
    assert!(
        graph.symbols.keys().any(|id| id.name == "processRequest"),
        "processRequest missing; symbols: {:?}",
        graph.symbols.keys().map(|k| &k.name).collect::<Vec<_>>()
    );

    // Python symbol
    assert!(graph.symbols.keys().any(|id| id.name == "verify"));
    assert!(graph.symbols.keys().any(|id| id.name == "run"));

    // Rust symbol
    assert!(graph.symbols.keys().any(|id| id.name == "start"));
    assert!(graph.symbols.keys().any(|id| id.name == "helper"));

    // File-level indexing works
    assert!(!graph.file_symbols.is_empty());

    // Intra-file call edges (Rust driver): start -> helper
    assert!(
        graph
            .forward_edges
            .values()
            .flatten()
            .any(|e| e.from.name == "start" && e.to.name == "helper"),
        "expected intra-file call edge start -> helper"
    );
}
