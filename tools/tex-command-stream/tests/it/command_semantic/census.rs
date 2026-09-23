//! Typed run accounting for the selected command-semantic compatibility lane.

use std::fmt;

use tex_command_stream::semantic::channels::{ChannelFailure, StreamDisposition};
use tex_command_stream::semantic::{DeclaredCase, Expectation, ExpectationError};

#[derive(Default)]
pub(super) struct CompatibilityCensus {
    matched: usize,
    known_failures: usize,
    unexpected_passes: usize,
    other_failures: usize,
    unselected: usize,
}

impl CompatibilityCensus {
    pub(super) fn total(&self) -> usize {
        self.matched
            + self.known_failures
            + self.unexpected_passes
            + self.other_failures
            + self.unselected
    }

    pub(super) fn skip_unselected(&mut self) {
        self.unselected += 1;
    }

    pub(super) fn unselected(&self) -> usize {
        self.unselected
    }

    pub(super) fn record(
        &mut self,
        case: &DeclaredCase,
        run_completed: bool,
        expectation: &Result<(), ExpectationError>,
        channels: &[ChannelFailure],
    ) {
        let unexpected_pass = matches!(expectation, Err(ExpectationError::Xpass))
            || channels
                .iter()
                .any(|failure| matches!(failure, ChannelFailure::Xpass { .. }));
        if unexpected_pass {
            self.unexpected_passes += 1;
        } else if !run_completed || expectation.is_err() || !channels.is_empty() {
            self.other_failures += 1;
        } else if has_known_failure(case) {
            self.known_failures += 1;
        } else {
            self.matched += 1;
        }
    }
}

fn has_known_failure(case: &DeclaredCase) -> bool {
    if matches!(case.case.expectation, Expectation::Xfail { .. }) {
        return true;
    }
    let channels = case
        .case
        .channels
        .as_ref()
        .expect("validated channel contract");
    [
        &channels.terminal,
        &channels.log,
        &channels.dvi,
        &channels.effects,
        &channels.diagnostics,
    ]
    .into_iter()
    .any(|channel| {
        matches!(
            channel,
            StreamDisposition::Xfail { .. } | StreamDisposition::XfailDiagnostics { .. }
        )
    })
}

impl fmt::Display for CompatibilityCensus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "matched={}, executed-known-failure={}, unexpected-pass={}, other-failure={}, dormant=0, unselected={}",
            self.matched,
            self.known_failures,
            self.unexpected_passes,
            self.other_failures,
            self.unselected,
        )
    }
}

/// The routine lane does not select this manual compatibility driver. Its
/// manifest cases are unselected, not executed or permanently dormant.
#[test]
fn routine_command_semantic_selection_reports_unselected_cases() {
    let cases = super::load_suite().expect("valid typed command-semantic corpus");
    println!(
        "command-semantic routine: matched=0, executed-known-failure=0, unexpected-pass=0, other-failure=0, dormant=0, unselected={}",
        cases.len()
    );
}
