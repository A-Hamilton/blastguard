//! Runner auto-detection — SPEC §3.3 auto-detect table.

use std::path::Path;

use super::Runner;

/// Inspect project files and return the first matching runner, or `None`
/// when detection fails (which maps to `isError: true` at the MCP boundary).
#[must_use]
pub fn autodetect(project_root: &Path) -> Option<Runner> {
    let pkg = project_root.join("package.json");
    if pkg.exists() {
        if let Ok(body) = std::fs::read_to_string(&pkg) {
            if body.contains("\"vitest\"") {
                return Some(Runner::Vitest);
            }
            if body.contains("\"jest\"") {
                return Some(Runner::Jest);
            }
        }
    }
    if project_root.join("pytest.ini").exists()
        || project_root.join("conftest.py").exists()
        || read_pyproject_has_pytest(project_root)
    {
        return Some(Runner::Pytest);
    }
    if project_root.join("Cargo.toml").exists() {
        return Some(Runner::CargoTest);
    }
    None
}

fn read_pyproject_has_pytest(project_root: &Path) -> bool {
    let path = project_root.join("pyproject.toml");
    let Ok(body) = std::fs::read_to_string(&path) else {
        return false;
    };
    body.contains("[tool.pytest")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_cargo() {
        let tmp = tempfile::tempdir().expect("tempdir");
        std::fs::write(tmp.path().join("Cargo.toml"), "[package]\nname=\"x\"\n").expect("write");
        assert_eq!(autodetect(tmp.path()), Some(Runner::CargoTest));
    }

    #[test]
    fn detects_vitest_before_jest() {
        let tmp = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            tmp.path().join("package.json"),
            "{\"devDependencies\":{\"jest\":\"1\",\"vitest\":\"1\"}}",
        )
        .expect("write");
        assert_eq!(autodetect(tmp.path()), Some(Runner::Vitest));
    }

    #[test]
    fn none_when_no_signals() {
        let tmp = tempfile::tempdir().expect("tempdir");
        assert_eq!(autodetect(tmp.path()), None);
    }
}
