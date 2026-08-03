//! Native gate for committed canonical command-trace microfixtures.

#![allow(
    clippy::disallowed_methods,
    reason = "this host-only test discovers the committed repository fixture suite"
)]

use tex_command_stream::{RunOptions, RunOutcome, run_committed_repository};

#[test]
fn committed_tex82_command_traces_are_clean() {
    let repository = test_support::repository_root();
    let report = run_committed_repository(repository, RunOptions::default())
        .expect("committed TeX82 command fixture suite must be present and valid");

    assert_eq!(report.outcome(), RunOutcome::Clean, "{report}");
    assert!(report.is_complete(), "{report}");
    assert!(report.uncompared().is_empty(), "{report}");
    assert!(!report.fixtures.is_empty(), "{report}");
    assert!(
        report
            .fixtures
            .iter()
            .any(|fixture| fixture.name == "tex82/geometry-v3" && fixture.advisory),
        "the committed schema-v3 geometry projection must be retained as advisory diagnostics: {report}"
    );
    assert!(
        report
            .to_string()
            .contains("ADVISORY geometry diagnostics (non-gating)"),
        "{report}"
    );
}
