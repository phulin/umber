//! Destination-local staging and atomic publication for detached formats.
//!
//! Section decoders first validate handle-free data. They then build a
//! complete destination-local value behind [`FormatStaging`] while borrowing
//! the destination immutably. Publication is the first mutable operation and
//! is deliberately infallible: every allocation, interning operation, local
//! index rewrite, and semantic check must have completed in staging.

use core::sync::atomic::{AtomicU64, Ordering};

#[cfg(test)]
#[path = "format/tests.rs"]
mod tests;

static NEXT_DESTINATION: AtomicU64 = AtomicU64::new(1);

/// Identity of one live destination into which detached format data may be
/// materialized.
///
/// The raw identity is private and is never serialized. It exists only to
/// prevent a successfully staged set of destination-local ids from being
/// published into a different session.
pub(crate) struct FormatDestination {
    identity: u64,
}

impl FormatDestination {
    #[must_use]
    pub(crate) fn new() -> Self {
        let identity = NEXT_DESTINATION
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |next| {
                next.checked_add(1)
            })
            .expect("format destination identity space exhausted");
        Self { identity }
    }

    /// Builds a complete unpublished value for this destination.
    ///
    /// The builder receives only a shared borrow of the destination marker, so
    /// rejection cannot alter the live destination through this API.
    pub(crate) fn stage<Value, Error>(
        &self,
        build: impl FnOnce() -> Result<Value, Error>,
    ) -> Result<FormatStaging<Value>, Error> {
        Ok(FormatStaging {
            destination: self.identity,
            value: build()?,
        })
    }

    /// Publishes one fully staged value with a single infallible mutation.
    pub(crate) fn publish<Value, Output>(
        &mut self,
        staging: FormatStaging<Value>,
        publish: impl FnOnce(Value) -> Output,
    ) -> Result<Output, FormatPublicationError> {
        if staging.destination != self.identity {
            return Err(FormatPublicationError::ForeignDestination);
        }
        Ok(publish(staging.value))
    }
}

impl Default for FormatDestination {
    fn default() -> Self {
        Self::new()
    }
}

/// Complete but unpublished destination-local format state.
///
/// The value has no cloning API. Publication moves it into the destination;
/// rejection drops the whole staging owner.
pub(crate) struct FormatStaging<Value> {
    destination: u64,
    value: Value,
}

/// Rejection at the final destination-identity barrier.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FormatPublicationError {
    ForeignDestination,
}
