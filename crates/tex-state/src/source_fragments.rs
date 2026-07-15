//! Edit-stable source fragments and current-document piece-table resolution.

mod layout_index;

use std::collections::HashMap;
use std::mem;
use std::ops::Range;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};

use crate::source_map::{
    LogicalPositionAllocator, RegisteredSource, SourceMapError, SourcePos, SourceSpan,
};
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

impl FragmentId {
    #[cfg(test)]
    #[must_use]
    pub(crate) const fn raw(self) -> u32 {
        self.slot
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
    bytes: Option<Arc<[u8]>>,
    removed_revision: Option<u64>,
    live_generation: LayoutGeneration,
}

impl SourceFragment {
    const fn anchor(&self) -> u64 {
        self.region_start.raw() + self.byte_len
    }
}

#[derive(Debug)]
enum FragmentNode {
    Leaf(SourceFragment),
    Branch {
        left: Option<Arc<Self>>,
        right: Option<Arc<Self>>,
    },
}

#[derive(Clone, Debug, Default)]
struct FragmentTable {
    root: Option<Arc<FragmentNode>>,
    len: u32,
    depth: u8,
}

impl FragmentTable {
    fn push(&mut self, fragment: SourceFragment) -> Result<(), SourceMapError> {
        if self.len == u32::MAX {
            return Err(SourceMapError::LogicalPositionExhausted);
        }
        if self.root.is_none() {
            self.root = Some(Arc::new(FragmentNode::Leaf(fragment)));
        } else if u64::from(self.len) == (1_u64 << self.depth) {
            self.root = Some(Arc::new(FragmentNode::Branch {
                left: self.root.take(),
                right: Some(Self::new_path(self.depth, fragment)),
            }));
            self.depth += 1;
        } else {
            let Some(root) = self.root.as_ref() else {
                unreachable!("nonempty fragment tree")
            };
            self.root = Some(Self::insert(root, self.depth, self.len, fragment));
        }
        self.len += 1;
        Ok(())
    }

    fn new_path(depth: u8, fragment: SourceFragment) -> Arc<FragmentNode> {
        if depth == 0 {
            Arc::new(FragmentNode::Leaf(fragment))
        } else {
            Arc::new(FragmentNode::Branch {
                left: Some(Self::new_path(depth - 1, fragment)),
                right: None,
            })
        }
    }

    fn insert(
        node: &Arc<FragmentNode>,
        depth: u8,
        index: u32,
        fragment: SourceFragment,
    ) -> Arc<FragmentNode> {
        if depth == 0 {
            return Arc::new(FragmentNode::Leaf(fragment));
        }
        let FragmentNode::Branch { left, right } = node.as_ref() else {
            unreachable!("fragment tree depth matches node shape")
        };
        let right_half = index & (1_u32 << (depth - 1)) != 0;
        if right_half {
            let child = match right {
                Some(child) => Self::insert(child, depth - 1, index, fragment),
                None => Self::new_path(depth - 1, fragment),
            };
            Arc::new(FragmentNode::Branch {
                left: left.clone(),
                right: Some(child),
            })
        } else {
            let child = match left {
                Some(child) => Self::insert(child, depth - 1, index, fragment),
                None => Self::new_path(depth - 1, fragment),
            };
            Arc::new(FragmentNode::Branch {
                left: Some(child),
                right: right.clone(),
            })
        }
    }

    fn get(&self, index: u32) -> Option<&SourceFragment> {
        if index >= self.len {
            return None;
        }
        let mut node = self.root.as_deref()?;
        for shift in (0..self.depth).rev() {
            let FragmentNode::Branch { left, right } = node else {
                return None;
            };
            node = if index & (1_u32 << shift) == 0 {
                left.as_deref()?
            } else {
                right.as_deref()?
            };
        }
        let FragmentNode::Leaf(fragment) = node else {
            return None;
        };
        Some(fragment)
    }

    fn visit(&self, mut visitor: impl FnMut(&SourceFragment)) {
        fn walk(node: &FragmentNode, visitor: &mut impl FnMut(&SourceFragment)) {
            match node {
                FragmentNode::Leaf(fragment) => visitor(fragment),
                FragmentNode::Branch { left, right } => {
                    if let Some(left) = left {
                        walk(left, visitor);
                    }
                    if let Some(right) = right {
                        walk(right, visitor);
                    }
                }
            }
        }
        if let Some(root) = &self.root {
            walk(root, &mut visitor);
        }
    }
}

/// Session-scoped append-only registry of immutable editor source fragments.
///
/// Clones share inherited metadata and byte ownership in O(1) and receive a
/// fresh append lineage. Engine generations install a metadata-only view;
/// the accepted session remains the sole byte-state mutator.
#[derive(Debug)]
pub struct FragmentStore {
    fragments: FragmentTable,
    sources: Arc<HashMap<FragmentId, FragmentSource>>,
    append_lineage: u64,
}

impl Clone for FragmentStore {
    fn clone(&self) -> Self {
        Self {
            fragments: self.fragments.clone(),
            sources: Arc::clone(&self.sources),
            append_lineage: next_fragment_lineage(),
        }
    }
}

impl Default for FragmentStore {
    fn default() -> Self {
        Self {
            fragments: FragmentTable::default(),
            sources: Arc::new(HashMap::new()),
            append_lineage: next_fragment_lineage(),
        }
    }
}

impl FragmentStore {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
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
        let slot = self.fragments.len;
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
        self.fragments.push(fragment)?;
        Arc::make_mut(&mut self.sources).insert(
            id,
            FragmentSource {
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
        self.fragments.len as usize
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.fragments.len == 0
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
        for (id, source) in sources.iter_mut() {
            if source.live_generation == layout.generation() {
                continue;
            }
            let fragment = self.fragments.get(id.slot).expect("source has metadata");
            let removed_revision = *source
                .removed_revision
                .get_or_insert(accepted_revision.max(fragment.minted_revision));
            if removed_revision <= oldest_retained_revision
                && let Some(bytes) = source.bytes.take()
            {
                dropped = dropped.saturating_add(bytes.len());
            }
        }
        sources.retain(|_, source| source.bytes.is_some());
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
        let mut total = 0_u64;
        self.fragments.visit(|fragment| {
            total = total.saturating_add(fragment.byte_len.saturating_add(1));
        });
        total
    }

    /// Requested diagnostic storage retained by this session-owned table.
    #[must_use]
    pub fn retained_bytes(&self) -> usize {
        mem::size_of::<Self>()
            .saturating_add(self.metadata_retained_bytes())
            .saturating_add(self.source_bytes())
    }

    pub(crate) fn metadata_snapshot(&self) -> Self {
        Self {
            fragments: self.fragments.clone(),
            sources: Arc::new(HashMap::new()),
            append_lineage: next_fragment_lineage(),
        }
    }

    /// Measurement-only access to the exact immutable view installed in an
    /// engine generation.
    #[cfg(feature = "testing")]
    #[must_use]
    pub fn testing_metadata_snapshot(&self) -> Self {
        self.metadata_snapshot()
    }

    pub(crate) fn metadata_retained_bytes(&self) -> usize {
        (self.fragments.len as usize)
            .saturating_mul(mem::size_of::<SourceFragment>() + mem::size_of::<FragmentNode>())
    }

    /// Returns the immutable bytes retained for one fragment.
    #[must_use]
    pub fn bytes(&self, id: FragmentId) -> Option<&[u8]> {
        self.get(id)?;
        self.sources.get(&id)?.bytes.as_deref()
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

    #[must_use]
    pub(crate) fn contains_registration(&self, registration: RegisteredSource) -> bool {
        self.fragment_at(registration.start())
            .is_some_and(|(_, fragment)| {
                RegisteredSource::new(fragment.region_start, fragment.byte_len) == registration
            })
    }

    #[must_use]
    pub(crate) fn contains_position(&self, position: SourcePos) -> bool {
        self.fragment_at(position).is_some()
    }

    fn get(&self, id: FragmentId) -> Option<&SourceFragment> {
        self.fragments
            .get(id.slot)
            .filter(|fragment| fragment.id == id)
    }

    fn fragment_at(&self, position: SourcePos) -> Option<(FragmentId, &SourceFragment)> {
        let mut low = 0_u32;
        let mut high = self.fragments.len;
        while low < high {
            let mid = low + (high - low) / 2;
            let fragment = self.fragments.get(mid)?;
            if fragment.region_start <= position {
                low = mid + 1;
            } else {
                high = mid;
            }
        }
        let fragment = self.fragments.get(low.checked_sub(1)?)?;
        (position.raw() <= fragment.anchor()).then_some((fragment.id, fragment))
    }

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

    /// Verifies that every piece still names the exact fragment allocation
    /// against which this layout was constructed.
    pub(crate) fn validate_store(
        &self,
        fragments: &FragmentStore,
    ) -> Result<(), EditorLayoutError> {
        for piece in self.pieces.iter() {
            let fragment = fragments
                .get(piece.fragment)
                .ok_or(EditorLayoutError::UnknownFragment)?;
            if piece.range.start > piece.range.end || u64::from(piece.range.end) > fragment.byte_len
            {
                return Err(EditorLayoutError::InvalidPieceRange);
            }
        }
        Ok(())
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
