//! Execute the detected runner and capture stdout + stderr with a timeout.
//!
//! Guards against the Berkeley `BenchJack` `conftest.py` exploit
//! (SPEC §15.4): the benchmark grader never trusts agent-written pytest
//! config files at grading time. The in-project `run_tests` tool does
//! respect them since the user owns that repo.

use std::path::Path;
use std::process::{Command, Stdio};

use super::Runner;

/// Build the [`Command`] for a given runner without spawning it.
///
/// Returning `Command` here (rather than `Child`) keeps the test simple —
/// inspect `get_program` / `get_args` without actually running the
/// runner. Task 3 wraps this with `run(cmd, timeout)` to spawn.
#[must_use]
pub fn build_command(runner: Runner, project_root: &Path, filter: Option<&str>) -> Command {
    let mut cmd = match runner {
        Runner::Jest => {
            let mut c = Command::new("npx");
            c.arg("jest").arg("--reporters=default").arg("--json");
            c
        }
        Runner::Vitest => {
            let mut c = Command::new("npx");
            c.arg("vitest").arg("run").arg("--reporter=json");
            c
        }
        Runner::Pytest => {
            let mut c = Command::new("python");
            c.arg("-m")
                .arg("pytest")
                .arg("--tb=short")
                .arg("-q")
                .arg("--json-report");
            c
        }
        Runner::CargoTest => {
            let mut c = Command::new("cargo");
            c.arg("test")
                .arg("--no-fail-fast")
                .arg("--")
                .arg("-Z")
                .arg("unstable-options")
                .arg("--format")
                .arg("json");
            c
        }
    };
    if let Some(f) = filter {
        match runner {
            Runner::Jest | Runner::Vitest => {
                cmd.arg("-t").arg(f);
            }
            Runner::Pytest => {
                cmd.arg("-k").arg(f);
            }
            Runner::CargoTest => {
                cmd.arg(f);
            }
        }
    }
    cmd.current_dir(project_root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    cmd
}

use std::time::{Duration, Instant};

use crate::error::{BlastGuardError, Result};

/// Captured output from a completed (or killed) runner process.
#[derive(Debug)]
pub struct ExecuteResult {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub duration: Duration,
}

/// Spawn the given [`Command`] and wait for it, killing on `timeout` overrun.
/// Polls `try_wait` at 50ms cadence — fine for test-runner workloads where
/// the inner test cost dwarfs polling overhead.
///
/// # Errors
/// Returns [`BlastGuardError::TestCrashed`] when the child cannot be
/// spawned (e.g., program not found) or the wait machinery fails.
pub fn run(mut cmd: Command, timeout: Duration) -> Result<ExecuteResult> {
    let started = Instant::now();
    let mut child = cmd.spawn().map_err(|e| BlastGuardError::TestCrashed {
        stderr: format!("failed to spawn: {e}"),
    })?;

    let mut timed_out = false;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {
                if started.elapsed() >= timeout {
                    let _ = child.kill();
                    timed_out = true;
                    let _ = child.wait();
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => {
                return Err(BlastGuardError::TestCrashed {
                    stderr: format!("wait error: {e}"),
                });
            }
        }
    }

    // Short-circuit on timeout: the child has already been reaped inside the
    // loop, so wait_with_output would fail with ECHILD. Return an empty
    // stdout/stderr marked timed_out=true.
    if timed_out {
        return Ok(ExecuteResult {
            stdout: Vec::new(),
            stderr: Vec::new(),
            exit_code: None,
            timed_out: true,
            duration: started.elapsed(),
        });
    }

    let output = child
        .wait_with_output()
        .map_err(|e| BlastGuardError::TestCrashed {
            stderr: format!("wait_with_output: {e}"),
        })?;

    Ok(ExecuteResult {
        stdout: output.stdout,
        stderr: output.stderr,
        exit_code: output.status.code(),
        timed_out: false,
        duration: started.elapsed(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn jest_command_has_json_reporter() {
        let cmd = build_command(Runner::Jest, Path::new("."), None);
        let args: Vec<String> = cmd
            .get_args()
            .map(|s| s.to_string_lossy().to_string())
            .collect();
        assert!(args.iter().any(|a| a == "--json"));
    }

    #[test]
    fn vitest_command_uses_run_and_reporter() {
        let cmd = build_command(Runner::Vitest, Path::new("."), None);
        let args: Vec<String> = cmd
            .get_args()
            .map(|s| s.to_string_lossy().to_string())
            .collect();
        assert!(args.contains(&"run".to_string()));
        assert!(args.iter().any(|a| a == "--reporter=json"));
    }

    #[test]
    fn pytest_command_json_report() {
        let cmd = build_command(Runner::Pytest, Path::new("."), None);
        let args: Vec<String> = cmd
            .get_args()
            .map(|s| s.to_string_lossy().to_string())
            .collect();
        assert!(args.contains(&"--json-report".to_string()));
    }

    #[test]
    fn cargo_command_uses_json_format() {
        let cmd = build_command(Runner::CargoTest, Path::new("."), None);
        let args: Vec<String> = cmd
            .get_args()
            .map(|s| s.to_string_lossy().to_string())
            .collect();
        assert!(args.contains(&"--format".to_string()));
        assert!(args.contains(&"json".to_string()));
    }

    #[test]
    fn filter_appended_for_jest_as_dash_t() {
        let cmd = build_command(Runner::Jest, Path::new("."), Some("auth"));
        let args: Vec<String> = cmd
            .get_args()
            .map(|s| s.to_string_lossy().to_string())
            .collect();
        let t_idx = args.iter().position(|a| a == "-t").expect("missing -t");
        assert_eq!(args[t_idx + 1], "auth");
    }

    #[test]
    fn filter_appended_for_pytest_as_dash_k() {
        let cmd = build_command(Runner::Pytest, Path::new("."), Some("auth"));
        let args: Vec<String> = cmd
            .get_args()
            .map(|s| s.to_string_lossy().to_string())
            .collect();
        let k_idx = args.iter().position(|a| a == "-k").expect("missing -k");
        assert_eq!(args[k_idx + 1], "auth");
    }

    #[test]
    fn run_within_timeout_captures_stdout() {
        let mut cmd = Command::new("echo");
        cmd.arg("hello");
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
        let result = run(cmd, Duration::from_secs(5)).expect("run");
        assert_eq!(result.exit_code, Some(0));
        assert!(String::from_utf8_lossy(&result.stdout).contains("hello"));
    }

    #[test]
    fn run_exceeds_timeout_returns_timeout_flag() {
        let mut cmd = Command::new("sleep");
        cmd.arg("5");
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
        let result =
            run(cmd, Duration::from_millis(200)).expect("run should return Ok with timed_out=true");
        assert!(result.timed_out, "expected timed_out=true, got {result:?}");
        assert!(result.stdout.is_empty());
        assert!(result.exit_code.is_none());
    }
}
