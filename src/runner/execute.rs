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
            c.arg("-m").arg("pytest").arg("--tb=short").arg("-q").arg("--json-report");
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jest_command_has_json_reporter() {
        let cmd = build_command(Runner::Jest, Path::new("."), None);
        let args: Vec<String> = cmd.get_args().map(|s| s.to_string_lossy().to_string()).collect();
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
}
