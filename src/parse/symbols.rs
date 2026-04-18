//! Shared symbol-extraction helpers across language drivers.
//!
//! Phase 1.2 holds this thin. Each language module in [`super`] is responsible
//! for its own tree-sitter queries and emits [`super::ParseOutput`]. Helpers
//! here avoid duplicating boilerplate across drivers.

/// Render a human-readable function/method signature for inline display in
/// search results.
///
/// Returns `"name(params): ret"` when a return type is present,
/// `"name(params)"` when absent.
#[must_use]
pub fn render_signature(name: &str, params: &str, return_type: Option<&str>) -> String {
    match return_type {
        Some(ret) if !ret.is_empty() => format!("{name}{params}: {ret}"),
        _ => format!("{name}{params}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_with_return_type() {
        assert_eq!(
            render_signature("foo", "(x: number)", Some("string")),
            "foo(x: number): string"
        );
    }

    #[test]
    fn render_without_return_type() {
        assert_eq!(render_signature("bar", "()", None), "bar()");
    }
}
