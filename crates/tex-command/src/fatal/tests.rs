use super::{FATAL_SEVERITY, FatalError};
use crate::{DiagnosticArgument, DiagnosticRecord};

#[test]
fn confusion_labels_like_tex_web_section_95() {
    // TeX82 §798 raises exactly this one.
    let fatal = FatalError::confusion("256 spans");
    assert_eq!(fatal.label(), "confusion(256 spans)");
    assert_eq!(
        fatal.record(),
        DiagnosticRecord {
            severity: FATAL_SEVERITY,
            diagnostic: "confusion",
            arguments: vec![DiagnosticArgument::Name("256 spans".into())],
        }
    );
}

#[test]
fn overflow_carries_section_94s_capacity_pair() {
    let fatal = FatalError::overflow("pattern memory", 8_000);
    assert_eq!(fatal.label(), "capacity-exceeded(pattern memory=8000)");
    assert_eq!(fatal.record().severity, FATAL_SEVERITY);
}

#[test]
fn emergency_stop_carries_section_93s_help_line() {
    let fatal = FatalError::emergency_stop("*** (job aborted, no legal \\end found)");
    assert_eq!(
        fatal.label(),
        "emergency-stop(*** (job aborted, no legal \\end found))"
    );
    assert_eq!(fatal.record().diagnostic, "emergency-stop");
}
