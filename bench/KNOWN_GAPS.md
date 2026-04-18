# Known gaps — bench harness is NOT a faithful SWE-bench Pro evaluator

This harness (Plan 7) was built from `SPEC.md` §15 without access to the
real dataset. Empirical contact on 2026-04-18 revealed the following
gaps that must be closed before numbers from `compare.py` are
publishable as SWE-bench Pro results.

## Gap 1 — Dataset schema mismatch

`bench/tasks.py::Task` was designed around guessed field names. Actual
columns in `ScaleAI/SWE-bench_Pro[test]` (731 rows):

| Real column | Our Task field | Fix needed |
|---|---|---|
| `instance_id` | `task_id` | Rename or map |
| `repo` | `repo` | ✓ |
| `base_commit` | `base_commit` | ✓ |
| `patch` | `reference_patch` | Rename or map |
| `test_patch` | (missing) | Required for grading — add |
| `problem_statement` | `problem_statement` | ✓ |
| `requirements` | (missing) | Structured spec the agent can read — add |
| `interface` | (missing) | Method / API signature the agent must implement — add |
| `repo_language` | (missing) | `js` / `py` / `go` / `java` / ... — needed to pick runner |
| `fail_to_pass` | `fail_to_pass` | Type mismatch: real field is a JSON-encoded string, not a list. Needs json.loads before use |
| `pass_to_pass` | `pass_to_pass` | Same type mismatch |
| `issue_specificity` | (missing) | Task categorisation |
| `issue_categories` | (missing) | Task categorisation |
| `before_repo_set_cmd` | (missing) | Setup shell commands to run before the agent — required |
| `selected_test_files_to_run` | (missing) | Scope for grading |
| `dockerhub_tag` | (missing) | **Points to the prebuilt eval image — required for faithful grading** |

## Gap 2 — Multi-language, not pytest-only

SWE-bench Pro spans JavaScript, Python, Go, Java, Rust, C++ and more.
Our `grader.py` hard-codes `pytest`. For a JS task like `NodeBB/NodeBB`,
running pytest does nothing useful and the task always grades as
unresolved. This would produce a ~0% resolution rate on most tasks for
reasons unrelated to the agent's ability.

Real grading path (per the paper and `scaleapi/SWE-bench_Pro-os`): pull
`jefzda/sweap-images:<dockerhub_tag>` which has the repo + deps +
language-appropriate runner preinstalled, apply the agent's patch inside
the container, run `selected_test_files_to_run` with the repo's native
runner, check `fail_to_pass` and `pass_to_pass` for the resolved verdict.

## Gap 3 — Bare clone vs. prebuilt image

Our `runner.py::setup_workspace` does a plain `git clone` at
`base_commit`. Real SWE-bench Pro evaluation runs inside the container
image which has:
- All runtime dependencies installed.
- Any native extensions built.
- The `before_repo_set_cmd` applied.
- Language-specific caches warm.

Without this, even a correct patch might fail grading because `npm
install` or `bundle install` or `pip install -e .` was never run.

## Gap 4 — HF cache path

`datasets.load_dataset` writes to `~/.cache/huggingface/hub/` by default.
If that directory is owned by root or otherwise unwritable, the loader
raises `PermissionError`. Workaround: set `HF_HOME=/tmp/bg-hf-cache`
(or any writable path) before invoking the runner.

## Path forward

### Option A — integrate with the official evaluator

Depend on `scaleapi/SWE-bench_Pro-os` from GitHub. Turn our `runner.py`
into a thin shim that:

1. Loads the task.
2. Runs our agent loop to produce a unified-diff patch.
3. Hands the patch + `instance_id` to the official evaluator
   (`run_evaluation.py` or equivalent in their repo), which pulls the
   Docker image and grades properly.

This is probably 1-2 days of harness work plus Docker + sufficient disk
(~100 GB for the image cache across 731 instances).

### Option B — stop and wait

Ship the harness as WIP with this document as the honest gap list. Skip
publishing any numbers until option A lands.

Until option A is done, **do not publish numbers from this harness**.
They will not be comparable to the official SWE-bench Pro leaderboard
and claiming otherwise violates `SPEC.md` §15.3's honesty contract.
