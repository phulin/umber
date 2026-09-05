//! Direct-operation scratch settlement and checkpoint-replay cleanup.

use super::CommandState;

impl<G> CommandState<G> {
    /// Captures every attempt-local table and subordinate builder cursor for
    /// an executor operation.
    pub fn begin_attempt_operation(&mut self) -> crate::CommandAttemptOperation {
        assert!(
            self.active_attempt_operation.is_none(),
            "direct command operations do not nest"
        );
        let mark = self
            .attempt
            .begin_operation(self.scratch.frame_len())
            .expect("command operation scope capacity is bounded");
        self.active_attempt_operation = Some(mark);
        crate::CommandAttemptOperation::new()
    }

    /// Opens one move-only synchronous child of the active direct operation.
    ///
    /// The child is attempt scratch only. Callers may consume its values while
    /// it is live, but must detach their final non-attempt result before
    /// [`Self::close_attempt_child_scope`] consumes the receipt. Semantic
    /// command mutations deliberately remain in the parent operation.
    pub fn begin_attempt_child_scope(
        &mut self,
    ) -> Result<crate::CommandAttemptChildScope, crate::AttemptError> {
        if self.active_attempt_operation.is_none() {
            return Err(crate::AttemptError::InvalidCoordinate);
        }
        let owner = self.attempt.begin_child_scope()?;
        Ok(crate::CommandAttemptChildScope::new(owner))
    }

    /// Consumes and closes exactly one synchronous LIFO child scope.
    pub fn close_attempt_child_scope(
        &mut self,
        scope: crate::CommandAttemptChildScope,
    ) -> Result<(), crate::AttemptError> {
        self.attempt.close_child_scope(scope.into_owner())
    }

    pub(crate) fn begin_attempt_scanner_scope(
        &mut self,
    ) -> Result<crate::attempt::OwnedAttemptScope, crate::AttemptError> {
        self.attempt.begin_child_scope()
    }

    pub(crate) fn defer_attempt_scope_retirement(
        &mut self,
        scope: crate::attempt::OwnedAttemptScope,
    ) -> Result<(), crate::AttemptError> {
        if self.active_attempt_operation.is_none() {
            return Err(crate::AttemptError::InvalidCoordinate);
        }
        self.attempt.validate_child_retirement(&scope)?;
        if self.attempt.child_scope_is_direct_operation_child(&scope) {
            self.attempt.defer_child_to_operation(scope)
        } else {
            self.attempt.close_child_scope(scope)
        }
    }

    pub(crate) fn validate_attempt_scope_retirement(
        &self,
        scope: &crate::attempt::OwnedAttemptScope,
    ) -> Result<(), crate::AttemptError> {
        if self.active_attempt_operation.is_none() {
            return Err(crate::AttemptError::InvalidCoordinate);
        }
        self.attempt.validate_child_retirement(scope)
    }

    pub(crate) fn discard_attempt_scope_suffix(
        &mut self,
        scope: crate::attempt::OwnedAttemptScope,
    ) -> Result<(), crate::AttemptError> {
        self.attempt.close_child_scope(scope)
    }

    /// Rejects the attempt-local suffix created by the active operation.
    ///
    /// Executor aggregate rollback restores semantic roots before invoking
    /// this method, so no surviving command coordinate can name the suffix.
    pub fn rollback_attempt_operation(
        &mut self,
        _operation: crate::CommandAttemptOperation,
    ) -> Result<(), crate::AttemptError> {
        let mark = self
            .active_attempt_operation
            .take()
            .ok_or(crate::AttemptError::InvalidCoordinate)?;
        let result = (|| {
            while self.scratch.frame_len() > mark.macro_depth() {
                let frame = self
                    .scratch
                    .active_argument_set()
                    .ok_or(crate::AttemptError::InvalidCoordinate)?;
                self.scratch
                    .release_argument_set(frame)
                    .map_err(|_| crate::AttemptError::InvalidCoordinate)?;
            }
            self.attempt.rollback_operation(mark)
        })();
        if result.is_err() {
            self.active_attempt_operation = Some(mark);
        }
        result
    }

    /// Drops an attempt owner after an aggregate checkpoint has already
    /// restored and truncated its command roots.
    ///
    /// Ordinary rollback must consume the linear operation capability through
    /// [`Self::rollback_attempt_operation`]. Checkpoint replay is the one
    /// coarse transaction that restores the aggregate command cursor first;
    /// its old operation coordinates are consequently no longer valid and
    /// must be discarded rather than replayed against the restored arena.
    #[doc(hidden)]
    pub fn abandon_attempt_after_checkpoint_restore(&mut self) {
        // Aggregate restore has already truncated every semantic root and
        // attempt mark. Replace the two command-side scratch owners so no
        // stale child scope or scanner builder can make the restored named
        // boundary appear active.
        self.scratch = crate::execution_scratch::ExecutionScratch::default();
        self.active_attempt_operation = None;
        self.attempt.abandon_operation();
    }

    /// Commits the exact direct-operation/scanner scope. Macro frames live in
    /// the disjoint generation-owned scratch lanes until input retirement.
    pub fn commit_attempt_operation(
        &mut self,
        _operation: crate::CommandAttemptOperation,
    ) -> Result<(), crate::AttemptError> {
        let mark = self
            .active_attempt_operation
            .take()
            .ok_or(crate::AttemptError::InvalidCoordinate)?;
        let result = self.attempt.commit_operation(mark);
        if result.is_err() {
            self.active_attempt_operation = Some(mark);
        }
        result
    }
}
