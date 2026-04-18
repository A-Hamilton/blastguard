//! Regex grep fallback — SPEC §3.1.
//!
//! Walks the project via the `ignore` crate (gitignore-aware) and returns
//! up to [`GREP_MAX_HITS`] matches of the given regex. Invalid regex is
//! retried as a literal-string search before giving up.

use std::io::{BufRead, BufReader};
use std::path::Path;

use regex::Regex;

use crate::search::hit::SearchHit;

/// Cap the number of grep hits returned (SPEC §3.1 grep row: "Regex grep …
/// cap 30").
pub const GREP_MAX_HITS: usize = 30;

/// Regex grep across the project respecting `.gitignore`. Caps at
/// [`GREP_MAX_HITS`] matches. Returns `file:line` plus the matching line as
/// `snippet` (trailing whitespace trimmed).
///
/// Invalid regex patterns are retried as literal-string searches via
/// [`regex::escape`]; if that also fails (impossible in practice — escaping
/// always produces a valid regex), returns an empty `Vec`.
#[must_use]
pub fn grep(project_root: &Path, pattern: &str) -> Vec<SearchHit> {
    let re = match Regex::new(pattern) {
        Ok(r) => r,
        Err(_) => match Regex::new(&regex::escape(pattern)) {
            Ok(r) => r,
            Err(_) => return Vec::new(),
        },
    };

    let mut hits = Vec::new();
    let walker = ignore::WalkBuilder::new(project_root)
        .standard_filters(true)
        .hidden(false)
        .build();

    for entry in walker.filter_map(std::result::Result::ok) {
        if hits.len() >= GREP_MAX_HITS {
            break;
        }
        let Some(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_file() {
            continue;
        }
        let Ok(file) = std::fs::File::open(entry.path()) else {
            continue;
        };
        let reader = BufReader::new(file);
        for (idx, line_res) in reader.lines().enumerate() {
            if hits.len() >= GREP_MAX_HITS {
                break;
            }
            let Ok(line) = line_res else {
                continue;
            };
            if re.is_match(&line) {
                let lineno = u32::try_from(idx)
                    .unwrap_or(u32::MAX)
                    .saturating_add(1);
                hits.push(SearchHit::grep(
                    entry.path().to_path_buf(),
                    lineno,
                    line.trim_end().to_string(),
                ));
            }
        }
    }

    hits
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seed(dir: &Path, files: &[(&str, &str)]) {
        std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(dir)
            .status()
            .expect("git init");
        for (p, body) in files {
            let full = dir.join(p);
            std::fs::create_dir_all(full.parent().expect("parent")).expect("mkdir");
            std::fs::write(&full, body).expect("write");
        }
    }

    #[test]
    fn grep_finds_matching_line() {
        let tmp = tempfile::tempdir().expect("tempdir");
        seed(
            tmp.path(),
            &[("src/a.ts", "function processRequest() {}\nconst x = 1;\n")],
        );
        let hits = grep(tmp.path(), "processRequest");
        assert!(!hits.is_empty());
        let snippet = hits[0].snippet.as_deref().unwrap();
        assert!(snippet.contains("processRequest"), "snippet = {snippet}");
        assert_eq!(hits[0].line, 1);
    }

    #[test]
    fn grep_respects_gitignore() {
        let tmp = tempfile::tempdir().expect("tempdir");
        std::fs::write(tmp.path().join(".gitignore"), "vendor/\n").expect("gitignore");
        seed(
            tmp.path(),
            &[
                ("src/a.ts", "processRequest();"),
                ("vendor/skip.ts", "processRequest();"),
            ],
        );
        let hits = grep(tmp.path(), "processRequest");
        assert!(
            hits.iter().all(|h| !h.file.to_string_lossy().contains("vendor")),
            "vendor hits leaked: {hits:?}"
        );
    }

    #[test]
    fn grep_caps_at_max_hits() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut files: Vec<(String, String)> = Vec::new();
        for i in 0..(GREP_MAX_HITS + 10) {
            files.push((format!("src/f{i}.ts"), "MATCHME".to_string()));
        }
        let refs: Vec<(&str, &str)> = files
            .iter()
            .map(|(a, b)| (a.as_str(), b.as_str()))
            .collect();
        seed(tmp.path(), &refs);
        let hits = grep(tmp.path(), "MATCHME");
        assert_eq!(hits.len(), GREP_MAX_HITS);
    }

    #[test]
    fn grep_invalid_regex_falls_back_to_literal() {
        let tmp = tempfile::tempdir().expect("tempdir");
        seed(tmp.path(), &[("src/a.ts", "const x = ?(invalid);")]);
        // `?(` is invalid as a bare regex; must still match as a literal.
        let hits = grep(tmp.path(), "?(invalid)");
        assert!(!hits.is_empty(), "literal fallback should match");
    }
}
