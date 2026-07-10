//! Rollback-coupled logical source positions and immutable source backings.

use std::sync::Arc;

use crate::input::SourceId;
use crate::world::{ContentHash, InputRecordId};

/// An opaque position in the current timeline's logical source space.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SourcePos(u64);

/// A validated half-open range within one live source region.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SourceSpan {
    lo: SourcePos,
    hi: SourcePos,
}

#[cfg(test)]
mod tests;

impl SourceSpan {
    pub(crate) const fn new(lo: SourcePos, hi: SourcePos) -> Self {
        Self { lo, hi }
    }

    #[must_use]
    pub const fn lo(self) -> SourcePos {
        self.lo
    }

    #[must_use]
    pub const fn hi(self) -> SourcePos {
        self.hi
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.lo.0 == self.hi.0
    }
}

/// Shared immutable content for a generated or in-memory input.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GeneratedSource {
    bytes: Arc<[u8]>,
    hash: ContentHash,
}

impl GeneratedSource {
    #[must_use]
    pub fn new(bytes: Arc<[u8]>) -> Self {
        let hash = ContentHash::from_bytes(&bytes);
        Self { bytes, hash }
    }

    #[must_use]
    pub fn from_bytes(bytes: impl Into<Vec<u8>>) -> Self {
        Self::new(Arc::from(bytes.into()))
    }

    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    #[must_use]
    pub const fn hash(&self) -> ContentHash {
        self.hash
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
}

/// Immutable descriptor supplied by an input adapter during registration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SourceDescriptor {
    World {
        input_record: InputRecordId,
        byte_len: u64,
    },
    Generated(GeneratedSource),
}

impl SourceDescriptor {
    #[must_use]
    pub const fn world(input_record: InputRecordId, byte_len: u64) -> Self {
        Self::World {
            input_record,
            byte_len,
        }
    }

    #[must_use]
    pub fn generated(bytes: Arc<[u8]>) -> Self {
        Self::Generated(GeneratedSource::new(bytes))
    }

    #[must_use]
    pub fn byte_len(&self) -> u64 {
        match self {
            Self::World { byte_len, .. } => *byte_len,
            Self::Generated(source) => u64::try_from(source.len()).unwrap_or(u64::MAX),
        }
    }
}

/// A rejected source registration or span assignment.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourceMapError {
    LogicalPositionExhausted,
    ConflictingRegistration,
    MissingWorldInput,
    WorldInputLengthMismatch,
    UnknownSource,
    OffsetOutsideSource,
    SpanCrossesSource,
}

impl std::fmt::Display for SourceMapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Self::LogicalPositionExhausted => "logical source position space exhausted",
                Self::ConflictingRegistration => "source id was registered with different backing",
                Self::MissingWorldInput => "source references a non-live World input record",
                Self::WorldInputLengthMismatch =>
                    "source length does not match its World input record",
                Self::UnknownSource => "source id is not live",
                Self::OffsetOutsideSource => "source byte offset is outside its backing",
                Self::SpanCrossesSource => "source span crosses a source-region boundary",
            }
        )
    }
}

impl std::error::Error for SourceMapError {}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct GeneratedSourceId(u32);

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum SourceBacking {
    World(InputRecordId),
    Generated(GeneratedSourceId),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SourceRegion {
    pub(crate) start: SourcePos,
    pub(crate) byte_len: u64,
    pub(crate) source: SourceId,
    pub(crate) backing: SourceBacking,
}

impl SourceRegion {
    pub(crate) const fn anchor(self) -> SourcePos {
        SourcePos(self.start.0 + self.byte_len)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SourceMapMark {
    regions: usize,
    generated: usize,
    next_pos: u64,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct SourceMap {
    regions: Vec<SourceRegion>,
    generated: Vec<GeneratedSource>,
    next_pos: u64,
}

impl SourceMap {
    pub(crate) fn register(
        &mut self,
        source: SourceId,
        descriptor: SourceDescriptor,
    ) -> Result<SourcePos, SourceMapError> {
        if let Some(region) = self.region_for_source(source) {
            return self
                .descriptor_matches(region, &descriptor)
                .then_some(region.start)
                .ok_or(SourceMapError::ConflictingRegistration);
        }

        let byte_len = descriptor.byte_len();
        let next_pos = self
            .next_pos
            .checked_add(byte_len)
            .and_then(|anchor| anchor.checked_add(1))
            .ok_or(SourceMapError::LogicalPositionExhausted)?;
        let backing = match descriptor {
            SourceDescriptor::World { input_record, .. } => SourceBacking::World(input_record),
            SourceDescriptor::Generated(generated) => {
                let raw = u32::try_from(self.generated.len())
                    .map_err(|_| SourceMapError::LogicalPositionExhausted)?;
                self.generated.push(generated);
                SourceBacking::Generated(GeneratedSourceId(raw))
            }
        };
        let start = SourcePos(self.next_pos);
        self.regions.push(SourceRegion {
            start,
            byte_len,
            source,
            backing,
        });
        self.next_pos = next_pos;
        Ok(start)
    }

    fn descriptor_matches(&self, region: SourceRegion, descriptor: &SourceDescriptor) -> bool {
        if region.byte_len != descriptor.byte_len() {
            return false;
        }
        match (region.backing, descriptor) {
            (SourceBacking::World(old), SourceDescriptor::World { input_record, .. }) => {
                old == *input_record
            }
            (SourceBacking::Generated(id), SourceDescriptor::Generated(source)) => {
                self.generated(id).is_some_and(|old| old == source)
            }
            _ => false,
        }
    }

    pub(crate) fn position(
        &self,
        source: SourceId,
        byte_offset: u64,
    ) -> Result<SourcePos, SourceMapError> {
        let region = self
            .region_for_source(source)
            .ok_or(SourceMapError::UnknownSource)?;
        if byte_offset > region.byte_len {
            return Err(SourceMapError::OffsetOutsideSource);
        }
        Ok(SourcePos(region.start.0 + byte_offset))
    }

    pub(crate) fn span(&self, lo: SourcePos, hi: SourcePos) -> Result<SourceSpan, SourceMapError> {
        let region = self
            .region_for_position(lo)
            .ok_or(SourceMapError::UnknownSource)?;
        if hi.0 < lo.0 || hi.0 > region.anchor().0 {
            return Err(SourceMapError::SpanCrossesSource);
        }
        Ok(SourceSpan::new(lo, hi))
    }

    pub(crate) fn region_for_source(&self, source: SourceId) -> Option<SourceRegion> {
        self.regions
            .iter()
            .rev()
            .copied()
            .find(|region| region.source == source)
    }

    pub(crate) fn region_for_position(&self, position: SourcePos) -> Option<SourceRegion> {
        let index = self
            .regions
            .partition_point(|region| region.start.0 <= position.0)
            .checked_sub(1)?;
        let region = self.regions[index];
        (position.0 <= region.anchor().0).then_some(region)
    }

    pub(crate) fn generated(&self, id: GeneratedSourceId) -> Option<&GeneratedSource> {
        self.generated.get(id.0 as usize)
    }

    pub(crate) const fn watermark(&self) -> SourceMapMark {
        SourceMapMark {
            regions: self.regions.len(),
            generated: self.generated.len(),
            next_pos: self.next_pos,
        }
    }

    pub(crate) fn truncate_to(&mut self, mark: SourceMapMark) {
        assert!(mark.regions <= self.regions.len());
        assert!(mark.generated <= self.generated.len());
        assert!(mark.next_pos <= self.next_pos);
        self.regions.truncate(mark.regions);
        self.generated.truncate(mark.generated);
        self.next_pos = mark.next_pos;
    }
}
