use std::collections::BTreeSet;
use std::fs;

#[test]
fn pdftex_diagnostic_matrix_is_exhaustive_for_supported_hooks() {
    let root = test_support::repository_root();
    let matrix = fs::read_to_string(
        root.join("tests/pdftex14029-oracle/diagnostic-event-matrix.txt"),
    )
    .expect("diagnostic matrix");
    let rows: Vec<_> = matrix
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| line.split('|').collect::<Vec<_>>())
        .collect();
    assert!(rows.iter().all(|row| row.len() == 5));

    let classes: BTreeSet<_> = rows.iter().map(|row| row[0]).collect();
    assert_eq!(
        classes,
        BTreeSet::from(["fatal", "outcome", "recoverable_error", "warning"]),
        "every supported report class and the closing outcome need a matrix row"
    );
    for required in [
        ("recoverable_error", "missing-number"),
        ("recoverable_error", "illegal-unit"),
        ("warning", "incomplete-source-nesting"),
        ("fatal", "emergency-stop"),
        ("fatal", "capacity-exceeded"),
        ("fatal", "confusion"),
    ] {
        assert!(
            rows.iter().any(|row| (row[0], row[1]) == required),
            "matrix omits {required:?}"
        );
    }

    let instrumentation = fs::read_to_string(
        root.join("tests/pdftex14029-oracle/instrumentation.ch"),
    )
    .expect("pdfTeX instrumentation");
    let hooks: BTreeSet<_> = instrumentation
        .lines()
        .filter(|line| line.contains("umber_diag_report(") && !line.contains("procedure "))
        .map(|line| {
        let class = if line.contains("umber_diag_report(0,") {
            "recoverable_error"
        } else if line.contains("umber_diag_report(1,") {
            "warning"
        } else if line.contains("umber_diag_report(2,") {
            "fatal"
        } else {
            panic!("unclassified diagnostic hook {line:?}");
        };
        let identity = line
            .split_once("umber_diag_report(")
            .expect("diagnostic hook")
            .1
            .split('"')
            .nth(1)
            .expect("literal diagnostic identity");
        (class, identity)
        })
        .collect();
    for &(class, identity) in &hooks {
        assert!(
            rows.iter().any(|row| row[0] == class && row[1] == identity),
            "instrumentation hook {class}/{identity} is absent from the matrix"
        );
    }
    for row in rows.iter().filter(|row| row[0] != "outcome") {
        assert!(
            hooks.contains(&(row[0], row[1])),
            "matrix row {}/{} names no instrumentation hook",
            row[0],
            row[1]
        );
    }
}
