use super::*;

impl<G, Facts: crate::CommandHostFacts<G>> CommandProcessor<'_, '_, G, Facts> {
    /// Delivers one command from TeX82 §1270's `do_assignments` fetch.
    ///
    /// The fetch itself is §404's "next non-blank non-relax non-call token".
    /// Callers must dispatch the returned command in place: `do_assignments`
    /// neither backs up assignments nor refetches the first non-assignment it
    /// stops on. This boundary is also used by §1206 after `fin_align`, where
    /// blanks before the display-closing command must not reach main control.
    pub fn next_do_assignments_command(
        &mut self,
    ) -> Result<Option<CurrentCommand<G>>, CommandError> {
        let mut destination = None;
        match self.next_non_blank_non_relax_x_token_into(&mut destination)? {
            DeliveryStatus::End => Ok(None),
            DeliveryStatus::Command => Ok(destination),
            _ => Err(CommandError::input_invariant()),
        }
    }
}
