//! Execute the detected runner under a timeout and collect stdout+stderr.
//!
//! Guards against the Berkeley `BenchJack` `conftest.py` exploit (SPEC §15.4):
//! the grader never trusts agent-written pytest config files at benchmark
//! time. The in-project `run_tests` tool does respect them since the user
//! owns that repo.

// TODO(phase-1.7): run(command, timeout) returning structured stdout+stderr.
