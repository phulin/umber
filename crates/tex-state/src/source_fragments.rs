//! Edit-stable source fragments and current-document piece-table resolution.

mod layout_index;

use std::collections::{HashMap, VecDeque};
use std::mem;
use std::ops::Range;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};

use crate::ContentHash;
#[cfg(test)]
use crate::source_map::SourceSpan;
use crate::source_map::{LogicalPositionAllocator, RegisteredSource, SourceMapError, SourcePos};
use layout_index::FragmentPieceIndex;

static NEXT_FRAGMENT_LINEAGE: AtomicU64 = AtomicU64::new(1);

fn next_fragment_lineage() -> u64 {
    NEXT_FRAGMENT_LINEAGE
        .fetch_update(
            AtomicOrdering::Relaxed,
            AtomicOrdering::Relaxed,
            |lineage| lineage.checked_add(1),
        )
        .expect("fragment lineage identity space exhausted")
}

/// Generation-tagged, session-local identity of an immutable source fragment.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FragmentId {
    lineage: u64,
    slot: u32,
}

/// Session-stable identity of immutable editor backing used by one or more pieces.
///
/// Splitting a piece preserves this identity; replaced backing receives a new
/// identity even when its bytes happen to be equal.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PieceId(FragmentId);

impl PieceId {
    #[must_use]
    pub const fn fragment(self) -> FragmentId {
        self.0
    }
}

/// Stable identity of a byte range in immutable editor backing.
///
/// The occurrence component distinguishes duplicate equal text, while the
/// content identity permits pure caches to share values deliberately.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RootSpanId {
    piece: PieceId,
    start: u32,
    end: u32,
    content: ContentHash,
    region_start: SourcePos,
    fragment_byte_len: u64,
    minted_revision: u64,
}

impl RootSpanId {
    /// Returns the zero-width anchor at the beginning of this rooted span.
    #[must_use]
    pub const fn start_anchor(self) -> Self {
        Self {
            piece: self.piece,
            start: self.start,
            end: self.start,
            content: self.content,
            region_start: self.region_start,
            fragment_byte_len: self.fragment_byte_len,
            minted_revision: self.minted_revision,
        }
    }

    #[must_use]
    pub const fn piece(self) -> PieceId {
        self.piece
    }

    #[must_use]
    pub const fn start(self) -> u32 {
        self.start
    }

    #[must_use]
    pub const fn end(self) -> u32 {
        self.end
    }

    #[must_use]
    pub const fn content(self) -> ContentHash {
        self.content
    }

    /// Rebuilds a span on the same immutable editor piece. Aggregate source
    /// lookup remains responsible for validating the resulting bounds.
    #[doc(hidden)]
    #[must_use]
    pub const fn with_offsets(self, start: u32, end: u32) -> Self {
        Self {
            piece: self.piece,
            start,
            end,
            content: self.content,
            region_start: self.region_start,
            fragment_byte_len: self.fragment_byte_len,
            minted_revision: self.minted_revision,
        }
    }
}

/// Immutable source text and its permanently reserved logical range.
#[derive(Clone, Debug)]
struct SourceFragment {
    id: FragmentId,
    region_start: SourcePos,
    byte_len: u64,
    minted_revision: u64,
}

#[derive(Clone, Debug)]
struct FragmentSource {
    fragment: SourceFragment,
    bytes: Option<Arc<[u8]>>,
    removed_revision: Option<u64>,
    live_generation: LayoutGeneration,
}

impl SourceFragment {
    const fn anchor(&self) -> u64 {
        self.region_start.raw() + self.byte_len
    }
}

/// Retired raw-coordinate history is diagnostic cache state, not semantic
/// ownership. Sixty-four rows cover the editor's immediate undo/hover window
/// while giving the long-session gate a fixed, architecture-derived charge.
pub const RETIRED_FRAGMENT_METADATA_ROWS: usize = 64;

const fn default_retired_fragment_metadata_bytes() -> usize {
    RETIRED_FRAGMENT_METADATA_ROWS * mem::size_of::<SourceFragment>()
}

/// Session-scoped registry of current fragments and bounded retired metadata.
///
/// Clones share inherited metadata and byte ownership in O(1) and receive a
/// fresh append lineage. Engine generations install a metadata-only view;
/// the accepted session remains the sole byte-state mutator.
#[derive(Debug)]
pub struct FragmentStore {
    sources: Arc<HashMap<FragmentId, FragmentSource>>,
    retired: Arc<VecDeque<SourceFragment>>,
    #[cfg(test)]
    root_coordinates: Option<RootCoordinateMap>,
    append_lineage: u64,
    next_slot: u32,
    reserved_position_bytes: u64,
    retired_metadata_budget_bytes: usize,
}

#[cfg(test)]
#[derive(Clone, Debug)]
struct RootCoordinateMap {
    logical_path: Arc<str>,
    byte_len: u64,
    content: ContentHash,
    pieces: Arc<[Piece]>,
    doc_starts: Arc<[u64]>,
    fragments: Arc<[SourceFragment]>,
    registrations: Arc<Vec<RegisteredSource>>,
    backing: Option<Arc<[u8]>>,
}

impl Clone for FragmentStore {
    fn clone(&self) -> Self {
        Self {
            sources: Arc::clone(&self.sources),
            retired: Arc::clone(&self.retired),
            #[cfg(test)]
            root_coordinates: self.root_coordinates.clone(),
            append_lineage: next_fragment_lineage(),
            next_slot: self.next_slot,
            reserved_position_bytes: self.reserved_position_bytes,
            retired_metadata_budget_bytes: self.retired_metadata_budget_bytes,
        }
    }
}

impl Default for FragmentStore {
    fn default() -> Self {
        Self {
            sources: Arc::new(HashMap::new()),
            retired: Arc::new(VecDeque::new()),
            #[cfg(test)]
            root_coordinates: None,
            append_lineage: next_fragment_lineage(),
            next_slot: 0,
            reserved_position_bytes: 0,
            retired_metadata_budget_bytes: default_retired_fragment_metadata_bytes(),
        }
    }
}

impl FragmentStore {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[cfg(any(test, feature = "testing"))]
    #[must_use]
    pub fn testing_with_retired_metadata_budget(budget_bytes: usize) -> Self {
        Self {
            retired_metadata_budget_bytes: budget_bytes,
            ..Self::default()
        }
    }

    /// Appends one immutable fragment and returns its opaque id and registration.
    ///
    /// The revision is diagnostic metadata only. The reserved range includes
    /// one end anchor even for empty fragments.
    pub fn append(
        &mut self,
        bytes: Arc<[u8]>,
        minted_revision: u64,
    ) -> Result<(FragmentId, RegisteredSource), SourceMapError> {
        let byte_len =
            u64::try_from(bytes.len()).map_err(|_| SourceMapError::LogicalPositionExhausted)?;
        let (start, _) = LogicalPositionAllocator.reserve(byte_len)?;
        self.append_at(bytes, minted_revision, byte_len, start)
    }

    /// Drops the session-owned bytes for a fragment that was never published
    /// in an accepted layout, while retaining its permanent metadata row.
    ///
    /// Failed editor advances use this after append so their logical position
    /// ranges and ids remain burned without retaining an orphan backing.
    pub fn discard_unpublished_bytes(&mut self, id: FragmentId) -> usize {
        let Some(source) = Arc::make_mut(&mut self.sources).remove(&id) else {
            return 0;
        };
        let dropped = source.bytes.as_ref().map_or(0, |bytes| bytes.len());
        self.retain_retired_metadata(source.fragment);
        dropped
    }

    /// Appends at an exact logical position for representation-boundary tests.
    #[cfg(any(test, feature = "testing"))]
    pub fn testing_append_at(
        &mut self,
        bytes: Arc<[u8]>,
        minted_revision: u64,
        start: u64,
    ) -> Result<(FragmentId, RegisteredSource), SourceMapError> {
        let byte_len =
            u64::try_from(bytes.len()).map_err(|_| SourceMapError::LogicalPositionExhausted)?;
        start
            .checked_add(byte_len)
            .and_then(|anchor| anchor.checked_add(1))
            .ok_or(SourceMapError::LogicalPositionExhausted)?;
        self.append_at(bytes, minted_revision, byte_len, start)
    }

    fn append_at(
        &mut self,
        bytes: Arc<[u8]>,
        minted_revision: u64,
        byte_len: u64,
        start: u64,
    ) -> Result<(FragmentId, RegisteredSource), SourceMapError> {
        let slot = self.next_slot;
        self.next_slot = self
            .next_slot
            .checked_add(1)
            .ok_or(SourceMapError::LogicalPositionExhausted)?;
        let id = FragmentId {
            lineage: self.append_lineage,
            slot,
        };
        let fragment = SourceFragment {
            id,
            region_start: SourcePos::from_raw_for_store(start),
            byte_len,
            minted_revision,
        };
        self.reserved_position_bytes = self
            .reserved_position_bytes
            .saturating_add(byte_len.saturating_add(1));
        Arc::make_mut(&mut self.sources).insert(
            id,
            FragmentSource {
                fragment,
                bytes: Some(bytes),
                removed_revision: None,
                live_generation: LayoutGeneration::new(u64::MAX),
            },
        );
        Ok((
            id,
            RegisteredSource::new(SourcePos::from_raw_for_store(start), byte_len),
        ))
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.next_slot as usize
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.next_slot == 0
    }

    /// Drops bytes from fragments that are absent from the accepted layout and
    /// no longer needed by a checkpoint predating their removal.
    pub fn prune_for_layout(
        &mut self,
        layout: &EditorLayout,
        accepted_revision: u64,
        oldest_retained_revision: u64,
    ) -> usize {
        let sources = Arc::make_mut(&mut self.sources);
        for piece in layout.pieces() {
            if let Some(source) = sources.get_mut(&piece.fragment()) {
                source.live_generation = layout.generation();
            }
        }
        let mut dropped = 0_usize;
        for source in sources.values_mut() {
            if source.live_generation == layout.generation() {
                continue;
            }
            let fragment = &source.fragment;
            let removed_revision = *source
                .removed_revision
                .get_or_insert(accepted_revision.max(fragment.minted_revision));
            if removed_revision <= oldest_retained_revision
                && let Some(bytes) = source.bytes.take()
            {
                dropped = dropped.saturating_add(bytes.len());
            }
        }
        let mut retired = sources
            .extract_if(|_, source| source.bytes.is_none())
            .map(|(_, source)| source.fragment)
            .collect::<Vec<_>>();
        retired.sort_unstable_by_key(|fragment| fragment.region_start);
        for fragment in retired {
            self.retain_retired_metadata(fragment);
        }
        dropped
    }

    /// Bytes of immutable source text still retained for live or protected fragments.
    #[must_use]
    pub fn source_bytes(&self) -> usize {
        self.sources
            .values()
            .filter_map(|source| source.bytes.as_ref())
            .map(|bytes| bytes.len())
            .sum()
    }

    /// Cumulative logical position space consumed, including one anchor per fragment.
    #[must_use]
    pub fn reserved_position_bytes(&self) -> u64 {
        self.reserved_position_bytes
    }

    /// Requested diagnostic storage retained by this session-owned table.
    #[must_use]
    pub fn retained_bytes(&self) -> usize {
        mem::size_of::<Self>()
            .saturating_add(self.metadata_retained_bytes())
            .saturating_add(self.source_bytes())
    }

    #[cfg(any(test, feature = "testing"))]
    pub(crate) fn metadata_snapshot(&self) -> Self {
        Self {
            sources: Arc::new(HashMap::new()),
            retired: Arc::clone(&self.retired),
            #[cfg(test)]
            root_coordinates: self.root_coordinates.clone(),
            append_lineage: next_fragment_lineage(),
            next_slot: self.next_slot,
            reserved_position_bytes: self.reserved_position_bytes,
            retired_metadata_budget_bytes: self.retired_metadata_budget_bytes,
        }
    }

    #[cfg(test)]
    pub(crate) fn metadata_snapshot_for_layout(
        &self,
        layout: &EditorLayout,
        retain_rendered_source: bool,
    ) -> Self {
        let mut snapshot = self.metadata_snapshot();
        let mut current_sources = HashMap::new();
        let mut current_fragments = Vec::new();
        let mut bytes = Vec::with_capacity(usize::try_from(layout.byte_len).unwrap_or(0));
        for piece in layout.pieces.iter() {
            let source = self
                .sources
                .get(&piece.fragment())
                .expect("validated editor layout has live accepted fragment metadata");
            let backing = source
                .bytes
                .as_deref()
                .expect("validated editor layout has live accepted backing");
            bytes.extend_from_slice(&backing[piece.start() as usize..piece.end() as usize]);
            if current_fragments
                .last()
                .is_none_or(|last: &SourceFragment| last.id != source.fragment.id)
            {
                current_fragments.push(source.fragment.clone());
            }
            if retain_rendered_source {
                current_sources
                    .entry(source.fragment.id)
                    .or_insert_with(|| source.clone());
            }
        }
        current_fragments.sort_unstable_by_key(|fragment| fragment.region_start);
        current_fragments.dedup_by_key(|fragment| fragment.id);
        snapshot.sources = Arc::new(current_sources);
        snapshot.root_coordinates = Some(RootCoordinateMap {
            logical_path: Arc::clone(&layout.path),
            byte_len: layout.byte_len,
            content: ContentHash::from_bytes(&bytes),
            pieces: Arc::clone(&layout.pieces),
            doc_starts: Arc::clone(&layout.doc_starts),
            fragments: current_fragments.into(),
            registrations: Arc::new(Vec::new()),
            backing: None,
        });
        snapshot
    }

    #[cfg(test)]
    pub(crate) fn bind_generated_root_registration(
        &mut self,
        registration: RegisteredSource,
        source: &crate::source_map::GeneratedSource,
    ) {
        let Some(root) = self.root_coordinates.as_mut() else {
            return;
        };
        if registration.byte_len() != root.byte_len
            || source.hash() != root.content
            || source.logical_path() != Some(root.logical_path.as_ref())
            || root.registrations.contains(&registration)
        {
            return;
        }
        Arc::make_mut(&mut root.registrations).push(registration);
        root.backing = Some(source.backing());
    }

    /// Measurement-only access to the exact immutable view installed in an
    /// engine generation.
    #[cfg(feature = "testing")]
    #[must_use]
    pub fn testing_metadata_snapshot(&self) -> Self {
        self.metadata_snapshot()
    }

    pub(crate) fn metadata_retained_bytes(&self) -> usize {
        self.sources
            .len()
            .saturating_mul(mem::size_of::<FragmentSource>())
            .saturating_add(self.retired_metadata_budget_bytes)
    }

    fn retain_retired_metadata(&mut self, fragment: SourceFragment) {
        let row_bytes = mem::size_of::<SourceFragment>();
        if row_bytes == 0 || self.retired_metadata_budget_bytes < row_bytes {
            return;
        }
        let rows = self.retired_metadata_budget_bytes / row_bytes;
        let retired = Arc::make_mut(&mut self.retired);
        while retired.len() >= rows {
            retired.pop_front();
        }
        retired.push_back(fragment);
    }

    #[cfg(any(test, feature = "testing"))]
    #[must_use]
    pub fn testing_retired_metadata_rows(&self) -> usize {
        self.retired.len()
    }

    /// Returns the immutable bytes retained for one fragment.
    #[must_use]
    pub fn bytes(&self, id: FragmentId) -> Option<&[u8]> {
        self.get(id)?;
        self.sources.get(&id)?.bytes.as_deref()
    }

    /// Identifies a range relative to a current piece without using document offsets.
    #[must_use]
    pub fn root_span_id(&self, piece: &Piece, range: Range<u32>) -> Option<RootSpanId> {
        let fragment = self.get(piece.fragment())?;
        let piece_len = piece.end().checked_sub(piece.start())?;
        if range.start > range.end || range.end > piece_len {
            return None;
        }
        let start = piece.start().checked_add(range.start)?;
        let end = piece.start().checked_add(range.end)?;
        let bytes = self
            .bytes(piece.fragment())?
            .get(start as usize..end as usize)?;
        Some(RootSpanId {
            piece: piece.id(),
            start,
            end,
            content: ContentHash::from_bytes(bytes),
            region_start: fragment.region_start,
            fragment_byte_len: fragment.byte_len,
            minted_revision: fragment.minted_revision,
        })
    }

    /// Identifies one document range when it is wholly backed by one layout piece.
    #[must_use]
    pub fn root_span_for_layout_range(
        &self,
        layout: &EditorLayout,
        range: Range<u64>,
    ) -> Option<RootSpanId> {
        if range.start > range.end || range.end > layout.byte_len {
            return None;
        }
        let index = layout
            .doc_starts()
            .partition_point(|&start| start <= range.start)
            .checked_sub(1)?;
        let piece = layout.pieces().get(index)?;
        let piece_end = layout.doc_starts()[index] + u64::from(piece.end() - piece.start());
        if range.end > piece_end {
            return None;
        }
        let start = u32::try_from(range.start - layout.doc_starts()[index]).ok()?;
        let end = u32::try_from(range.end - layout.doc_starts()[index]).ok()?;
        self.root_span_id(piece, start..end)
    }

    /// Resolves a registered editor-fragment delivery to stable backing identity.
    #[must_use]
    pub fn registered_root_span_id(
        &self,
        registration: RegisteredSource,
        range: Range<u64>,
    ) -> Option<RootSpanId> {
        if range.start > range.end || range.end > registration.byte_len() {
            return None;
        }
        let (fragment_id, fragment) = self.fragment_at(registration.start())?;
        if RegisteredSource::new(fragment.region_start, fragment.byte_len) != registration {
            return None;
        }
        let start = u32::try_from(range.start).ok()?;
        let end = u32::try_from(range.end).ok()?;
        let content = self
            .bytes(fragment_id)
            .and_then(|bytes| bytes.get(start as usize..end as usize))
            .map_or_else(|| ContentHash::from_bytes(&[]), ContentHash::from_bytes);
        Some(RootSpanId {
            piece: PieceId(fragment_id),
            start,
            end,
            content,
            region_start: fragment.region_start,
            fragment_byte_len: fragment.byte_len,
            minted_revision: fragment.minted_revision,
        })
    }

    /// Rematches detached generated backing to one immutable editor fragment.
    #[must_use]
    pub fn root_span_for_generated_bytes(
        &self,
        bytes: &[u8],
        range: Range<u64>,
    ) -> Option<RootSpanId> {
        if range.start > range.end || range.end > bytes.len() as u64 {
            return None;
        }
        for source in self.sources.values() {
            let fragment = &source.fragment;
            if source.bytes.as_deref()? != bytes {
                continue;
            }
            let start = u32::try_from(range.start).ok()?;
            let end = u32::try_from(range.end).ok()?;
            return Some(RootSpanId {
                piece: PieceId(fragment.id),
                start,
                end,
                content: ContentHash::from_bytes(bytes.get(start as usize..end as usize)?),
                region_start: fragment.region_start,
                fragment_byte_len: fragment.byte_len,
                minted_revision: fragment.minted_revision,
            });
        }
        None
    }

    #[cfg(test)]
    pub(crate) fn direct_root_span_id(&self, origin: crate::token::OriginId) -> Option<RootSpanId> {
        let crate::token::OriginEncoding::DirectSource(position) = origin.decode() else {
            return None;
        };
        if let Some(span) = self.root_span_for_registered_position(position) {
            return Some(span);
        }
        let span = self.span_for_direct(position)?;
        let (fragment_id, fragment) = self.fragment_at(span.lo())?;
        if span.hi().raw() > fragment.anchor() {
            return None;
        }
        let start = u32::try_from(span.lo().raw() - fragment.region_start.raw()).ok()?;
        let end = u32::try_from(span.hi().raw() - fragment.region_start.raw()).ok()?;
        let content = self
            .bytes(fragment_id)
            .and_then(|bytes| bytes.get(start as usize..end as usize))
            .map_or_else(|| ContentHash::from_bytes(&[]), ContentHash::from_bytes);
        Some(RootSpanId {
            piece: PieceId(fragment_id),
            start,
            end,
            content,
            region_start: fragment.region_start,
            fragment_byte_len: fragment.byte_len,
            minted_revision: fragment.minted_revision,
        })
    }

    #[cfg(test)]
    fn root_span_for_registered_position(&self, position: SourcePos) -> Option<RootSpanId> {
        self.root_span_for_registered_span(SourceSpan::new(
            position,
            SourcePos::from_raw_for_store(position.raw().checked_add(1)?),
        ))
    }

    #[cfg(test)]
    fn root_span_for_registered_span(&self, span: SourceSpan) -> Option<RootSpanId> {
        let root = self.root_coordinates.as_ref()?;
        let registration = root.registrations.iter().find(|registration| {
            registration.start() <= span.lo()
                && registration
                    .start()
                    .raw()
                    .checked_add(registration.byte_len())
                    .is_some_and(|end| span.hi().raw() <= end)
        })?;
        let offset = span.lo().raw().checked_sub(registration.start().raw())?;
        let offset_end = span.hi().raw().checked_sub(registration.start().raw())?;
        let piece_index = root
            .doc_starts
            .partition_point(|&start| start <= offset)
            .checked_sub(1)?;
        let piece = root.pieces.get(piece_index)?;
        let piece_offset = u32::try_from(offset.checked_sub(root.doc_starts[piece_index])?).ok()?;
        let start = piece.start().checked_add(piece_offset)?;
        let span_len = u32::try_from(offset_end.checked_sub(offset)?).ok()?;
        let end = start.checked_add(span_len)?;
        if end > piece.end() {
            return None;
        }
        let content = self
            .bytes(piece.fragment())
            .and_then(|bytes| bytes.get(start as usize..end as usize))
            .or_else(|| {
                let offset = usize::try_from(offset).ok()?;
                let offset_end = usize::try_from(offset_end).ok()?;
                root.backing.as_deref()?.get(offset..offset_end)
            })
            .map(ContentHash::from_bytes)?;
        Some(RootSpanId {
            piece: piece.id(),
            start,
            end,
            content,
            region_start: self.get(piece.fragment())?.region_start,
            fragment_byte_len: self.get(piece.fragment())?.byte_len,
            minted_revision: self.get(piece.fragment())?.minted_revision,
        })
    }

    /// Returns the allocation-free registration capability for one fragment.
    #[must_use]
    pub fn registration(&self, id: FragmentId) -> Option<RegisteredSource> {
        let fragment = self.get(id)?;
        Some(RegisteredSource::new(
            fragment.region_start,
            fragment.byte_len,
        ))
    }

    fn get(&self, id: FragmentId) -> Option<&SourceFragment> {
        let fragment = self
            .sources
            .get(&id)
            .map(|source| &source.fragment)
            .or_else(|| self.retired.iter().find(|fragment| fragment.id == id));
        #[cfg(test)]
        let fragment = fragment.or_else(|| {
            self.root_coordinates
                .as_ref()?
                .fragments
                .iter()
                .find(|fragment| fragment.id == id)
        });
        fragment
    }

    fn fragment_at(&self, position: SourcePos) -> Option<(FragmentId, &SourceFragment)> {
        #[cfg(test)]
        if let Some(fragment) = self
            .root_coordinates
            .as_ref()
            .and_then(|root| fragment_at_in_sorted(&root.fragments, position))
        {
            return Some((fragment.id, fragment));
        }
        if let Some(fragment) =
            self.sources
                .values()
                .map(|source| &source.fragment)
                .find(|fragment| {
                    fragment.region_start <= position && position.raw() <= fragment.anchor()
                })
        {
            return Some((fragment.id, fragment));
        }
        self.retired
            .iter()
            .find(|fragment| {
                fragment.region_start <= position && position.raw() <= fragment.anchor()
            })
            .map(|fragment| (fragment.id, fragment))
    }

    #[cfg(test)]
    fn span_for_direct(&self, position: SourcePos) -> Option<SourceSpan> {
        let (_, fragment) = self.fragment_at(position)?;
        let offset = position.raw().checked_sub(fragment.region_start.raw())?;
        if offset >= fragment.byte_len {
            return None;
        }
        let offset = usize::try_from(offset).ok()?;
        let width = self.bytes(fragment.id).map_or(1, |bytes| {
            std::str::from_utf8(bytes.get(offset..).unwrap_or_default())
                .ok()
                .and_then(|suffix| suffix.chars().next())
                .map_or(1, |character| character.len_utf8() as u64)
        });
        let hi = position.raw().checked_add(width)?;
        (hi <= fragment.anchor())
            .then(|| SourceSpan::new(position, SourcePos::from_raw_for_store(hi)))
    }
}

#[cfg(test)]
fn fragment_at_in_sorted(
    fragments: &[SourceFragment],
    position: SourcePos,
) -> Option<&SourceFragment> {
    let index = fragments
        .partition_point(|fragment| fragment.region_start <= position)
        .checked_sub(1)?;
    let fragment = &fragments[index];
    (position.raw() <= fragment.anchor()).then_some(fragment)
}

/// Monotonic identity of one accepted editor piece-table layout.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct LayoutGeneration(u64);

impl LayoutGeneration {
    #[must_use]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    #[must_use]
    pub const fn raw(self) -> u64 {
        self.0
    }
}

/// One current-document view into an immutable fragment.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Piece {
    fragment: FragmentId,
    range: Range<u32>,
}

impl Piece {
    #[must_use]
    pub const fn new(fragment: FragmentId, start: u32, end: u32) -> Self {
        Self {
            fragment,
            range: start..end,
        }
    }

    #[must_use]
    pub const fn fragment(&self) -> FragmentId {
        self.fragment
    }

    #[must_use]
    pub const fn id(&self) -> PieceId {
        PieceId(self.fragment)
    }

    #[must_use]
    pub const fn start(&self) -> u32 {
        self.range.start
    }

    #[must_use]
    pub const fn end(&self) -> u32 {
        self.range.end
    }
}

/// Invalid piece-table construction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EditorLayoutError {
    UnknownFragment,
    InvalidPieceRange,
    DocumentTooLarge,
}

impl std::fmt::Display for EditorLayoutError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let message = match self {
            Self::UnknownFragment => "piece references an unknown source fragment",
            Self::InvalidPieceRange => "piece range is outside its source fragment",
            Self::DocumentTooLarge => "editor document offset space exhausted",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for EditorLayoutError {}

#[derive(Debug)]
struct LineIndex {
    generation: LayoutGeneration,
    starts: Arc<[u64]>,
}

/// Immutable piece table for one accepted editor document generation.
#[derive(Debug)]
pub struct EditorLayout {
    path: Arc<str>,
    generation: LayoutGeneration,
    pieces: Arc<[Piece]>,
    doc_starts: Arc<[u64]>,
    fragment_index: Box<[FragmentPieceIndex]>,
    byte_len: u64,
    line_index: OnceLock<LineIndex>,
    #[cfg(test)]
    line_index_builds: AtomicUsize,
}

impl EditorLayout {
    pub fn new(
        path: impl Into<Arc<str>>,
        generation: LayoutGeneration,
        pieces: Vec<Piece>,
        fragments: &FragmentStore,
    ) -> Result<Self, EditorLayoutError> {
        let mut doc_starts = Vec::with_capacity(pieces.len());
        let mut fragment_pieces: HashMap<FragmentId, Vec<(u32, u32, usize)>> = HashMap::new();
        let mut byte_len = 0_u64;
        for (piece_index, piece) in pieces.iter().enumerate() {
            let fragment = fragments
                .get(piece.fragment)
                .ok_or(EditorLayoutError::UnknownFragment)?;
            if piece.range.start > piece.range.end || u64::from(piece.range.end) > fragment.byte_len
            {
                return Err(EditorLayoutError::InvalidPieceRange);
            }
            doc_starts.push(byte_len);
            fragment_pieces.entry(piece.fragment).or_default().push((
                piece.range.start,
                piece.range.end,
                piece_index,
            ));
            byte_len = byte_len
                .checked_add(u64::from(piece.range.end - piece.range.start))
                .ok_or(EditorLayoutError::DocumentTooLarge)?;
        }
        let mut fragment_index = fragment_pieces
            .into_iter()
            .map(|(fragment, pieces)| FragmentPieceIndex::build(fragment, pieces))
            .collect::<Result<Vec<_>, _>>()?;
        fragment_index.sort_unstable_by_key(|index| index.fragment);
        Ok(Self {
            path: path.into(),
            generation,
            pieces: pieces.into(),
            doc_starts: doc_starts.into(),
            fragment_index: fragment_index.into_boxed_slice(),
            byte_len,
            line_index: OnceLock::new(),
            #[cfg(test)]
            line_index_builds: AtomicUsize::new(0),
        })
    }

    #[must_use]
    pub const fn generation(&self) -> LayoutGeneration {
        self.generation
    }

    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    #[must_use]
    pub fn pieces(&self) -> &[Piece] {
        &self.pieces
    }

    #[must_use]
    pub fn doc_starts(&self) -> &[u64] {
        &self.doc_starts
    }

    /// Requested diagnostic storage retained by this accepted layout.
    #[must_use]
    pub fn retained_bytes(&self) -> usize {
        mem::size_of::<Self>()
            .saturating_add(self.path.len())
            .saturating_add(self.pieces.len().saturating_mul(mem::size_of::<Piece>()))
            .saturating_add(self.doc_starts.len().saturating_mul(mem::size_of::<u64>()))
            .saturating_add(
                self.fragment_index
                    .iter()
                    .map(FragmentPieceIndex::retained_bytes)
                    .sum::<usize>(),
            )
            .saturating_add(self.line_index.get().map_or(0, |index| {
                index.starts.len().saturating_mul(mem::size_of::<u64>())
            }))
    }

    #[cfg(test)]
    fn current_range(&self, fragment: FragmentId, lo: u64, hi: u64) -> Option<(u64, u64)> {
        let fragment_index = self
            .fragment_index
            .binary_search_by_key(&fragment, |index| index.fragment)
            .ok()
            .map(|index| &self.fragment_index[index])?;
        let index = fragment_index.covering_piece(lo, hi)?;
        let piece = &self.pieces[index];
        let start = u64::from(piece.range.start);
        let doc_lo = self.doc_starts[index] + (lo - start);
        Some((doc_lo, doc_lo + (hi - lo)))
    }

    fn line_column(&self, fragments: &FragmentStore, offset: u64) -> Option<(u32, u32)> {
        if offset > self.byte_len {
            return None;
        }
        let index = self.line_index.get_or_init(|| LineIndex {
            generation: self.generation,
            starts: self.build_line_starts(fragments),
        });
        debug_assert_eq!(index.generation, self.generation);
        let line_index = index.starts.partition_point(|start| *start <= offset) - 1;
        let line = u32::try_from(line_index).ok()?.checked_add(1)?;
        let column = u32::try_from(offset - index.starts[line_index])
            .ok()?
            .checked_add(1)?;
        Some((line, column))
    }

    /// Materializes the accepted-layout line cache for a rendered-source
    /// query, including queries whose origin ultimately resolves as foreign.
    #[doc(hidden)]
    pub fn prepare_line_index(&self, fragments: &FragmentStore) {
        let _ = self.line_column(fragments, 0);
    }

    fn build_line_starts(&self, fragments: &FragmentStore) -> Arc<[u64]> {
        #[cfg(test)]
        self.line_index_builds.fetch_add(1, Ordering::Relaxed);
        let mut starts = vec![0];
        for (piece_index, piece) in self.pieces.iter().enumerate() {
            let Some(bytes) = fragments.bytes(piece.fragment) else {
                continue;
            };
            let range = piece.range.start as usize..piece.range.end as usize;
            for (offset, byte) in bytes[range].iter().enumerate() {
                if *byte == b'\n' {
                    starts.push(self.doc_starts[piece_index] + offset as u64 + 1);
                }
            }
        }
        starts.into()
    }

    #[cfg(test)]
    fn line_index_build_count(&self) -> usize {
        self.line_index_builds.load(Ordering::Relaxed)
    }
}

/// Layout-aware result for one compact provenance origin.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LayoutResolvedOrigin {
    Current {
        path: String,
        doc_offset_lo: u64,
        doc_offset_hi: u64,
        line: u32,
        column: u32,
    },
    Deleted {
        minted_revision: u64,
    },
    Foreign,
    Unknown,
}

#[cfg(test)]
pub(crate) fn resolve_fragment_span(
    span: SourceSpan,
    fragments: &FragmentStore,
    layout: &EditorLayout,
) -> Option<LayoutResolvedOrigin> {
    let (fragment_id, fragment) = fragments.fragment_at(span.lo())?;
    if span.hi().raw() < span.lo().raw() || span.hi().raw() > fragment.anchor() {
        return Some(LayoutResolvedOrigin::Unknown);
    }
    let lo = span.lo().raw() - fragment.region_start.raw();
    let hi = span.hi().raw() - fragment.region_start.raw();
    let Some((doc_offset_lo, doc_offset_hi)) = layout.current_range(fragment_id, lo, hi) else {
        return Some(LayoutResolvedOrigin::Deleted {
            minted_revision: fragment.minted_revision,
        });
    };
    let Some((line, column)) = layout.line_column(fragments, doc_offset_lo) else {
        return Some(LayoutResolvedOrigin::Unknown);
    };
    Some(LayoutResolvedOrigin::Current {
        path: layout.path.to_string(),
        doc_offset_lo,
        doc_offset_hi,
        line,
        column,
    })
}

#[cfg(test)]
pub(crate) fn resolve_root_span(
    span: RootSpanId,
    fragments: &FragmentStore,
    layout: &EditorLayout,
) -> LayoutResolvedOrigin {
    if span.start > span.end || u64::from(span.end) > span.fragment_byte_len {
        return LayoutResolvedOrigin::Unknown;
    }
    let fragment = span.piece.fragment();
    let Some((doc_offset_lo, doc_offset_hi)) =
        layout.current_range(fragment, u64::from(span.start), u64::from(span.end))
    else {
        return LayoutResolvedOrigin::Deleted {
            minted_revision: span.minted_revision,
        };
    };
    if fragments.bytes(fragment).is_none() {
        return LayoutResolvedOrigin::Unknown;
    }
    let Some((line, column)) = layout.line_column(fragments, doc_offset_lo) else {
        return LayoutResolvedOrigin::Unknown;
    };
    LayoutResolvedOrigin::Current {
        path: layout.path.to_string(),
        doc_offset_lo,
        doc_offset_hi,
        line,
        column,
    }
}

#[cfg(test)]
pub(crate) fn direct_fragment_span(
    origin: crate::token::OriginId,
    fragments: &FragmentStore,
) -> Option<SourceSpan> {
    let crate::token::OriginEncoding::DirectSource(position) = origin.decode() else {
        return None;
    };
    fragments.span_for_direct(position)
}

#[cfg(test)]
mod tests;
