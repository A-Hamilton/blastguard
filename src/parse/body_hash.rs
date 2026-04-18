//! Stable body hash — SPEC §8.4.
//!
//! Strip line whitespace, collapse blank lines, remove single-line `//`/`#`
//! and block `/* */` comments, then `seahash`. The result is stable across
//! formatting-only changes, so a re-indent does not trigger `MODIFIED_BODY`
//! cascade reconsideration.

/// Hash a function/class body for equivalence comparison. See module docs.
#[must_use]
pub fn body_hash(src: &str) -> u64 {
    let normalized = normalize(src);
    seahash::hash(normalized.as_bytes())
}

fn normalize(src: &str) -> String {
    // Strip /* ... */ block comments first (may span lines).
    let without_block = strip_block_comments(src);

    // Collect non-empty trimmed tokens from each line, then join with a single
    // space. This makes multi-line block comment removal stable: removing a
    // comment that spans lines does not change the hash versus the equivalent
    // single-line form (e.g. `{\n/* x */ return 1;` hashes the same as
    // `{ return 1;`).
    let mut tokens: Vec<&str> = Vec::new();
    for line in without_block.lines() {
        let stripped = strip_line_comment(line).trim();
        if !stripped.is_empty() {
            tokens.push(stripped);
        }
    }
    tokens.join(" ")
}

fn strip_line_comment(line: &str) -> &str {
    // Conservative: treat `//` and `#` as comment starters, but only outside
    // strings. We don't bother with string tracking in Phase 1; a hash
    // collision from a commented-out string literal is low-harm.
    if let Some(idx) = line.find("//") {
        return &line[..idx];
    }
    if let Some(idx) = line.find('#') {
        return &line[..idx];
    }
    line
}

fn strip_block_comments(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let bytes = src.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            // Scan to closing */
            let mut j = i + 2;
            while j + 1 < bytes.len() && !(bytes[j] == b'*' && bytes[j + 1] == b'/') {
                j += 1;
            }
            i = j.saturating_add(2).min(bytes.len());
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whitespace_changes_do_not_shift_hash() {
        let a = "fn foo() {\n    return 1;\n}";
        let b = "fn foo() {\nreturn 1;\n}";
        assert_eq!(body_hash(a), body_hash(b));
    }

    #[test]
    fn comment_changes_do_not_shift_hash() {
        let a = "fn foo() { // old comment\n    return 1;\n}";
        let b = "fn foo() { // brand new comment\n    return 1;\n}";
        assert_eq!(body_hash(a), body_hash(b));
    }

    #[test]
    fn block_comments_stripped() {
        let a = "fn foo() {\n/* block\n spanning */ return 1;\n}";
        let b = "fn foo() { return 1;\n}";
        assert_eq!(body_hash(a), body_hash(b));
    }

    #[test]
    fn body_changes_shift_hash() {
        let a = "fn foo() { return 1; }";
        let b = "fn foo() { return 2; }";
        assert_ne!(body_hash(a), body_hash(b));
    }

    #[test]
    fn python_hash_comments() {
        let a = "def foo():\n    # old\n    return 1";
        let b = "def foo():\n    # new text\n    return 1";
        assert_eq!(body_hash(a), body_hash(b));
    }
}
