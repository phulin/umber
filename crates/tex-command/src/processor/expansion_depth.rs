//! Web2C's shared recursive expansion capacity (tex.ch §366, etex.ch [53a]).

use super::CommandProcessor;
use crate::{CommandError, CommandObservation, FatalError};

pub(crate) const EXPANSION_DEPTH_LIMIT: u32 = 10_000;

impl<G> CommandProcessor<'_, '_, G> {
    /// Count semantic expansion calls, never the scalar delivery back edge.
    /// The saved depth is restored on every ordinary Rust return, including
    /// a fatal child or the resource error that triggers checkpoint replay.
    #[inline(always)]
    pub(crate) fn with_expansion_depth<T>(
        &mut self,
        operation: impl FnOnce(&mut Self) -> Result<T, CommandError>,
    ) -> Result<T, CommandError> {
        let depth = self.expansion_depth;
        if depth >= EXPANSION_DEPTH_LIMIT - 1 {
            return Err(self.expansion_depth_overflow());
        }
        self.expansion_depth = depth + 1;
        let result = operation(self);
        self.expansion_depth = depth;
        result
    }

    #[cold]
    #[inline(never)]
    fn expansion_depth_overflow(&mut self) -> CommandError {
        let fatal = FatalError::overflow(
            "expansion depth",
            i32::try_from(EXPANSION_DEPTH_LIMIT).expect("expansion depth fits i32"),
        );
        self.observe(CommandObservation::Diagnostic(fatal.record()));
        CommandError::Fatal(fatal)
    }
}
