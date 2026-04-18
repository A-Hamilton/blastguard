//! Parallel indexer, BLAKE3 Merkle cache, and file watcher.
//!
//! SPEC §9–§11. The indexer walks the project via `ignore` (gitignore-aware),
//! dispatches parsing across `rayon` workers (one tree-sitter parser per
//! worker), and persists the result to `.blastguard/cache.bin` via
//! `rmp-serde`. The watcher drives incremental reindexing with a 100ms
//! debounce.

pub mod cache;
pub mod indexer;
pub mod watcher;
