"""Centralized registry of micro-bench tasks.

Each task is a dict with:
- `id`: stable identifier for result tables.
- `prompt`: format-string with `{project_root}` as the only placeholder.
- `expected_substrings`: list of case-insensitive substrings that must all
  appear in a correct `final_answer` for `bench/microbench_grader.py` to
  mark the rollout `correct=True`. Quality is Priority 1 — a regression
  here blocks a commit even if token/wall numbers improved.

Design: tasks span BlastGuard's strengths and weaknesses. Rounds 2-6
showed BG wins on intra-file outline/find and loses on cross-file
dependency chains. The expanded set keeps that balance so aggregate
wins are meaningful.
"""

from __future__ import annotations

from typing import Any

TASKS: list[dict[str, Any]] = [
    # --- Existing 4 tasks (kept for continuity with rounds 2-6) ---
    {
        "id": "explore-cold-index",
        "prompt": (
            "Using the tools available, explore the BlastGuard Rust codebase at "
            "{project_root} and explain what the `cold_index` function does and "
            "what calls it. Answer in 3-5 sentences. When done, write 'DONE' "
            "on its own line."
        ),
        "expected_substrings": ["cold_index", "warm_start"],
    },
    {
        "id": "callers-apply-edit",
        "prompt": (
            "In the BlastGuard Rust repo at {project_root}, find every caller of "
            "the `apply_edit` function. For each caller, briefly describe what "
            "it is (function name + file) and what kind of value it passes for "
            "the `old_text` argument. Answer concisely. Write 'DONE' when finished."
        ),
        "expected_substrings": ["orchestrate", "apply.rs"],
    },
    {
        "id": "chain-search-to-graph",
        "prompt": (
            "In the BlastGuard Rust repo at {project_root}, find the call chain "
            "from the MCP `search` tool entry point down into the code-graph "
            "module. In other words: when the MCP search tool is invoked, which "
            "intermediate function(s) get called on the way to the graph "
            "operations? Name each function (file + function name) in order. "
            "Keep the answer under 10 lines. Write 'DONE' when finished."
        ),
        "expected_substrings": ["search_tool", "dispatch", "structural"],
    },
    {
        "id": "cascade-signature-change",
        "prompt": (
            "In the BlastGuard Rust repo at {project_root}, suppose we wanted "
            "to change the signature of `apply_edit` to take a single `Edit` "
            "struct instead of three separate `&Path`, `&str`, `&str` "
            "arguments. List every function that would need to be updated, "
            "and explain why. Keep the answer concise — just a bulleted list "
            "with the file:line of each caller and a one-line reason. "
            "Write 'DONE' when finished."
        ),
        "expected_substrings": ["orchestrate", "apply.rs"],
    },
    # --- Six new tasks (added in Plan 14 Task 3) ---
    {
        # Clean intra-file exploration — should favor BlastGuard outline.
        "id": "outline-tree-sitter-rust",
        "prompt": (
            "In the BlastGuard Rust repo at {project_root}, list every public "
            "function defined in `src/parse/rust.rs` (file path relative to "
            "project root) with its signature. Group them by category (parsing "
            "entry points, helper utilities, edge emitters). Write 'DONE' when "
            "finished."
        ),
        "expected_substrings": ["extract", "emit_function"],
    },
    {
        # Cross-file investigation — currently a BlastGuard weakness in Phase 1.
        "id": "trace-cache-persistence",
        "prompt": (
            "In the BlastGuard Rust repo at {project_root}, explain how the "
            "BLAKE3 Merkle cache is persisted to disk. Identify: (1) where the "
            "cache gets written, (2) where it gets read back, and (3) the "
            "format it's serialized in. Answer in under 8 sentences. Write "
            "'DONE' when finished."
        ),
        "expected_substrings": ["cache.bin", "rmp"],
    },
    {
        # Easy find + grep task — direct-symbol question where grep usually wins.
        "id": "find-tamper-patterns",
        "prompt": (
            "In the BlastGuard Python harness at {project_root}/bench, list "
            "every filename pattern that counts as benchmark tampering under "
            "the BenchJack defense. Where is this list defined? Answer in 2-3 "
            "lines. Write 'DONE' when finished."
        ),
        "expected_substrings": ["conftest.py", "pytest.ini"],
    },
    {
        # Refactor-lite scoping — caller graph + test impact question.
        "id": "impact-of-removing-libraries",
        "prompt": (
            "In the BlastGuard Rust repo at {project_root}, suppose we wanted "
            "to remove support for the `libraries` query type from the search "
            "dispatcher. List every file that would need to change, and "
            "describe what the change would look like in each. Keep the answer "
            "concise — bulleted list format. Write 'DONE' when finished."
        ),
        "expected_substrings": ["query.rs", "dispatcher.rs"],
    },
    {
        # Multi-file orientation + compare — no single clear tool winner.
        "id": "compare-parse-modules",
        "prompt": (
            "In the BlastGuard Rust repo at {project_root}, compare the parse "
            "drivers in `src/parse/python.rs` and `src/parse/rust.rs`. What is "
            "the same between them (structure-wise), and what is meaningfully "
            "different? Keep the comparison to 6 sentences or fewer. Write "
            "'DONE' when finished."
        ),
        "expected_substrings": ["tree-sitter", "extract"],
    },
    {
        # Tests-for style question — exercises BlastGuard's run_tests or its
        # structural tests-for query depending on Phase 1 capability.
        "id": "tests-for-apply-change",
        "prompt": (
            "In the BlastGuard Rust repo at {project_root}, identify every "
            "test function that exercises the `apply_change` or `apply_edit` "
            "code paths. Give the test function name and its file:line. Keep "
            "the answer concise — bulleted list. Write 'DONE' when finished."
        ),
        "expected_substrings": ["apply_edit", "apply.rs"],
    },
]
