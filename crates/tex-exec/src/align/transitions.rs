//! Typed command-core boundary for executor-owned alignment structure.
//!
//! This module intentionally has no `Token`, `Meaning`, `Universe`, or input
//! stack API.  Raw brace and delimiter classification is complete before an
//! [`AlignmentDeliveryEvent`] reaches it.

use tex_command::{
    AlignmentDeliveryEvent, AlignmentIdentity, AlignmentRequest, AlignmentRequestResult,
    CommandError, CommandProcessor, CommandState,
};

/// Applies an executor-selected structural lifecycle change.
#[allow(dead_code)] // adopted by the executor when its command loop migrates
pub(super) fn request(
    command: &mut CommandState,
    request: AlignmentRequest,
) -> Result<AlignmentRequestResult, tex_command::AlignmentLifecycleError> {
    command.apply_alignment_request(request)
}

/// Starts the v-template selected by a command-core end-template event.
///
/// The event carries the exact raw delivery proof, so this boundary neither
/// receives nor recognizes the original tab, `\span`, or `\cr` spelling.
#[allow(dead_code)] // adopted by the executor when its command loop migrates
pub(super) fn begin_v_template(
    processor: &mut CommandProcessor<'_>,
    alignment: AlignmentIdentity,
    event: AlignmentDeliveryEvent,
) -> Result<(), CommandError> {
    processor.begin_alignment_v_template(alignment, event)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn structural_boundary_accepts_only_typed_requests() {
        let mut command = CommandState::default();
        let alignment = AlignmentIdentity::new(1);

        assert_eq!(
            request(&mut command, AlignmentRequest::Begin(alignment)),
            Ok(AlignmentRequestResult::Applied)
        );
        assert_eq!(
            request(&mut command, AlignmentRequest::Finish(alignment)),
            Ok(AlignmentRequestResult::Applied)
        );
    }
}
