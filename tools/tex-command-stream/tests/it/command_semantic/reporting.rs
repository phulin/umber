//! Case-level failure accounting for the manually selected semantic corpus.

use std::fmt;

use tex_command_stream::semantic::ExpectationError;
use tex_command_stream::semantic::channels::ChannelFailure;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PrimaryFailure {
    ExecutionBlocked,
    SemanticProjection,
    ChannelDiscrepancy,
}

impl fmt::Display for PrimaryFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::ExecutionBlocked => "execution-blocked",
            Self::SemanticProjection => "semantic-projection",
            Self::ChannelDiscrepancy => "channel-discrepancy",
        };
        formatter.write_str(name)
    }
}

pub(super) struct CaseFailure {
    label: String,
    primary: PrimaryFailure,
    discrepancies: Vec<String>,
}

impl CaseFailure {
    pub(super) fn from_outcomes(
        label: String,
        execution_error: Option<&str>,
        expectation: &Result<(), ExpectationError>,
        channels: &[ChannelFailure],
    ) -> Option<Self> {
        let primary = if execution_error.is_some() {
            PrimaryFailure::ExecutionBlocked
        } else if expectation.is_err() {
            PrimaryFailure::SemanticProjection
        } else if !channels.is_empty() {
            PrimaryFailure::ChannelDiscrepancy
        } else {
            return None;
        };
        let mut discrepancies = Vec::new();
        if let Some(error) = execution_error {
            // The projection receives this same execution error as input. Its
            // disposition is useful context, but it is not a second failure
            // from an engine run that never reached the channel contract.
            let disposition = match expectation {
                Ok(()) => "matched the declared failure fingerprint".to_owned(),
                Err(error) => format!("{error:?}"),
            };
            discrepancies.push(format!(
                "execution: {error} (projection disposition: {disposition}; channels not evaluated)"
            ));
        } else if let Err(error) = expectation {
            discrepancies.push(format!("projection: {error:?}"));
        }
        discrepancies.extend(
            channels
                .iter()
                .map(|failure| format!("channel: {failure:?}")),
        );
        Some(Self {
            label,
            primary,
            discrepancies,
        })
    }
}

impl fmt::Display for CaseFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}: primary={} discrepancies={}",
            self.label,
            self.primary,
            self.discrepancies.len()
        )?;
        for discrepancy in &self.discrepancies {
            write!(formatter, "\n  {discrepancy}")?;
        }
        Ok(())
    }
}

#[derive(Default)]
pub(super) struct FailureSummary {
    cases: usize,
    discrepancies: usize,
    execution_blocked: usize,
    semantic_projection: usize,
    channel_discrepancy: usize,
}

impl FailureSummary {
    pub(super) fn from_cases(cases: &[CaseFailure]) -> Self {
        let mut summary = Self::default();
        for case in cases {
            summary.cases += 1;
            summary.discrepancies += case.discrepancies.len();
            match case.primary {
                PrimaryFailure::ExecutionBlocked => summary.execution_blocked += 1,
                PrimaryFailure::SemanticProjection => summary.semantic_projection += 1,
                PrimaryFailure::ChannelDiscrepancy => summary.channel_discrepancy += 1,
            }
        }
        summary
    }
}

impl fmt::Display for FailureSummary {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "cases={}, discrepancies={}, primary execution-blocked={}, semantic-projection={}, channel-discrepancy={}",
            self.cases,
            self.discrepancies,
            self.execution_blocked,
            self.semantic_projection,
            self.channel_discrepancy
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tex_command_stream::semantic::MismatchFingerprint;

    fn projection_mismatch() -> ExpectationError {
        ExpectationError::PassMismatch(MismatchFingerprint {
            index: 0,
            kind: "observation".into(),
            expected: "page 1".into(),
            actual: "page 2".into(),
        })
    }

    #[test]
    fn multiple_discrepancies_remain_one_failing_case() {
        let channel = ChannelFailure::NotEmpty {
            channel: "terminal",
            bytes: 5,
        };
        let failure = CaseFailure::from_outcomes(
            "page-output/example".into(),
            None,
            &Err(projection_mismatch()),
            &[channel],
        )
        .expect("failed case");
        let summary = FailureSummary::from_cases(&[failure]);
        assert_eq!(summary.cases, 1);
        assert_eq!(summary.discrepancies, 2);
        assert_eq!(summary.semantic_projection, 1);
        assert_eq!(summary.channel_discrepancy, 0);
    }

    #[test]
    fn blocked_execution_is_one_discrepancy_and_names_unreached_channels() {
        let failure = CaseFailure::from_outcomes(
            "scanners/example".into(),
            Some("resource suspension"),
            &Err(projection_mismatch()),
            &[],
        )
        .expect("blocked case");
        let text = failure.to_string();
        let summary = FailureSummary::from_cases(&[failure]);
        assert_eq!(summary.cases, 1);
        assert_eq!(summary.discrepancies, 1);
        assert_eq!(summary.execution_blocked, 1);
        assert!(text.contains("channels not evaluated"));
        assert!(text.contains("projection disposition"));
    }

    #[test]
    fn channel_only_failure_and_match_are_distinct() {
        let failure = CaseFailure::from_outcomes(
            "main-control/example".into(),
            None,
            &Ok(()),
            &[ChannelFailure::NotEmpty {
                channel: "log",
                bytes: 2,
            }],
        )
        .expect("channel failure");
        let summary = FailureSummary::from_cases(&[failure]);
        assert_eq!(summary.channel_discrepancy, 1);
        assert_eq!(summary.discrepancies, 1);
        assert!(CaseFailure::from_outcomes("match".into(), None, &Ok(()), &[]).is_none());
    }
}
