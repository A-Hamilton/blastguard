//! End-to-end: seed a small project in a tempdir, cold-index it, apply a
//! signature-changing edit, assert SIGNATURE fires + bundled context
//! names the caller.

use std::sync::Mutex;

use blastguard::edit::{apply_change, ApplyChangeRequest, ApplyStatus, Change};
use blastguard::graph::impact::WarningKind;
use blastguard::index::indexer::cold_index;
use blastguard::session::SessionState;

#[test]
fn signature_edit_end_to_end_cascade() {
    let tmp = tempfile::tempdir().expect("tempdir");
    std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(tmp.path())
        .status()
        .expect("git init");
    std::fs::create_dir_all(tmp.path().join("src")).expect("mkdir");
    // Both processRequest and its caller `api` live in handler.ts, because
    // Phase 1.2 only emits intra-file Calls edges. Cross-file call
    // resolution is Task 13 (run_tests-adjacent work in Plan 4).
    std::fs::write(
        tmp.path().join("src/handler.ts"),
        "export function processRequest(req) { return req; }\n\
         export function api() { return processRequest({}); }\n",
    )
    .expect("write handler");

    let graph = Mutex::new(cold_index(tmp.path()).expect("cold_index"));
    let session = Mutex::new(SessionState::new());

    let req = ApplyChangeRequest {
        file: tmp.path().join("src/handler.ts"),
        changes: vec![Change {
            old_text: "processRequest(req)".to_string(),
            new_text: "processRequest(req, res)".to_string(),
        }],
        create_file: false,
        delete_file: false,
    };

    let resp = apply_change(&graph, &session, tmp.path(), &req).expect("apply");
    assert_eq!(resp.status, ApplyStatus::Applied);
    assert!(
        resp.warnings
            .iter()
            .any(|w| w.kind == WarningKind::Signature),
        "expected SIGNATURE; got {:?}",
        resp.warnings
    );
    assert!(
        resp.context
            .callers
            .iter()
            .any(|c| c.contains("processRequest") || c.contains("api")),
        "expected caller in context; got {:?}",
        resp.context.callers
    );
}

/// Cross-file Python: edit `login()` in utils/auth.py where the only caller
/// lives in handler.py. `resolve_calls` must stitch the graph so SIGNATURE
/// surfaces `handle` from across the file boundary.
#[test]
fn signature_edit_cross_file_python_cascade() {
    let tmp = tempfile::tempdir().expect("tempdir");
    std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(tmp.path())
        .status()
        .expect("git init");
    std::fs::create_dir_all(tmp.path().join("src/utils")).expect("mkdir");
    std::fs::write(
        tmp.path().join("src/utils/auth.py"),
        "def login(user):\n    return user\n",
    )
    .expect("write auth");
    std::fs::write(
        tmp.path().join("src/handler.py"),
        "from utils.auth import login\n\ndef handle(req):\n    return login(req)\n",
    )
    .expect("write handler");

    let graph = Mutex::new(cold_index(tmp.path()).expect("cold_index"));
    let session = Mutex::new(SessionState::new());

    let req = ApplyChangeRequest {
        file: tmp.path().join("src/utils/auth.py"),
        changes: vec![Change {
            old_text: "def login(user):".to_string(),
            new_text: "def login(user, token):".to_string(),
        }],
        create_file: false,
        delete_file: false,
    };

    let resp = apply_change(&graph, &session, tmp.path(), &req).expect("apply");
    assert_eq!(resp.status, ApplyStatus::Applied);
    let sig_warning = resp
        .warnings
        .iter()
        .find(|w| w.kind == WarningKind::Signature)
        .unwrap_or_else(|| {
            panic!(
                "expected SIGNATURE warning naming the cross-file caller; got {:?}",
                resp.warnings
            )
        });
    // The warning body should name handle() as an impacted caller — that's
    // the payoff of cross-file Calls edge resolution.
    assert!(
        sig_warning.body.contains("handle"),
        "warning body should name `handle` (cross-file caller); got: {}",
        sig_warning.body
    );
}
