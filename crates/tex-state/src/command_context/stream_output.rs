//! Immediate stream output writes through the actual admitted World.
use super::*;

impl<G> CommandContext<'_, G> {
    pub fn publish_immediate_write(
        &mut self,
        effects: &mut crate::diagnostic::DiagnosticEffects,
        sink: crate::PrintSink,
        text: &str,
        bytes: &[u8],
    ) {
        self.publish_diagnostic_effects_before_synchronous_print(effects);
        let max_line = if matches!(sink, crate::PrintSink::Stream(_)) {
            crate::print::MAX_PRINT_LINE
        } else {
            self.printer().max_print_line()
        };
        match (
            bytes == text.as_bytes(),
            matches!(sink, crate::PrintSink::Stream(_)),
        ) {
            (true, true) => self.resident.world.publish_print_text(sink, text, max_line),
            (true, false) => self
                .resident
                .world
                .publish_print_nl_text(sink, text, max_line),
            (false, true) => self
                .resident
                .world
                .publish_print_encoded_bytes(sink, bytes, max_line),
            (false, false) => self
                .resident
                .world
                .publish_print_nl_encoded_bytes(sink, bytes, max_line),
        }
    }

    pub fn publish_openout_notice(
        &mut self,
        effects: &mut crate::diagnostic::DiagnosticEffects,
        sink: crate::PrintSink,
        text: &str,
    ) {
        self.publish_diagnostic_effects_before_synchronous_print(effects);
        let max_line = self.printer().max_print_line();
        self.resident.world.publish_print_text(sink, text, max_line);
    }
}
