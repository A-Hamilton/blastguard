# Known Gaps — resolved by Plan 8 rewrite

Gap 1 (schema mismatch) — RESOLVED. `bench/tasks.py` now loads the real
`ScaleAI/SWE-bench_Pro` schema with lowercase keys and JSON-encoded
test lists.

Gap 2 (pytest-only grading) — RESOLVED. Grading is delegated to
`scaleapi/SWE-bench_Pro-os` via `bench/evaluator.py`. Multi-language
tasks grade correctly because the evaluator uses per-repo Docker images
and per-repo test runners. We still filter to Python-only in `load_tasks`
as an MVP scope control — remove `python_only=True` to grade all
languages.

Gap 3 (no dep install) — RESOLVED. The evaluator's per-repo Docker
images include dependencies. `setup_workspace` in `runner.py` is no
longer needed; patches apply inside the evaluator container.

Gap 4 (HF_HOME) — UNCHANGED. Set `HF_HOME=/tmp/hf` before running
anything that touches the datasets library.

# Remaining deferred work (Plan 9+)

- JS / Go / Rust / Java / C++ task support — drop `python_only=True` and
  confirm evaluator runs green against non-Python repos.
- Multi-seed analysis to shrink CI further.
- Parallel per-task rollouts (currently sequential).
- Watch SWE-bench_Pro-os issues #75 (image inconsistency), #76 (test
  name mismatches), #78 (silent rate-limit failures). Our wrapper
  exposes #78 as `infra_failure` so it doesn't contaminate McNemar's.

## Gap 5 — SWE-agent Docker tag truncation (upstream, open)

SWE-agent's `swerex.deployment.docker.DockerDeployment._build_image()`
runs `docker build -q --build-arg BASE_IMAGE=<long_tag> -` as a step
in every task's environment setup. The `<long_tag>` is passed straight
through from our instance JSON's `image_name` field
(`jefzda/sweap-images:<dockerhub_tag>`).

For SWE-bench Pro, `dockerhub_tag` can run 100-130 characters
(`internetarchive.openlibrary-internetarchive__openlibrary-<40-char-sha>-v<40-char-sha>`).
Docker enforces a **128-character max on image tags**; anything longer
is silently truncated in the shell command. The truncated tag doesn't
exist on Docker Hub, the build fails with `CalledProcessError(1)`, and
the task never reaches the agent.

Related: `scaleapi/SWE-bench_Pro-os` issue #75 ("Inconsistency between
images on DockerHub and Dockerfiles in repo").

**Unblock options:**

1. Pre-process step: `docker pull jefzda/sweap-images:<long>` then
   `docker tag jefzda/sweap-images:<long> bench/pro:<short_hash>` for
   every task. Rewrite `instances.jsonl` to point at the short tag.
   Adds disk + pull time up front.
2. Upstream patch to SWE-agent: skip the `docker build` wrapper when the
   base image is already usable (`--use_local_docker` beta flag area).
3. Pivot to SWE-bench Verified: `princeton-nlp/SWE-bench_Verified` ships
   `image_name` natively with tags under the 128-char limit. All our
   scaffold code (bundle / bridge / compare.py / McNemar's) transfers
   without change.

Option 3 is the cheapest path to a first BlastGuard lift number.
