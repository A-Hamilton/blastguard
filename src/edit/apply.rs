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

/// Scan `body` for the line with the highest normalised-Levenshtein
/// similarity to `needle`. Returns `(line_number_1_based, similarity_0_to_1, fragment)`.
fn closest_line(body: &str, needle: &str) -> (u32, f32, String) {
    let mut best_line: u32 = 0;
    let mut best_sim: f32 = 0.0;
    let mut best_fragment = String::new();
    for (idx, line) in body.lines().enumerate() {
        let dist = strsim::levenshtein(line, needle);
        let max_len = line.len().max(needle.len()).max(1);
        #[allow(clippy::cast_precision_loss)]
        let sim = 1.0_f32 - (dist as f32 / max_len as f32);
        if sim > best_sim {
            best_sim = sim;
            best_line = u32::try_from(idx)
                .unwrap_or(u32::MAX)
                .saturating_add(1);
            best_fragment = line.to_string();
        }
    }
    (best_line, best_sim, best_fragment)
}

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
        0 => {
            let (line, similarity, fragment) = closest_line(&body, old_text);
            Err(BlastGuardError::EditNotFound {
                path: path.to_path_buf(),
                line,
                similarity,
                fragment,
            })
        }
        1 => {
            let updated = body.replacen(old_text, new_text, 1);
            std::fs::write(path, updated).map_err(|source| BlastGuardError::Io {
                path: path.to_path_buf(),
                source,
            })
        }
        n => {
            let lines = find_match_lines(&body, old_text);
            Err(BlastGuardError::AmbiguousEdit {
                path: path.to_path_buf(),
                count: n,
                lines,
            })
        }
    }
}

/// Enumerate 1-based line numbers where `needle` appears in `body`.
/// Multi-line needles count once per starting line.
fn find_match_lines(body: &str, needle: &str) -> Vec<u32> {
    let mut lines = Vec::new();
    let mut cursor = 0usize;
    while let Some(found) = body[cursor..].find(needle) {
        let offset = cursor + found;
        let line = body[..offset].chars().filter(|&c| c == '\n').count();
        let line_1based = u32::try_from(line)
            .unwrap_or(u32::MAX)
            .saturating_add(1);
        lines.push(line_1based);
        cursor = offset + needle.len().max(1);
    }
    lines
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

    #[test]
    fn ambiguous_edit_lists_all_match_line_numbers() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("a.ts");
        std::fs::write(&path, "a = 1\nb = 1\nc = 1\n").expect("write");
        let err = apply_edit(&path, "= 1", "= 2").expect_err("ambiguous");
        match err {
            BlastGuardError::AmbiguousEdit { count, lines, .. } => {
                assert_eq!(count, 3);
                assert_eq!(lines, vec![1, 2, 3]);
            }
            e => panic!("wrong variant: {e:?}"),
        }
    }

    #[test]
    fn edit_not_found_carries_closest_match_and_similarity() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("a.ts");
        std::fs::write(
            &path,
            "function processRequest(req) {\n    return handler(req);\n}\n",
        ).expect("write");
        // Caller provided the function header without the parameter.
        let err = apply_edit(&path, "function processRequest() {", "function x() {")
            .expect_err("not found");
        match err {
            BlastGuardError::EditNotFound { line, similarity, fragment, .. } => {
                assert_eq!(line, 1, "closest line should be the function header");
                assert!(similarity >= 0.7, "similarity {similarity} too low for a near-miss");
                assert!(fragment.contains("processRequest"), "fragment = {fragment}");
            }
            e => panic!("wrong variant: {e:?}"),
        }
    }
}
