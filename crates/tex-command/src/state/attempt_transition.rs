//! Synchronous scanner scopes and halted checkpoint abandonment.

use super::CommandState;

impl<G> CommandState<G> {
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

    /// Drops halted command scratch at the aggregate checkpoint boundary.
    /// The executor invokes this before destructive backing restore; command
    /// installation also enforces the empty scratch shape.
    ///
    /// No command is ever rolled back in place: a failed or suspended
    /// command halts, and checkpoint replay is the one coarse transaction
    /// that restores the aggregate command cursor.
    #[doc(hidden)]
    pub fn abandon_checkpoint_attempt(&mut self) {
        // Captures are taken only at command boundaries, where the attempt
        // arena is clear. Clear it and replace the scratch owner so no stale
        // row or scanner builder can make the restored boundary appear
        // active.
        self.scratch = crate::execution_scratch::ExecutionScratch::default();
        self.attempt.clear();
    }
}
