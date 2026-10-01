//! Command-boundary scratch settlement and checkpoint-replay cleanup.

use super::CommandState;

impl<G> CommandState<G> {
    /// Releases the attempt rows of the command that just finished.
    ///
    /// Attempt rows never outlive their command, so the arena is clear at
    /// every command boundary. A command that allocated nothing pays one
    /// test of the arena's root lengths.
    #[inline(always)]
    pub fn settle_attempt(&mut self) {
        self.attempt.settle();
    }

    /// Whether the running command holds no attempt rows.
    #[must_use]
    pub fn attempt_is_clear(&self) -> bool {
        self.attempt.is_clear()
    }

    /// Opens one move-only synchronous child of the running command.
    ///
    /// The child is attempt scratch only. Callers may consume its values while
    /// it is live, but must detach their final non-attempt result before
    /// [`Self::close_attempt_child_scope`] consumes it. Semantic command
    /// mutations deliberately remain with the command.
    pub fn begin_attempt_child_scope(&mut self) -> crate::CommandAttemptChildScope {
        self.attempt.begin_child_scope()
    }

    /// Consumes and closes exactly one synchronous LIFO child scope.
    pub fn close_attempt_child_scope(
        &mut self,
        scope: crate::CommandAttemptChildScope,
    ) -> Result<(), crate::AttemptError> {
        self.attempt.close_child_scope(scope)
    }

    /// Drops the attempt rows of a halted command after an aggregate
    /// checkpoint has already restored its command roots.
    ///
    /// No command is ever rolled back in place: a failed or suspended
    /// command halts, and checkpoint replay is the one coarse transaction
    /// that restores the aggregate command cursor.
    #[doc(hidden)]
    pub fn abandon_attempt_after_checkpoint_restore(&mut self) {
        // Captures are taken only at command boundaries, where the attempt
        // arena is clear. Clear it and replace the scratch owner so no stale
        // row or scanner builder can make the restored boundary appear
        // active.
        self.scratch = crate::execution_scratch::ExecutionScratch::default();
        self.attempt.clear();
    }
}
