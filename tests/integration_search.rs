//! End-to-end: cold_index the sample fixture, then exercise several search
//! dispatcher patterns. This pins that the search surface works against
//! real indexed data, not just synthetic fixtures.

use blastguard::index::indexer::cold_index;
use blastguard::search::dispatch;

#[test]
fn search_against_fixture_covers_multiple_patterns() {
    let root = std::path::Path::new("tests/fixtures/sample_project");
    let graph = cold_index(root).expect("cold_index");

    // `find` — processRequest exists in the fixture's handler.ts.
    let hits = dispatch(&graph, root, "find processRequest");
    assert!(!hits.is_empty(), "find processRequest returned no hits");
    assert!(hits[0].signature.is_some());

    // `find` — verify (Python) exists in utils/auth.py.
    let hits = dispatch(&graph, root, "find verify");
    assert!(!hits.is_empty(), "find verify returned no hits");

    // `find` — start (Rust) exists in lib.rs.
    let hits = dispatch(&graph, root, "find start");
    assert!(!hits.is_empty(), "find start returned no hits");

    // Grep fallback — "helper" appears in multiple fixture files as a literal.
    let hits = dispatch(&graph, root, "helper");
    assert!(
        !hits.is_empty(),
        "grep should find 'helper' literally in the fixture"
    );
    assert!(
        hits.iter().any(|h| h.snippet.is_some()),
        "grep hits must carry snippets"
    );
}
