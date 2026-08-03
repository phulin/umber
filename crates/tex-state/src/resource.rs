//! Host-resource lookup outcomes shared across engine layers.

/// A host resource lookup distinguishes authoritative absence from a request
/// which can be satisfied before replaying the current operation.
#[derive(Debug)]
pub enum ResourceLookup<T> {
    Available(T),
    Unavailable,
    NeedResource(ResourceNeed),
}

impl<T> ResourceLookup<T> {
    #[must_use]
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> ResourceLookup<U> {
        match self {
            Self::Available(value) => ResourceLookup::Available(f(value)),
            Self::Unavailable => ResourceLookup::Unavailable,
            Self::NeedResource(need) => ResourceLookup::NeedResource(need),
        }
    }
}

/// Stable identity of one resolver call within an execution attempt.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ResourceNeed {
    request_index: u64,
}

impl ResourceNeed {
    #[must_use]
    pub const fn new(request_index: u64) -> Self {
        Self { request_index }
    }

    #[must_use]
    pub const fn request_index(self) -> u64 {
        self.request_index
    }
}

/// Fatal resolver failures remain errors; normal absence and suspension are
/// represented by [`ResourceLookup`] rather than diagnostic strings.
pub type ResourceResult<T> = Result<ResourceLookup<T>, String>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_lookup_mapping_preserves_absence_and_suspension() {
        assert!(matches!(
            ResourceLookup::Available(7_u8).map(u16::from),
            ResourceLookup::Available(7_u16)
        ));
        assert!(matches!(
            ResourceLookup::<u8>::Unavailable.map(u16::from),
            ResourceLookup::Unavailable
        ));
        let need = ResourceNeed::new(19);
        assert!(matches!(
            ResourceLookup::<u8>::NeedResource(need).map(u16::from),
            ResourceLookup::NeedResource(found) if found == need
        ));
    }
}
