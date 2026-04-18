<!-- Thanks for contributing. Small PRs that do one thing merge fastest.
Delete sections that don't apply. -->

## What this changes

<!-- One paragraph. What does this PR do that main doesn't? -->

## Why

<!-- Why does this need to exist? Link any related issue. -->

Closes #

## How I tested it

<!-- What did you actually run? Paste the `cargo test` summary + any
manual repro steps. If you added a test, name it here. -->

```
cargo test
cargo clippy --all-targets -- -W clippy::pedantic -D warnings
cargo fmt --all -- --check
```

## Checklist

- [ ] CI passes locally (`cargo fmt --check` + `cargo clippy -D warnings` +
  `cargo test` + `cargo build --release`).
- [ ] If I added a behaviour, there's a test that pins it. If I fixed a
  bug, there's a regression test that would have caught it.
- [ ] No `println!` / `eprintln!` in production paths.
- [ ] No `.unwrap()` / `.expect()` outside `#[cfg(test)]` or documented
  unreachable branches.
- [ ] `///` docs on any new `pub` item.
- [ ] If I added a dependency, Cargo.toml has a comment explaining why.

## Evidence-gate check (Phase 2 features only)

<!-- Only fill this out if you're adding semantic search, Go support, or
a new cascade detector. Otherwise delete this section. -->

- [ ] I've read `SPEC.md` §Decision Log and `CLAUDE.md`'s phase rules.
- [ ] I have Phase 1 benchmark data that supports this scope (link or
  describe).
