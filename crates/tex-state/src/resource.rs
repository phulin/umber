//! Host-resource lookup outcomes shared across engine layers.

use std::path::Path;

use crate::world::{InputDependencyAccess, InputDependencyOutcome, World, WorldError};
use crate::{FileContent, SharedBytes, Universe};

/// Narrow mutable capability exposed to driver-owned input resolvers.
pub trait InputReadState {
    fn read_input_file(&mut self, path: &Path) -> Result<FileContent, WorldError>;

    fn read_pending_output_file(&mut self, path: &Path) -> Result<Option<FileContent>, WorldError>;

    fn read_supplied_input_file(
        &mut self,
        path: &Path,
        bytes: SharedBytes,
    ) -> Result<FileContent, WorldError>;

    fn record_input_dependency(
        &mut self,
        path: &Path,
        outcome: InputDependencyOutcome,
        access: InputDependencyAccess,
    ) -> Result<(), WorldError>;
}

/// Borrowed input-only view of the ambient world.
pub struct InputOpenContext<'a> {
    world: &'a mut World,
}

impl InputReadState for InputOpenContext<'_> {
    fn read_input_file(&mut self, path: &Path) -> Result<FileContent, WorldError> {
        self.world.read_file(path)
    }

    fn read_pending_output_file(&mut self, path: &Path) -> Result<Option<FileContent>, WorldError> {
        self.world.read_pending_output_file(path)
    }

    fn read_supplied_input_file(
        &mut self,
        path: &Path,
        bytes: SharedBytes,
    ) -> Result<FileContent, WorldError> {
        self.world.read_supplied_file(path, bytes)
    }

    fn record_input_dependency(
        &mut self,
        path: &Path,
        outcome: InputDependencyOutcome,
        access: InputDependencyAccess,
    ) -> Result<(), WorldError> {
        self.world
            .record_input_dependency(path.to_owned(), outcome, access)
    }
}

/// Aggregate capability used by retained resource sessions.
pub trait InputOpenState {
    type Input<'a>: InputReadState
    where
        Self: 'a;

    fn input_open_context(&mut self) -> Self::Input<'_>;
}

impl<G> InputOpenState for Universe<G> {
    type Input<'a>
        = InputOpenContext<'a>
    where
        Self: 'a;

    fn input_open_context(&mut self) -> Self::Input<'_> {
        InputOpenContext {
            world: self.world_mut(),
        }
    }
}

impl<G> Universe<G> {
    pub fn input_open_context(&mut self) -> InputOpenContext<'_> {
        InputOpenState::input_open_context(self)
    }
}

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

/// Object-safe host boundary for legacy expansion-time input acquisition.
///
/// Resolvers return immutable state-owned content. Input-stack construction
/// remains private to the command-delivery implementation that consumes it.
pub trait InputResolver {
    fn open_input(
        &mut self,
        input: &mut dyn InputReadState,
        name: &str,
        request_index: u64,
    ) -> ResourceResult<FileContent>;

    fn input_file_size(
        &mut self,
        input: &mut dyn InputReadState,
        name: &str,
        request_index: u64,
    ) -> ResourceResult<u64> {
        self.open_input(input, name, request_index)
            .map(|lookup| lookup.map(|content| content.bytes().len() as u64))
    }

    fn input_file_content(
        &mut self,
        input: &mut dyn InputReadState,
        name: &str,
        request_index: u64,
    ) -> ResourceResult<FileContent> {
        self.open_stream_input(input, name, request_index)
    }

    fn open_stream_input(
        &mut self,
        input: &mut dyn InputReadState,
        name: &str,
        _request_index: u64,
    ) -> ResourceResult<FileContent> {
        Ok(match input.read_input_file(Path::new(name)) {
            Ok(content) => ResourceLookup::Available(content),
            Err(_) => ResourceLookup::Unavailable,
        })
    }
}

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
