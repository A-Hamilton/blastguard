//! On-disk file edit primitive.
//!
//! [`apply_edit`] performs one `old_text → new_text` swap in the target
//! file. If `old_text` appears exactly once, the swap succeeds. If it
//! doesn't appear or appears multiple times, returns an error from
//! [`crate::error::BlastGuardError`]; the `apply_change` orchestrator
//! (Task 12) maps those into `CallToolResult { is_error: true, .. }`.
//!
//! Task 3 extends [`BlastGuardError::EditNotFound`] with closest-line
//! hints; Task 4 populates [`BlastGuardError::AmbiguousEdit::lines`].

use std::path::Path;

use crate::error::{BlastGuardError, Result};

/// Replace the single occurrence of `old_text` with `new_text` in `path`.
///
/// # Errors
/// - [`BlastGuardError::Io`] on read/write failure.
/// - [`BlastGuardError::EditNotFound`] when `old_text` doesn't appear.
/// - [`BlastGuardError::AmbiguousEdit`] when `old_text` appears 2+ times.
pub fn apply_edit(path: &Path, old_text: &str, new_text: &str) -> Result<()> {
    let body = std::fs::read_to_string(path).map_err(|source| BlastGuardError::Io {
        path: path.to_path_buf(),
        source,
    })?;

    let occurrences = body.matches(old_text).count();
    match occurrences {
        0 => Err(BlastGuardError::EditNotFound {
            path: path.to_path_buf(),
            line: 0,
            similarity: 0.0,
            fragment: String::new(),
        }),
        1 => {
            let updated = body.replacen(old_text, new_text, 1);
            std::fs::write(path, updated).map_err(|source| BlastGuardError::Io {
                path: path.to_path_buf(),
                source,
            })
        }
        n => Err(BlastGuardError::AmbiguousEdit {
            path: path.to_path_buf(),
            count: n,
            lines: Vec::new(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_edit_exact_single_match_rewrites_file() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("a.ts");
        std::fs::write(&path, "fn foo() { return 1; }").expect("write");

        apply_edit(&path, "return 1", "return 2").expect("apply_edit");

        let after = std::fs::read_to_string(&path).expect("read");
        assert_eq!(after, "fn foo() { return 2; }");
    }

    #[test]
    fn apply_edit_missing_old_text_returns_edit_not_found() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("a.ts");
        std::fs::write(&path, "fn foo() {}").expect("write");
        let err = apply_edit(&path, "NOT_PRESENT", "x").expect_err("should error");
        assert!(matches!(err, BlastGuardError::EditNotFound { .. }), "got {err:?}");
    }

    #[test]
    fn apply_edit_ambiguous_old_text_returns_ambiguous_edit() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("a.ts");
        std::fs::write(&path, "a = 1\nb = 1\n").expect("write");
        let err = apply_edit(&path, "= 1", "= 2").expect_err("ambiguous");
        match err {
            BlastGuardError::AmbiguousEdit { count, .. } => assert_eq!(count, 2),
            e => panic!("wrong variant: {e:?}"),
        }
    }

    #[test]
    fn apply_edit_missing_file_returns_io_error() {
        let err = apply_edit(std::path::Path::new("/nope/does/not/exist"), "x", "y")
            .expect_err("should error");
        assert!(matches!(err, BlastGuardError::Io { .. }), "got {err:?}");
    }
}
