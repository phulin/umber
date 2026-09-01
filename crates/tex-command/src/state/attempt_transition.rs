//! Direct-operation scratch settlement and resource handoff.

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

    /// Moves the complete operation arena into a resource continuation.
    pub fn suspend_attempt<R>(
        &mut self,
        universe: &tex_state::Universe<G>,
        operation: crate::CommandAttemptOperation,
        resume: crate::AttemptResumePoint,
        pending: R,
    ) -> Result<crate::PendingCommandAttempt<G, R>, crate::AttemptSuspendFailure> {
        let Some(opening) = self.active_attempt_operation else {
            return Err(crate::AttemptSuspendFailure::new(
                operation,
                crate::AttemptSuspendError::StaleMark(crate::AttemptError::InvalidCoordinate),
            ));
        };
        if let Err(error) = self.attempt.arena().validate_mark(opening.attempt_mark()) {
            return Err(crate::AttemptSuspendFailure::new(
                operation,
                crate::AttemptSuspendError::StaleMark(error),
            ));
        }
        if let Err(error) = self.attempt.validate_operation(opening) {
            return Err(crate::AttemptSuspendFailure::new(
                operation,
                crate::AttemptSuspendError::StaleMark(error),
            ));
        }
        let generation = match universe.generation_owner() {
            Ok(generation) => generation,
            Err(error) => {
                return Err(crate::AttemptSuspendFailure::new(
                    operation,
                    crate::AttemptSuspendError::Generation(error),
                ));
            }
        };
        let attempt = core::mem::take(&mut self.attempt);
        Ok(crate::PendingCommandAttempt::new_at_validated_mark(
            attempt, generation, opening, operation, resume, pending,
        ))
    }

    /// Reinstalls a returned arena after validating its coarse generation.
    #[allow(
        clippy::result_large_err,
        reason = "stale admission must return the complete move-only continuation without a lifecycle allocation"
    )]
    pub fn resume_attempt<R>(
        &mut self,
        universe: &tex_state::Universe<G>,
        pending: crate::PendingCommandAttempt<G, R>,
    ) -> Result<
        (crate::CommandAttemptOperation, crate::AttemptResumePoint, R),
        crate::PendingCommandAttempt<G, R>,
    > {
        if !self.attempt.is_empty()
            || self.active_attempt_operation != Some(pending.operation_coordinate())
        {
            return Err(pending);
        }
        let (attempt, operation, resume, pending) = pending.resume(universe)?;
        self.attempt = attempt;
        Ok((operation, resume, pending))
    }
}
