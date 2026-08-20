//! Destination-stamped staging and atomic continuation publication.

use core::sync::atomic::{AtomicU64, Ordering};

use super::schema::ContinuationSchema;
use super::{CommandContinuationError, CommandContinuationLimits, OwnedCommandContinuation};

static NEXT_DESTINATION: AtomicU64 = AtomicU64::new(1);

/// Read-only evidence that the complete detached schema passed validation.
pub(crate) struct ValidatedCommandContinuation<'a> {
    schema: &'a ContinuationSchema,
}

impl<'a> ValidatedCommandContinuation<'a> {
    pub(super) const fn new(schema: &'a ContinuationSchema) -> Self {
        Self { schema }
    }

    #[must_use]
    pub(crate) const fn schema(&self) -> &'a ContinuationSchema {
        self.schema
    }
}

/// One live destination and the private identity of its admission domain.
///
/// Staging receives only `&Live`, so ordinary safe builders cannot mutate the
/// destination. Publication checks the identity and then performs one
/// infallible move into `&mut Live`.
pub(crate) struct CommandContinuationDestination<Live> {
    identity: u64,
    live: Live,
}

impl<Live> CommandContinuationDestination<Live> {
    #[must_use]
    pub(crate) fn new(live: Live) -> Self {
        let identity = NEXT_DESTINATION
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |next| {
                next.checked_add(1)
            })
            .expect("command continuation destination identity space exhausted");
        Self { identity, live }
    }

    #[must_use]
    pub(crate) const fn live(&self) -> &Live {
        &self.live
    }

    pub(crate) fn stage<Value, Error>(
        &self,
        continuation: &OwnedCommandContinuation,
        limits: CommandContinuationLimits,
        build: impl FnOnce(&Live, ValidatedCommandContinuation<'_>) -> Result<Value, Error>,
    ) -> Result<StagedCommandContinuation<Value>, MaterializationError<Error>> {
        continuation
            .validate(limits)
            .map_err(MaterializationError::Continuation)?;
        let value = build(
            &self.live,
            ValidatedCommandContinuation::new(&continuation.schema),
        )
        .map_err(MaterializationError::Build)?;
        Ok(StagedCommandContinuation {
            destination: self.identity,
            value,
        })
    }

    pub(crate) fn publish<Value, Output>(
        &mut self,
        staging: StagedCommandContinuation<Value>,
        publish: impl FnOnce(&mut Live, Value) -> Output,
    ) -> Result<Output, CommandContinuationError> {
        if staging.destination != self.identity {
            return Err(CommandContinuationError::ForeignDestination);
        }
        Ok(publish(&mut self.live, staging.value))
    }

    #[must_use]
    pub(crate) fn into_live(self) -> Live {
        self.live
    }
}

/// A complete unpublished destination-local rebuild.
///
/// The value is move-only. It cannot be cloned into two live destinations.
pub(crate) struct StagedCommandContinuation<Value> {
    destination: u64,
    value: Value,
}

/// Validation/staging failures happen before publication begins.
#[derive(Debug, Eq, PartialEq)]
pub(crate) enum MaterializationError<Error> {
    Continuation(CommandContinuationError),
    Build(Error),
}
