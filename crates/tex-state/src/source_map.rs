//! Rollback-coupled logical source positions and immutable source backings.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use ahash::AHashMap;

use crate::identity::{AcceptedIdentityTail, HandleIdentity, IdentityAllocator, IdentityMark};
use crate::input::SourceId;
use crate::state_hash::{SemanticSequenceIdentity, semantic_scalar_root};
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
    editor_revision: bool,
}

impl GeneratedSource {
    #[must_use]
    pub fn new(bytes: Arc<[u8]>) -> Self {
        let hash = ContentHash::from_bytes(&bytes);
        Self {
            bytes,
            hash,
            logical_path: None,
            editor_revision: false,
        }
    }

    #[must_use]
    pub fn named(logical_path: impl Into<String>, bytes: Arc<[u8]>) -> Self {
        let mut source = Self::new(bytes);
        source.logical_path = Some(Arc::new(logical_path.into()));
        source
    }

    fn editor_revision(logical_path: Option<&str>, bytes: Arc<[u8]>) -> Self {
        let mut source = logical_path.map_or_else(
            || Self::new(Arc::clone(&bytes)),
            |path| Self::named(path, Arc::clone(&bytes)),
        );
        source.editor_revision = true;
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

    #[cfg(test)]
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

    const fn contributes_reachable_state_identity(&self) -> bool {
        !self.editor_revision
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

    /// Describes an editor root substituted by the aggregate incremental
    /// revision-rebind operation. Its bytes and path remain available to
    /// provenance, but the registration itself is revision metadata rather
    /// than an additional future-semantic input.
    #[doc(hidden)]
    #[must_use]
    pub fn editor_revision(logical_path: Option<&str>, bytes: Arc<[u8]>) -> Self {
        Self::Generated(GeneratedSource::editor_revision(logical_path, bytes))
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
    reachable_state_identity: Option<SemanticSequenceIdentity>,
}

impl SourceMapMark {
    pub(crate) fn checkpoint_retained_bytes(self) -> usize {
        std::mem::size_of::<Self>()
            .saturating_add(
                self.regions
                    .saturating_mul(std::mem::size_of::<SourceRegion>()),
            )
            .saturating_add(
                self.generated
                    .saturating_mul(std::mem::size_of::<GeneratedSource>()),
            )
            .saturating_add(usize::try_from(self.next_pos).unwrap_or(usize::MAX))
    }
}

/// One immutable source-map prefix accepted at a revision boundary.
///
/// Rows and their coarse indexes are shared as one block. A candidate opens
/// empty vectors and maps for its suffix; source values never acquire
/// per-registration owners.
#[derive(Debug)]
struct AcceptedSourceBlock {
    parent: Option<Arc<Self>>,
    regions: Arc<Vec<SourceRegion>>,
    registration_roots: Arc<Vec<SourceRegistrationRef>>,
    region_by_source: Arc<AHashMap<SourceId, usize>>,
    generated: Arc<Vec<GeneratedSource>>,
    region_len: usize,
    generated_len: usize,
    total_regions: usize,
    total_generated: usize,
}

impl AcceptedSourceBlock {
    fn region_for_source(&self, source: SourceId) -> Option<SourceRegion> {
        self.region_by_source
            .get(&source)
            .and_then(|index| {
                let base = self.total_regions - self.region_len;
                (*index >= base)
                    .then(|| self.regions.get(*index - base))
                    .flatten()
            })
            .copied()
            .or_else(|| self.parent.as_ref()?.region_for_source(source))
    }

    fn region_for_position(&self, position: SourcePos) -> Option<SourceRegion> {
        let region = self.regions[..self.region_len]
            .iter()
            .rev()
            .find(|region| region.start.0 <= position.0)
            .copied();
        region.or_else(|| self.parent.as_ref()?.region_for_position(position))
    }

    fn registration(&self, index: usize) -> Option<&SourceRegistrationRef> {
        let base = self.total_regions - self.region_len;
        if index < base {
            return self.parent.as_ref()?.registration(index);
        }
        self.registration_roots
            .get(index - base)
            .filter(|_| index < self.total_regions)
    }

    fn generated(&self, index: usize) -> Option<&GeneratedSource> {
        let base = self.total_generated - self.generated_len;
        if index < base {
            return self.parent.as_ref()?.generated(index);
        }
        self.generated
            .get(index - base)
            .filter(|_| index < self.total_generated)
    }
}

#[derive(Debug)]
pub(crate) struct SourceMap {
    accepted: Option<Arc<AcceptedSourceBlock>>,
    regions: Arc<Vec<SourceRegion>>,
    registration_roots: Arc<Vec<SourceRegistrationRef>>,
    region_by_source: Arc<AHashMap<SourceId, usize>>,
    generated: Arc<Vec<GeneratedSource>>,
    next_pos: u64,
    forced_next_pos: bool,
    identities: IdentityAllocator,
    reachable_state_identity: Option<SemanticSequenceIdentity>,
}

/// Source rows detached from the accepted head while one candidate owns the
/// mutable map. The checkpoint itself retains only [`SourceMapMark`].
pub(crate) struct AcceptedSourceMapTail {
    regions: Vec<SourceRegion>,
    registration_roots: Vec<SourceRegistrationRef>,
    generated: Vec<GeneratedSource>,
    next_pos: u64,
    identities: AcceptedIdentityTail,
}

impl Default for SourceMap {
    fn default() -> Self {
        Self {
            accepted: None,
            regions: Arc::new(Vec::new()),
            registration_roots: Arc::new(Vec::new()),
            region_by_source: Arc::new(AHashMap::new()),
            generated: Arc::new(Vec::new()),
            next_pos: 0,
            forced_next_pos: false,
            identities: IdentityAllocator::new(0),
            reachable_state_identity: None,
        }
    }
}

impl Clone for SourceMap {
    fn clone(&self) -> Self {
        Self {
            accepted: self.accepted.clone(),
            regions: Arc::clone(&self.regions),
            registration_roots: Arc::clone(&self.registration_roots),
            region_by_source: Arc::clone(&self.region_by_source),
            generated: Arc::clone(&self.generated),
            next_pos: self.next_pos,
            forced_next_pos: self.forced_next_pos,
            identities: self.identities.fork(),
            reachable_state_identity: self.reachable_state_identity,
        }
    }
}

impl SourceMap {
    fn accepted_region_len(&self) -> usize {
        self.accepted
            .as_ref()
            .map_or(0, |block| block.total_regions)
    }

    fn accepted_generated_len(&self) -> usize {
        self.accepted
            .as_ref()
            .map_or(0, |block| block.total_generated)
    }

    fn region_len(&self) -> usize {
        self.accepted_region_len()
            .saturating_add(self.regions.len())
    }

    fn generated_len(&self) -> usize {
        self.accepted_generated_len()
            .saturating_add(self.generated.len())
    }

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

        let semantic_registration = self
            .reachable_state_identity
            .as_ref()
            .and_then(|_| source_descriptor_identity(&descriptor));

        let byte_len = descriptor.byte_len();
        let owned_descriptor = descriptor.clone();
        let (start, next_pos) = self.reserve_positions(byte_len)?;
        let backing = match descriptor {
            SourceDescriptor::World { input_record, .. } => SourceBacking::World(input_record),
            SourceDescriptor::Generated(generated) => {
                let raw = u32::try_from(self.generated_len())
                    .map_err(|_| SourceMapError::LogicalPositionExhausted)?;
                Arc::make_mut(&mut self.generated).push(generated);
                SourceBacking::Generated(GeneratedSourceId(raw))
            }
        };
        let identity = self
            .identities
            .allocate()
            .map_err(|_| SourceMapError::LogicalPositionExhausted)?;
        let region_index = self.region_len();
        Arc::make_mut(&mut self.regions).push(SourceRegion {
            start: SourcePos(start),
            byte_len,
            source,
            backing,
            identity,
        });
        Arc::make_mut(&mut self.registration_roots).push(SourceRegistrationRef(Arc::new(
            SourceRegistrationValue {
                region: *self.regions.last().expect("registered source row exists"),
                descriptor: owned_descriptor,
                line_starts: Arc::clone(&line_starts),
            },
        )));
        if let (Some(root), Some(registration)) =
            (&mut self.reachable_state_identity, semantic_registration)
        {
            root.push(registration);
        }
        assert_eq!(
            Arc::make_mut(&mut self.region_by_source).insert(source, region_index),
            None,
            "live source registration is unique"
        );
        self.next_pos = next_pos;
        Ok(SourcePos(start))
    }

    /// Registers one source whose caller does not retain a line index.
    ///
    /// Command input calls this only at physical acquisition or the cold
    /// source-retirement seam. The caller's committed bit prevents duplicate
    /// successful registration; the lookup here preserves idempotence at the
    /// aggregate boundary itself.
    pub(crate) fn register_without_line_starts(
        &mut self,
        source: SourceId,
        descriptor: SourceDescriptor,
    ) -> Result<SourcePos, SourceMapError> {
        if let Some(position) = self.existing_registration(source, &descriptor)? {
            return Ok(position);
        }
        self.register_with_line_starts(source, descriptor, Arc::from([0_usize]))
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

    pub(crate) fn span(&self, lo: SourcePos, hi: SourcePos) -> Result<SourceSpan, SourceMapError> {
        let region = self
            .region_for_position(lo)
            .ok_or(SourceMapError::UnknownSource)?;
        if hi.0 < lo.0 || hi.0 > region.anchor().0 {
            return Err(SourceMapError::SpanCrossesSource);
        }
        Ok(SourceSpan::new(lo, hi))
    }

    #[cfg(test)]
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

    pub(crate) fn region_for_source(&self, source: SourceId) -> Option<SourceRegion> {
        let base = self.accepted_region_len();
        let region = self
            .region_by_source
            .get(&source)
            .and_then(|index| {
                (*index >= base)
                    .then(|| self.regions.get(*index - base))
                    .flatten()
                    .copied()
            })
            .or_else(|| self.accepted.as_ref()?.region_for_source(source))?;
        (region.source == source && self.identities.contains(region.identity)).then_some(region)
    }

    pub(crate) fn registration_for_span(&self, span: SourceSpan) -> Option<SourceRegistrationRef> {
        let region = self.region_for_position(span.lo())?;
        if span.hi().0 > region.anchor().0 {
            return None;
        }
        self.registration(region.identity.slot() as usize)
            .filter(|registration| registration.region() == region)
            .cloned()
    }

    pub(crate) fn registration_for_source(
        &self,
        source: SourceId,
    ) -> Option<SourceRegistrationRef> {
        let region = self.region_for_source(source)?;
        self.registration(region.identity.slot() as usize)
            .filter(|registration| registration.region() == region)
            .cloned()
    }

    pub(crate) fn registered_source(&self, source: SourceId) -> Option<RegisteredSource> {
        let region = self.region_for_source(source)?;
        Some(RegisteredSource::new(region.start, region.byte_len))
    }

    pub(crate) fn region_for_position(&self, position: SourcePos) -> Option<SourceRegion> {
        let region = self
            .regions
            .partition_point(|region| region.start.0 <= position.0)
            .checked_sub(1)
            .map(|index| self.regions[index])
            .or_else(|| self.accepted.as_ref()?.region_for_position(position))?;
        (position.0 <= region.anchor().0 && self.identities.contains(region.identity))
            .then_some(region)
    }

    pub(crate) fn region_for_backed_position(&self, position: SourcePos) -> Option<SourceRegion> {
        self.region_for_position(position)
            .filter(|region| position.0 < region.anchor().0)
    }

    pub(crate) fn generated(&self, id: GeneratedSourceId) -> Option<&GeneratedSource> {
        let index = id.0 as usize;
        let base = self.accepted_generated_len();
        if index < base {
            return self.accepted.as_ref()?.generated(index);
        }
        self.generated.get(index - base)
    }

    #[cfg(test)]
    pub(crate) fn line_starts(&self, region: SourceRegion) -> Option<&[usize]> {
        self.identities
            .contains(region.identity)
            .then(|| self.line_starts_at(region.identity.slot() as usize))
            .flatten()
            .map(AsRef::as_ref)
    }

    pub(crate) fn watermark(&self) -> SourceMapMark {
        SourceMapMark {
            regions: self.region_len(),
            generated: self.generated_len(),
            next_pos: self.next_pos,
            identities: self.identities.watermark(),
            reachable_state_identity: self.reachable_state_identity,
        }
    }

    pub(crate) fn validates(&self, mark: SourceMapMark) -> bool {
        mark.regions >= self.accepted_region_len()
            && mark.regions <= self.region_len()
            && mark.generated >= self.accepted_generated_len()
            && mark.generated <= self.generated_len()
            && (mark.next_pos <= self.next_pos || !self.forced_next_pos)
            && self.identities.validate_rollback(mark.identities).is_ok()
    }

    pub(crate) fn truncate_to(&mut self, mark: SourceMapMark) {
        self.truncate_to_inner(mark);
    }

    fn truncate_to_inner(&mut self, mark: SourceMapMark) {
        let region_base = self.accepted_region_len();
        let generated_base = self.accepted_generated_len();
        assert!((region_base..=self.region_len()).contains(&mark.regions));
        assert!((generated_base..=self.generated_len()).contains(&mark.generated));
        assert!(mark.next_pos <= self.next_pos || !self.forced_next_pos);
        self.identities
            .rollback(mark.identities)
            .expect("source-map mark is not an ancestor");
        let local_regions = mark.regions - region_base;
        for (index, region) in self.regions[local_regions..].iter().enumerate() {
            assert_eq!(
                Arc::make_mut(&mut self.region_by_source).remove(&region.source),
                Some(region_base + local_regions + index),
                "live source index matches rollback suffix"
            );
        }
        Arc::make_mut(&mut self.regions).truncate(local_regions);
        Arc::make_mut(&mut self.registration_roots).truncate(local_regions);
        Arc::make_mut(&mut self.generated).truncate(mark.generated - generated_base);
        if self.forced_next_pos {
            self.next_pos = mark.next_pos;
        }
        self.reachable_state_identity = mark.reachable_state_identity;
    }

    pub(crate) fn enable_reachable_state_identity(&mut self) -> bool {
        if self.reachable_state_identity.is_some() {
            return true;
        }
        if self.region_len() != 0 || self.generated_len() != 0 {
            return false;
        }
        self.reachable_state_identity =
            Some(SemanticSequenceIdentity::empty(0x736f_7572_6365_5f31));
        true
    }

    pub(crate) fn reachable_state_identity_root(&self) -> Option<u64> {
        self.reachable_state_identity.map(|root| root.root())
    }

    pub(crate) fn begin_checkpoint_candidate(
        &mut self,
        mark: SourceMapMark,
    ) -> AcceptedSourceMapTail {
        assert!(self.validates(mark));
        let region_base = self.accepted_region_len();
        let generated_base = self.accepted_generated_len();
        let local_regions = mark.regions - region_base;
        let local_generated = mark.generated - generated_base;
        let regions = Arc::make_mut(&mut self.regions).split_off(local_regions);
        let registration_roots =
            Arc::make_mut(&mut self.registration_roots).split_off(local_regions);
        let generated = Arc::make_mut(&mut self.generated).split_off(local_generated);
        for region in &regions {
            Arc::make_mut(&mut self.region_by_source).remove(&region.source);
        }
        let identities = self
            .identities
            .begin_checkpoint_candidate(mark.identities)
            .expect("validated source identity mark remains rewindable");
        self.reachable_state_identity = mark.reachable_state_identity;
        let next_pos = std::mem::replace(&mut self.next_pos, mark.next_pos);
        AcceptedSourceMapTail {
            regions,
            registration_roots,
            generated,
            next_pos,
            identities,
        }
    }

    pub(crate) fn reject_checkpoint_candidate(
        &mut self,
        mark: SourceMapMark,
        mut tail: AcceptedSourceMapTail,
    ) {
        self.truncate_to_inner(mark);
        self.identities.reject_checkpoint_candidate(tail.identities);
        let region_base = self.region_len();
        for (offset, region) in tail.regions.iter().enumerate() {
            assert_eq!(
                Arc::make_mut(&mut self.region_by_source)
                    .insert(region.source, region_base + offset),
                None
            );
        }
        Arc::make_mut(&mut self.regions).append(&mut tail.regions);
        Arc::make_mut(&mut self.registration_roots).append(&mut tail.registration_roots);
        Arc::make_mut(&mut self.generated).append(&mut tail.generated);
        self.next_pos = tail.next_pos;
    }

    pub(crate) fn accept_checkpoint_candidate(&mut self, tail: AcceptedSourceMapTail) {
        self.identities.accept_checkpoint_candidate(tail.identities);
    }

    fn registration(&self, index: usize) -> Option<&SourceRegistrationRef> {
        let base = self.accepted_region_len();
        if index < base {
            return self.accepted.as_ref()?.registration(index);
        }
        self.registration_roots.get(index - base)
    }

    #[cfg(test)]
    fn line_starts_at(&self, index: usize) -> Option<&[usize]> {
        self.registration(index)
            .map(|registration| registration.0.line_starts.as_ref())
    }

    #[cfg(any(test, feature = "profiling"))]
    pub(crate) fn fork_at(&self, mark: SourceMapMark) -> Self {
        assert!(self.validates(mark));
        let parent_regions = self.accepted_region_len();
        let parent_generated = self.accepted_generated_len();
        let region_len = mark.regions - parent_regions;
        let generated_len = mark.generated - parent_generated;
        let accepted = if region_len == 0 && generated_len == 0 {
            self.accepted.clone()
        } else {
            Some(Arc::new(AcceptedSourceBlock {
                parent: self.accepted.clone(),
                regions: Arc::clone(&self.regions),
                registration_roots: Arc::clone(&self.registration_roots),
                region_by_source: Arc::clone(&self.region_by_source),
                generated: Arc::clone(&self.generated),
                region_len,
                generated_len,
                total_regions: mark.regions,
                total_generated: mark.generated,
            }))
        };
        let identities = self
            .identities
            .fork_at(mark.identities)
            .expect("source-map fork mark is an ancestor");
        Self {
            accepted,
            regions: Arc::new(Vec::new()),
            registration_roots: Arc::new(Vec::new()),
            region_by_source: Arc::new(AHashMap::new()),
            generated: Arc::new(Vec::new()),
            next_pos: mark.next_pos,
            forced_next_pos: self.forced_next_pos,
            identities,
            reachable_state_identity: mark.reachable_state_identity,
        }
    }

    #[cfg(feature = "profiling")]
    pub(crate) fn retained_payload_bytes(&self) -> usize {
        let region_bytes = self
            .regions_for_profile()
            .map(|region| {
                std::mem::size_of::<SourceRegion>()
                    + self
                        .registration(region.identity.slot() as usize)
                        .map_or(0, |registration| {
                            registration.0.line_starts.len() * std::mem::size_of::<usize>()
                        })
            })
            .sum::<usize>();
        let generated_bytes = (0..self.generated_len())
            .filter_map(|index| self.generated(GeneratedSourceId(index as u32)))
            .map(GeneratedSource::len)
            .sum::<usize>();
        region_bytes.saturating_add(generated_bytes)
    }

    #[cfg(feature = "profiling")]
    fn regions_for_profile(&self) -> impl Iterator<Item = SourceRegion> + '_ {
        (0..self.region_len()).filter_map(|index| {
            if index < self.accepted_region_len() {
                self.accepted
                    .as_ref()?
                    .registration(index)
                    .map(SourceRegistrationRef::region)
            } else {
                self.regions
                    .get(index - self.accepted_region_len())
                    .copied()
            }
        })
    }
}

fn source_descriptor_identity(descriptor: &SourceDescriptor) -> Option<u64> {
    match descriptor {
        SourceDescriptor::Generated(source) if !source.contributes_reachable_state_identity() => {
            return None;
        }
        SourceDescriptor::World { .. } | SourceDescriptor::Generated(_) => {}
    }
    Some(semantic_scalar_root(
        0x736f_7572_6365_5f64,
        |hasher| match descriptor {
            SourceDescriptor::World { byte_len, .. } => {
                hasher.tag(0);
                hasher.u64(*byte_len);
            }
            SourceDescriptor::Generated(source) => {
                hasher.tag(1);
                match source.logical_path() {
                    Some(path) => {
                        hasher.bool(true);
                        hasher.str(path);
                    }
                    None => {
                        hasher.bool(false);
                        hasher.bytes(&source.hash().bytes());
                    }
                }
            }
        },
    ))
}
