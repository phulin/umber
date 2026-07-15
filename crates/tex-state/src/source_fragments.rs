//! Edit-stable source fragments and current-document piece-table resolution.

use std::mem;
use std::ops::Range;
#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};

use crate::source_map::{
    LogicalPositionAllocator, RegisteredSource, SourceMapError, SourcePos, SourceSpan,
};

/// Dense, session-local identity of an immutable source fragment.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FragmentId(u32);

impl FragmentId {
    #[must_use]
    pub const fn raw(self) -> u32 {
        self.0
    }
}

/// Immutable source text and its permanently reserved logical range.
#[derive(Clone, Debug)]
struct SourceFragment {
    bytes: Option<Arc<[u8]>>,
    region_start: SourcePos,
    byte_len: u64,
    minted_revision: u64,
    removed_revision: Option<u64>,
}

impl SourceFragment {
    const fn anchor(&self) -> u64 {
        self.region_start.raw() + self.byte_len
    }
}

/// Session-scoped append-only registry of immutable editor source fragments.
///
/// Clones are O(1) read-only snapshots. Appending uses copy-on-write, so an
/// engine generation retains exactly the fragment table installed for it and
/// never receives mutation authority.
#[derive(Clone, Debug, Default)]
pub struct FragmentStore {
    fragments: Arc<[SourceFragment]>,
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
        let raw = u32::try_from(self.fragments.len())
            .map_err(|_| SourceMapError::LogicalPositionExhausted)?;
        let fragment = SourceFragment {
            bytes: Some(bytes),
            region_start: SourcePos::from_raw_for_store(start),
            byte_len,
            minted_revision,
            removed_revision: None,
        };
        let mut fragments = self.fragments.to_vec();
        fragments.push(fragment);
        self.fragments = fragments.into();
        Ok((
            FragmentId(raw),
            RegisteredSource::new(SourcePos::from_raw_for_store(start), byte_len),
        ))
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.fragments.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.fragments.is_empty()
    }

    /// Drops bytes from fragments that are absent from the accepted layout and
    /// no longer needed by a checkpoint predating their removal.
    pub fn prune_for_layout(
        &mut self,
        layout: &EditorLayout,
        accepted_revision: u64,
        oldest_retained_revision: u64,
    ) -> usize {
        let mut live = vec![false; self.fragments.len()];
        for piece in layout.pieces() {
            live[piece.fragment().raw() as usize] = true;
        }
        let mut fragments = self.fragments.to_vec();
        let mut dropped = 0_usize;
        for (index, fragment) in fragments.iter_mut().enumerate() {
            if live[index] {
                continue;
            }
            let removed_revision = *fragment
                .removed_revision
                .get_or_insert(accepted_revision.max(fragment.minted_revision));
            if removed_revision <= oldest_retained_revision
                && let Some(bytes) = fragment.bytes.take()
            {
                dropped = dropped.saturating_add(bytes.len());
            }
        }
        self.fragments = fragments.into();
        dropped
    }

    /// Bytes of immutable source text still retained for live or protected fragments.
    #[must_use]
    pub fn source_bytes(&self) -> usize {
        self.fragments
            .iter()
            .filter_map(|fragment| fragment.bytes.as_ref())
            .map(|bytes| bytes.len())
            .sum()
    }

    /// Cumulative logical position space consumed, including one anchor per fragment.
    #[must_use]
    pub fn reserved_position_bytes(&self) -> u64 {
        self.fragments.iter().fold(0_u64, |total, fragment| {
            total.saturating_add(fragment.byte_len.saturating_add(1))
        })
    }

    /// Requested diagnostic storage retained by this session-owned table.
    #[must_use]
    pub fn retained_bytes(&self) -> usize {
        mem::size_of::<Self>()
            .saturating_add(self.metadata_retained_bytes())
            .saturating_add(self.source_bytes())
    }

    pub(crate) fn metadata_snapshot(&self) -> Self {
        let fragments = self
            .fragments
            .iter()
            .cloned()
            .map(|mut fragment| {
                fragment.bytes = None;
                fragment
            })
            .collect::<Vec<_>>();
        Self {
            fragments: fragments.into(),
        }
    }

    pub(crate) fn metadata_retained_bytes(&self) -> usize {
        self.fragments
            .len()
            .saturating_mul(mem::size_of::<SourceFragment>())
    }

    /// Returns the immutable bytes retained for one fragment.
    #[must_use]
    pub fn bytes(&self, id: FragmentId) -> Option<&[u8]> {
        self.get(id)?.bytes.as_deref()
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
        self.fragments.get(id.0 as usize)
    }

    fn fragment_at(&self, position: SourcePos) -> Option<(FragmentId, &SourceFragment)> {
        let index = self
            .fragments
            .partition_point(|fragment| fragment.region_start <= position)
            .checked_sub(1)?;
        let fragment = &self.fragments[index];
        (position.raw() <= fragment.anchor()).then_some((FragmentId(index as u32), fragment))
    }

    fn span_for_direct(&self, position: SourcePos) -> Option<SourceSpan> {
        let (_, fragment) = self.fragment_at(position)?;
        let offset = position.raw().checked_sub(fragment.region_start.raw())?;
        if offset >= fragment.byte_len {
            return None;
        }
        let offset = usize::try_from(offset).ok()?;
        let width = fragment.bytes.as_deref().map_or(1, |bytes| {
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
        let mut byte_len = 0_u64;
        for piece in &pieces {
            let fragment = fragments
                .get(piece.fragment)
                .ok_or(EditorLayoutError::UnknownFragment)?;
            if piece.range.start > piece.range.end || u64::from(piece.range.end) > fragment.byte_len
            {
                return Err(EditorLayoutError::InvalidPieceRange);
            }
            doc_starts.push(byte_len);
            byte_len = byte_len
                .checked_add(u64::from(piece.range.end - piece.range.start))
                .ok_or(EditorLayoutError::DocumentTooLarge)?;
        }
        Ok(Self {
            path: path.into(),
            generation,
            pieces: pieces.into(),
            doc_starts: doc_starts.into(),
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
            .saturating_add(self.line_index.get().map_or(0, |index| {
                index.starts.len().saturating_mul(mem::size_of::<u64>())
            }))
    }

    fn current_range(&self, fragment: FragmentId, lo: u64, hi: u64) -> Option<(u64, u64)> {
        self.pieces.iter().enumerate().find_map(|(index, piece)| {
            if piece.fragment != fragment {
                return None;
            }
            let start = u64::from(piece.range.start);
            let end = u64::from(piece.range.end);
            let covered = if lo == hi {
                start <= lo && lo <= end
            } else {
                start <= lo && lo < end && hi <= end
            };
            covered.then(|| {
                let doc_lo = self.doc_starts[index] + (lo - start);
                (doc_lo, doc_lo + (hi - lo))
            })
        })
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
            let Some(bytes) = fragments
                .get(piece.fragment)
                .and_then(|fragment| fragment.bytes.as_deref())
            else {
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
