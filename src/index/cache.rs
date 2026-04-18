//! On-disk cache at `.blastguard/cache.bin` — SPEC §9.
//!
//! Format: rmp-serde. Keyed by BLAKE3 file + subtree hashes so we skip
//! unchanged directories on warm start.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::graph::types::CodeGraph;

/// Bump when the serialised schema changes in an incompatible way.
/// Drop + rebuild on mismatch (SPEC §9).
pub const CACHE_VERSION: u32 = 3; // bump: SymbolKind::Enum added in Phase 1.2 Rust driver.

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct CacheFile {
    pub version: u32,
    pub file_hashes: HashMap<PathBuf, u64>,
    pub tree_hashes: HashMap<PathBuf, u64>,
    pub graph: CodeGraph,
    pub tsconfig: Option<TsConfigSnapshot>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct TsConfigSnapshot {
    pub base_url: Option<PathBuf>,
    pub paths: HashMap<String, Vec<String>>,
}

use std::io::Read;
use std::path::Path;

use crate::error::{BlastGuardError, Result};

/// BLAKE3 hash of a file's content, truncated to the first 8 bytes as a `u64`
/// (little-endian). Streams the file in 8 KB chunks so we never OOM on large
/// inputs.
///
/// # Errors
/// Returns [`BlastGuardError::Io`] when the file cannot be opened or read.
///
/// # Panics
/// Never panics in practice: the `expect` calls are on fixed-size BLAKE3 digest
/// slicing (digest is always 32 bytes) and are unreachable by construction.
#[must_use = "use the returned hash or propagate the error"]
pub fn hash_file(path: &Path) -> Result<u64> {
    let mut hasher = blake3::Hasher::new();
    let mut file = std::fs::File::open(path).map_err(|source| BlastGuardError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let mut buf = [0u8; 8192];
    loop {
        let n = file.read(&mut buf).map_err(|source| BlastGuardError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let digest = hasher.finalize();
    // 64 bits of entropy is plenty for cache keying. BLAKE3's first 8 bytes
    // are uniformly distributed.
    Ok(u64::from_le_bytes(
        digest.as_bytes()[..8]
            .try_into()
            .expect("digest is always 32 bytes"),
    ))
}

/// Merkle hash of a directory tree. Sorted child order makes the result
/// deterministic. Each child contributes its name plus its own hash
/// (recursive for sub-directories, [`hash_file`] for regular files).
/// Non-regular entries (symlinks, devices) are skipped.
///
/// # Errors
/// Returns [`BlastGuardError::Io`] on any directory-read or file-hash failure.
///
/// # Panics
/// Never panics in practice: the `expect` calls are on fixed-size BLAKE3 digest
/// slicing (digest is always 32 bytes) and are unreachable by construction.
#[must_use = "directory hash should be stored or compared"]
pub fn hash_directory_tree(dir: &Path) -> Result<u64> {
    let mut entries: Vec<std::fs::DirEntry> = std::fs::read_dir(dir)
        .map_err(|source| BlastGuardError::Io {
            path: dir.to_path_buf(),
            source,
        })?
        .filter_map(std::result::Result::ok)
        .collect();
    entries.sort_by_key(std::fs::DirEntry::path);

    let mut hasher = blake3::Hasher::new();
    for entry in entries {
        let file_type = entry.file_type().map_err(|source| BlastGuardError::Io {
            path: entry.path(),
            source,
        })?;
        let name = entry.file_name();
        hasher.update(name.as_encoded_bytes());
        let child_hash = if file_type.is_dir() {
            hash_directory_tree(&entry.path())?
        } else if file_type.is_file() {
            hash_file(&entry.path())?
        } else {
            // Symlinks / devices — skip rather than dereference.
            continue;
        };
        hasher.update(&child_hash.to_le_bytes());
    }
    Ok(u64::from_le_bytes(
        hasher.finalize().as_bytes()[..8]
            .try_into()
            .expect("digest is always 32 bytes"),
    ))
}

/// Merkle hash of the project's gitignore-filtered file tree.
///
/// Takes the sorted list of `(relative_path, file_hash)` pairs produced by
/// walking the project via [`crate::index::indexer::walk_project`], feeds
/// both components into BLAKE3, and returns the first 8 bytes as a `u64`.
/// This definition stays invariant under changes inside gitignored paths
/// (`node_modules`, `target`, `__pycache__`) — exactly the property
/// `warm_start` relies on to fast-path.
///
/// # Errors
/// Returns [`BlastGuardError::Io`] if any walked file cannot be hashed.
///
/// # Panics
/// Never panics in practice: the `expect` call is on a fixed-size BLAKE3
/// digest slice (always 32 bytes) and is unreachable by construction.
#[must_use = "directory hash should be stored or compared"]
pub fn hash_project_tree(
    project_root: &Path,
    walk_fn: impl Fn(&Path) -> Vec<std::path::PathBuf>,
) -> Result<u64> {
    let mut files = walk_fn(project_root);
    files.sort();
    let mut hasher = blake3::Hasher::new();
    for file in &files {
        let rel = file.strip_prefix(project_root).unwrap_or(file);
        hasher.update(rel.to_string_lossy().as_bytes());
        let h = hash_file(file)?;
        hasher.update(&h.to_le_bytes());
    }
    Ok(u64::from_le_bytes(
        hasher.finalize().as_bytes()[..8]
            .try_into()
            .expect("digest is 32 bytes"),
    ))
}

/// Persist a `CacheFile` to disk using `rmp-serde`. Ensures the parent
/// directory exists before writing.
///
/// # Errors
/// Returns [`BlastGuardError::Io`] on filesystem failure and
/// [`BlastGuardError::CacheCorrupt`] on serialisation failure.
pub fn save(path: &Path, cache: &CacheFile) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| BlastGuardError::Io {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    let bytes =
        rmp_serde::to_vec(cache).map_err(|e| BlastGuardError::CacheCorrupt(e.to_string()))?;
    std::fs::write(path, bytes).map_err(|source| BlastGuardError::Io {
        path: path.to_path_buf(),
        source,
    })
}

/// Load a `CacheFile` from disk. Returns `Ok(None)` when the file is absent
/// OR the stored version does not match [`CACHE_VERSION`] (the `BlastGuard`
/// version was bumped; drop + rebuild).
///
/// # Errors
/// Returns [`BlastGuardError::Io`] on read failure and
/// [`BlastGuardError::CacheCorrupt`] on deserialisation failure.
#[must_use = "a returned cache should be consumed or ignored explicitly"]
pub fn load(path: &Path) -> Result<Option<CacheFile>> {
    if !path.exists() {
        return Ok(None);
    }
    let bytes = std::fs::read(path).map_err(|source| BlastGuardError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let cache: CacheFile =
        rmp_serde::from_slice(&bytes).map_err(|e| BlastGuardError::CacheCorrupt(e.to_string()))?;
    if cache.version != CACHE_VERSION {
        tracing::info!(
            stored = cache.version,
            current = CACHE_VERSION,
            "cache version mismatch — dropping and rebuilding"
        );
        return Ok(None);
    }
    Ok(Some(cache))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_hash_is_deterministic_for_same_content() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let p = tmp.path().join("a.ts");
        std::fs::write(&p, b"hello world").expect("write");
        let h1 = hash_file(&p).expect("hash");
        let h2 = hash_file(&p).expect("hash");
        assert_eq!(h1, h2);
    }

    #[test]
    fn file_hash_differs_when_content_changes() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let p = tmp.path().join("a.ts");
        std::fs::write(&p, b"one").expect("write");
        let h1 = hash_file(&p).expect("hash");
        std::fs::write(&p, b"two").expect("write");
        let h2 = hash_file(&p).expect("hash");
        assert_ne!(h1, h2);
    }

    #[test]
    fn file_hash_handles_large_streaming_input() {
        // Write > 8 KB so the streaming read loop runs at least twice.
        let tmp = tempfile::tempdir().expect("tempdir");
        let p = tmp.path().join("big.bin");
        let bytes: Vec<u8> = (0..32_000_u32)
            .map(|i| u8::try_from(i % 256).expect("i % 256 always fits u8"))
            .collect();
        std::fs::write(&p, &bytes).expect("write");
        let h = hash_file(&p).expect("hash");
        // Compare to all-at-once hash via blake3::hash for regression.
        let expected = blake3::hash(&bytes);
        let expected_u64 =
            u64::from_le_bytes(expected.as_bytes()[..8].try_into().expect("8 bytes"));
        assert_eq!(h, expected_u64);
    }

    #[test]
    fn directory_hash_changes_when_child_content_changes() {
        let tmp = tempfile::tempdir().expect("tempdir");
        std::fs::write(tmp.path().join("a.ts"), b"one").expect("write");
        let h1 = hash_directory_tree(tmp.path()).expect("hash");
        std::fs::write(tmp.path().join("a.ts"), b"two").expect("write");
        let h2 = hash_directory_tree(tmp.path()).expect("hash");
        assert_ne!(h1, h2);
    }

    #[test]
    fn directory_hash_changes_when_file_added() {
        let tmp = tempfile::tempdir().expect("tempdir");
        std::fs::write(tmp.path().join("a.ts"), b"one").expect("write");
        let h1 = hash_directory_tree(tmp.path()).expect("hash");
        std::fs::write(tmp.path().join("b.ts"), b"one").expect("write");
        let h2 = hash_directory_tree(tmp.path()).expect("hash");
        assert_ne!(h1, h2);
    }

    #[test]
    fn directory_hash_traverses_subdirs() {
        let tmp = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(tmp.path().join("sub")).expect("mkdir");
        std::fs::write(tmp.path().join("sub/a.ts"), b"one").expect("write");
        let h1 = hash_directory_tree(tmp.path()).expect("hash");
        std::fs::write(tmp.path().join("sub/a.ts"), b"two").expect("write");
        let h2 = hash_directory_tree(tmp.path()).expect("hash");
        assert_ne!(h1, h2, "nested change must ripple up to root hash");
    }

    #[test]
    fn directory_hash_is_stable_across_calls() {
        let tmp = tempfile::tempdir().expect("tempdir");
        std::fs::write(tmp.path().join("a.ts"), b"one").expect("write");
        std::fs::write(tmp.path().join("b.py"), b"two").expect("write");
        std::fs::create_dir_all(tmp.path().join("sub")).expect("mkdir");
        std::fs::write(tmp.path().join("sub/c.rs"), b"three").expect("write");
        let h1 = hash_directory_tree(tmp.path()).expect("hash");
        let h2 = hash_directory_tree(tmp.path()).expect("hash");
        assert_eq!(h1, h2);
    }

    #[test]
    fn round_trip_cache_file() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let cache_path = tmp.path().join(".blastguard").join("cache.bin");

        let original = CacheFile {
            version: CACHE_VERSION,
            ..CacheFile::default()
        };
        save(&cache_path, &original).expect("save");
        assert!(cache_path.is_file());

        let loaded = load(&cache_path).expect("load ok").expect("present");
        assert_eq!(loaded.version, CACHE_VERSION);
    }

    #[test]
    fn missing_cache_returns_ok_none() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let cache_path = tmp.path().join(".blastguard").join("cache.bin");
        assert!(load(&cache_path).expect("no-err").is_none());
    }

    #[test]
    fn version_mismatch_returns_ok_none() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let cache_path = tmp.path().join("cache.bin");
        let stale = CacheFile {
            version: 0,
            ..CacheFile::default()
        };
        save(&cache_path, &stale).expect("save");
        let loaded = load(&cache_path).expect("no-err");
        assert!(
            loaded.is_none(),
            "stale cache should be rejected; returned {loaded:?}"
        );
    }

    #[test]
    fn corrupt_cache_returns_err() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let cache_path = tmp.path().join("cache.bin");
        std::fs::create_dir_all(cache_path.parent().expect("parent")).expect("mkdir");
        std::fs::write(&cache_path, b"not valid msgpack").expect("write");
        let r = load(&cache_path);
        assert!(r.is_err(), "corrupt cache should error; got {r:?}");
    }

    #[test]
    fn save_creates_parent_directory() {
        let tmp = tempfile::tempdir().expect("tempdir");
        // Non-existent .blastguard/ — save must create it.
        let cache_path = tmp.path().join(".blastguard").join("cache.bin");
        assert!(!cache_path.parent().expect("parent").exists());
        save(
            &cache_path,
            &CacheFile {
                version: CACHE_VERSION,
                ..CacheFile::default()
            },
        )
        .expect("save creates dir");
        assert!(cache_path.is_file());
    }
}
