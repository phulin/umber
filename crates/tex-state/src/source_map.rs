//! Rollback-coupled logical source positions and immutable source backings.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use ahash::AHashMap;

use crate::identity::{HandleIdentity, IdentityAllocator, IdentityMark};
use crate::input::SourceId;
use crate::token::OriginId;
use crate::world::{ContentHash, InputRecordId};

static NEXT_LOGICAL_SOURCE_POSITION: AtomicU64 = AtomicU64::new(0);

/// Shared high-water allocator for every logical source-coordinate range.
///
/// The process-wide counter is deliberately not part of rollback state: a
/// discarded timeline permanently consumes its range, so no surviving fork
/// can reinterpret an old packed origin. Fragment and engine registrations
/// use this same allocator.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct LogicalPositionAllocator;

impl LogicalPositionAllocator {
    pub(crate) fn reserve(self, byte_len: u64) -> Result<(u64, u64), SourceMapError> {
        NEXT_LOGICAL_SOURCE_POSITION
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |start| {
                start.checked_add(byte_len)?.checked_add(1)
            })
            .map(|start| (start, start + byte_len + 1))
            .map_err(|_| SourceMapError::LogicalPositionExhausted)
    }
}

/// An opaque position in the current timeline's logical source space.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SourcePos(u64);

/// Opaque capability for allocation-free origins within one registered input.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RegisteredSource {
    start: SourcePos,
    byte_len: u64,
}

impl RegisteredSource {
    pub(crate) const fn new(start: SourcePos, byte_len: u64) -> Self {
        Self { start, byte_len }
    }

    pub(crate) const fn start(self) -> SourcePos {
        self.start
    }

    pub(crate) const fn byte_len(self) -> u64 {
        self.byte_len
    }

    /// Encodes a nonempty backed scalar directly when it fits the packed form.
    #[must_use]
    #[inline(always)]
    pub fn direct_origin(self, byte_offset: u64, byte_end: u64) -> Option<OriginId> {
        if byte_offset >= byte_end || byte_end > self.byte_len {
            return None;
        }
        let raw = self.start.0.checked_add(byte_offset)?;
        OriginId::direct_source(SourcePos(raw))
    }

    /// Validates a half-open byte range against this registered input.
    pub fn span(self, byte_offset: u64, byte_end: u64) -> Result<SourceSpan, SourceMapError> {
        if byte_offset > byte_end || byte_end > self.byte_len {
            return Err(SourceMapError::OffsetOutsideSource);
        }
        let lo = self
            .start
            .0
            .checked_add(byte_offset)
            .ok_or(SourceMapError::LogicalPositionExhausted)?;
        let hi = self
            .start
            .0
            .checked_add(byte_end)
            .ok_or(SourceMapError::LogicalPositionExhausted)?;
        Ok(SourceSpan::new(SourcePos(lo), SourcePos(hi)))
    }
}

impl SourcePos {
    pub(crate) const fn from_origin_payload(raw: u32) -> Self {
        Self(raw as u64)
    }

    #[must_use]
    pub(crate) const fn raw(self) -> u64 {
        self.0
    }

    pub(crate) const fn from_raw_for_store(raw: u64) -> Self {
        Self(raw)
    }
}

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
    logical_path: Option<Arc<String>>,
}

impl GeneratedSource {
    #[must_use]
    pub fn new(bytes: Arc<[u8]>) -> Self {
        let hash = ContentHash::from_bytes(&bytes);
        Self {
            bytes,
            hash,
            logical_path: None,
        }
    }

    #[must_use]
    pub fn named(logical_path: impl Into<String>, bytes: Arc<[u8]>) -> Self {
        let mut source = Self::new(bytes);
        source.logical_path = Some(Arc::new(logical_path.into()));
        source
    }

    #[must_use]
    pub fn from_bytes(bytes: impl Into<Vec<u8>>) -> Self {
        Self::new(Arc::from(bytes.into()))
    }

    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub(crate) fn backing(&self) -> Arc<[u8]> {
        Arc::clone(&self.bytes)
    }

    #[must_use]
    pub const fn hash(&self) -> ContentHash {
        self.hash
    }

    #[must_use]
    pub fn logical_path(&self) -> Option<&str> {
        self.logical_path.as_deref().map(String::as_str)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    fn same_backing(&self, other: &Self) -> bool {
        self.hash == other.hash
            && self.logical_path == other.logical_path
            && (Arc::ptr_eq(&self.bytes, &other.bytes) || self.bytes == other.bytes)
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
    pub fn named_generated(logical_path: impl Into<String>, bytes: Arc<[u8]>) -> Self {
        Self::Generated(GeneratedSource::named(logical_path, bytes))
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
    identity: HandleIdentity,
}

/// Strong ownership of one immutable source registration.
///
/// Logical positions remain compact, non-owning values. A diagnostic consumer
/// that can outlive the aggregate source-map row retains this handle instead.
#[derive(Clone, Debug)]
pub struct SourceRegistrationRef(Arc<SourceRegistrationValue>);

#[derive(Debug)]
struct SourceRegistrationValue {
    region: SourceRegion,
    #[allow(dead_code)]
    descriptor: SourceDescriptor,
    #[allow(dead_code)]
    line_starts: Arc<[usize]>,
}

impl SourceRegistrationRef {
    #[must_use]
    pub(crate) fn region(&self) -> SourceRegion {
        self.0.region
    }

    pub(crate) fn descriptor(&self) -> &SourceDescriptor {
        &self.0.descriptor
    }

    pub(crate) fn line_starts(&self) -> &[usize] {
        &self.0.line_starts
    }
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
    identities: IdentityMark,
}

#[derive(Debug)]
pub(crate) struct SourceMap {
    regions: Vec<SourceRegion>,
    registration_roots: Vec<SourceRegistrationRef>,
    region_by_source: AHashMap<SourceId, usize>,
    line_starts: Vec<Arc<[usize]>>,
    generated: Vec<GeneratedSource>,
    next_pos: u64,
    forced_next_pos: bool,
    identities: IdentityAllocator,
}

impl Default for SourceMap {
    fn default() -> Self {
        Self {
            regions: Vec::new(),
            registration_roots: Vec::new(),
            region_by_source: AHashMap::new(),
            line_starts: Vec::new(),
            generated: Vec::new(),
            next_pos: 0,
            forced_next_pos: false,
            identities: IdentityAllocator::new(0),
        }
    }
}

impl Clone for SourceMap {
    fn clone(&self) -> Self {
        Self {
            regions: self.regions.clone(),
            registration_roots: self.registration_roots.clone(),
            region_by_source: self.region_by_source.clone(),
            line_starts: self.line_starts.clone(),
            generated: self.generated.clone(),
            next_pos: self.next_pos,
            forced_next_pos: self.forced_next_pos,
            identities: self.identities.fork(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct SourceMapStats {
    pub(crate) regions: usize,
    pub(crate) generated_backings: usize,
    pub(crate) live_bytes: usize,
    pub(crate) retained_bytes: usize,
}

impl SourceMap {
    #[cfg(test)]
    pub(crate) fn set_next_position_for_test(&mut self, next_pos: u64) {
        assert!(self.regions.is_empty());
        self.next_pos = next_pos;
        self.forced_next_pos = true;
    }

    #[cfg(test)]
    pub(crate) fn register(
        &mut self,
        source: SourceId,
        descriptor: SourceDescriptor,
    ) -> Result<SourcePos, SourceMapError> {
        let SourceDescriptor::Generated(generated) = &descriptor else {
            panic!("source-map unit tests register generated sources")
        };
        let mut starts = vec![0];
        starts.extend(
            generated
                .bytes()
                .iter()
                .enumerate()
                .filter(|(_, byte)| **byte == b'\n')
                .map(|(index, _)| index + 1),
        );
        self.register_with_line_starts(source, descriptor, starts.into())
    }

    pub(crate) fn register_with_line_starts(
        &mut self,
        source: SourceId,
        descriptor: SourceDescriptor,
        line_starts: Arc<[usize]>,
    ) -> Result<SourcePos, SourceMapError> {
        if let Some(position) = self.existing_registration(source, &descriptor)? {
            return Ok(position);
        }

        let byte_len = descriptor.byte_len();
        let owned_descriptor = descriptor.clone();
        let (start, next_pos) = self.reserve_positions(byte_len)?;
        let backing = match descriptor {
            SourceDescriptor::World { input_record, .. } => SourceBacking::World(input_record),
            SourceDescriptor::Generated(generated) => {
                let raw = u32::try_from(self.generated.len())
                    .map_err(|_| SourceMapError::LogicalPositionExhausted)?;
                self.generated.push(generated);
                SourceBacking::Generated(GeneratedSourceId(raw))
            }
        };
        let identity = self
            .identities
            .allocate()
            .map_err(|_| SourceMapError::LogicalPositionExhausted)?;
        let region_index = self.regions.len();
        self.regions.push(SourceRegion {
            start: SourcePos(start),
            byte_len,
            source,
            backing,
            identity,
        });
        self.registration_roots
            .push(SourceRegistrationRef(Arc::new(SourceRegistrationValue {
                region: self.regions[region_index],
                descriptor: owned_descriptor,
                line_starts: Arc::clone(&line_starts),
            })));
        assert_eq!(
            self.region_by_source.insert(source, region_index),
            None,
            "live source registration is unique"
        );
        self.line_starts.push(line_starts);
        self.next_pos = next_pos;
        Ok(SourcePos(start))
    }

    /// Resolves an already-live registration before callers build derived
    /// indexes for an immutable backing.
    pub(crate) fn existing_registration(
        &self,
        source: SourceId,
        descriptor: &SourceDescriptor,
    ) -> Result<Option<SourcePos>, SourceMapError> {
        let Some(region) = self.region_for_source(source) else {
            return Ok(None);
        };
        self.descriptor_matches(region, descriptor)
            .then_some(Some(region.start))
            .ok_or(SourceMapError::ConflictingRegistration)
    }

    fn reserve_positions(&mut self, byte_len: u64) -> Result<(u64, u64), SourceMapError> {
        if self.forced_next_pos {
            let start = self.next_pos;
            let next = start
                .checked_add(byte_len)
                .and_then(|anchor| anchor.checked_add(1))
                .ok_or(SourceMapError::LogicalPositionExhausted)?;
            return Ok((start, next));
        }
        LogicalPositionAllocator.reserve(byte_len)
    }

    fn descriptor_matches(&self, region: SourceRegion, descriptor: &SourceDescriptor) -> bool {
        if region.byte_len != descriptor.byte_len() {
            return false;
        }
        match (region.backing, descriptor) {
            (SourceBacking::World(old), SourceDescriptor::World { input_record, .. }) => {
                old == *input_record
            }
            (SourceBacking::Generated(id), SourceDescriptor::Generated(source)) => self
                .generated(id)
                .is_some_and(|old| old.same_backing(source)),
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

    pub(crate) fn span_for_source_offsets(
        &self,
        source: SourceId,
        lo: u64,
        hi: u64,
    ) -> Result<SourceSpan, SourceMapError> {
        let region = self
            .region_for_source(source)
            .ok_or(SourceMapError::UnknownSource)?;
        if lo > hi || hi > region.byte_len {
            return Err(SourceMapError::OffsetOutsideSource);
        }
        Ok(SourceSpan::new(
            SourcePos(region.start.0 + lo),
            SourcePos(region.start.0 + hi),
        ))
    }

    pub(crate) fn region_for_source(&self, source: SourceId) -> Option<SourceRegion> {
        let region = self
            .region_by_source
            .get(&source)
            .and_then(|index| self.regions.get(*index))
            .copied()?;
        (region.source == source && self.identities.contains(region.identity)).then_some(region)
    }

    pub(crate) fn registration_for_span(&self, span: SourceSpan) -> Option<SourceRegistrationRef> {
        let region = self.region_for_position(span.lo())?;
        if span.hi().0 > region.anchor().0 {
            return None;
        }
        self.registration_roots
            .get(region.identity.slot() as usize)
            .filter(|registration| registration.region() == region)
            .cloned()
    }

    pub(crate) fn registration_for_position(
        &self,
        position: SourcePos,
    ) -> Option<SourceRegistrationRef> {
        let region = self.region_for_position(position)?;
        self.registration_roots
            .get(region.identity.slot() as usize)
            .filter(|registration| registration.region() == region)
            .cloned()
    }

    pub(crate) fn registration_for_source(
        &self,
        source: SourceId,
    ) -> Option<SourceRegistrationRef> {
        let region = self.region_for_source(source)?;
        self.registration_roots
            .get(region.identity.slot() as usize)
            .filter(|registration| registration.region() == region)
            .cloned()
    }

    pub(crate) fn registered_source(&self, source: SourceId) -> Option<RegisteredSource> {
        let region = self.region_for_source(source)?;
        Some(RegisteredSource::new(region.start, region.byte_len))
    }

    pub(crate) fn descriptor_for_source(&self, source: SourceId) -> Option<SourceDescriptor> {
        let region = self.region_for_source(source)?;
        self.registration_roots
            .get(region.identity.slot() as usize)
            .filter(|registration| registration.region() == region)
            .map(|registration| registration.0.descriptor.clone())
    }

    pub(crate) fn contains_registration(
        &self,
        source: SourceId,
        registration: RegisteredSource,
    ) -> bool {
        self.region_for_source(source).is_some_and(|region| {
            region.start == registration.start && region.byte_len == registration.byte_len
        })
    }

    pub(crate) fn region_for_position(&self, position: SourcePos) -> Option<SourceRegion> {
        let index = self
            .regions
            .partition_point(|region| region.start.0 <= position.0)
            .checked_sub(1)?;
        let region = self.regions[index];
        (position.0 <= region.anchor().0 && self.identities.contains(region.identity))
            .then_some(region)
    }

    pub(crate) fn region_for_backed_position(&self, position: SourcePos) -> Option<SourceRegion> {
        self.region_for_position(position)
            .filter(|region| position.0 < region.anchor().0)
    }

    pub(crate) fn generated(&self, id: GeneratedSourceId) -> Option<&GeneratedSource> {
        self.generated.get(id.0 as usize)
    }

    pub(crate) fn line_starts(&self, region: SourceRegion) -> Option<&[usize]> {
        self.identities
            .contains(region.identity)
            .then(|| self.line_starts.get(region.identity.slot() as usize))
            .flatten()
            .map(AsRef::as_ref)
    }

    pub(crate) fn stats(&self) -> SourceMapStats {
        SourceMapStats {
            regions: self.regions.len(),
            generated_backings: self.generated.len(),
            live_bytes: self.regions.len() * std::mem::size_of::<SourceRegion>()
                + self.region_by_source.len() * std::mem::size_of::<(SourceId, usize)>()
                + self.generated.len() * std::mem::size_of::<GeneratedSource>()
                + self
                    .line_starts
                    .iter()
                    .map(|starts| starts.len() * std::mem::size_of::<usize>())
                    .sum::<usize>(),
            retained_bytes: self.regions.capacity() * std::mem::size_of::<SourceRegion>()
                + self.region_by_source.capacity() * std::mem::size_of::<(SourceId, usize)>()
                + self.generated.capacity() * std::mem::size_of::<GeneratedSource>()
                + self
                    .generated
                    .iter()
                    .map(GeneratedSource::len)
                    .sum::<usize>()
                + self
                    .generated
                    .iter()
                    .filter_map(GeneratedSource::logical_path)
                    .map(str::len)
                    .sum::<usize>()
                + self.line_starts.capacity() * std::mem::size_of::<Arc<[usize]>>()
                + self
                    .line_starts
                    .iter()
                    .map(|starts| starts.len() * std::mem::size_of::<usize>())
                    .sum::<usize>(),
        }
    }

    pub(crate) fn watermark(&self) -> SourceMapMark {
        SourceMapMark {
            regions: self.regions.len(),
            generated: self.generated.len(),
            next_pos: self.next_pos,
            identities: self.identities.watermark(),
        }
    }

    pub(crate) fn validates(&self, mark: SourceMapMark) -> bool {
        mark.regions <= self.regions.len()
            && mark.generated <= self.generated.len()
            && (mark.next_pos <= self.next_pos || !self.forced_next_pos)
            && self.identities.validate_rollback(mark.identities).is_ok()
    }

    pub(crate) fn truncate_to(&mut self, mark: SourceMapMark) {
        self.truncate_to_inner(mark);
    }

    fn truncate_to_inner(&mut self, mark: SourceMapMark) {
        assert!(mark.regions <= self.regions.len());
        assert!(mark.generated <= self.generated.len());
        assert!(mark.next_pos <= self.next_pos || !self.forced_next_pos);
        self.identities
            .rollback(mark.identities)
            .expect("source-map mark is not an ancestor");
        for (index, region) in self.regions[mark.regions..].iter().enumerate() {
            assert_eq!(
                self.region_by_source.remove(&region.source),
                Some(mark.regions + index),
                "live source index matches rollback suffix"
            );
        }
        self.regions.truncate(mark.regions);
        self.registration_roots.truncate(mark.regions);
        self.line_starts.truncate(mark.regions);
        self.generated.truncate(mark.generated);
        if self.forced_next_pos {
            self.next_pos = mark.next_pos;
        }
    }
}
