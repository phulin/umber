//! Bounded checkpoint marks and prevalidated aggregate restoration.
//!
//! This module owns ordering, not subsystem policy. A checkpoint moves one
//! coarse owner beside a copy-only tuple of subsystem cursors. Preparation is
//! read-only. Once preparation succeeds, the target's infallible phase methods
//! run in the only order which can keep both restored and abandoned
//! coordinates live: acquire owner, restore dense state, transfer roots,
//! truncate suffixes, then release replaced owners.

#[cfg(test)]
#[path = "checkpoint/tests.rs"]
mod tests;

/// Fixed-size state-layer portion of an aggregate operation mark.
///
/// The component types are owner-checked cursors supplied by their respective
/// stores. No arena payload, root collection, or individual value owner can be
/// placed in this mark through this API.
pub struct BoundedStateMark<Journal: Copy, Durable: Copy, Page: Copy, Input: Copy> {
    journal: Journal,
    durable: Durable,
    page: Page,
    input: Input,
}

impl<Journal: Copy, Durable: Copy, Page: Copy, Input: Copy> Clone
    for BoundedStateMark<Journal, Durable, Page, Input>
{
    fn clone(&self) -> Self {
        *self
    }
}

impl<Journal: Copy, Durable: Copy, Page: Copy, Input: Copy> Copy
    for BoundedStateMark<Journal, Durable, Page, Input>
{
}

impl<Journal: Copy, Durable: Copy, Page: Copy, Input: Copy>
    BoundedStateMark<Journal, Durable, Page, Input>
{
    #[must_use]
    pub(crate) const fn new(journal: Journal, durable: Durable, page: Page, input: Input) -> Self {
        Self {
            journal,
            durable,
            page,
            input,
        }
    }

    #[must_use]
    pub(crate) const fn journal(&self) -> &Journal {
        &self.journal
    }

    #[must_use]
    pub(crate) const fn durable(&self) -> &Durable {
        &self.durable
    }

    #[must_use]
    pub(crate) const fn page(&self) -> &Page {
        &self.page
    }

    #[must_use]
    pub(crate) const fn input(&self) -> &Input {
        &self.input
    }
}

/// One retained checkpoint: a coarse generation owner plus bounded cursors.
///
/// `Owner` is intentionally singular. Callers cannot attach owners to the
/// individual values reachable through `Mark`.
pub struct GenerationCheckpoint<Owner, Mark: Copy> {
    owner: Owner,
    mark: Mark,
}

impl<Owner: Clone, Mark: Copy> Clone for GenerationCheckpoint<Owner, Mark> {
    fn clone(&self) -> Self {
        Self {
            owner: self.owner.clone(),
            mark: self.mark,
        }
    }
}

impl<Owner, Mark: Copy> GenerationCheckpoint<Owner, Mark> {
    #[must_use]
    pub(crate) const fn new(owner: Owner, mark: Mark) -> Self {
        Self { owner, mark }
    }

    #[must_use]
    pub(crate) const fn owner(&self) -> &Owner {
        &self.owner
    }

    #[must_use]
    pub(crate) const fn mark(&self) -> &Mark {
        &self.mark
    }
}

/// Aggregate restore phases implemented by the owner of the live timeline.
///
/// Only `validate_restore` can fail. Each later method must be infallible for a
/// successfully validated mark, which prevents a half-restored visible state.
pub(crate) trait RestoreTarget<Owner, Mark: Copy> {
    type Error;
    type Output;

    fn validate_restore(&self, owner: &Owner, mark: &Mark) -> Result<(), Self::Error>;
    fn acquire_target_owner(&mut self, owner: Owner);
    fn restore_dense_state(&mut self, mark: &Mark);
    fn transfer_roots(&mut self, mark: &Mark);
    fn truncate_suffixes(&mut self, mark: &Mark);
    fn release_replaced_owners(&mut self) -> Self::Output;
}

/// Evidence that every coordinate in one checkpoint has been validated
/// against the still-unmodified destination.
pub(crate) struct RestorePlan<Owner, Mark: Copy> {
    checkpoint: GenerationCheckpoint<Owner, Mark>,
}

impl<Owner, Mark: Copy> RestorePlan<Owner, Mark> {
    /// Applies the already validated plan in the normative restore order.
    pub(crate) fn apply<Target>(self, target: &mut Target) -> Target::Output
    where
        Target: RestoreTarget<Owner, Mark>,
    {
        let GenerationCheckpoint { owner, mark } = self.checkpoint;
        target.acquire_target_owner(owner);
        target.restore_dense_state(&mark);
        target.transfer_roots(&mark);
        target.truncate_suffixes(&mark);
        target.release_replaced_owners()
    }
}

/// Validates a complete checkpoint without mutating its destination.
pub(crate) fn prepare_restore<Owner, Mark: Copy, Target>(
    target: &Target,
    checkpoint: GenerationCheckpoint<Owner, Mark>,
) -> Result<RestorePlan<Owner, Mark>, Target::Error>
where
    Target: RestoreTarget<Owner, Mark>,
{
    target.validate_restore(checkpoint.owner(), checkpoint.mark())?;
    Ok(RestorePlan { checkpoint })
}
