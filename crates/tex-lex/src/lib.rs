//! TeX input sources and line handling.
//!
//! This crate owns the line-oriented part of TeX's eyes. It normalizes
//! physical input lines before the semantic lexer state machine assigns
//! catcodes and produces tokens.

use ahash::AHashMap;
use std::collections::VecDeque;
use std::fmt;
use std::ops::{Index, IndexMut};
use std::sync::Arc;
#[cfg(feature = "profiling-stats")]
use std::time::{Duration, Instant};

use tex_state::env::banks::TokParam;
use tex_state::ids::{OriginListId, TokenListId};
use tex_state::provenance::OriginListBuilder;
use tex_state::provenance::{
    DiagnosticSite, InsertedOriginKind, RelatedLocation, SyntheticOriginKind,
};
use tex_state::source_map::{RegisteredSource, SourceDescriptor};
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};
use tex_state::token_store::TokenListBuilder;
use tex_state::{
    EditorLayout, ExpansionState, FileContent, FragmentStore, InputRecordId, WorldError,
};

use tex_state::MacroArguments as MacroArgumentsSummary;
pub use tex_state::{
    ConditionFrameSummary, ConditionFrameToken, ConditionKind, ConditionLimb, InputFrameSummary,
    InputSummary, LexerState, MACRO_ARGUMENT_SLOTS, MacroArgumentRange, SourceFrameSummary,
    SourceId, TokenListReplayKind, TracedTokenList,
};

/// One invalid editor layout encountered while freezing its lexer cursor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LayoutCursorError {
    MissingFragmentBytes,
    MissingFragmentRegistration,
    PieceBoundaryInsideLine,
    DocumentOffsetOverflow,
}

impl fmt::Display for LayoutCursorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::MissingFragmentBytes => "layout cursor fragment bytes are unavailable",
            Self::MissingFragmentRegistration => {
                "layout cursor fragment registration is unavailable"
            }
            Self::PieceBoundaryInsideLine => {
                "layout cursor piece boundary is not a physical-line boundary"
            }
            Self::DocumentOffsetOverflow => "layout cursor document offset overflowed",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for LayoutCursorError {}

#[derive(Clone, Copy, Debug)]
struct LayoutCursorSegment {
    document_start: usize,
    document_end: usize,
    registration: RegisteredSource,
    fragment_start: u64,
}

/// Frozen per-revision mapping from document lines to fragment coordinates.
///
/// Segment storage is immutable and shared. Only the monotonic segment index
/// changes while physical lines are refilled.
#[derive(Clone, Debug)]
pub struct LayoutCursor {
    segments: Arc<[LayoutCursorSegment]>,
    root_registration: Option<RegisteredSource>,
    index: usize,
}

impl LayoutCursor {
    /// Builds the O(pieces) refill cursor for one validated editor layout.
    pub fn new(
        layout: &EditorLayout,
        fragments: &FragmentStore,
    ) -> Result<Self, LayoutCursorError> {
        let nonempty: Vec<_> = layout
            .pieces()
            .iter()
            .enumerate()
            .filter(|(_, piece)| piece.start() != piece.end())
            .collect();
        let mut segments = Vec::with_capacity(nonempty.len());
        for (position, (piece_index, piece)) in nonempty.iter().copied().enumerate() {
            let bytes = fragments
                .bytes(piece.fragment())
                .ok_or(LayoutCursorError::MissingFragmentBytes)?;
            let start = piece.start() as usize;
            let end = piece.end() as usize;
            if (start != 0 && bytes.get(start.wrapping_sub(1)) != Some(&b'\n'))
                || (position + 1 != nonempty.len()
                    && (end == 0 || bytes.get(end - 1) != Some(&b'\n')))
            {
                return Err(LayoutCursorError::PieceBoundaryInsideLine);
            }
            let document_start = usize::try_from(layout.doc_starts()[piece_index])
                .map_err(|_| LayoutCursorError::DocumentOffsetOverflow)?;
            let document_end = document_start
                .checked_add(end - start)
                .ok_or(LayoutCursorError::DocumentOffsetOverflow)?;
            let registration = fragments
                .registration(piece.fragment())
                .ok_or(LayoutCursorError::MissingFragmentRegistration)?;
            segments.push(LayoutCursorSegment {
                document_start,
                document_end,
                registration,
                fragment_start: piece.start().into(),
            });
        }
        let root_registration = layout
            .pieces()
            .first()
            .and_then(|piece| fragments.registration(piece.fragment()));
        Ok(Self {
            segments: segments.into(),
            root_registration,
            index: 0,
        })
    }

    fn seek(&mut self, document_offset: usize) {
        self.index = self
            .segments
            .partition_point(|segment| segment.document_start <= document_offset)
            .saturating_sub(1);
    }

    fn line_registration(
        &mut self,
        document_start: usize,
        document_end: usize,
    ) -> Option<(RegisteredSource, u64)> {
        while self.index + 1 < self.segments.len()
            && document_start >= self.segments[self.index + 1].document_start
        {
            self.index += 1;
        }
        let segment = self.segments.get(self.index)?;
        if document_start < segment.document_start || document_end > segment.document_end {
            return None;
        }
        let within = u64::try_from(document_start - segment.document_start).ok()?;
        Some((
            segment.registration,
            segment.fragment_start.checked_add(within)?,
        ))
    }
}

/// Source of physical input lines.
///
/// The trait is local so M3's `World` can implement it without forcing the
/// lexer to know where bytes came from.
pub trait InputSource: fmt::Debug {
    /// Reads the next physical line with its original backing-byte metadata.
    fn read_line(&mut self) -> Result<Option<PhysicalLine>, InputSourceError>;

    /// Returns the durable `World` record for a file-backed source.
    fn input_record(&self) -> Option<InputRecordId> {
        None
    }

    /// Returns immutable backing metadata used by diagnostic source mapping.
    fn source_descriptor(&self) -> Option<SourceDescriptor> {
        None
    }

    /// Whether this is the pseudo-file created by e-TeX `\scantokens`.
    fn is_scantokens(&self) -> bool {
        false
    }
}

impl<T> InputSource for Box<T>
where
    T: InputSource + ?Sized,
{
    fn read_line(&mut self) -> Result<Option<PhysicalLine>, InputSourceError> {
        (**self).read_line()
    }

    fn input_record(&self) -> Option<InputRecordId> {
        (**self).input_record()
    }

    fn source_descriptor(&self) -> Option<SourceDescriptor> {
        (**self).source_descriptor()
    }

    fn is_scantokens(&self) -> bool {
        (**self).is_scantokens()
    }
}

#[derive(Debug)]
pub enum InputSourceError {
    World(WorldError),
    InvalidUtf8 {
        byte_start: usize,
        byte_end: usize,
        line: usize,
        column: usize,
    },
}

impl fmt::Display for InputSourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::World(error) => error.fmt(f),
            Self::InvalidUtf8 {
                byte_start,
                byte_end,
                ..
            } => write!(
                f,
                "invalid UTF-8 at physical bytes {byte_start}..{byte_end}"
            ),
        }
    }
}

impl std::error::Error for InputSourceError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::World(error) => Some(error),
            Self::InvalidUtf8 { .. } => None,
        }
    }
}

impl From<WorldError> for InputSourceError {
    fn from(error: WorldError) -> Self {
        Self::World(error)
    }
}

/// One valid UTF-8 physical line and its exact range in the source backing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PhysicalLine {
    text: String,
    start: usize,
    content_end: usize,
    terminator_start: usize,
    terminator_end: usize,
}

impl PhysicalLine {
    /// Constructs a physical line whose terminator, if any, immediately
    /// follows `text` in the source backing.
    #[must_use]
    pub fn new(text: String, start: usize, terminator_end: usize) -> Self {
        let content_end = start
            .checked_add(text.len())
            .expect("physical line byte range overflowed");
        assert!(terminator_end >= content_end);
        Self {
            text,
            start,
            content_end,
            terminator_start: content_end,
            terminator_end,
        }
    }
}

/// In-memory input source for tests, `\scantokens`, and editor buffers.
#[derive(Debug)]
pub struct MemoryInput {
    backing: Arc<[u8]>,
    next_offset: usize,
    scantokens: bool,
}

impl MemoryInput {
    #[must_use]
    pub fn new(input: impl Into<String>) -> Self {
        let input = input.into();
        let backing: Arc<[u8]> = Arc::from(input.as_bytes());
        Self {
            backing,
            next_offset: 0,
            scantokens: false,
        }
    }

    /// Constructs the generated pseudo-file used by e-TeX `\scantokens`.
    #[must_use]
    pub fn scantokens(input: impl Into<String>) -> Self {
        let mut source = Self::new(input);
        source.scantokens = true;
        source
    }

    /// Reopens an editor buffer at a validated physical-line boundary.
    #[must_use]
    pub fn from_offset(input: impl Into<String>, next_offset: usize) -> Self {
        let input = input.into();
        assert!(next_offset <= input.len() && input.is_char_boundary(next_offset));
        Self {
            backing: Arc::from(input.as_bytes()),
            next_offset,
            scantokens: false,
        }
    }
}

impl InputSource for MemoryInput {
    fn read_line(&mut self) -> Result<Option<PhysicalLine>, InputSourceError> {
        Ok(next_physical_line(&self.backing, &mut self.next_offset))
    }

    fn source_descriptor(&self) -> Option<SourceDescriptor> {
        Some(SourceDescriptor::generated(Arc::clone(&self.backing)))
    }

    fn is_scantokens(&self) -> bool {
        self.scantokens
    }
}

/// Content-addressed input source created from `World` file content.
#[derive(Debug)]
pub struct WorldInput {
    input_record: Option<InputRecordId>,
    backing: Arc<[u8]>,
    next_offset: usize,
    invalid_utf8: Option<(usize, usize, usize, usize)>,
    scantokens: bool,
}

impl WorldInput {
    #[must_use]
    pub fn from_content(content: FileContent) -> Self {
        let input_record = content.record();
        Self::from_bytes(input_record, content.shared_bytes(), 0)
    }

    #[must_use]
    pub fn generated(input: impl Into<String>) -> Self {
        let input = input.into();
        Self {
            input_record: None,
            backing: Arc::from(input.as_bytes()),
            next_offset: 0,
            invalid_utf8: None,
            scantokens: false,
        }
    }

    #[must_use]
    pub fn from_content_after_lines(content: FileContent, lines_read: usize) -> Self {
        let input_record = content.record();
        Self::from_bytes(input_record, content.shared_bytes(), lines_read)
    }

    /// Reopens pinned content at a checkpoint's next physical byte offset.
    #[must_use]
    pub fn from_content_at_offset(content: FileContent, next_offset: usize) -> Self {
        let input_record = content.record();
        let mut source = Self::from_bytes(input_record, content.shared_bytes(), 0);
        assert!(next_offset <= source.backing.len());
        source.next_offset = next_offset;
        source
    }

    fn from_bytes(input_record: InputRecordId, bytes: Arc<[u8]>, lines_read: usize) -> Self {
        match std::str::from_utf8(&bytes) {
            Ok(_) => {
                let mut next_offset = 0;
                for _ in 0..lines_read {
                    if next_physical_line(&bytes, &mut next_offset).is_none() {
                        break;
                    }
                }
                Self {
                    input_record: Some(input_record),
                    backing: bytes,
                    next_offset,
                    invalid_utf8: None,
                    scantokens: false,
                }
            }
            Err(error) => {
                let byte_start = error.valid_up_to();
                let byte_end = error
                    .error_len()
                    .map_or(bytes.len(), |len| byte_start.saturating_add(len));
                let valid = std::str::from_utf8(&bytes[..byte_start])
                    .expect("valid_up_to prefix must be valid UTF-8");
                let line = valid.bytes().filter(|byte| *byte == b'\n').count() + 1;
                let column = valid
                    .rsplit_once('\n')
                    .map_or(valid, |(_, suffix)| suffix)
                    .trim_end_matches('\r')
                    .chars()
                    .count();
                Self {
                    input_record: Some(input_record),
                    backing: bytes,
                    next_offset: 0,
                    invalid_utf8: Some((byte_start, byte_end, line, column)),
                    scantokens: false,
                }
            }
        }
    }
}

impl InputSource for WorldInput {
    fn read_line(&mut self) -> Result<Option<PhysicalLine>, InputSourceError> {
        if let Some((byte_start, byte_end, line, column)) = self.invalid_utf8.take() {
            return Err(InputSourceError::InvalidUtf8 {
                byte_start,
                byte_end,
                line,
                column,
            });
        }
        Ok(next_physical_line(&self.backing, &mut self.next_offset))
    }

    fn input_record(&self) -> Option<InputRecordId> {
        self.input_record
    }

    fn source_descriptor(&self) -> Option<SourceDescriptor> {
        Some(self.input_record.map_or_else(
            || SourceDescriptor::generated(Arc::clone(&self.backing)),
            |record| {
                SourceDescriptor::world(
                    record,
                    u64::try_from(self.backing.len()).unwrap_or(u64::MAX),
                )
            },
        ))
    }

    fn is_scantokens(&self) -> bool {
        self.scantokens
    }
}

/// A TeX-normalized logical input line.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LineEvent {
    /// A line after trailing spaces were removed and `\endlinechar` was
    /// appended when it names a valid Unicode scalar value.
    Text(String),
}

/// Drives TeX line normalization for an input source.
#[derive(Debug)]
pub struct LineReader<S> {
    source: S,
}

/// Source-frame-local lexer state.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SourceFrame {
    state: LexerState,
    line: String,
    byte_offset: usize,
    pending: VecDeque<TracedTokenWord>,
    physical_line_start: usize,
    /// Fragment-relative base for this physical line. For ordinary sources it
    /// equals `physical_line_start`, keeping token construction branch-free.
    origin_line_start: u64,
    physical_content_end: usize,
    terminator_start: usize,
    terminator_end: usize,
    normalized_end_anchor: usize,
    synthetic_endline_start: Option<usize>,
    line_number: usize,
    column: usize,
    end_after_current_line: bool,
}

impl SourceFrame {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn state(&self) -> LexerState {
        self.state
    }

    #[must_use]
    pub fn offset(&self) -> usize {
        self.byte_offset
    }

    #[must_use]
    pub fn buffer_offset(&self) -> usize {
        self.physical_line_start
    }

    #[must_use]
    pub fn line_number(&self) -> usize {
        self.line_number
    }

    #[must_use]
    pub fn column(&self) -> usize {
        self.column
    }

    fn summary(&self, next_source_offset: usize) -> SourceFrameSummary {
        SourceFrameSummary::new_with_physical_metadata(
            self.physical_line_start,
            next_source_offset,
            self.line_number,
            self.column,
            self.state,
            self.line.as_str(),
            self.byte_offset,
            self.physical_content_end,
            self.terminator_start,
            self.terminator_end,
            self.normalized_end_anchor,
            self.synthetic_endline_start,
            self.pending.iter().copied().collect(),
            self.end_after_current_line,
        )
        .with_origin_line_start(self.origin_line_start)
    }

    fn from_summary(summary: &SourceFrameSummary) -> Self {
        assert!(
            summary.is_resume_complete(),
            "source frame summary must be complete enough to resume"
        );
        Self {
            state: summary.lexer_state(),
            line: summary.normalized_line().to_owned(),
            byte_offset: summary.line_byte_offset(),
            pending: summary.pending().iter().copied().collect(),
            physical_line_start: summary.buffer_offset(),
            origin_line_start: summary.origin_line_start(),
            physical_content_end: summary.physical_content_end(),
            terminator_start: summary.terminator_start(),
            terminator_end: summary.terminator_end(),
            normalized_end_anchor: summary.normalized_end_anchor(),
            synthetic_endline_start: summary.synthetic_endline_start(),
            line_number: summary.line_number(),
            column: summary.column(),
            end_after_current_line: summary.end_after_current_line(),
        }
    }
}

#[derive(Debug)]
struct SourceInputFrame {
    source_id: SourceId,
    input_record: Option<InputRecordId>,
    lines: LineReader<Box<dyn InputSource>>,
    frame: SourceFrame,
    next_source_offset: usize,
    descriptor: Option<SourceDescriptor>,
    registration_attempted: bool,
    registration: Option<RegisteredSource>,
    layout_cursor: Option<LayoutCursor>,
    scantokens: bool,
}

impl SourceInputFrame {
    fn new(source_id: SourceId, source: Box<dyn InputSource>) -> Self {
        let input_record = source.input_record();
        let descriptor = source.source_descriptor();
        let scantokens = source.is_scantokens();
        Self {
            source_id,
            input_record,
            lines: LineReader::new(source),
            frame: SourceFrame::new(),
            next_source_offset: 0,
            descriptor,
            registration_attempted: false,
            registration: None,
            layout_cursor: None,
            scantokens,
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
enum ReplayPayload {
    Stored {
        token_list: TokenListId,
        origin_list: OriginListId,
    },
    Transient {
        tokens: Vec<TracedTokenWord>,
    },
}

/// Pooled packed arguments owned by one live macro-body replay frame.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MacroArguments {
    tokens: Vec<TracedTokenWord>,
    slots: [Option<MacroArgumentRange>; MACRO_ARGUMENT_SLOTS],
}

impl MacroArguments {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            tokens: Vec::new(),
            slots: [None; MACRO_ARGUMENT_SLOTS],
        }
    }

    #[must_use]
    pub fn from_parts(
        tokens: Vec<TracedTokenWord>,
        slots: [Option<MacroArgumentRange>; MACRO_ARGUMENT_SLOTS],
    ) -> Self {
        for range in slots.iter().flatten().copied() {
            assert!(range.start().saturating_add(range.len()) <= tokens.len());
        }
        Self { tokens, slots }
    }

    fn from_summary(summary: &MacroArgumentsSummary) -> Self {
        Self::from_parts(summary.tokens().to_vec(), *summary.ranges())
    }

    fn summary(&self) -> MacroArgumentsSummary {
        MacroArgumentsSummary::from_parts(Arc::from(self.tokens.as_slice()), self.slots)
    }

    #[must_use]
    pub fn get(&self, slot: u8) -> Option<&[TracedTokenWord]> {
        let index = argument_index(slot);
        let range = self.slots[index]?;
        Some(&self.tokens[range.start()..range.start() + range.len()])
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.slots.iter().all(Option::is_none)
    }

    fn take_tokens(&mut self) -> Vec<TracedTokenWord> {
        std::mem::take(&mut self.tokens)
    }
}

fn argument_index(slot: u8) -> usize {
    assert!((1..=MACRO_ARGUMENT_SLOTS as u8).contains(&slot));
    usize::from(slot - 1)
}

#[derive(Debug, Eq, PartialEq)]
struct TokenListInputFrame {
    payload: ReplayPayload,
    replay_kind: TokenListReplayKind,
    index: usize,
    macro_arguments: MacroArguments,
    macro_invocation: OriginId,
    parent_macro_invocation: OriginId,
    replay_marker: Option<TokenListReplayMarker>,
}

impl TokenListInputFrame {
    fn stored_ids(&self) -> Option<(TokenListId, OriginListId)> {
        match self.payload {
            ReplayPayload::Stored {
                token_list,
                origin_list,
            } => Some((token_list, origin_list)),
            ReplayPayload::Transient { .. } => None,
        }
    }

    fn len(&self, stores: &impl ExpansionState) -> usize {
        match &self.payload {
            ReplayPayload::Stored { token_list, .. } => stores.tokens(*token_list).len(),
            ReplayPayload::Transient { tokens } => tokens.len(),
        }
    }

    fn semantic_token_at(&self, stores: &impl ExpansionState, index: usize) -> Option<Token> {
        match &self.payload {
            ReplayPayload::Stored { token_list, .. } => {
                stores.tokens(*token_list).get(index).copied()
            }
            ReplayPayload::Transient { tokens } => tokens.get(index)?.token(),
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct TokenListRetirement {
    replay_kind: TokenListReplayKind,
    macro_invocation: OriginId,
    parent_macro_invocation: OriginId,
}

impl From<&TokenListInputFrame> for TokenListRetirement {
    fn from(frame: &TokenListInputFrame) -> Self {
        Self {
            replay_kind: frame.replay_kind,
            macro_invocation: frame.macro_invocation,
            parent_macro_invocation: frame.parent_macro_invocation,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct MacroLiteralSpan {
    token_list: TokenListId,
    origin_list: OriginListId,
    replay_kind: TokenListReplayKind,
    start: usize,
    end: usize,
}

type LiteralSpanBounds = (usize, usize);
type LiteralSpanCache = AHashMap<(TokenListId, LiteralSpanPolicy), Arc<[LiteralSpanBounds]>>;

const LITERAL_SPAN_CACHE_MAX_ENTRIES: usize = 4096;
const TRANSIENT_BUFFER_POOL_MAX_ENTRIES: usize = 64;
const TRANSIENT_BUFFER_POOL_MAX_CAPACITY: usize = 4096;

/// Identifies one live token-list replay frame independently of its content.
///
/// The marker is intentionally absent from resumable input summaries: callers
/// use it only to delimit a synchronous replay operation on the current stack.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TokenListReplayMarker {
    sequence: u64,
    frame_index: usize,
}

#[derive(Debug)]
enum InputFrame {
    Source(SourceInputFrame),
    TokenList(TokenListInputFrame),
    Condition {
        token: ConditionFrameToken,
        condition: ConditionFrameSummary,
    },
}

#[derive(Debug)]
struct StableFrames {
    slots: Vec<Option<InputFrame>>,
    active: usize,
}

impl StableFrames {
    fn new() -> Self {
        Self {
            slots: Vec::new(),
            active: 0,
        }
    }
    fn from_vec(frames: Vec<InputFrame>) -> Self {
        let active = frames.len();
        Self {
            slots: frames.into_iter().map(Some).collect(),
            active,
        }
    }
    fn push(&mut self, frame: InputFrame) {
        self.slots.push(Some(frame));
        self.active += 1;
    }
    fn remove(&mut self, index: usize) -> InputFrame {
        self.active -= 1;
        let frame = self.slots[index].take().expect("input frame slot is live");
        while self.slots.last().is_some_and(Option::is_none) {
            self.slots.pop();
        }
        frame
    }
    fn len(&self) -> usize {
        self.active
    }
    fn slot_len(&self) -> usize {
        self.slots.len()
    }
    fn is_empty(&self) -> bool {
        self.active == 0
    }
    fn iter(&self) -> impl DoubleEndedIterator<Item = &InputFrame> {
        self.slots.iter().filter_map(Option::as_ref)
    }
    fn iter_mut(&mut self) -> impl DoubleEndedIterator<Item = &mut InputFrame> {
        self.slots.iter_mut().filter_map(Option::as_mut)
    }
    #[cfg(test)]
    fn last_mut(&mut self) -> Option<&mut InputFrame> {
        self.slots.iter_mut().rev().find_map(Option::as_mut)
    }
    fn iter_indexed_from(
        &self,
        start: usize,
    ) -> impl DoubleEndedIterator<Item = (usize, &InputFrame)> {
        self.slots[start..]
            .iter()
            .enumerate()
            .filter_map(move |(offset, frame)| frame.as_ref().map(|frame| (start + offset, frame)))
    }
    fn get(&self, index: usize) -> Option<&InputFrame> {
        self.slots.get(index)?.as_ref()
    }
}

impl Index<usize> for StableFrames {
    type Output = InputFrame;
    fn index(&self, index: usize) -> &Self::Output {
        self.slots[index]
            .as_ref()
            .expect("input frame slot is live")
    }
}
impl IndexMut<usize> for StableFrames {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        self.slots[index]
            .as_mut()
            .expect("input frame slot is live")
    }
}
impl<'a> IntoIterator for &'a mut StableFrames {
    type Item = &'a mut InputFrame;
    type IntoIter = std::iter::FilterMap<
        std::slice::IterMut<'a, Option<InputFrame>>,
        fn(&'a mut Option<InputFrame>) -> Option<&'a mut InputFrame>,
    >;
    fn into_iter(self) -> Self::IntoIter {
        self.slots.iter_mut().filter_map(Option::as_mut)
    }
}
impl IntoIterator for StableFrames {
    type Item = InputFrame;
    type IntoIter = std::iter::Flatten<std::vec::IntoIter<Option<InputFrame>>>;
    fn into_iter(self) -> Self::IntoIter {
        self.slots.into_iter().flatten()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct LastSourceFrame {
    source_id: SourceId,
    input_record: Option<InputRecordId>,
    registration: Option<RegisteredSource>,
    frame: SourceFrame,
    next_source_offset: usize,
}

enum TokenReplay {
    Deliver(Token),
    DeliverNoExpand(Token),
    PushArgument(u8),
}

enum TracedTokenReplay {
    Deliver(DecodedTracedToken),
    DeliverNoExpand(DecodedTracedToken),
    PushArgument(u8),
}

/// A validated token and origin kept decoded while crossing lexer hot paths.
/// Compact words remain the storage and snapshot representation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DecodedTracedToken {
    token: Token,
    origin: OriginId,
}

impl DecodedTracedToken {
    const fn new(token: Token, origin: OriginId) -> Self {
        Self { token, origin }
    }

    fn from_word(word: TracedTokenWord) -> Self {
        Self::new(decode_traced_token(word), word.origin())
    }

    fn packed(self) -> TracedTokenWord {
        TracedTokenWord::pack(self.token, self.origin)
    }
}

/// Which immutable macro-replay characters a direct span consumer accepts.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum LiteralSpanPolicy {
    /// Characters that are inert to an expanded replacement scanner.
    /// Group and parameter catcodes remain seams because that scanner must
    /// update its own brace/parameter state for them.
    ExpandedReplacement,
    /// Ordinary text characters accepted by horizontal main control.
    HorizontalText,
}

/// Feature-gated attribution counters and wall-clock timers for token-list
/// expansion delivery.
///
/// With `profiling-stats` disabled the input stack contains no counter field
/// and all snapshots returned by [`InputStack::expansion_stats`] are zero.
/// Timer totals are diagnostic extrapolations from sparse samples: exact event
/// counters remain available, while only one event in 1024 reads the wall
/// clock so profiling does not dominate the hot path. The estimates do not
/// partition all expansion work or the whole engine run.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ExpansionStats {
    pub token_frame_steps: u64,
    pub provenance_resolutions: u64,
    pub character_tokens: u64,
    pub meaning_lookups: u64,
    pub literal_spans: u64,
    pub literal_tokens: u64,
    pub segmentation_cache_hits: u64,
    pub segmentation_cache_misses: u64,
    pub builder_appends: u64,
    pub source_text_span_attempts: u64,
    pub source_text_spans: u64,
    pub source_text_tokens: u64,
    pub meaning_cache_hits: u64,
    pub meaning_cache_misses: u64,
    pub frame_step_nanos: u64,
    pub provenance_nanos: u64,
    pub classification_meaning_nanos: u64,
    pub builder_append_nanos: u64,
    pub frame_step_timer_samples: u64,
    pub provenance_timer_samples: u64,
    pub classification_meaning_timer_samples: u64,
    pub builder_append_timer_samples: u64,
    frame_step_timer_events: u64,
    provenance_timer_events: u64,
    classification_meaning_timer_events: u64,
    builder_append_timer_events: u64,
}

#[cfg(feature = "profiling-stats")]
const EXPANSION_TIMER_SAMPLE_MASK: u64 = 1023;

#[cfg(feature = "profiling-stats")]
fn duration_nanos_saturating(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}

#[cfg(feature = "profiling-stats")]
fn add_elapsed(total: &mut u64, started: Instant) {
    let sampled = duration_nanos_saturating(started.elapsed());
    *total = total.saturating_add(sampled.saturating_mul(EXPANSION_TIMER_SAMPLE_MASK + 1));
}

#[cfg(feature = "profiling-stats")]
fn should_sample_timer(events: &mut u64) -> bool {
    let sample = *events & EXPANSION_TIMER_SAMPLE_MASK == 0;
    *events = events.wrapping_add(1);
    sample
}

impl ExpansionStats {
    #[must_use]
    pub fn attributed_nanos(self) -> u64 {
        self.frame_step_nanos
            .saturating_add(self.provenance_nanos)
            .saturating_add(self.classification_meaning_nanos)
            .saturating_add(self.builder_append_nanos)
    }

    #[must_use]
    pub fn character_fraction(self) -> f64 {
        if self.token_frame_steps == 0 {
            0.0
        } else {
            self.character_tokens as f64 / self.token_frame_steps as f64
        }
    }

    #[must_use]
    pub fn mean_literal_run(self) -> f64 {
        if self.literal_spans == 0 {
            0.0
        } else {
            self.literal_tokens as f64 / self.literal_spans as f64
        }
    }

    #[must_use]
    pub fn mean_source_text_run(self) -> f64 {
        if self.source_text_spans == 0 {
            0.0
        } else {
            self.source_text_tokens as f64 / self.source_text_spans as f64
        }
    }
}

impl LiteralSpanPolicy {
    #[inline(always)]
    fn accepts(self, token: Token) -> bool {
        match (self, token) {
            (
                Self::ExpandedReplacement,
                Token::Char {
                    cat:
                        Catcode::BeginGroup | Catcode::EndGroup | Catcode::Parameter | Catcode::Active,
                    ..
                },
            ) => false,
            (Self::ExpandedReplacement, Token::Char { .. }) => true,
            (
                Self::HorizontalText,
                Token::Char {
                    cat: Catcode::Letter | Catcode::Other,
                    ..
                },
            ) => true,
            (
                Self::ExpandedReplacement | Self::HorizontalText,
                Token::Cs(_) | Token::Param(_) | Token::Frozen(_),
            )
            | (Self::HorizontalText, Token::Char { .. }) => false,
        }
    }
}

/// A token read from the input stack with expansion-control metadata.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExpansionToken {
    token: Token,
    suppress_expansion: bool,
}

impl ExpansionToken {
    #[must_use]
    pub const fn new(token: Token, suppress_expansion: bool) -> Self {
        Self {
            token,
            suppress_expansion,
        }
    }

    #[must_use]
    pub const fn token(self) -> Token {
        self.token
    }

    #[must_use]
    pub const fn suppress_expansion(self) -> bool {
        self.suppress_expansion
    }
}

/// A traced token read from the input stack with expansion-control metadata.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TracedExpansionToken {
    token: Token,
    origin: OriginId,
    suppress_expansion: bool,
    expand_for_command_demand: bool,
    macro_replay_site: Option<MacroReplaySite>,
}

/// Lexical location of a token delivered directly from immutable macro-body
/// replay. Meaning interpretation and caching belong to `tex-expand`; this
/// value carries only replay identity and position.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct MacroReplaySite {
    token_list: TokenListId,
    token_index: usize,
}

impl MacroReplaySite {
    #[must_use]
    pub const fn token_list(self) -> TokenListId {
        self.token_list
    }

    #[must_use]
    pub const fn token_index(self) -> usize {
        self.token_index
    }
}

impl TracedExpansionToken {
    #[must_use]
    pub fn new(token: TracedTokenWord, suppress_expansion: bool) -> Self {
        Self::from_decoded(
            DecodedTracedToken::from_word(token),
            suppress_expansion,
            false,
            None,
        )
    }

    fn from_decoded(
        token: DecodedTracedToken,
        suppress_expansion: bool,
        expand_for_command_demand: bool,
        macro_replay_site: Option<MacroReplaySite>,
    ) -> Self {
        Self {
            token: token.token,
            origin: token.origin,
            suppress_expansion,
            expand_for_command_demand,
            macro_replay_site,
        }
    }

    #[must_use]
    pub fn traced_token(self) -> TracedTokenWord {
        TracedTokenWord::pack(self.token, self.origin)
    }

    #[must_use]
    pub const fn token(self) -> Token {
        self.token
    }

    #[must_use]
    pub const fn origin(self) -> OriginId {
        self.origin
    }

    #[must_use]
    pub const fn suppress_expansion(self) -> bool {
        self.suppress_expansion
    }

    /// Returns whether nested command demand should resume expansion.
    #[must_use]
    pub const fn expand_for_command_demand(self) -> bool {
        self.expand_for_command_demand
    }

    #[must_use]
    pub const fn macro_replay_site(self) -> Option<MacroReplaySite> {
        self.macro_replay_site
    }
}

/// TeX input stack for source frames and frozen token-list replay.
#[derive(Debug)]
pub struct InputStack {
    frames: StableFrames,
    source_frame_count: usize,
    token_frame_indices: Vec<usize>,
    condition_frame_indices: Vec<usize>,
    next_source_id: u32,
    unicode_superscript_notation: bool,
    last_source_frame: Option<LastSourceFrame>,
    next_replay_marker: u64,
    next_condition_token: u64,
    alignment_inputs: Vec<AlignmentInput>,
    /// Derived, discardable segmentation of immutable macro token lists.
    literal_span_cache: LiteralSpanCache,
    transient_buffer_pool: Vec<Vec<TracedTokenWord>>,
    #[cfg(feature = "profiling-stats")]
    expansion_stats: ExpansionStats,
    active_macro_invocation: OriginId,
    recently_popped_invocation: Option<OriginId>,
}

/// Proof that one token was delivered directly from a physical source frame.
/// Fields are private so replayed or expanded tokens cannot manufacture it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DirectSourceDelivery {
    token: TracedTokenWord,
    source: SourceId,
    start: u64,
    end: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AlignmentCellPhase {
    UTemplate(TokenListReplayMarker),
    Body,
    VTemplate,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AlignmentTerminator {
    Tab,
    Cr,
    Span,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AlignmentTokenDelivery {
    Other,
    LeftBrace,
    RightBrace,
}

#[derive(Clone, Debug)]
struct AlignmentCellInput {
    phase: AlignmentCellPhase,
    v_template: TokenListId,
    terminator: Option<TracedTokenWord>,
}

#[derive(Clone, Debug)]
struct AlignmentInput {
    align_state: i32,
    cell: Option<AlignmentCellInput>,
}

/// Saved alignment-cell interception state while a nested preamble and body run.
///
/// Like TeX82's alignment-stack node, this value owns the exact outer state;
/// nested input cannot observe or replace it before the matching restore.
#[must_use]
pub struct AlignmentCellSuspension(Option<AlignmentInput>);

impl InputStack {
    /// Constructs a stack with no physical source frames.
    ///
    /// This is used for synchronous expansion of an already-frozen token
    /// list. Any `\input` encountered during that replay pushes an ordinary
    /// erased source frame through the installed resolver.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            frames: StableFrames::new(),
            source_frame_count: 0,
            token_frame_indices: Vec::new(),
            condition_frame_indices: Vec::new(),
            next_source_id: 0,
            unicode_superscript_notation: true,
            last_source_frame: None,
            next_replay_marker: 0,
            next_condition_token: 0,
            alignment_inputs: Vec::new(),
            literal_span_cache: AHashMap::new(),
            transient_buffer_pool: Vec::new(),
            #[cfg(feature = "profiling-stats")]
            expansion_stats: ExpansionStats::default(),
            active_macro_invocation: OriginId::UNKNOWN,
            recently_popped_invocation: None,
        }
    }

    /// Rebases a fresh, not-yet-registered stack into the aggregate source-id
    /// domain used by earlier execution runs.
    pub fn ensure_source_ids_at_least(&mut self, minimum: u32) {
        if minimum == 0
            || self.frames.iter().any(|frame| {
                matches!(frame, InputFrame::Source(source) if source.registration_attempted)
            })
        {
            return;
        }
        let delta = minimum;
        for frame in &mut self.frames {
            if let InputFrame::Source(source) = frame {
                assert!(
                    source.registration.is_none() && !source.registration_attempted,
                    "cannot rebase an already-registered input stack"
                );
                source.source_id = SourceId::new(
                    source
                        .source_id
                        .raw()
                        .checked_add(delta)
                        .expect("source id counter overflowed"),
                );
            }
        }
        if let Some(source) = &mut self.last_source_frame {
            assert!(
                source.registration.is_none(),
                "cannot rebase an already-registered last source"
            );
            source.source_id = SourceId::new(
                source
                    .source_id
                    .raw()
                    .checked_add(delta)
                    .expect("source id counter overflowed"),
            );
        }
        self.next_source_id = self
            .next_source_id
            .checked_add(delta)
            .expect("source id counter overflowed");
    }

    #[must_use]
    pub fn new<S>(source: S) -> Self
    where
        S: InputSource + 'static,
    {
        let mut stack = Self::empty();
        stack.push_source(source);
        stack
    }

    pub fn push_source<S>(&mut self, source: S) -> SourceId
    where
        S: InputSource + 'static,
    {
        self.push_boxed_source(Box::new(source))
    }

    /// Pushes an erased source returned by an input resolver.
    pub fn push_boxed_source(&mut self, source: Box<dyn InputSource>) -> SourceId {
        let source_id = SourceId::new(self.next_source_id);
        self.next_source_id = self
            .next_source_id
            .checked_add(1)
            .expect("source id counter overflowed");
        self.push_frame(InputFrame::Source(SourceInputFrame::new(source_id, source)));
        source_id
    }

    /// Installs the frozen editor layout on the root physical source without
    /// changing its session-stable `SourceId`.
    pub fn install_root_layout_cursor(&mut self, mut cursor: LayoutCursor) -> Option<SourceId> {
        let source = self.frames.iter_mut().find_map(|frame| match frame {
            InputFrame::Source(source) => Some(source),
            InputFrame::TokenList(_) | InputFrame::Condition { .. } => None,
        })?;
        cursor.seek(source.frame.physical_line_start);
        source.registration = if source.frame.line_number == 0 {
            cursor.root_registration
        } else {
            let (registration, origin_line_start) = cursor
                .line_registration(
                    source.frame.physical_line_start,
                    source.frame.terminator_end,
                )
                .expect("restored editor line must be contained by one layout piece");
            source.frame.origin_line_start = origin_line_start;
            Some(registration)
        };
        source.descriptor = None;
        source.registration_attempted = true;
        source.layout_cursor = Some(cursor);
        Some(source.source_id)
    }

    pub fn from_summary<E, F, S>(summary: &InputSummary, mut reopen_source: F) -> Result<Self, E>
    where
        S: InputSource + 'static,
        F: FnMut(SourceId, Option<InputRecordId>, &SourceFrameSummary) -> Result<S, E>,
    {
        let mut frames = Vec::with_capacity(summary.frames().len());
        for frame in summary.frames() {
            match frame {
                InputFrameSummary::Source {
                    source_id,
                    input_record,
                    source,
                } => {
                    let reopened = reopen_source(*source_id, *input_record, source)?;
                    let descriptor = reopened.source_descriptor();
                    let reopened: Box<dyn InputSource> = Box::new(reopened);
                    frames.push(InputFrame::Source(SourceInputFrame {
                        source_id: *source_id,
                        input_record: *input_record,
                        lines: LineReader::new(reopened),
                        frame: SourceFrame::from_summary(source),
                        next_source_offset: source.next_source_offset(),
                        descriptor,
                        registration_attempted: source.registration().is_some(),
                        registration: source.registration(),
                        layout_cursor: None,
                        scantokens: source.is_scantokens(),
                    }));
                }
                InputFrameSummary::TokenList {
                    token_list,
                    origin_list,
                    replay_kind,
                    index,
                    macro_arguments,
                    macro_invocation,
                    parent_macro_invocation,
                } => frames.push(InputFrame::TokenList(TokenListInputFrame {
                    payload: ReplayPayload::Stored {
                        token_list: *token_list,
                        origin_list: *origin_list,
                    },
                    replay_kind: *replay_kind,
                    index: *index,
                    macro_arguments: MacroArguments::from_summary(macro_arguments),
                    macro_invocation: *macro_invocation,
                    parent_macro_invocation: *parent_macro_invocation,
                    replay_marker: None,
                })),
                InputFrameSummary::TransientTokenList {
                    tokens,
                    replay_kind,
                    macro_invocation,
                    parent_macro_invocation,
                } => frames.push(InputFrame::TokenList(TokenListInputFrame {
                    payload: ReplayPayload::Transient {
                        tokens: tokens.to_vec(),
                    },
                    replay_kind: *replay_kind,
                    index: 0,
                    macro_arguments: MacroArguments::new(),
                    macro_invocation: *macro_invocation,
                    parent_macro_invocation: *parent_macro_invocation,
                    replay_marker: None,
                })),
                InputFrameSummary::Condition { token, condition } => {
                    frames.push(InputFrame::Condition {
                        token: *token,
                        condition: *condition,
                    });
                }
            }
        }

        let active_macro_invocation = frames
            .iter()
            .rev()
            .find_map(|frame| match frame {
                InputFrame::TokenList(frame) if frame.macro_invocation != OriginId::UNKNOWN => {
                    Some(frame.macro_invocation)
                }
                InputFrame::Source(_) | InputFrame::TokenList(_) | InputFrame::Condition { .. } => {
                    None
                }
            })
            .unwrap_or(OriginId::UNKNOWN);

        let source_frame_count = frames
            .iter()
            .filter(|frame| matches!(frame, InputFrame::Source(_)))
            .count();
        let token_frame_indices = frames
            .iter()
            .enumerate()
            .filter_map(|(index, frame)| {
                matches!(frame, InputFrame::Source(_) | InputFrame::TokenList(_)).then_some(index)
            })
            .collect();
        let condition_frame_indices = frames
            .iter()
            .enumerate()
            .filter_map(|(index, frame)| {
                matches!(frame, InputFrame::Condition { .. }).then_some(index)
            })
            .collect();
        Ok(Self {
            frames: StableFrames::from_vec(frames),
            source_frame_count,
            token_frame_indices,
            condition_frame_indices,
            next_source_id: summary.next_source_id(),
            unicode_superscript_notation: summary.unicode_superscript_notation(),
            last_source_frame: summary.last_source_frame().map(|source| LastSourceFrame {
                source_id: summary
                    .last_source_id()
                    .expect("last source frame must retain its source id"),
                input_record: summary.last_source_record(),
                registration: source.registration(),
                frame: SourceFrame::from_summary(source),
                next_source_offset: source.next_source_offset(),
            }),
            next_replay_marker: 0,
            next_condition_token: summary
                .frames()
                .iter()
                .filter_map(|frame| match frame {
                    InputFrameSummary::Condition { token, .. } => Some(token.raw()),
                    InputFrameSummary::Source { .. }
                    | InputFrameSummary::TokenList { .. }
                    | InputFrameSummary::TransientTokenList { .. } => None,
                })
                .max()
                .map_or(0, |token| {
                    token
                        .checked_add(1)
                        .expect("condition frame token overflowed")
                }),
            alignment_inputs: Vec::new(),
            literal_span_cache: AHashMap::new(),
            transient_buffer_pool: Vec::new(),
            #[cfg(feature = "profiling-stats")]
            expansion_stats: ExpansionStats::default(),
            active_macro_invocation,
            recently_popped_invocation: None,
        })
    }

    /// Starts one TeX82 alignment-scanner level before `scan_spec`.
    pub fn begin_alignment(&mut self) {
        self.alignment_inputs.push(AlignmentInput {
            align_state: -1_000_000,
            cell: None,
        });
    }

    /// Completes the active alignment-scanner level after `fin_align`.
    pub fn finish_alignment(&mut self) {
        let alignment = self
            .alignment_inputs
            .pop()
            .expect("alignment input level must be active");
        assert!(alignment.cell.is_none(), "alignment cell remained active");
    }

    /// Matches the sentinel assignments in TeX82's `align_peek` and preamble.
    pub fn set_alignment_state(&mut self, state: i32) {
        if let Some(alignment) = self.alignment_inputs.last_mut() {
            alignment.align_state = state;
        }
    }

    #[must_use]
    pub fn alignment_state_is(&self, state: i32) -> bool {
        self.alignment_inputs
            .last()
            .is_some_and(|alignment| alignment.align_state == state)
    }

    /// Starts TeX82's `get_next` alignment-cell interception.
    ///
    /// The u-template marker is an exact live-frame boundary: once it is
    /// retired, brace accounting begins on the first cell token. A top-level
    /// tab, `\span`, or `\cr` is retained with its traced origin while the
    /// v-template is inserted ahead of it, just as TeX82 does in `get_next`.
    pub fn begin_alignment_cell(
        &mut self,
        u_template: Option<TokenListReplayMarker>,
        v_template: TokenListId,
        _group_depth: u32,
    ) {
        let alignment = self
            .alignment_inputs
            .last_mut()
            .expect("alignment input level must be active");
        assert!(alignment.cell.is_none(), "alignment cell already active");
        if u_template.is_none() {
            alignment.align_state = 0;
        }
        alignment.cell = Some(AlignmentCellInput {
            phase: u_template.map_or(AlignmentCellPhase::Body, AlignmentCellPhase::UTemplate),
            v_template,
            terminator: None,
        });
    }

    /// Completes the active cell after its frozen end-v token is delivered.
    pub fn finish_alignment_cell(&mut self) -> Option<TracedTokenWord> {
        let cell = self.alignment_inputs.last_mut()?.cell.take()?;
        assert_eq!(cell.phase, AlignmentCellPhase::VTemplate);
        cell.terminator
    }

    /// Completes a cell whose terminator was already intercepted when later
    /// recovery consumed the synthetic end-v marker.
    pub fn finish_terminating_alignment_cell(&mut self) -> Option<TracedTokenWord> {
        if self.alignment_inputs.last().is_some_and(|alignment| {
            alignment
                .cell
                .as_ref()
                .is_some_and(|cell| cell.phase == AlignmentCellPhase::VTemplate)
        }) {
            return self.alignment_inputs.last_mut()?.cell.take()?.terminator;
        }
        None
    }

    #[must_use]
    pub fn has_active_alignment_cell(&self) -> bool {
        self.alignment_inputs
            .last()
            .is_some_and(|alignment| alignment.cell.is_some())
    }

    /// Whether any alignment scanner level can observe token delivery.
    ///
    /// Callers use this coarse predicate before classifying a token or
    /// resolving its meaning for alignment interception. The overwhelmingly
    /// common non-alignment path therefore pays one empty-vector test and no
    /// alignment-specific semantic work.
    #[must_use]
    #[inline(always)]
    pub fn has_active_alignment(&self) -> bool {
        !self.alignment_inputs.is_empty()
    }

    /// Returns a point-in-time copy of feature-gated expansion counters.
    #[must_use]
    pub fn expansion_stats(&self) -> ExpansionStats {
        #[cfg(feature = "profiling-stats")]
        {
            self.expansion_stats
        }
        #[cfg(not(feature = "profiling-stats"))]
        {
            ExpansionStats::default()
        }
    }

    /// Records a semantic meaning lookup performed by the expansion layer.
    #[inline(always)]
    pub fn record_expansion_meaning_lookup(&mut self) {
        #[cfg(feature = "profiling-stats")]
        {
            self.expansion_stats.meaning_lookups += 1;
        }
    }

    #[inline(always)]
    pub fn record_expansion_meaning_cache_hit(&mut self) {
        #[cfg(feature = "profiling-stats")]
        {
            self.expansion_stats.meaning_cache_hits += 1;
        }
    }

    #[inline(always)]
    pub fn record_expansion_meaning_cache_miss(&mut self) {
        #[cfg(feature = "profiling-stats")]
        {
            self.expansion_stats.meaning_cache_misses += 1;
        }
    }

    #[cfg(feature = "profiling-stats")]
    #[inline(always)]
    pub fn should_sample_expansion_meaning_timer(&mut self) -> bool {
        should_sample_timer(&mut self.expansion_stats.classification_meaning_timer_events)
    }

    #[inline(always)]
    pub fn record_expansion_meaning_resolution_nanos(&mut self, elapsed: u64) {
        #[cfg(feature = "profiling-stats")]
        {
            self.expansion_stats.classification_meaning_nanos = self
                .expansion_stats
                .classification_meaning_nanos
                .saturating_add(elapsed.saturating_mul(EXPANSION_TIMER_SAMPLE_MASK + 1));
            self.expansion_stats.classification_meaning_timer_samples += 1;
        }
        #[cfg(not(feature = "profiling-stats"))]
        let _ = elapsed;
    }

    #[must_use]
    pub fn alignment_cell_at_base_depth(&self) -> bool {
        self.alignment_inputs.last().is_some_and(|alignment| {
            alignment
                .cell
                .as_ref()
                .is_some_and(|cell| cell.phase == AlignmentCellPhase::Body)
                && alignment.align_state == 0
        })
    }

    #[must_use]
    pub fn alignment_cell_below_base_depth(&self) -> bool {
        self.alignment_inputs.last().is_some_and(|alignment| {
            alignment
                .cell
                .as_ref()
                .is_some_and(|cell| cell.phase == AlignmentCellPhase::Body)
                && alignment.align_state < 0
        })
    }

    /// Suspends an outer cell while a nested alignment scans and executes.
    ///
    /// TeX's `push_alignment` preserves the complete alignment scanner state;
    /// a nested alignment can begin while an outer u- or v-template is still
    /// replaying, not only from the cell body.
    pub fn suspend_alignment_cell(&mut self) -> AlignmentCellSuspension {
        AlignmentCellSuspension(self.alignment_inputs.pop())
    }

    pub fn resume_alignment_cell(&mut self, suspended: AlignmentCellSuspension) {
        assert!(
            self.alignment_inputs.is_empty(),
            "nested alignment input remained active at pop_alignment"
        );
        if let Some(alignment) = suspended.0 {
            self.alignment_inputs.push(alignment);
        }
    }

    /// Unwinds nested interception state and restores a suspended outer cell.
    #[doc(hidden)]
    pub fn abort_alignment_and_resume(&mut self, suspended: AlignmentCellSuspension) {
        self.alignment_inputs.clear();
        if let Some(alignment) = suspended.0 {
            self.alignment_inputs.push(alignment);
        }
    }

    /// Applies the alignment-sensitive part of TeX82 `get_next`.
    ///
    /// Returns `true` when the token was a cell terminator and has been
    /// replaced in the input by the active v-template.
    pub fn intercept_alignment_token(
        &mut self,
        traced: TracedTokenWord,
        delivery: AlignmentTokenDelivery,
        terminator: Option<AlignmentTerminator>,
        _group_depth: u32,
    ) -> bool {
        let retired_u_template = self
            .alignment_inputs
            .last()
            .and_then(|alignment| alignment.cell.as_ref())
            .and_then(|cell| match cell.phase {
                AlignmentCellPhase::UTemplate(marker) => Some(marker),
                AlignmentCellPhase::Body | AlignmentCellPhase::VTemplate => None,
            })
            .is_some_and(|marker| !self.contains_token_list_replay_marker(marker));
        let Some(alignment) = self.alignment_inputs.last_mut() else {
            return false;
        };
        if retired_u_template && alignment.align_state > 500_000 {
            alignment.align_state = 0;
        }
        match delivery {
            AlignmentTokenDelivery::Other => {}
            AlignmentTokenDelivery::LeftBrace => alignment.align_state += 1,
            AlignmentTokenDelivery::RightBrace => alignment.align_state -= 1,
        }
        let Some(cell) = alignment.cell.as_mut() else {
            return false;
        };
        if retired_u_template {
            cell.phase = AlignmentCellPhase::Body;
        }
        if cell.phase != AlignmentCellPhase::Body {
            return false;
        }
        let terminates = alignment.align_state == 0 && terminator.is_some();
        let v_template = if terminates {
            cell.phase = AlignmentCellPhase::VTemplate;
            cell.terminator = Some(traced);
            Some(cell.v_template)
        } else {
            None
        };
        if let Some(v_template) = v_template {
            self.push_token_list(v_template, TokenListReplayKind::Inserted);
        }
        terminates
    }

    /// Reverses the alignment brace accounting for a token that was consumed
    /// in a context where TeX explicitly cancels `get_next`'s adjustment.
    pub fn undo_alignment_token_delivery(&mut self, traced: TracedTokenWord) {
        let Some(alignment) = self.alignment_inputs.last_mut() else {
            return;
        };
        match traced.token() {
            Some(Token::Char {
                cat: Catcode::BeginGroup,
                ..
            }) => alignment.align_state -= 1,
            Some(Token::Char {
                cat: Catcode::EndGroup,
                ..
            }) => alignment.align_state += 1,
            Some(Token::Char { .. } | Token::Cs(_) | Token::Param(_) | Token::Frozen(_)) | None => {
            }
        }
    }

    pub fn back_input_alignment_token(&mut self, traced: TracedTokenWord) {
        self.undo_alignment_token_delivery(traced);
    }

    pub fn push_token_list(
        &mut self,
        token_list: TokenListId,
        replay_kind: TokenListReplayKind,
    ) -> TokenListReplayMarker {
        self.push_token_list_with_origins(token_list, OriginListId::EMPTY, replay_kind)
    }

    pub fn rewind_current_token_list_frame(&mut self) -> bool {
        let Some(index) = self.current_token_frame_index() else {
            return false;
        };
        let InputFrame::TokenList(frame) = &mut self.frames[index] else {
            return false;
        };
        let Some(previous) = frame.index.checked_sub(1) else {
            return false;
        };
        frame.index = previous;
        true
    }

    pub fn push_current_source_pending(&mut self, token: TracedTokenWord) -> bool {
        let Some(index) = self.current_token_frame_index() else {
            return false;
        };
        let InputFrame::Source(source) = &mut self.frames[index] else {
            return false;
        };
        source.frame.pending.push_front(token);
        true
    }

    pub fn push_token_list_with_origins(
        &mut self,
        token_list: TokenListId,
        origin_list: OriginListId,
        replay_kind: TokenListReplayKind,
    ) -> TokenListReplayMarker {
        let replay_marker = TokenListReplayMarker {
            sequence: self.next_replay_marker,
            frame_index: self.frames.slot_len(),
        };
        self.next_replay_marker = self
            .next_replay_marker
            .checked_add(1)
            .expect("token-list replay marker overflowed");
        self.push_frame(InputFrame::TokenList(TokenListInputFrame {
            payload: ReplayPayload::Stored {
                token_list,
                origin_list,
            },
            replay_kind,
            index: 0,
            macro_arguments: MacroArguments::new(),
            macro_invocation: OriginId::UNKNOWN,
            parent_macro_invocation: OriginId::UNKNOWN,
            replay_marker: Some(replay_marker),
        }));
        replay_marker
    }

    /// Takes a cleared packed-token buffer from the replay pool.
    ///
    /// Callers fill the buffer and transfer it back with
    /// [`Self::push_transient_tokens`] or [`Self::recycle_transient_token_buffer`].
    pub fn take_transient_token_buffer(&mut self) -> Vec<TracedTokenWord> {
        self.transient_buffer_pool.pop().unwrap_or_default()
    }

    /// Returns an unused packed-token buffer to the bounded replay pool.
    pub fn recycle_transient_token_buffer(&mut self, mut tokens: Vec<TracedTokenWord>) {
        tokens.clear();
        if tokens.capacity() > 0
            && tokens.capacity() <= TRANSIENT_BUFFER_POOL_MAX_CAPACITY
            && self.transient_buffer_pool.len() < TRANSIENT_BUFFER_POOL_MAX_ENTRIES
        {
            self.transient_buffer_pool.push(tokens);
        }
    }

    /// Pushes execution-local traced tokens without publishing durable list ids.
    pub fn push_transient_tokens(
        &mut self,
        tokens: Vec<TracedTokenWord>,
        replay_kind: TokenListReplayKind,
    ) -> TokenListReplayMarker {
        let replay_marker = TokenListReplayMarker {
            sequence: self.next_replay_marker,
            frame_index: self.frames.slot_len(),
        };
        self.next_replay_marker = self
            .next_replay_marker
            .checked_add(1)
            .expect("token-list replay marker overflowed");
        self.push_frame(InputFrame::TokenList(TokenListInputFrame {
            payload: ReplayPayload::Transient { tokens },
            replay_kind,
            index: 0,
            macro_arguments: MacroArguments::new(),
            macro_invocation: OriginId::UNKNOWN,
            parent_macro_invocation: OriginId::UNKNOWN,
            replay_marker: Some(replay_marker),
        }));
        replay_marker
    }

    pub fn push_macro_body(&mut self, token_list: TokenListId, macro_arguments: MacroArguments) {
        self.push_macro_body_with_origins(token_list, OriginListId::EMPTY, macro_arguments);
    }

    pub fn push_macro_body_with_origins(
        &mut self,
        token_list: TokenListId,
        origin_list: OriginListId,
        macro_arguments: MacroArguments,
    ) {
        self.push_macro_body_with_origins_and_invocation(
            token_list,
            origin_list,
            macro_arguments,
            OriginId::UNKNOWN,
        );
    }

    pub fn push_macro_body_with_origins_and_invocation(
        &mut self,
        token_list: TokenListId,
        origin_list: OriginListId,
        macro_arguments: MacroArguments,
        macro_invocation: OriginId,
    ) {
        let parent_macro_invocation = self.active_macro_invocation;
        self.push_frame(InputFrame::TokenList(TokenListInputFrame {
            payload: ReplayPayload::Stored {
                token_list,
                origin_list,
            },
            replay_kind: TokenListReplayKind::MacroBody,
            index: 0,
            macro_arguments,
            macro_invocation,
            parent_macro_invocation,
            replay_marker: None,
        }));
        if macro_invocation != OriginId::UNKNOWN {
            self.active_macro_invocation = macro_invocation;
        }
    }

    /// Copies the next maximal literal span from macro-body/argument replay.
    ///
    /// This is deliberately a builder-oriented API: it advances the live
    /// replay frame and copies semantic tokens and provenance side by side,
    /// without manufacturing a second traced-token buffer. Parameter slots
    /// still push their frozen argument frame, while control sequences,
    /// active characters, scanner-sensitive characters, and stale/absent
    /// argument provenance deopt to ordinary per-token replay.
    pub fn append_macro_literal_span(
        &mut self,
        stores: &impl ExpansionState,
        tokens_out: &mut TokenListBuilder,
        origins_out: &mut OriginListBuilder,
        policy: LiteralSpanPolicy,
    ) -> usize {
        let Some(span) = self.take_macro_literal_span(stores, policy) else {
            return 0;
        };
        #[cfg(feature = "profiling-stats")]
        let started = should_sample_timer(&mut self.expansion_stats.builder_append_timer_events)
            .then(Instant::now);
        let stored = stores.tokens(span.token_list);
        tokens_out.extend_from_slice(&stored[span.start..span.end]);
        if span.origin_list == OriginListId::EMPTY {
            debug_assert_eq!(span.replay_kind, TokenListReplayKind::MacroBody);
            origins_out.extend_repeated(OriginId::UNKNOWN, span.end - span.start);
        } else if let Some(origins) = stores.origin_list_if_live(span.origin_list) {
            assert_eq!(origins.len(), stored.len());
            origins_out.extend_from_slice(&origins[span.start..span.end]);
        } else {
            origins_out.extend_repeated(OriginId::UNKNOWN, span.end - span.start);
        }
        #[cfg(feature = "profiling-stats")]
        if let Some(started) = started {
            add_elapsed(&mut self.expansion_stats.builder_append_nanos, started);
            self.expansion_stats.builder_append_timer_samples += 1;
        }
        self.record_literal_span(span.end - span.start, true);
        span.end - span.start
    }

    /// Appends the next maximal ordinary horizontal-text span.
    ///
    /// Tokens and their existing origins are copied side by side so execution
    /// can retain source provenance without giving up the batched path.
    pub fn append_macro_text_span(
        &mut self,
        stores: &impl ExpansionState,
        tokens_out: &mut Vec<TracedTokenWord>,
    ) -> usize {
        let Some(span) = self.take_macro_literal_span(stores, LiteralSpanPolicy::HorizontalText)
        else {
            return 0;
        };
        #[cfg(feature = "profiling-stats")]
        let started = should_sample_timer(&mut self.expansion_stats.builder_append_timer_events)
            .then(Instant::now);
        let stored = stores.tokens(span.token_list);
        let origins = (span.origin_list != OriginListId::EMPTY)
            .then(|| stores.origin_list_if_live(span.origin_list))
            .flatten();
        for (offset, &token) in stored[span.start..span.end].iter().enumerate() {
            let origin = origins
                .and_then(|origins| origins.get(span.start + offset).copied())
                .unwrap_or(OriginId::UNKNOWN);
            tokens_out.push(TracedTokenWord::pack(token, origin));
        }
        #[cfg(feature = "profiling-stats")]
        if let Some(started) = started {
            add_elapsed(&mut self.expansion_stats.builder_append_nanos, started);
            self.expansion_stats.builder_append_timer_samples += 1;
        }
        self.record_literal_span(span.end - span.start, false);
        span.end - span.start
    }

    /// Appends directly backed physical-source characters that horizontal
    /// main control can consume without expansion or provenance allocation.
    ///
    /// The run is deliberately limited to current-catcode `Letter` and
    /// `Other` scalars. Every other category remains a seam for the ordinary
    /// lexer, including superscript notation, active and structural tokens,
    /// whitespace, synthetic end lines, and degraded source origins.
    pub fn append_source_text_span(
        &mut self,
        stores: &mut impl ExpansionState,
        tokens_out: &mut Vec<TracedTokenWord>,
    ) -> usize {
        if self.has_active_alignment() {
            return 0;
        }
        let Some(frame_index) = self.current_token_frame_index() else {
            return 0;
        };
        let InputFrame::Source(source) = &mut self.frames[frame_index] else {
            return 0;
        };
        #[cfg(feature = "profiling-stats")]
        {
            self.expansion_stats.source_text_span_attempts += 1;
        }
        ensure_source_registered(source, stores);
        if !source.frame.pending.is_empty() || source.frame.byte_offset >= source.frame.line.len() {
            return 0;
        }
        let Some(registration) = source.registration else {
            return 0;
        };

        let start = tokens_out.len();
        let mut byte_offset = source.frame.byte_offset;
        let mut column = source.frame.column;
        while byte_offset < source.frame.line.len() {
            let ch = source.frame.line[byte_offset..]
                .chars()
                .next()
                .expect("byte cursor remains at a scalar boundary");
            let cat = stores.catcode(ch);
            if !matches!(cat, Catcode::Letter | Catcode::Other) {
                break;
            }
            let next = byte_offset + ch.len_utf8();
            let Some(physical_start) = source
                .frame
                .origin_line_start
                .checked_add(byte_offset as u64)
            else {
                break;
            };
            let Some(physical_end) = source.frame.origin_line_start.checked_add(next as u64) else {
                break;
            };
            let Some(origin) = registration.direct_origin(physical_start, physical_end) else {
                break;
            };
            tokens_out.push(TracedTokenWord::pack(Token::Char { ch, cat }, origin));
            byte_offset = next;
            column += 1;
        }
        let appended = tokens_out.len() - start;
        if appended == 0 {
            return 0;
        }
        source.frame.byte_offset = byte_offset;
        source.frame.column = column;
        source.frame.state = LexerState::MidLine;
        #[cfg(feature = "profiling-stats")]
        {
            self.expansion_stats.source_text_spans += 1;
            self.expansion_stats.source_text_tokens += u64::try_from(appended).unwrap_or(u64::MAX);
        }
        appended
    }

    fn take_macro_literal_span(
        &mut self,
        stores: &impl ExpansionState,
        policy: LiteralSpanPolicy,
    ) -> Option<MacroLiteralSpan> {
        // Alignment interception observes character-token delivery (not only
        // control sequences). Any active scanner level therefore forces the
        // existing per-token path, including while no cell is active yet.
        if self.has_active_alignment() {
            return None;
        }
        loop {
            let frame_index = self.current_token_frame_index()?;
            let exhausted = match &self.frames[frame_index] {
                InputFrame::TokenList(frame)
                    if matches!(
                        frame.replay_kind,
                        TokenListReplayKind::MacroBody | TokenListReplayKind::MacroArgument
                    ) =>
                {
                    frame.index >= frame.len(stores)
                }
                InputFrame::Source(_) | InputFrame::TokenList(_) | InputFrame::Condition { .. } => {
                    false
                }
            };
            if exhausted {
                let frame = self.discard_token_list_frame(frame_index);
                self.retire_token_list_frame(frame);
                continue;
            }
            let argument_slot = match &mut self.frames[frame_index] {
                InputFrame::TokenList(frame)
                    if frame.replay_kind == TokenListReplayKind::MacroBody
                        && matches!(
                            frame.semantic_token_at(stores, frame.index),
                            Some(Token::Param(_))
                        ) =>
                {
                    let Some(Token::Param(slot)) = frame.semantic_token_at(stores, frame.index)
                    else {
                        unreachable!("guard restricts the token kind")
                    };
                    let present = frame.macro_arguments.get(slot).is_some();
                    if present {
                        frame.index += 1;
                    }
                    present.then_some(slot)
                }
                InputFrame::Source(_) | InputFrame::TokenList(_) | InputFrame::Condition { .. } => {
                    None
                }
            };
            if let Some(slot) = argument_slot {
                self.push_macro_argument_frame(frame_index, slot);
                continue;
            }

            let (token_list, origin_list, replay_kind, start) = match &self.frames[frame_index] {
                InputFrame::TokenList(frame)
                    if matches!(
                        frame.replay_kind,
                        TokenListReplayKind::MacroBody | TokenListReplayKind::MacroArgument
                    ) && frame.stored_ids().is_some() =>
                {
                    let (token_list, origin_list) = frame
                        .stored_ids()
                        .expect("guard restricts replay to stored content");
                    (token_list, origin_list, frame.replay_kind, frame.index)
                }
                InputFrame::Source(_) | InputFrame::TokenList(_) | InputFrame::Condition { .. } => {
                    return None;
                }
            };
            let end = self.cached_literal_span_end(stores, token_list, start, policy);
            if end == start {
                return None;
            }
            if replay_kind == TokenListReplayKind::MacroArgument
                && origin_list == OriginListId::EMPTY
            {
                // Ordinary replay synthesizes a distinct inserted origin for
                // this degraded case, so it cannot be represented as a span.
                return None;
            }

            let InputFrame::TokenList(frame) = &mut self.frames[frame_index] else {
                unreachable!("frame identity is stable during span selection")
            };
            frame.index = end;
            return Some(MacroLiteralSpan {
                token_list,
                origin_list,
                replay_kind,
                start,
                end,
            });
        }
    }

    fn cached_literal_span_end(
        &mut self,
        stores: &impl ExpansionState,
        token_list: TokenListId,
        start: usize,
        policy: LiteralSpanPolicy,
    ) -> usize {
        let key = (token_list, policy);
        let cache_hit = self.literal_span_cache.contains_key(&key);
        #[cfg(feature = "profiling-stats")]
        if cache_hit {
            self.expansion_stats.segmentation_cache_hits += 1;
        } else {
            self.expansion_stats.segmentation_cache_misses += 1;
        }
        if !cache_hit && self.literal_span_cache.len() >= LITERAL_SPAN_CACHE_MAX_ENTRIES {
            // Body plans are derived and cheap to rebuild. Bound retention for
            // long-lived interactive stacks and discard stale timeline keys
            // in one allocation-free operation.
            self.literal_span_cache.clear();
        }
        let spans = self.literal_span_cache.entry(key).or_insert_with(|| {
            let tokens = stores.tokens(token_list);
            let mut spans = Vec::new();
            let mut cursor = 0;
            while cursor < tokens.len() {
                if !policy.accepts(tokens[cursor]) {
                    cursor += 1;
                    continue;
                }
                let span_start = cursor;
                cursor += 1;
                while cursor < tokens.len() && policy.accepts(tokens[cursor]) {
                    cursor += 1;
                }
                spans.push((span_start, cursor));
            }
            Arc::from(spans)
        });
        let index = spans.partition_point(|&(_, span_end)| span_end <= start);
        spans.get(index).map_or(
            start,
            |&(span_start, span_end)| {
                if span_start <= start { span_end } else { start }
            },
        )
    }

    #[inline(always)]
    fn record_literal_span(&mut self, len: usize, builder_append: bool) {
        #[cfg(feature = "profiling-stats")]
        {
            self.expansion_stats.literal_spans += 1;
            self.expansion_stats.literal_tokens += len as u64;
            if builder_append {
                self.expansion_stats.builder_appends += len as u64;
            }
        }
        #[cfg(not(feature = "profiling-stats"))]
        let _ = (len, builder_append);
    }

    #[must_use]
    pub const fn active_macro_invocation(&self) -> OriginId {
        self.active_macro_invocation
    }

    pub fn push_condition(&mut self, condition: ConditionFrameSummary) -> ConditionFrameToken {
        let token = ConditionFrameToken::new(self.next_condition_token);
        self.next_condition_token = self
            .next_condition_token
            .checked_add(1)
            .expect("condition frame token overflowed");
        self.push_frame(InputFrame::Condition { token, condition });
        token
    }

    pub fn update_condition(
        &mut self,
        token: ConditionFrameToken,
        condition: ConditionFrameSummary,
    ) -> Option<ConditionFrameSummary> {
        let index = self.condition_frame_indices.iter().rev().copied().find(|index| {
            matches!(self.frames[*index], InputFrame::Condition { token: frame_token, .. } if frame_token == token)
        })?;
        let InputFrame::Condition {
            condition: frame, ..
        } = &mut self.frames[index]
        else {
            unreachable!("condition index names a condition frame")
        };
        Some(std::mem::replace(frame, condition))
    }

    /// Updates the innermost live conditional frame.
    pub fn update_current_condition(
        &mut self,
        condition: ConditionFrameSummary,
    ) -> Option<ConditionFrameSummary> {
        let token = self.current_condition_token()?;
        self.update_condition(token, condition)
    }

    #[must_use]
    pub fn current_condition(&self) -> Option<ConditionFrameSummary> {
        let index = *self.condition_frame_indices.last()?;
        let InputFrame::Condition { condition, .. } = self.frames[index] else {
            unreachable!()
        };
        Some(condition)
    }

    #[must_use]
    pub fn condition_depth(&self) -> usize {
        self.condition_frame_indices.len()
    }

    /// Number of currently live physical source frames.
    ///
    /// Expanded-definition scanners use this to detect the TeX error boundary
    /// where an `\input` or `\scantokens` source ends while defining text is
    /// still unbalanced. Token-list and conditional frames do not count.
    #[must_use]
    pub fn source_depth(&self) -> usize {
        self.source_frame_count
    }

    #[must_use]
    pub fn conditions(&self) -> impl DoubleEndedIterator<Item = ConditionFrameSummary> + '_ {
        self.condition_frame_indices.iter().map(|&index| {
            let InputFrame::Condition { condition, .. } = self.frames[index] else {
                unreachable!("condition index must address a condition frame")
            };
            condition
        })
    }

    #[must_use]
    pub fn current_condition_token(&self) -> Option<ConditionFrameToken> {
        let index = *self.condition_frame_indices.last()?;
        let InputFrame::Condition { token, .. } = self.frames[index] else {
            unreachable!()
        };
        Some(token)
    }

    pub fn pop_condition(&mut self) -> Option<ConditionFrameSummary> {
        let index = *self.condition_frame_indices.last()?;
        match self.remove_frame(index) {
            InputFrame::Condition { condition, .. } => Some(condition),
            InputFrame::Source(_) | InputFrame::TokenList(_) => unreachable!("rposition matched"),
        }
    }

    #[must_use]
    pub fn summary(&self) -> InputSummary {
        InputSummary::new_with_resume_state(
            self.frames
                .iter()
                .map(|frame| match frame {
                    InputFrame::Source(source) => InputFrameSummary::Source {
                        source_id: source.source_id,
                        input_record: source.input_record,
                        source: source
                            .frame
                            .summary(source.next_source_offset)
                            .with_registration(source.registration)
                            .with_scantokens(source.scantokens),
                    },
                    InputFrame::TokenList(frame) => match &frame.payload {
                        ReplayPayload::Stored {
                            token_list,
                            origin_list,
                        } => InputFrameSummary::TokenList {
                            token_list: *token_list,
                            origin_list: *origin_list,
                            replay_kind: frame.replay_kind,
                            index: frame.index,
                            macro_arguments: frame.macro_arguments.summary(),
                            macro_invocation: frame.macro_invocation,
                            parent_macro_invocation: frame.parent_macro_invocation,
                        },
                        ReplayPayload::Transient { tokens } => {
                            InputFrameSummary::TransientTokenList {
                                tokens: Arc::from(&tokens[frame.index..]),
                                replay_kind: frame.replay_kind,
                                macro_invocation: frame.macro_invocation,
                                parent_macro_invocation: frame.parent_macro_invocation,
                            }
                        }
                    },
                    InputFrame::Condition { token, condition } => InputFrameSummary::Condition {
                        token: *token,
                        condition: *condition,
                    },
                })
                .collect(),
            self.last_source_frame.as_ref().map(|last| last.source_id),
            self.last_source_frame
                .as_ref()
                .and_then(|last| last.input_record),
            self.last_source_frame.as_ref().map(|last| {
                last.frame
                    .summary(last.next_source_offset)
                    .with_registration(last.registration)
            }),
            self.next_source_id,
            self.unicode_superscript_notation,
        )
    }

    /// Captures a summary whose source capabilities are ready for publication
    /// through [`tex_state::Universe::set_input_summary`].
    pub fn publication_summary(&mut self, stores: &mut impl ExpansionState) -> InputSummary {
        for frame in &mut self.frames {
            if let InputFrame::Source(source) = frame {
                ensure_source_registered(source, stores);
            }
        }
        // A degraded summary may have restored only the diagnostic coordinates
        // of an already-popped source, without its runtime registration
        // capability. It is not resumable input and cannot be republished as a
        // live source graph, so discard that diagnostic-only tail.
        if self
            .last_source_frame
            .as_ref()
            .is_some_and(|source| source.registration.is_none())
        {
            self.last_source_frame = None;
        }
        self.summary()
    }

    #[must_use]
    pub fn current_source_frame(&self) -> Option<&SourceFrame> {
        let current = self.frames.iter().rev().find_map(|frame| match frame {
            InputFrame::Source(source) => Some(&source.frame),
            InputFrame::TokenList(_) | InputFrame::Condition { .. } => None,
        });
        current.or_else(|| self.last_source_frame.as_ref().map(|last| &last.frame))
    }

    pub fn current_input_origin(&mut self, stores: &mut impl ExpansionState) -> OriginId {
        if let Some(source) = self.frames.iter_mut().rev().find_map(|frame| match frame {
            InputFrame::Source(source) => Some(source),
            InputFrame::TokenList(_) | InputFrame::Condition { .. } => None,
        }) {
            ensure_source_registered(source, stores);
            return allocate_source_origin(stores, source.registration, source_coordinate(source));
        }
        if let Some(last) = &self.last_source_frame {
            return allocate_source_origin(
                stores,
                last.registration,
                source_coordinate_from_frame(last.source_id, last.input_record, &last.frame),
            );
        }
        stores.synthetic_origin(SyntheticOriginKind::Engine)
    }

    /// Captures the persistent expansion-chain head at an error boundary.
    #[must_use]
    pub fn diagnostic_site(
        &self,
        primary: Option<OriginId>,
        related: impl IntoIterator<Item = RelatedLocation>,
    ) -> DiagnosticSite {
        DiagnosticSite::new(primary, related, self.diagnostic_expansion_head())
    }

    /// Takes proof for the immediately preceding delivery when it was the
    /// supplied token and came straight from a physical source frame.
    pub fn take_direct_source_delivery(
        &mut self,
        token: TracedTokenWord,
    ) -> Option<DirectSourceDelivery> {
        let Token::Char { ch, .. } = token.token()? else {
            return None;
        };
        let frame_index = self.current_token_frame_index()?;
        let InputFrame::Source(source) = &self.frames[frame_index] else {
            return None;
        };
        let end = source_coordinate(source).byte_offset;
        let start = end.checked_sub(u64::try_from(ch.len_utf8()).ok()?)?;
        if source.registration?.direct_origin(start, end)? != token.origin() {
            return None;
        }
        Some(DirectSourceDelivery {
            token,
            source: source.source_id,
            start,
            end,
        })
    }

    /// Joins two proven deliveries only while their one shared source frame is
    /// still live. Failure leaves callers free to retain separate sites.
    pub fn join_direct_source_deliveries(
        &self,
        stores: &mut impl ExpansionState,
        first: DirectSourceDelivery,
        last: DirectSourceDelivery,
    ) -> Option<OriginId> {
        if first.source != last.source || first.start > last.start || first.end > last.end {
            return None;
        }
        let registration = self.frames.iter().find_map(|frame| match frame {
            InputFrame::Source(source) if source.source_id == first.source => source.registration,
            InputFrame::Source(_) | InputFrame::TokenList(_) | InputFrame::Condition { .. } => None,
        })?;
        let span = registration.span(first.start, last.end).ok()?;
        Some(stores.source_span_origin(span))
    }

    fn diagnostic_expansion_head(&self) -> Option<OriginId> {
        self.recently_popped_invocation
            .or((self.active_macro_invocation != OriginId::UNKNOWN)
                .then_some(self.active_macro_invocation))
    }

    fn retire_token_list_frame(&mut self, frame: TokenListRetirement) {
        if frame.replay_kind == TokenListReplayKind::AlignmentUTemplate
            && let Some(alignment) = self.alignment_inputs.last_mut()
            && alignment.align_state > 500_000
        {
            alignment.align_state = 0;
        }
        if frame.macro_invocation == OriginId::UNKNOWN {
            return;
        }
        debug_assert_eq!(self.active_macro_invocation, frame.macro_invocation);
        self.active_macro_invocation = frame.parent_macro_invocation;
        self.recently_popped_invocation
            .get_or_insert(frame.macro_invocation);
    }

    #[must_use]
    pub fn depth(&self) -> usize {
        self.frames.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    pub fn set_unicode_superscript_notation(&mut self, enabled: bool) {
        self.unicode_superscript_notation = enabled;
    }

    /// Stops the current source frame after its current normalized line.
    ///
    /// This is the lexer-owned half of TeX's `\endinput`: expansion/execution
    /// decide when the primitive is seen, while the input stack controls the
    /// exact source-frame pop point.
    pub fn end_current_source_after_current_line(&mut self) -> bool {
        let Some(source) = self.frames.iter_mut().rev().find_map(|frame| match frame {
            InputFrame::Source(source) => Some(source),
            InputFrame::TokenList(_) | InputFrame::Condition { .. } => None,
        }) else {
            return false;
        };
        source.frame.end_after_current_line = true;
        true
    }

    /// Clears and returns the pending end-of-input flag on the current source.
    /// An `\input` expansion uses this to transfer TeX's global `force_eof`
    /// state to the source it opens while scanning the file name.
    pub fn take_current_source_end_after_current_line(&mut self) -> bool {
        let Some(source) = self.frames.iter_mut().rev().find_map(|frame| match frame {
            InputFrame::Source(source) => Some(source),
            InputFrame::TokenList(_) | InputFrame::Condition { .. } => None,
        }) else {
            return false;
        };
        std::mem::take(&mut source.frame.end_after_current_line)
    }
}

/// Mandatory source coordinates for failures that occur before a valid TeX token exists.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LexSourceContext {
    source_id: SourceId,
    input_record: Option<InputRecordId>,
    byte_offset: u64,
    byte_end: u64,
    line: u32,
    column: u32,
}

impl LexSourceContext {
    #[must_use]
    pub const fn source_id(self) -> SourceId {
        self.source_id
    }

    #[must_use]
    pub const fn byte_offset(self) -> u64 {
        self.byte_offset
    }

    #[must_use]
    pub const fn byte_end(self) -> u64 {
        self.byte_end
    }

    #[must_use]
    pub const fn byte_range(self) -> std::ops::Range<u64> {
        self.byte_offset..self.byte_end
    }

    #[must_use]
    pub const fn line(self) -> u32 {
        self.line
    }

    #[must_use]
    pub const fn column(self) -> u32 {
        self.column
    }
}

/// Errors produced while converting characters to TeX tokens.
#[derive(Debug)]
pub enum LexError {
    Input {
        error: Box<WorldError>,
        context: LexSourceContext,
        site: Box<DiagnosticSite>,
    },
    InvalidCharacter {
        ch: char,
        context: LexSourceContext,
        site: Box<DiagnosticSite>,
    },
    InvalidUtf8 {
        context: LexSourceContext,
        site: Box<DiagnosticSite>,
    },
    MissingControlSequence {
        name: String,
        context: LexSourceContext,
        site: Box<DiagnosticSite>,
    },
}

impl fmt::Display for LexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Input { error, .. } => write!(f, "input read failed: {error}"),
            Self::InvalidCharacter { ch, .. } => {
                write!(
                    f,
                    "input contains invalid TeX character U+{:04X}",
                    *ch as u32
                )
            }
            Self::InvalidUtf8 { context, .. } => write!(
                f,
                "input contains invalid UTF-8 in physical byte range {}..{}",
                context.byte_offset, context.byte_end
            ),
            Self::MissingControlSequence { name, .. } => {
                write!(f, "control sequence {name:?} is not interned")
            }
        }
    }
}

impl std::error::Error for LexError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Input { error, .. } => Some(error.as_ref()),
            Self::InvalidCharacter { .. }
            | Self::InvalidUtf8 { .. }
            | Self::MissingControlSequence { .. } => None,
        }
    }
}

impl LexError {
    #[must_use]
    pub const fn source_context(&self) -> LexSourceContext {
        match self {
            Self::Input { context, .. }
            | Self::InvalidCharacter { context, .. }
            | Self::InvalidUtf8 { context, .. }
            | Self::MissingControlSequence { context, .. } => *context,
        }
    }

    #[must_use]
    pub const fn diagnostic_site(&self) -> &DiagnosticSite {
        match self {
            Self::Input { site, .. }
            | Self::InvalidCharacter { site, .. }
            | Self::InvalidUtf8 { site, .. }
            | Self::MissingControlSequence { site, .. } => site,
        }
    }

    fn with_physical_site(mut self, stores: &mut impl ExpansionState) -> Self {
        let context = self.source_context();
        let origin =
            stores.source_range_origin(context.source_id, context.byte_offset, context.byte_end);
        let site = DiagnosticSite::primary(origin);
        match &mut self {
            Self::Input { site: value, .. }
            | Self::InvalidCharacter { site: value, .. }
            | Self::InvalidUtf8 { site: value, .. }
            | Self::MissingControlSequence { site: value, .. } => **value = site,
        }
        self
    }

    fn with_expansion_head(mut self, expansion_head: Option<OriginId>) -> Self {
        let captured = DiagnosticSite::new(
            self.diagnostic_site().primary_origin(),
            self.diagnostic_site().related().iter().copied(),
            expansion_head,
        );
        match &mut self {
            Self::Input { site, .. }
            | Self::InvalidCharacter { site, .. }
            | Self::InvalidUtf8 { site, .. }
            | Self::MissingControlSequence { site, .. } => **site = captured,
        }
        self
    }
}

/// Semantic TeX lexer over a normalized input source.
#[derive(Debug)]
pub struct Lexer {
    input: InputStack,
}

impl Lexer {
    #[must_use]
    pub fn new<S>(source: S) -> Self
    where
        S: InputSource + 'static,
    {
        Self {
            input: InputStack::new(source),
        }
    }

    #[must_use]
    pub fn frame(&self) -> &SourceFrame {
        self.input
            .current_source_frame()
            .expect("Lexer always starts with one source frame")
    }

    #[must_use]
    pub fn input_summary(&self) -> InputSummary {
        self.input.summary()
    }

    #[must_use]
    pub fn input_stack(&self) -> &InputStack {
        &self.input
    }

    pub fn input_stack_mut(&mut self) -> &mut InputStack {
        &mut self.input
    }

    #[must_use]
    pub fn into_inner(self) -> Box<dyn InputSource> {
        match self.input.frames.into_iter().next() {
            Some(InputFrame::Source(source)) => source.lines.into_inner(),
            Some(InputFrame::TokenList(_) | InputFrame::Condition { .. }) | None => {
                panic!("Lexer source was not at the bottom of the input stack")
            }
        }
    }

    pub fn set_unicode_superscript_notation(&mut self, enabled: bool) {
        self.input.set_unicode_superscript_notation(enabled);
    }
}

impl Lexer {
    pub fn next_token(
        &mut self,
        stores: &mut impl ExpansionState,
    ) -> Result<Option<Token>, LexError> {
        self.input.next_token(stores)
    }

    #[inline(always)]
    pub fn next_traced_token(
        &mut self,
        stores: &mut impl ExpansionState,
    ) -> Result<Option<TracedTokenWord>, LexError> {
        self.input.next_traced_token(stores)
    }
}

impl InputStack {
    pub fn next_token(
        &mut self,
        stores: &mut impl ExpansionState,
    ) -> Result<Option<Token>, LexError> {
        Ok(self.next_traced_token(stores)?.map(decode_traced_token))
    }

    pub fn next_traced_token(
        &mut self,
        stores: &mut impl ExpansionState,
    ) -> Result<Option<TracedTokenWord>, LexError> {
        match self.next_traced_token_inner(stores) {
            Ok(token) => Ok(token),
            Err(error) => Err(self.capture_lex_error(error)),
        }
    }

    #[inline(always)]
    fn next_traced_token_inner(
        &mut self,
        stores: &mut impl ExpansionState,
    ) -> Result<Option<TracedTokenWord>, LexError> {
        self.recently_popped_invocation = None;
        loop {
            let Some(frame_index) = self.current_token_frame_index() else {
                return Ok(None);
            };
            match &mut self.frames[frame_index] {
                InputFrame::TokenList(token_list) => {
                    match next_traced_token_from_token_list_frame(
                        token_list,
                        stores,
                        #[cfg(feature = "profiling-stats")]
                        None,
                    ) {
                        Some(TracedTokenReplay::PushArgument(slot)) => {
                            self.push_macro_argument_frame(frame_index, slot);
                            continue;
                        }
                        Some(
                            TracedTokenReplay::Deliver(token)
                            | TracedTokenReplay::DeliverNoExpand(token),
                        ) => {
                            return Ok(Some(token.packed()));
                        }
                        None => {
                            let frame = self.discard_token_list_frame(frame_index);
                            let closes_scantokens =
                                frame.replay_kind == TokenListReplayKind::ScantokensEveryEof;
                            self.retire_token_list_frame(frame);
                            if closes_scantokens {
                                stores.trace_scantokens_boundary(false);
                            }
                        }
                    };
                }
                InputFrame::Source(source) => {
                    ensure_source_registered(source, stores);
                    if let Some(token) = source.frame.pending.pop_front() {
                        return Ok(Some(token));
                    }

                    if source.frame.byte_offset >= source.frame.line.len() {
                        if source.frame.end_after_current_line {
                            let popped = self.remove_frame(frame_index);
                            if let InputFrame::Source(source) = popped {
                                let scantokens = source.scantokens;
                                self.last_source_frame = Some(LastSourceFrame {
                                    source_id: source.source_id,
                                    input_record: source.input_record,
                                    registration: source.registration,
                                    frame: source.frame,
                                    next_source_offset: source.next_source_offset,
                                });
                                if scantokens {
                                    stores.trace_scantokens_boundary(false);
                                }
                            }
                            continue;
                        }
                        if !load_next_line(source, stores)? {
                            let popped = self.remove_frame(frame_index);
                            let mut scantokens = false;
                            if let InputFrame::Source(source) = popped {
                                scantokens = source.scantokens;
                                self.last_source_frame = Some(LastSourceFrame {
                                    source_id: source.source_id,
                                    input_record: source.input_record,
                                    registration: source.registration,
                                    frame: source.frame,
                                    next_source_offset: source.next_source_offset,
                                });
                            }
                            let everyeof = stores.tok_param(TokParam::EVERY_EOF);
                            if everyeof != TokenListId::EMPTY {
                                self.push_token_list(
                                    everyeof,
                                    if scantokens {
                                        TokenListReplayKind::ScantokensEveryEof
                                    } else {
                                        TokenListReplayKind::Inserted
                                    },
                                );
                            } else if scantokens {
                                stores.trace_scantokens_boundary(false);
                            }
                        }
                        continue;
                    }

                    let Some(token) =
                        next_token_from_line(source, stores, self.unicode_superscript_notation)?
                    else {
                        continue;
                    };
                    return Ok(Some(token.packed()));
                }
                InputFrame::Condition { .. } => {
                    unreachable!("current_token_frame_index skips conditions")
                }
            }
        }
    }

    pub fn next_token_readonly(
        &mut self,
        stores: &impl ExpansionState,
    ) -> Result<Option<Token>, LexError> {
        Ok(self
            .next_expansion_token_readonly(stores)?
            .map(ExpansionToken::token))
    }

    pub fn next_expansion_token(
        &mut self,
        stores: &mut impl ExpansionState,
    ) -> Result<Option<ExpansionToken>, LexError> {
        Ok(self
            .next_traced_expansion_token(stores)?
            .map(|token| ExpansionToken::new(token.token(), token.suppress_expansion())))
    }

    #[inline(always)]
    pub fn next_traced_expansion_token(
        &mut self,
        stores: &mut impl ExpansionState,
    ) -> Result<Option<TracedExpansionToken>, LexError> {
        match self.next_traced_expansion_token_inner(stores) {
            Ok(token) => Ok(token),
            Err(error) => Err(self.capture_lex_error(error)),
        }
    }

    #[inline(always)]
    fn next_traced_expansion_token_inner(
        &mut self,
        stores: &mut impl ExpansionState,
    ) -> Result<Option<TracedExpansionToken>, LexError> {
        self.recently_popped_invocation = None;
        loop {
            let Some(frame_index) = self.current_token_frame_index() else {
                return Ok(None);
            };
            match &mut self.frames[frame_index] {
                InputFrame::TokenList(token_list) => {
                    let stored_token_list =
                        token_list.stored_ids().map(|(token_list, _)| token_list);
                    let macro_replay_site = (token_list.replay_kind
                        == TokenListReplayKind::MacroBody
                        && matches!(
                            token_list.semantic_token_at(stores, token_list.index),
                            Some(
                                Token::Cs(_)
                                    | Token::Char {
                                        cat: Catcode::Active,
                                        ..
                                    }
                            )
                        ))
                    .then_some(stored_token_list)
                    .flatten()
                    .map(|stored_token_list| MacroReplaySite {
                        token_list: stored_token_list,
                        token_index: token_list.index,
                    });
                    #[cfg(feature = "profiling-stats")]
                    if let Some(token) = token_list.semantic_token_at(stores, token_list.index) {
                        self.expansion_stats.token_frame_steps += 1;
                        if matches!(token, Token::Char { .. }) {
                            self.expansion_stats.character_tokens += 1;
                        }
                        if !matches!(
                            &token,
                            Token::Param(slot)
                                if token_list.replay_kind == TokenListReplayKind::MacroBody
                                    && token_list.macro_arguments.get(*slot).is_some()
                        ) {
                            self.expansion_stats.provenance_resolutions += 1;
                        }
                    }
                    match next_traced_token_from_token_list_frame(
                        token_list,
                        stores,
                        #[cfg(feature = "profiling-stats")]
                        Some(&mut self.expansion_stats),
                    ) {
                        Some(TracedTokenReplay::PushArgument(slot)) => {
                            self.push_macro_argument_frame(frame_index, slot);
                            continue;
                        }
                        Some(TracedTokenReplay::Deliver(token)) => {
                            return Ok(Some(TracedExpansionToken::from_decoded(
                                token,
                                false,
                                false,
                                macro_replay_site,
                            )));
                        }
                        Some(TracedTokenReplay::DeliverNoExpand(token)) => {
                            return Ok(Some(TracedExpansionToken::from_decoded(
                                token,
                                true,
                                token_list.replay_kind == TokenListReplayKind::Unexpanded,
                                macro_replay_site,
                            )));
                        }
                        None => {
                            let frame = self.discard_token_list_frame(frame_index);
                            let closes_scantokens =
                                frame.replay_kind == TokenListReplayKind::ScantokensEveryEof;
                            self.retire_token_list_frame(frame);
                            if closes_scantokens {
                                stores.trace_scantokens_boundary(false);
                            }
                        }
                    };
                }
                InputFrame::Source(source) => {
                    ensure_source_registered(source, stores);
                    if let Some(token) = source.frame.pending.pop_front() {
                        return Ok(Some(TracedExpansionToken::new(token, false)));
                    }

                    if source.frame.byte_offset >= source.frame.line.len() {
                        if source.frame.end_after_current_line {
                            let popped = self.remove_frame(frame_index);
                            if let InputFrame::Source(source) = popped {
                                let scantokens = source.scantokens;
                                self.last_source_frame = Some(LastSourceFrame {
                                    source_id: source.source_id,
                                    input_record: source.input_record,
                                    registration: source.registration,
                                    frame: source.frame,
                                    next_source_offset: source.next_source_offset,
                                });
                                if scantokens {
                                    stores.trace_scantokens_boundary(false);
                                }
                            }
                            continue;
                        }
                        if !load_next_line(source, stores)? {
                            let popped = self.remove_frame(frame_index);
                            let mut scantokens = false;
                            if let InputFrame::Source(source) = popped {
                                scantokens = source.scantokens;
                                self.last_source_frame = Some(LastSourceFrame {
                                    source_id: source.source_id,
                                    input_record: source.input_record,
                                    registration: source.registration,
                                    frame: source.frame,
                                    next_source_offset: source.next_source_offset,
                                });
                            }
                            let everyeof = stores.tok_param(TokParam::EVERY_EOF);
                            if everyeof != TokenListId::EMPTY {
                                self.push_token_list(
                                    everyeof,
                                    if scantokens {
                                        TokenListReplayKind::ScantokensEveryEof
                                    } else {
                                        TokenListReplayKind::Inserted
                                    },
                                );
                            } else if scantokens {
                                stores.trace_scantokens_boundary(false);
                            }
                        }
                        continue;
                    }

                    let Some(token) =
                        next_token_from_line(source, stores, self.unicode_superscript_notation)?
                    else {
                        continue;
                    };
                    return Ok(Some(TracedExpansionToken::from_decoded(
                        token, false, false, None,
                    )));
                }
                InputFrame::Condition { .. } => {
                    unreachable!("current_token_frame_index skips conditions")
                }
            }
        }
    }

    pub fn next_expansion_token_readonly(
        &mut self,
        stores: &impl ExpansionState,
    ) -> Result<Option<ExpansionToken>, LexError> {
        loop {
            let Some(frame_index) = self.current_token_frame_index() else {
                return Ok(None);
            };
            match &mut self.frames[frame_index] {
                InputFrame::TokenList(token_list) => {
                    match next_token_from_token_list_frame(token_list, stores) {
                        Some(TokenReplay::PushArgument(slot)) => {
                            self.push_macro_argument_frame(frame_index, slot);
                            continue;
                        }
                        Some(TokenReplay::Deliver(token)) => {
                            return Ok(Some(ExpansionToken::new(token, false)));
                        }
                        Some(TokenReplay::DeliverNoExpand(token)) => {
                            return Ok(Some(ExpansionToken::new(token, true)));
                        }
                        None => {
                            self.discard_token_list_frame(frame_index);
                        }
                    };
                }
                InputFrame::Source(source) => {
                    if let Some(token) = source.frame.pending.pop_front() {
                        return Ok(Some(ExpansionToken::new(decode_traced_token(token), false)));
                    }

                    if source.frame.byte_offset >= source.frame.line.len() {
                        if source.frame.end_after_current_line {
                            let popped = self.remove_frame(frame_index);
                            if let InputFrame::Source(source) = popped {
                                self.last_source_frame = Some(LastSourceFrame {
                                    source_id: source.source_id,
                                    input_record: source.input_record,
                                    registration: source.registration,
                                    frame: source.frame,
                                    next_source_offset: source.next_source_offset,
                                });
                            }
                            continue;
                        }
                        if !load_next_line_readonly(source, stores)? {
                            let popped = self.remove_frame(frame_index);
                            if let InputFrame::Source(source) = popped {
                                self.last_source_frame = Some(LastSourceFrame {
                                    source_id: source.source_id,
                                    input_record: source.input_record,
                                    registration: source.registration,
                                    frame: source.frame,
                                    next_source_offset: source.next_source_offset,
                                });
                            }
                        }
                        continue;
                    }

                    let Some(token) = next_token_from_line_readonly(
                        source,
                        stores,
                        self.unicode_superscript_notation,
                    )?
                    else {
                        continue;
                    };
                    return Ok(Some(ExpansionToken::new(token, false)));
                }
                InputFrame::Condition { .. } => {
                    unreachable!("current_token_frame_index skips conditions")
                }
            }
        }
    }

    #[cold]
    #[inline(never)]
    fn capture_lex_error(&self, error: LexError) -> LexError {
        error.with_expansion_head(self.diagnostic_expansion_head())
    }
}

impl InputStack {
    #[must_use]
    pub fn current_token_list_frame(&self) -> Option<(TokenListId, TokenListReplayKind, usize)> {
        let frame_index = self.current_token_frame_index()?;
        match &self.frames[frame_index] {
            InputFrame::TokenList(frame) => {
                let (token_list, _) = frame.stored_ids()?;
                Some((token_list, frame.replay_kind, frame.index))
            }
            InputFrame::Source(_) | InputFrame::Condition { .. } => None,
        }
    }

    pub fn pop_current_token_list_frame(
        &mut self,
        token_list: TokenListId,
        replay_kind: TokenListReplayKind,
    ) -> bool {
        let Some(frame_index) = self.current_token_frame_index() else {
            return false;
        };
        let matches = matches!(
            &self.frames[frame_index],
            InputFrame::TokenList(frame)
                if frame.stored_ids().is_some_and(|(stored, _)| stored == token_list)
                    && frame.replay_kind == replay_kind
        );
        if matches {
            let frame = self.discard_token_list_frame(frame_index);
            self.retire_token_list_frame(frame);
        }
        matches
    }

    /// Removes a scoped replay frame and every nested frame it introduced.
    ///
    /// This is the input-stack half of recursive execution rollback. It does
    /// not rewind source frames that predate the replay capability.
    pub fn abort_token_list_replay(&mut self, marker: TokenListReplayMarker) -> bool {
        let Some(target) = self.replay_marker_frame_index(marker) else {
            return false;
        };
        let indices = self
            .frames
            .iter_indexed_from(target)
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        for index in indices.into_iter().rev() {
            let frame = self.discard_token_list_frame(index);
            self.retire_token_list_frame(frame);
        }
        true
    }

    #[must_use]
    pub fn contains_token_list_frame(
        &self,
        token_list: TokenListId,
        replay_kind: TokenListReplayKind,
    ) -> bool {
        self.frames.iter().any(|frame| {
            matches!(
                frame,
                InputFrame::TokenList(frame)
                    if frame.stored_ids().is_some_and(|(stored, _)| stored == token_list)
                        && frame.replay_kind == replay_kind
            )
        })
    }

    /// Returns whether a synchronously delimited replay frame is still live.
    #[must_use]
    pub fn contains_token_list_replay_marker(&self, marker: TokenListReplayMarker) -> bool {
        self.replay_marker_frame_index(marker).is_some()
    }

    /// Retires a marked replay once it and every token-list replay above it
    /// are exhausted, without reading the next token from the underlying
    /// input frame.
    ///
    /// TeX82 performs this cleanup in `end_token_list` before `get_next`
    /// resumes the input below a u-template. Macro and argument frames can be
    /// exhausted above that template, so checking only the current frame
    /// would read one token beyond the template boundary.
    pub fn finish_exhausted_token_list_replay(
        &mut self,
        marker: TokenListReplayMarker,
        stores: &impl ExpansionState,
    ) -> bool {
        let Some(marked_index) = self.replay_marker_frame_index(marker) else {
            return true;
        };

        let can_finish =
            self.frames
                .iter_indexed_from(marked_index)
                .all(|(_, frame)| match frame {
                    InputFrame::TokenList(frame) => frame.index >= frame.len(stores),
                    InputFrame::Condition { .. } => true,
                    InputFrame::Source(_) => false,
                });
        if !can_finish {
            return false;
        }

        let retire = self
            .frames
            .iter_indexed_from(marked_index)
            .filter_map(|(index, frame)| matches!(frame, InputFrame::TokenList(_)).then_some(index))
            .collect::<Vec<_>>();
        for index in retire.into_iter().rev() {
            if matches!(self.frames[index], InputFrame::TokenList(_)) {
                let frame = self.discard_token_list_frame(index);
                self.retire_token_list_frame(frame);
            }
        }
        true
    }

    fn replay_marker_frame_index(&self, marker: TokenListReplayMarker) -> Option<usize> {
        matches!(
            self.frames.get(marker.frame_index),
            Some(InputFrame::TokenList(frame)) if frame.replay_marker == Some(marker)
        )
        .then_some(marker.frame_index)
    }

    fn current_token_frame_index(&self) -> Option<usize> {
        self.token_frame_indices.last().copied()
    }

    fn push_macro_argument_frame(&mut self, parent_index: usize, slot: u8) {
        let mut tokens = self.take_transient_token_buffer();
        let InputFrame::TokenList(parent) = &self.frames[parent_index] else {
            unreachable!("macro argument parent must be a token-list frame")
        };
        tokens.extend_from_slice(
            parent
                .macro_arguments
                .get(slot)
                .expect("parameter replay requires a matched argument"),
        );
        self.push_frame(InputFrame::TokenList(TokenListInputFrame {
            payload: ReplayPayload::Transient { tokens },
            replay_kind: TokenListReplayKind::MacroArgument,
            index: 0,
            macro_arguments: MacroArguments::new(),
            macro_invocation: OriginId::UNKNOWN,
            parent_macro_invocation: OriginId::UNKNOWN,
            replay_marker: None,
        }));
    }

    fn push_frame(&mut self, frame: InputFrame) {
        let index = self.frames.slot_len();
        match frame {
            InputFrame::Source(_) => {
                self.source_frame_count += 1;
                self.token_frame_indices.push(index);
            }
            InputFrame::TokenList(_) => self.token_frame_indices.push(index),
            InputFrame::Condition { .. } => self.condition_frame_indices.push(index),
        }
        self.frames.push(frame);
    }

    fn remove_frame(&mut self, index: usize) -> InputFrame {
        if matches!(self.frames[index], InputFrame::Source(_)) {
            self.source_frame_count = self
                .source_frame_count
                .checked_sub(1)
                .expect("source frame count must match live source frames");
        }
        let indices = match self.frames[index] {
            InputFrame::Source(_) | InputFrame::TokenList(_) => &mut self.token_frame_indices,
            InputFrame::Condition { .. } => &mut self.condition_frame_indices,
        };
        if indices.last() == Some(&index) {
            indices.pop();
        } else if let Ok(position) = indices.binary_search(&index) {
            indices.remove(position);
        }
        self.frames.remove(index)
    }

    fn discard_token_list_frame(&mut self, index: usize) -> TokenListRetirement {
        let InputFrame::TokenList(frame) = &self.frames[index] else {
            panic!("token-list discard requires a token-list frame");
        };
        let retirement = TokenListRetirement::from(frame);
        if self.token_frame_indices.last() == Some(&index) {
            self.token_frame_indices.pop();
        } else if let Ok(position) = self.token_frame_indices.binary_search(&index) {
            self.token_frame_indices.remove(position);
        }
        let InputFrame::TokenList(mut frame) = self.frames.remove(index) else {
            unreachable!("validated token-list frame changed during removal")
        };
        if let ReplayPayload::Transient { tokens } = &mut frame.payload {
            let tokens = std::mem::take(tokens);
            self.recycle_transient_token_buffer(tokens);
        }
        let argument_tokens = frame.macro_arguments.take_tokens();
        self.recycle_transient_token_buffer(argument_tokens);
        retirement
    }
}

fn next_token_from_token_list_frame(
    frame: &mut TokenListInputFrame,
    stores: &impl ExpansionState,
) -> Option<TokenReplay> {
    let token = frame.semantic_token_at(stores, frame.index)?;
    frame.index += 1;

    if frame.replay_kind == TokenListReplayKind::MacroBody
        && let Token::Param(slot) = token
        && frame.macro_arguments.get(slot).is_some()
    {
        return Some(TokenReplay::PushArgument(slot));
    }

    if matches!(
        frame.replay_kind,
        TokenListReplayKind::NoExpand | TokenListReplayKind::Unexpanded
    ) {
        return Some(TokenReplay::DeliverNoExpand(token));
    }

    Some(TokenReplay::Deliver(token))
}

fn next_traced_token_from_token_list_frame(
    frame: &mut TokenListInputFrame,
    stores: &mut impl ExpansionState,
    #[cfg(feature = "profiling-stats")] mut stats: Option<&mut ExpansionStats>,
) -> Option<TracedTokenReplay> {
    #[cfg(feature = "profiling-stats")]
    let frame_started = stats.as_deref_mut().and_then(|stats| {
        should_sample_timer(&mut stats.frame_step_timer_events).then(Instant::now)
    });
    let Some(token) = frame.semantic_token_at(stores, frame.index) else {
        #[cfg(feature = "profiling-stats")]
        if let (Some(stats), Some(frame_started)) = (stats, frame_started) {
            add_elapsed(&mut stats.frame_step_nanos, frame_started);
            stats.frame_step_timer_samples += 1;
        }
        return None;
    };
    frame.index += 1;

    if frame.replay_kind == TokenListReplayKind::MacroBody
        && let Token::Param(slot) = token
        && frame.macro_arguments.get(slot).is_some()
    {
        #[cfg(feature = "profiling-stats")]
        if let (Some(stats), Some(frame_started)) = (stats, frame_started) {
            add_elapsed(&mut stats.frame_step_nanos, frame_started);
            stats.frame_step_timer_samples += 1;
        }
        return Some(TracedTokenReplay::PushArgument(slot));
    }

    #[cfg(feature = "profiling-stats")]
    if let (Some(stats), Some(frame_started)) = (stats.as_deref_mut(), frame_started) {
        add_elapsed(&mut stats.frame_step_nanos, frame_started);
        stats.frame_step_timer_samples += 1;
    }
    #[cfg(feature = "profiling-stats")]
    let provenance_started = stats.as_deref_mut().and_then(|stats| {
        should_sample_timer(&mut stats.provenance_timer_events).then(Instant::now)
    });
    let origin = match &frame.payload {
        ReplayPayload::Stored { .. } => replay_origin(frame, stores, token),
        ReplayPayload::Transient { tokens } => tokens[frame.index - 1].origin(),
    };
    #[cfg(feature = "profiling-stats")]
    if let (Some(stats), Some(provenance_started)) = (stats, provenance_started) {
        add_elapsed(&mut stats.provenance_nanos, provenance_started);
        stats.provenance_timer_samples += 1;
    }
    let token = DecodedTracedToken::new(token, origin);
    if matches!(
        frame.replay_kind,
        TokenListReplayKind::NoExpand | TokenListReplayKind::Unexpanded
    ) {
        return Some(TracedTokenReplay::DeliverNoExpand(token));
    }

    Some(TracedTokenReplay::Deliver(token))
}

fn replay_origin(
    frame: &TokenListInputFrame,
    stores: &mut impl ExpansionState,
    token: Token,
) -> OriginId {
    let (token_list, origin_list) = frame
        .stored_ids()
        .expect("stored replay origin requested for transient frame");
    if origin_list == OriginListId::EMPTY {
        if frame.replay_kind == TokenListReplayKind::MacroBody {
            return OriginId::UNKNOWN;
        }
        let parent = stores.bootstrap_origin();
        return stores.inserted_origin(
            InsertedOriginKind::TokenListReplay(frame.replay_kind),
            token,
            parent,
        );
    }

    let Some(origins) = stores.origin_list_if_live(origin_list) else {
        return OriginId::UNKNOWN;
    };
    if frame.index == 1 {
        let token_len = stores.tokens(token_list).len();
        assert_eq!(
            origins.len(),
            token_len,
            "token-list replay origin-list length does not match token-list length"
        );
    }
    origins[frame.index - 1]
}

fn load_next_line_readonly(
    source: &mut SourceInputFrame,
    stores: &impl ExpansionState,
) -> Result<bool, LexError> {
    let context = next_line_source_context(source);
    match source
        .lines
        .next_normalized_line(stores)
        .map_err(|error| map_input_source_error(source, error, context))?
    {
        Some(line) => {
            install_line_coordinates(source, &line);
            source.frame.state = LexerState::NewLine;
            source.frame.line = line.text;
            source.frame.byte_offset = 0;
            source.frame.physical_line_start = line.physical_start;
            source.frame.physical_content_end = line.physical_content_end;
            source.frame.terminator_start = line.terminator_start;
            source.frame.terminator_end = line.terminator_end;
            source.frame.normalized_end_anchor = line.normalized_end_anchor;
            source.frame.synthetic_endline_start = line.synthetic_endline_start;
            source.frame.line_number += 1;
            source.frame.column = 0;
            source.next_source_offset = line.terminator_end;
            Ok(true)
        }
        None => Ok(false),
    }
}

fn load_next_line(
    source: &mut SourceInputFrame,
    stores: &mut impl ExpansionState,
) -> Result<bool, LexError> {
    let context = next_line_source_context(source);
    match source.lines.next_normalized_line(stores).map_err(|error| {
        map_input_source_error(source, error, context).with_physical_site(stores)
    })? {
        Some(line) => {
            install_line_coordinates(source, &line);
            source.frame.state = LexerState::NewLine;
            source.frame.line = line.text;
            source.frame.byte_offset = 0;
            source.frame.physical_line_start = line.physical_start;
            source.frame.physical_content_end = line.physical_content_end;
            source.frame.terminator_start = line.terminator_start;
            source.frame.terminator_end = line.terminator_end;
            source.frame.normalized_end_anchor = line.normalized_end_anchor;
            source.frame.synthetic_endline_start = line.synthetic_endline_start;
            source.frame.line_number += 1;
            source.frame.column = 0;
            source.next_source_offset = line.terminator_end;
            Ok(true)
        }
        None => Ok(false),
    }
}

fn ensure_source_registered(source: &mut SourceInputFrame, stores: &mut impl ExpansionState) {
    if source.registration_attempted {
        return;
    }
    source.registration_attempted = true;
    if let Some(descriptor) = source.descriptor.clone() {
        // Diagnostic metadata exhaustion must not stop semantic tokenization.
        source.registration = stores
            .register_input_source(source.source_id, descriptor)
            .ok();
    }
}

fn install_line_coordinates(source: &mut SourceInputFrame, line: &NormalizedLine) {
    if let Some(cursor) = &mut source.layout_cursor {
        let (registration, origin_line_start) = cursor
            .line_registration(line.physical_start, line.terminator_end)
            .expect("physical editor line must be contained by one layout piece");
        source.registration = Some(registration);
        source.frame.origin_line_start = origin_line_start;
    } else {
        source.frame.origin_line_start = u64::try_from(line.physical_start).unwrap_or(u64::MAX);
    }
}

fn map_input_source_error(
    source: &SourceInputFrame,
    error: InputSourceError,
    fallback: LexSourceContext,
) -> LexError {
    match error {
        InputSourceError::World(error) => LexError::Input {
            error: Box::new(error),
            context: fallback,
            site: Box::new(DiagnosticSite::unknown()),
        },
        InputSourceError::InvalidUtf8 {
            byte_start,
            byte_end,
            line,
            column,
        } => LexError::InvalidUtf8 {
            context: LexSourceContext {
                source_id: source.source_id,
                input_record: source.input_record,
                byte_offset: u64::try_from(byte_start).unwrap_or(u64::MAX),
                byte_end: u64::try_from(byte_end).unwrap_or(u64::MAX),
                line: u32::try_from(line).unwrap_or(u32::MAX),
                column: u32::try_from(column).unwrap_or(u32::MAX),
            },
            site: Box::new(DiagnosticSite::unknown()),
        },
    }
}

fn next_line_source_context(source: &SourceInputFrame) -> LexSourceContext {
    LexSourceContext {
        source_id: source.source_id,
        input_record: source.input_record,
        byte_offset: u64::try_from(source.next_source_offset).unwrap_or(u64::MAX),
        byte_end: u64::try_from(source.next_source_offset).unwrap_or(u64::MAX),
        line: u32::try_from(source.frame.line_number.saturating_add(1)).unwrap_or(u32::MAX),
        column: 0,
    }
}

fn source_coordinate(source: &SourceInputFrame) -> LexSourceContext {
    source_coordinate_from_frame(source.source_id, source.input_record, &source.frame)
}

fn source_coordinate_from_frame(
    source_id: SourceId,
    input_record: Option<InputRecordId>,
    frame: &SourceFrame,
) -> LexSourceContext {
    let line_offset = frame
        .synthetic_endline_start
        .filter(|start| frame.byte_offset >= *start)
        .map_or(frame.byte_offset, |_| {
            frame
                .normalized_end_anchor
                .saturating_sub(frame.physical_line_start)
        });
    let byte_offset = frame
        .origin_line_start
        .saturating_add(u64::try_from(line_offset).unwrap_or(u64::MAX));
    LexSourceContext {
        source_id,
        input_record,
        byte_offset,
        byte_end: byte_offset,
        line: u32::try_from(frame.line_number).unwrap_or(u32::MAX),
        column: u32::try_from(frame.column).unwrap_or(u32::MAX),
    }
}

fn traced_source_token(
    stores: &mut impl ExpansionState,
    registration: Option<RegisteredSource>,
    token: Token,
    start: LexSourceContext,
    end: LexSourceContext,
) -> DecodedTracedToken {
    let origin = match registration
        .and_then(|source| source.span(start.byte_offset, end.byte_offset).ok())
    {
        Some(span) => stores.source_span_origin(span),
        None => stores.source_range_origin(start.source_id, start.byte_offset, end.byte_offset),
    };
    DecodedTracedToken::new(token, origin)
}

fn source_range_origin(
    stores: &mut impl ExpansionState,
    registration: Option<RegisteredSource>,
    start: LexSourceContext,
    end: LexSourceContext,
) -> OriginId {
    match registration.and_then(|source| source.span(start.byte_offset, end.byte_offset).ok()) {
        Some(span) => stores.source_span_origin(span),
        None => stores.source_range_origin(start.source_id, start.byte_offset, end.byte_offset),
    }
}

#[inline(always)]
fn traced_ordinary_source_token(
    stores: &mut impl ExpansionState,
    registration: Option<RegisteredSource>,
    token: Token,
    start: LexSourceContext,
    end: LexSourceContext,
    scalar: char,
) -> DecodedTracedToken {
    let backed_one_scalar =
        end.byte_offset.checked_sub(start.byte_offset) == u64::try_from(scalar.len_utf8()).ok();
    let origin = if backed_one_scalar {
        match registration {
            Some(source) => source
                .direct_origin(start.byte_offset, end.byte_offset)
                .or_else(|| {
                    source
                        .span(start.byte_offset, end.byte_offset)
                        .ok()
                        .map(|span| stores.source_span_origin(span))
                })
                .unwrap_or(OriginId::UNKNOWN),
            None => stores.source_token_origin(start.source_id, start.byte_offset, end.byte_offset),
        }
    } else {
        source_range_origin(stores, registration, start, end)
    };
    DecodedTracedToken::new(token, origin)
}

fn allocate_source_origin(
    stores: &mut impl ExpansionState,
    registration: Option<RegisteredSource>,
    coordinate: LexSourceContext,
) -> OriginId {
    match registration.and_then(|registration| {
        registration
            .span(coordinate.byte_offset, coordinate.byte_end)
            .ok()
    }) {
        Some(span) => stores.source_span_origin(span),
        None => stores.source_range_origin(
            coordinate.source_id,
            coordinate.byte_offset,
            coordinate.byte_end,
        ),
    }
}

fn traced_inserted_token(
    stores: &mut impl ExpansionState,
    kind: InsertedOriginKind,
    token: Token,
    parent: OriginId,
) -> DecodedTracedToken {
    let origin = stores.inserted_origin(kind, token, parent);
    DecodedTracedToken::new(token, origin)
}

fn decode_traced_token(token: TracedTokenWord) -> Token {
    token
        .token()
        .expect("input stack must only deliver valid traced tokens")
}

fn next_token_from_line(
    source: &mut SourceInputFrame,
    stores: &mut impl ExpansionState,
    unicode_superscript_notation: bool,
) -> Result<Option<DecodedTracedToken>, LexError> {
    let start = source_coordinate(source);
    let ch = read_expanded_char(source, stores, unicode_superscript_notation);
    let cat = stores.catcode(ch);
    match cat {
        Catcode::Ignored => Ok(None),
        Catcode::Invalid => {
            let end = source_coordinate(source);
            let origin = source_range_origin(stores, source.registration, start, end);
            Err(LexError::InvalidCharacter {
                ch,
                context: LexSourceContext {
                    byte_end: end.byte_offset,
                    ..start
                },
                site: Box::new(DiagnosticSite::primary(origin)),
            })
        }
        Catcode::Comment => {
            source.frame.column += source.frame.line[source.frame.byte_offset..]
                .chars()
                .count();
            source.frame.byte_offset = source.frame.line.len();
            Ok(None)
        }
        Catcode::EndLine => {
            let parent = allocate_source_origin(stores, source.registration, start);
            let (token, kind) = match source.frame.state {
                LexerState::NewLine => {
                    let par = stores.intern("par");
                    (Token::Cs(par.symbol()), InsertedOriginKind::Paragraph)
                }
                LexerState::MidLine => (
                    Token::Char {
                        ch: ' ',
                        cat: Catcode::Space,
                    },
                    InsertedOriginKind::EndLine,
                ),
                LexerState::SkippingBlanks => return Ok(None),
            };
            source.frame.state = LexerState::NewLine;
            Ok(Some(traced_inserted_token(stores, kind, token, parent)))
        }
        Catcode::Space => match source.frame.state {
            LexerState::MidLine => {
                source.frame.state = LexerState::SkippingBlanks;
                Ok(Some(traced_ordinary_source_token(
                    stores,
                    source.registration,
                    Token::Char {
                        ch: ' ',
                        cat: Catcode::Space,
                    },
                    start,
                    source_coordinate(source),
                    ch,
                )))
            }
            LexerState::NewLine | LexerState::SkippingBlanks => Ok(None),
        },
        Catcode::Escape => Ok(Some(scan_control_sequence(
            source,
            stores,
            unicode_superscript_notation,
            start,
        ))),
        Catcode::Letter | Catcode::Superscript => {
            source.frame.state = LexerState::MidLine;
            Ok(Some(traced_ordinary_source_token(
                stores,
                source.registration,
                Token::Char { ch, cat },
                start,
                source_coordinate(source),
                ch,
            )))
        }
        Catcode::BeginGroup
        | Catcode::EndGroup
        | Catcode::MathShift
        | Catcode::AlignmentTab
        | Catcode::Parameter
        | Catcode::Subscript
        | Catcode::Other
        | Catcode::Active => {
            source.frame.state = LexerState::MidLine;
            Ok(Some(traced_ordinary_source_token(
                stores,
                source.registration,
                Token::Char { ch, cat },
                start,
                source_coordinate(source),
                ch,
            )))
        }
    }
}

fn next_token_from_line_readonly(
    source: &mut SourceInputFrame,
    stores: &impl ExpansionState,
    unicode_superscript_notation: bool,
) -> Result<Option<Token>, LexError> {
    let start = source_coordinate(source);
    let ch = read_expanded_char(source, stores, unicode_superscript_notation);
    let cat = stores.catcode(ch);
    match cat {
        Catcode::Ignored => Ok(None),
        Catcode::Invalid => Err(LexError::InvalidCharacter {
            ch,
            context: start,
            site: Box::new(DiagnosticSite::unknown()),
        }),
        Catcode::Comment => {
            source.frame.column += source.frame.line[source.frame.byte_offset..]
                .chars()
                .count();
            source.frame.byte_offset = source.frame.line.len();
            Ok(None)
        }
        Catcode::EndLine => {
            let token = match source.frame.state {
                LexerState::NewLine => {
                    let Some(par) = stores.symbol("par") else {
                        return Err(LexError::MissingControlSequence {
                            name: "par".to_owned(),
                            context: start,
                            site: Box::new(DiagnosticSite::unknown()),
                        });
                    };
                    Token::Cs(par.symbol())
                }
                LexerState::MidLine => Token::Char {
                    ch: ' ',
                    cat: Catcode::Space,
                },
                LexerState::SkippingBlanks => return Ok(None),
            };
            source.frame.state = LexerState::NewLine;
            Ok(Some(token))
        }
        Catcode::Space => match source.frame.state {
            LexerState::MidLine => {
                source.frame.state = LexerState::SkippingBlanks;
                Ok(Some(Token::Char {
                    ch: ' ',
                    cat: Catcode::Space,
                }))
            }
            LexerState::NewLine | LexerState::SkippingBlanks => Ok(None),
        },
        Catcode::Escape => Ok(Some(scan_control_sequence_readonly(
            source,
            stores,
            unicode_superscript_notation,
            start,
        )?)),
        Catcode::Letter | Catcode::Superscript => {
            source.frame.state = LexerState::MidLine;
            Ok(Some(Token::Char { ch, cat }))
        }
        Catcode::BeginGroup
        | Catcode::EndGroup
        | Catcode::MathShift
        | Catcode::AlignmentTab
        | Catcode::Parameter
        | Catcode::Subscript
        | Catcode::Other
        | Catcode::Active => {
            source.frame.state = LexerState::MidLine;
            Ok(Some(Token::Char { ch, cat }))
        }
    }
}

fn scan_control_sequence(
    source: &mut SourceInputFrame,
    stores: &mut impl ExpansionState,
    unicode_superscript_notation: bool,
    start: LexSourceContext,
) -> DecodedTracedToken {
    if source.frame.byte_offset >= source.frame.line.len() {
        source.frame.state = LexerState::SkippingBlanks;
        let token = Token::Cs(stores.intern("").symbol());
        return traced_source_token(
            stores,
            source.registration,
            token,
            start,
            source_coordinate(source),
        );
    }

    let ch = read_expanded_char(source, stores, unicode_superscript_notation);
    let cat = stores.catcode(ch);
    if cat != Catcode::Letter {
        source.frame.state = if cat == Catcode::Space {
            LexerState::SkippingBlanks
        } else {
            LexerState::MidLine
        };
        let token = Token::Cs(stores.intern(&ch.to_string()));
        return traced_source_token(
            stores,
            source.registration,
            token,
            start,
            source_coordinate(source),
        );
    }

    let mut name = String::from(ch);
    while source.frame.byte_offset < source.frame.line.len() {
        let mark = source.frame.byte_offset;
        let mark_col = source.frame.column;
        let next = read_expanded_char(source, stores, unicode_superscript_notation);
        if stores.catcode(next) == Catcode::Letter {
            name.push(next);
        } else {
            source.frame.byte_offset = mark;
            source.frame.column = mark_col;
            break;
        }
    }
    source.frame.state = LexerState::SkippingBlanks;
    let token = Token::Cs(stores.intern(&name).symbol());
    traced_source_token(
        stores,
        source.registration,
        token,
        start,
        source_coordinate(source),
    )
}

fn scan_control_sequence_readonly(
    source: &mut SourceInputFrame,
    stores: &impl ExpansionState,
    unicode_superscript_notation: bool,
    context: LexSourceContext,
) -> Result<Token, LexError> {
    if source.frame.byte_offset >= source.frame.line.len() {
        source.frame.state = LexerState::SkippingBlanks;
        return readonly_cs_token(stores, "", context);
    }

    let ch = read_expanded_char(source, stores, unicode_superscript_notation);
    let cat = stores.catcode(ch);
    if cat != Catcode::Letter {
        source.frame.state = if cat == Catcode::Space {
            LexerState::SkippingBlanks
        } else {
            LexerState::MidLine
        };
        return readonly_cs_token(stores, &ch.to_string(), context);
    }

    let mut name = String::from(ch);
    while source.frame.byte_offset < source.frame.line.len() {
        let mark = source.frame.byte_offset;
        let mark_col = source.frame.column;
        let next = read_expanded_char(source, stores, unicode_superscript_notation);
        if stores.catcode(next) == Catcode::Letter {
            name.push(next);
        } else {
            source.frame.byte_offset = mark;
            source.frame.column = mark_col;
            break;
        }
    }
    source.frame.state = LexerState::SkippingBlanks;
    readonly_cs_token(stores, &name, context)
}

fn readonly_cs_token(
    stores: &impl ExpansionState,
    name: &str,
    context: LexSourceContext,
) -> Result<Token, LexError> {
    stores
        .symbol(name)
        .map(Token::Cs)
        .ok_or_else(|| LexError::MissingControlSequence {
            name: name.to_owned(),
            context,
            site: Box::new(DiagnosticSite::unknown()),
        })
}

fn read_expanded_char(
    source: &mut SourceInputFrame,
    stores: &impl ExpansionState,
    unicode_superscript_notation: bool,
) -> char {
    let ch = source.frame.line[source.frame.byte_offset..]
        .chars()
        .next()
        .expect("caller checks that the byte cursor is not at line end");
    source.frame.byte_offset += ch.len_utf8();
    source.frame.column += 1;
    expand_superscript_notation(source, ch, stores, unicode_superscript_notation).unwrap_or(ch)
}

#[derive(Clone, Copy)]
struct CursorMark {
    byte_offset: usize,
    column: usize,
}

fn cursor_mark(frame: &SourceFrame) -> CursorMark {
    CursorMark {
        byte_offset: frame.byte_offset,
        column: frame.column,
    }
}

fn restore_cursor(frame: &mut SourceFrame, mark: CursorMark) {
    frame.byte_offset = mark.byte_offset;
    frame.column = mark.column;
}

fn take_char(frame: &mut SourceFrame) -> Option<char> {
    let ch = frame.line[frame.byte_offset..].chars().next()?;
    frame.byte_offset += ch.len_utf8();
    frame.column += 1;
    Some(ch)
}

fn take_ascii_hex(frame: &mut SourceFrame, count: usize) -> Option<u32> {
    let mark = cursor_mark(frame);
    let mut value = 0_u32;
    for _ in 0..count {
        let Some(ch) = take_char(frame) else {
            restore_cursor(frame, mark);
            return None;
        };
        let Some(digit) = ch.to_digit(16).filter(|_| ch.is_ascii()) else {
            restore_cursor(frame, mark);
            return None;
        };
        value = value * 16 + digit;
    }
    Some(value)
}

fn expand_superscript_notation(
    source: &mut SourceInputFrame,
    ch: char,
    stores: &impl ExpansionState,
    unicode_superscript_notation: bool,
) -> Option<char> {
    if stores.catcode(ch) != Catcode::Superscript {
        return None;
    }
    expand_superscript_after_first(source, stores, unicode_superscript_notation)
}

fn expand_superscript_after_first(
    source: &mut SourceInputFrame,
    stores: &impl ExpansionState,
    unicode_superscript_notation: bool,
) -> Option<char> {
    let saved = cursor_mark(&source.frame);
    let second = source.frame.line[source.frame.byte_offset..]
        .chars()
        .next()?;
    if stores.catcode(second) != Catcode::Superscript {
        return None;
    }
    take_char(&mut source.frame);

    if unicode_superscript_notation {
        let unicode_mark = cursor_mark(&source.frame);
        let third = take_char(&mut source.frame);
        let fourth = take_char(&mut source.frame);
        if third.is_some_and(|ch| stores.catcode(ch) == Catcode::Superscript)
            && fourth.is_some_and(|ch| stores.catcode(ch) == Catcode::Superscript)
            && let Some(value) = take_ascii_hex(&mut source.frame, 4)
            && let Some(decoded) = char::from_u32(value)
        {
            return Some(chain_superscript_expansion(
                source,
                decoded,
                stores,
                unicode_superscript_notation,
            ));
        }
        restore_cursor(&mut source.frame, unicode_mark);
    }

    let hex_mark = cursor_mark(&source.frame);
    if let Some(value) = take_ascii_hex(&mut source.frame, 2)
        && let Some(decoded) = char::from_u32(value)
    {
        return Some(chain_superscript_expansion(
            source,
            decoded,
            stores,
            unicode_superscript_notation,
        ));
    }
    restore_cursor(&mut source.frame, hex_mark);

    let Some(target) = take_char(&mut source.frame) else {
        restore_cursor(&mut source.frame, saved);
        return None;
    };
    let code = target as u32;
    let decoded = if code < 64 { code + 64 } else { code - 64 };
    let decoded = char::from_u32(decoded).or_else(|| {
        restore_cursor(&mut source.frame, saved);
        None
    })?;
    Some(chain_superscript_expansion(
        source,
        decoded,
        stores,
        unicode_superscript_notation,
    ))
}

fn chain_superscript_expansion(
    source: &mut SourceInputFrame,
    decoded: char,
    stores: &impl ExpansionState,
    unicode_superscript_notation: bool,
) -> char {
    if stores.catcode(decoded) == Catcode::Superscript {
        expand_superscript_after_first(source, stores, unicode_superscript_notation)
            .unwrap_or(decoded)
    } else {
        decoded
    }
}

impl<S> LineReader<S> {
    #[must_use]
    pub fn new(source: S) -> Self {
        Self { source }
    }

    #[must_use]
    pub fn into_inner(self) -> S {
        self.source
    }
}

impl<S> LineReader<S>
where
    S: InputSource,
{
    pub fn next_event(
        &mut self,
        stores: &impl ExpansionState,
    ) -> Result<Option<LineEvent>, InputSourceError> {
        Ok(self
            .next_normalized_line(stores)?
            .map(|line| LineEvent::Text(line.text)))
    }

    fn next_normalized_line(
        &mut self,
        stores: &impl ExpansionState,
    ) -> Result<Option<NormalizedLine>, InputSourceError> {
        let Some(line) = self.source.read_line()? else {
            return Ok(None);
        };
        Ok(Some(normalize_line(&line, stores.endlinechar())))
    }
}

#[derive(Debug)]
struct NormalizedLine {
    text: String,
    physical_start: usize,
    physical_content_end: usize,
    terminator_start: usize,
    terminator_end: usize,
    normalized_end_anchor: usize,
    synthetic_endline_start: Option<usize>,
}

fn normalize_line(line: &PhysicalLine, endlinechar: i32) -> NormalizedLine {
    let stripped = line.text.trim_end_matches(' ');
    let normalized_end_anchor = line.start + stripped.len();
    let mut normalized = stripped.to_owned();
    let mut synthetic_endline_start = None;
    if let Ok(value) = u32::try_from(endlinechar)
        && let Some(ch) = char::from_u32(value)
    {
        synthetic_endline_start = Some(normalized.len());
        normalized.push(ch);
    }
    NormalizedLine {
        text: normalized,
        physical_start: line.start,
        physical_content_end: line.content_end,
        terminator_start: line.terminator_start,
        terminator_end: line.terminator_end,
        normalized_end_anchor,
        synthetic_endline_start,
    }
}

fn next_physical_line(bytes: &[u8], next_offset: &mut usize) -> Option<PhysicalLine> {
    let start = *next_offset;
    if start >= bytes.len() {
        return None;
    }
    let newline = bytes[start..]
        .iter()
        .position(|byte| *byte == b'\n')
        .map(|relative| start + relative);
    let (terminator_start, terminator_end) = match newline {
        Some(index) => {
            let terminator_start = if index > start && bytes[index - 1] == b'\r' {
                index - 1
            } else {
                index
            };
            (terminator_start, index + 1)
        }
        None => (bytes.len(), bytes.len()),
    };
    let text = std::str::from_utf8(&bytes[start..terminator_start])
        .expect("input backing was validated as UTF-8")
        .to_owned();
    *next_offset = terminator_end;
    Some(PhysicalLine {
        text,
        start,
        content_end: terminator_start,
        terminator_start,
        terminator_end,
    })
}

#[cfg(test)]
mod tests;
