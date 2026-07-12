//! TeX input sources and line handling.
//!
//! This crate owns the line-oriented part of TeX's eyes. It normalizes
//! physical input lines before the semantic lexer state machine assigns
//! catcodes and produces tokens.

use std::collections::VecDeque;
use std::fmt;
use std::ops::{Index, IndexMut};
use std::sync::Arc;

use tex_state::ids::{OriginListId, TokenListId};
use tex_state::provenance::{
    DiagnosticSite, InsertedOriginKind, RelatedLocation, SyntheticOriginKind,
};
use tex_state::source_map::{RegisteredSource, SourceDescriptor};
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};
use tex_state::{ExpansionState, FileContent, InputRecordId, WorldError};

pub use tex_state::{
    ConditionFrameSummary, ConditionFrameToken, ConditionKind, ConditionLimb, InputFrameSummary,
    InputSummary, LexerState, MACRO_ARGUMENT_SLOTS, MacroArguments, SourceFrameSummary, SourceId,
    TokenListReplayKind, TracedTokenList,
};

/// Source of physical input lines.
///
/// The trait is local so M3's `World` can implement it without forcing the
/// lexer to know where bytes came from.
pub trait InputSource {
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
}

impl MemoryInput {
    #[must_use]
    pub fn new(input: impl Into<String>) -> Self {
        let input = input.into();
        let backing: Arc<[u8]> = Arc::from(input.as_bytes());
        Self {
            backing,
            next_offset: 0,
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
}

/// Content-addressed input source created from `World` file content.
#[derive(Debug)]
pub struct WorldInput {
    input_record: InputRecordId,
    backing: Arc<[u8]>,
    next_offset: usize,
    invalid_utf8: Option<(usize, usize, usize, usize)>,
}

impl WorldInput {
    #[must_use]
    pub fn from_content(content: FileContent) -> Self {
        let input_record = content.record();
        Self::from_bytes(input_record, content.shared_bytes(), 0)
    }

    #[must_use]
    pub fn from_content_after_lines(content: FileContent, lines_read: usize) -> Self {
        let input_record = content.record();
        Self::from_bytes(input_record, content.shared_bytes(), lines_read)
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
                    input_record,
                    backing: bytes,
                    next_offset,
                    invalid_utf8: None,
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
                    input_record,
                    backing: bytes,
                    next_offset: 0,
                    invalid_utf8: Some((byte_start, byte_end, line, column)),
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
        Some(self.input_record)
    }

    fn source_descriptor(&self) -> Option<SourceDescriptor> {
        Some(SourceDescriptor::world(
            self.input_record,
            u64::try_from(self.backing.len()).unwrap_or(u64::MAX),
        ))
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
            self.line.clone(),
            self.byte_offset,
            self.physical_content_end,
            self.terminator_start,
            self.terminator_end,
            self.normalized_end_anchor,
            self.synthetic_endline_start,
            self.pending.iter().copied().collect(),
            self.end_after_current_line,
        )
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
struct SourceInputFrame<S> {
    source_id: SourceId,
    input_record: Option<InputRecordId>,
    lines: LineReader<S>,
    frame: SourceFrame,
    next_source_offset: usize,
    descriptor: Option<SourceDescriptor>,
    registration_attempted: bool,
    registration: Option<RegisteredSource>,
}

impl<S> SourceInputFrame<S> {
    fn new(source_id: SourceId, source: S) -> Self
    where
        S: InputSource,
    {
        let input_record = source.input_record();
        let descriptor = source.source_descriptor();
        Self {
            source_id,
            input_record,
            lines: LineReader::new(source),
            frame: SourceFrame::new(),
            next_source_offset: 0,
            descriptor,
            registration_attempted: false,
            registration: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TokenListInputFrame {
    token_list: TokenListId,
    origin_list: OriginListId,
    replay_kind: TokenListReplayKind,
    index: usize,
    macro_arguments: MacroArguments,
    macro_invocation: OriginId,
    parent_macro_invocation: OriginId,
    replay_marker: Option<TokenListReplayMarker>,
}

/// Identifies one live token-list replay frame independently of its content.
///
/// The marker is intentionally absent from resumable input summaries: callers
/// use it only to delimit a synchronous replay operation on the current stack.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TokenListReplayMarker(u64);

#[derive(Debug)]
enum InputFrame<S> {
    Source(SourceInputFrame<S>),
    TokenList(TokenListInputFrame),
    Condition {
        token: ConditionFrameToken,
        condition: ConditionFrameSummary,
    },
}

#[derive(Debug)]
struct StableFrames<S> {
    slots: Vec<Option<InputFrame<S>>>,
    active: usize,
}

impl<S> StableFrames<S> {
    fn new() -> Self {
        Self {
            slots: Vec::new(),
            active: 0,
        }
    }
    fn from_vec(frames: Vec<InputFrame<S>>) -> Self {
        let active = frames.len();
        Self {
            slots: frames.into_iter().map(Some).collect(),
            active,
        }
    }
    fn push(&mut self, frame: InputFrame<S>) {
        self.slots.push(Some(frame));
        self.active += 1;
    }
    fn remove(&mut self, index: usize) -> InputFrame<S> {
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
    fn iter(&self) -> impl DoubleEndedIterator<Item = &InputFrame<S>> {
        self.slots.iter().filter_map(Option::as_ref)
    }
    fn iter_mut(&mut self) -> impl DoubleEndedIterator<Item = &mut InputFrame<S>> {
        self.slots.iter_mut().filter_map(Option::as_mut)
    }
    #[cfg(test)]
    fn last_mut(&mut self) -> Option<&mut InputFrame<S>> {
        self.slots.iter_mut().rev().find_map(Option::as_mut)
    }
    fn iter_indexed(&self) -> impl DoubleEndedIterator<Item = (usize, &InputFrame<S>)> {
        self.slots
            .iter()
            .enumerate()
            .filter_map(|(index, frame)| frame.as_ref().map(|frame| (index, frame)))
    }
}

impl<S> Index<usize> for StableFrames<S> {
    type Output = InputFrame<S>;
    fn index(&self, index: usize) -> &Self::Output {
        self.slots[index]
            .as_ref()
            .expect("input frame slot is live")
    }
}
impl<S> IndexMut<usize> for StableFrames<S> {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        self.slots[index]
            .as_mut()
            .expect("input frame slot is live")
    }
}
impl<'a, S> IntoIterator for &'a mut StableFrames<S> {
    type Item = &'a mut InputFrame<S>;
    type IntoIter = std::iter::FilterMap<
        std::slice::IterMut<'a, Option<InputFrame<S>>>,
        fn(&'a mut Option<InputFrame<S>>) -> Option<&'a mut InputFrame<S>>,
    >;
    fn into_iter(self) -> Self::IntoIter {
        self.slots.iter_mut().filter_map(Option::as_mut)
    }
}
impl<S> IntoIterator for StableFrames<S> {
    type Item = InputFrame<S>;
    type IntoIter = std::iter::Flatten<std::vec::IntoIter<Option<InputFrame<S>>>>;
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
    PushArgument(TracedTokenList),
}

enum TracedTokenReplay {
    Deliver(TracedTokenWord),
    DeliverNoExpand(TracedTokenWord),
    PushArgument(TracedTokenList),
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
    token: TracedTokenWord,
    suppress_expansion: bool,
}

impl TracedExpansionToken {
    #[must_use]
    pub const fn new(token: TracedTokenWord, suppress_expansion: bool) -> Self {
        Self {
            token,
            suppress_expansion,
        }
    }

    #[must_use]
    pub const fn traced_token(self) -> TracedTokenWord {
        self.token
    }

    #[must_use]
    pub fn token(self) -> Token {
        decode_traced_token(self.token)
    }

    #[must_use]
    pub const fn origin(self) -> OriginId {
        self.token.origin()
    }

    #[must_use]
    pub const fn suppress_expansion(self) -> bool {
        self.suppress_expansion
    }
}

/// TeX input stack for source frames and frozen token-list replay.
#[derive(Debug)]
pub struct InputStack<S> {
    frames: StableFrames<S>,
    token_frame_indices: Vec<usize>,
    condition_frame_indices: Vec<usize>,
    next_source_id: u32,
    unicode_superscript_notation: bool,
    last_source_frame: Option<LastSourceFrame>,
    next_replay_marker: u64,
    next_condition_token: u64,
    alignment_inputs: Vec<AlignmentInput>,
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
    delivered: Vec<(TracedTokenWord, AlignmentTokenDelivery)>,
}

/// Saved alignment-cell interception state while a nested preamble and body run.
///
/// Like TeX82's alignment-stack node, this value owns the exact outer state;
/// nested input cannot observe or replace it before the matching restore.
#[must_use]
pub struct AlignmentCellSuspension(Option<AlignmentInput>);

impl<S> InputStack<S> {
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
    pub fn new(source: S) -> Self
    where
        S: InputSource,
    {
        let mut stack = Self {
            frames: StableFrames::new(),
            token_frame_indices: Vec::new(),
            condition_frame_indices: Vec::new(),
            next_source_id: 0,
            unicode_superscript_notation: true,
            last_source_frame: None,
            next_replay_marker: 0,
            next_condition_token: 0,
            alignment_inputs: Vec::new(),
            active_macro_invocation: OriginId::UNKNOWN,
            recently_popped_invocation: None,
        };
        stack.push_source(source);
        stack
    }

    pub fn push_source(&mut self, source: S) -> SourceId
    where
        S: InputSource,
    {
        let source_id = SourceId::new(self.next_source_id);
        self.next_source_id = self
            .next_source_id
            .checked_add(1)
            .expect("source id counter overflowed");
        self.push_frame(InputFrame::Source(SourceInputFrame::new(source_id, source)));
        source_id
    }

    pub fn from_summary<E, F>(summary: &InputSummary, mut reopen_source: F) -> Result<Self, E>
    where
        S: InputSource,
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
                    frames.push(InputFrame::Source(SourceInputFrame {
                        source_id: *source_id,
                        input_record: *input_record,
                        lines: LineReader::new(reopened),
                        frame: SourceFrame::from_summary(source),
                        next_source_offset: source.next_source_offset(),
                        descriptor,
                        registration_attempted: source.registration().is_some(),
                        registration: source.registration(),
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
                    token_list: *token_list,
                    origin_list: *origin_list,
                    replay_kind: *replay_kind,
                    index: *index,
                    macro_arguments: *macro_arguments,
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
                    InputFrameSummary::Source { .. } | InputFrameSummary::TokenList { .. } => None,
                })
                .max()
                .map_or(0, |token| {
                    token
                        .checked_add(1)
                        .expect("condition frame token overflowed")
                }),
            alignment_inputs: Vec::new(),
            active_macro_invocation,
            recently_popped_invocation: None,
        })
    }

    /// Starts one TeX82 alignment-scanner level before `scan_spec`.
    pub fn begin_alignment(&mut self) {
        self.alignment_inputs.push(AlignmentInput {
            align_state: -1_000_000,
            cell: None,
            delivered: Vec::new(),
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

    /// Restores the entry baseline after the triggering token is backed up,
    /// so an error-recovery `\\cr` is intercepted at entry depth.
    pub fn reset_alignment_cell_to_base_depth(&mut self) {
        if let Some(alignment) = self.alignment_inputs.last_mut()
            && alignment
                .cell
                .as_ref()
                .is_some_and(|cell| cell.phase == AlignmentCellPhase::Body)
        {
            alignment.align_state = 0;
        }
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
        alignment.delivered.push((traced, delivery));
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
        let Some(index) = alignment
            .delivered
            .iter()
            .rposition(|(token, _)| *token == traced)
        else {
            return;
        };
        let (_, delivery) = alignment.delivered.remove(index);
        match delivery {
            AlignmentTokenDelivery::Other => {}
            AlignmentTokenDelivery::LeftBrace => alignment.align_state -= 1,
            AlignmentTokenDelivery::RightBrace => alignment.align_state += 1,
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
        let replay_marker = TokenListReplayMarker(self.next_replay_marker);
        self.next_replay_marker = self
            .next_replay_marker
            .checked_add(1)
            .expect("token-list replay marker overflowed");
        self.push_frame(InputFrame::TokenList(TokenListInputFrame {
            token_list,
            origin_list,
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
            token_list,
            origin_list,
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
                            .with_registration(source.registration),
                    },
                    InputFrame::TokenList(token_list) => InputFrameSummary::TokenList {
                        token_list: token_list.token_list,
                        origin_list: token_list.origin_list,
                        replay_kind: token_list.replay_kind,
                        index: token_list.index,
                        macro_arguments: token_list.macro_arguments,
                        macro_invocation: token_list.macro_invocation,
                        parent_macro_invocation: token_list.parent_macro_invocation,
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
            return allocate_source_origin(stores, source_coordinate(source));
        }
        if let Some(last) = &self.last_source_frame {
            return allocate_source_origin(
                stores,
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
        let frame_is_live = self.frames.iter().any(
            |frame| matches!(frame, InputFrame::Source(source) if source.source_id == first.source),
        );
        frame_is_live.then(|| stores.source_range_origin(first.source, first.start, last.end))
    }

    fn diagnostic_expansion_head(&self) -> Option<OriginId> {
        self.recently_popped_invocation
            .or((self.active_macro_invocation != OriginId::UNKNOWN)
                .then_some(self.active_macro_invocation))
    }

    fn retire_token_list_frame(&mut self, frame: TokenListInputFrame) {
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
pub struct Lexer<S> {
    input: InputStack<S>,
}

impl<S> Lexer<S> {
    #[must_use]
    pub fn new(source: S) -> Self
    where
        S: InputSource,
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
    pub fn input_stack(&self) -> &InputStack<S> {
        &self.input
    }

    pub fn input_stack_mut(&mut self) -> &mut InputStack<S> {
        &mut self.input
    }

    #[must_use]
    pub fn into_inner(self) -> S {
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

impl<S> Lexer<S>
where
    S: InputSource,
{
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

impl<S> InputStack<S>
where
    S: InputSource,
{
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
                    match next_traced_token_from_token_list_frame(token_list, stores) {
                        Some(TracedTokenReplay::PushArgument(argument)) => {
                            self.push_frame(InputFrame::TokenList(TokenListInputFrame {
                                token_list: argument.token_list(),
                                origin_list: argument.origin_list(),
                                replay_kind: TokenListReplayKind::MacroArgument,
                                index: 0,
                                macro_arguments: MacroArguments::new(),
                                macro_invocation: OriginId::UNKNOWN,
                                parent_macro_invocation: OriginId::UNKNOWN,
                                replay_marker: None,
                            }));
                            continue;
                        }
                        Some(
                            TracedTokenReplay::Deliver(token)
                            | TracedTokenReplay::DeliverNoExpand(token),
                        ) => {
                            return Ok(Some(token));
                        }
                        None => {
                            let removed = self.remove_frame(frame_index);
                            if let InputFrame::TokenList(frame) = removed {
                                self.retire_token_list_frame(frame);
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
                        if !load_next_line(source, stores)? {
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

                    let Some(token) =
                        next_token_from_line(source, stores, self.unicode_superscript_notation)?
                    else {
                        continue;
                    };
                    return Ok(Some(token));
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
                    match next_traced_token_from_token_list_frame(token_list, stores) {
                        Some(TracedTokenReplay::PushArgument(argument)) => {
                            self.push_frame(InputFrame::TokenList(TokenListInputFrame {
                                token_list: argument.token_list(),
                                origin_list: argument.origin_list(),
                                replay_kind: TokenListReplayKind::MacroArgument,
                                index: 0,
                                macro_arguments: MacroArguments::new(),
                                macro_invocation: OriginId::UNKNOWN,
                                parent_macro_invocation: OriginId::UNKNOWN,
                                replay_marker: None,
                            }));
                            continue;
                        }
                        Some(TracedTokenReplay::Deliver(token)) => {
                            return Ok(Some(TracedExpansionToken::new(token, false)));
                        }
                        Some(TracedTokenReplay::DeliverNoExpand(token)) => {
                            return Ok(Some(TracedExpansionToken::new(token, true)));
                        }
                        None => {
                            let removed = self.remove_frame(frame_index);
                            if let InputFrame::TokenList(frame) = removed {
                                self.retire_token_list_frame(frame);
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
                        if !load_next_line(source, stores)? {
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

                    let Some(token) =
                        next_token_from_line(source, stores, self.unicode_superscript_notation)?
                    else {
                        continue;
                    };
                    return Ok(Some(TracedExpansionToken::new(token, false)));
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
                        Some(TokenReplay::PushArgument(argument)) => {
                            self.push_frame(InputFrame::TokenList(TokenListInputFrame {
                                token_list: argument.token_list(),
                                origin_list: argument.origin_list(),
                                replay_kind: TokenListReplayKind::MacroArgument,
                                index: 0,
                                macro_arguments: MacroArguments::new(),
                                macro_invocation: OriginId::UNKNOWN,
                                parent_macro_invocation: OriginId::UNKNOWN,
                                replay_marker: None,
                            }));
                            continue;
                        }
                        Some(TokenReplay::Deliver(token)) => {
                            return Ok(Some(ExpansionToken::new(token, false)));
                        }
                        Some(TokenReplay::DeliverNoExpand(token)) => {
                            return Ok(Some(ExpansionToken::new(token, true)));
                        }
                        None => {
                            self.remove_frame(frame_index);
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

impl<S> InputStack<S> {
    #[must_use]
    pub fn current_token_list_frame(&self) -> Option<(TokenListId, TokenListReplayKind, usize)> {
        let frame_index = self.current_token_frame_index()?;
        match &self.frames[frame_index] {
            InputFrame::TokenList(token_list) => Some((
                token_list.token_list,
                token_list.replay_kind,
                token_list.index,
            )),
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
                if frame.token_list == token_list && frame.replay_kind == replay_kind
        );
        if matches {
            let removed = self.remove_frame(frame_index);
            if let InputFrame::TokenList(frame) = removed {
                self.retire_token_list_frame(frame);
            }
        }
        matches
    }

    /// Removes a scoped replay frame and every nested frame it introduced.
    ///
    /// This is the input-stack half of recursive execution rollback. It does
    /// not rewind source frames that predate the replay capability.
    pub fn abort_token_list_replay(&mut self, marker: TokenListReplayMarker) -> bool {
        let Some(target) = self.frames.iter_indexed().find_map(|(index, frame)| {
            matches!(frame, InputFrame::TokenList(frame) if frame.replay_marker == Some(marker))
                .then_some(index)
        }) else {
            return false;
        };
        let indices = self
            .frames
            .iter_indexed()
            .filter_map(|(index, _)| (index >= target).then_some(index))
            .collect::<Vec<_>>();
        for index in indices.into_iter().rev() {
            let removed = self.remove_frame(index);
            if let InputFrame::TokenList(frame) = removed {
                self.retire_token_list_frame(frame);
            }
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
                    if frame.token_list == token_list && frame.replay_kind == replay_kind
            )
        })
    }

    /// Returns whether a synchronously delimited replay frame is still live.
    #[must_use]
    pub fn contains_token_list_replay_marker(&self, marker: TokenListReplayMarker) -> bool {
        self.frames.iter().any(|frame| {
            matches!(
                frame,
                InputFrame::TokenList(frame) if frame.replay_marker == Some(marker)
            )
        })
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
        let Some(marked_index) = self.frames.iter_indexed().find_map(|(index, frame)| {
            matches!(
                frame,
                InputFrame::TokenList(frame) if frame.replay_marker == Some(marker)
            )
            .then_some(index)
        }) else {
            return true;
        };

        let can_finish = self
            .frames
            .iter_indexed()
            .filter(|(index, _)| *index >= marked_index)
            .all(|(_, frame)| match frame {
                InputFrame::TokenList(frame) => {
                    frame.index >= stores.tokens(frame.token_list).len()
                }
                InputFrame::Condition { .. } => true,
                InputFrame::Source(_) => false,
            });
        if !can_finish {
            return false;
        }

        let retire = self
            .frames
            .iter_indexed()
            .filter_map(|(index, frame)| {
                (index >= marked_index && matches!(frame, InputFrame::TokenList(_)))
                    .then_some(index)
            })
            .collect::<Vec<_>>();
        for index in retire.into_iter().rev() {
            if matches!(self.frames[index], InputFrame::TokenList(_)) {
                let removed = self.remove_frame(index);
                if let InputFrame::TokenList(frame) = removed {
                    self.retire_token_list_frame(frame);
                }
            }
        }
        true
    }

    fn current_token_frame_index(&self) -> Option<usize> {
        self.token_frame_indices.last().copied()
    }

    fn push_frame(&mut self, frame: InputFrame<S>) {
        let index = self.frames.slot_len();
        match frame {
            InputFrame::Source(_) | InputFrame::TokenList(_) => {
                self.token_frame_indices.push(index)
            }
            InputFrame::Condition { .. } => self.condition_frame_indices.push(index),
        }
        self.frames.push(frame);
    }

    fn remove_frame(&mut self, index: usize) -> InputFrame<S> {
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
}

fn next_token_from_token_list_frame(
    frame: &mut TokenListInputFrame,
    stores: &impl ExpansionState,
) -> Option<TokenReplay> {
    let tokens = stores.tokens(frame.token_list);
    let token = tokens.get(frame.index).copied()?;
    frame.index += 1;

    if frame.replay_kind == TokenListReplayKind::MacroBody
        && let Token::Param(slot) = token
        && let Some(argument) = frame.macro_arguments.get_traced(slot)
    {
        return Some(TokenReplay::PushArgument(argument));
    }

    if frame.replay_kind == TokenListReplayKind::NoExpand {
        return Some(TokenReplay::DeliverNoExpand(token));
    }

    Some(TokenReplay::Deliver(token))
}

fn next_traced_token_from_token_list_frame(
    frame: &mut TokenListInputFrame,
    stores: &mut impl ExpansionState,
) -> Option<TracedTokenReplay> {
    let tokens = stores.tokens(frame.token_list);
    let token = tokens.get(frame.index).copied()?;
    frame.index += 1;

    if frame.replay_kind == TokenListReplayKind::MacroBody
        && let Token::Param(slot) = token
        && let Some(argument) = frame.macro_arguments.get_traced(slot)
    {
        return Some(TracedTokenReplay::PushArgument(argument));
    }

    let origin = replay_origin(frame, stores, token);
    let token = TracedTokenWord::pack(token, origin);
    if frame.replay_kind == TokenListReplayKind::NoExpand {
        return Some(TracedTokenReplay::DeliverNoExpand(token));
    }

    Some(TracedTokenReplay::Deliver(token))
}

fn replay_origin(
    frame: &TokenListInputFrame,
    stores: &mut impl ExpansionState,
    token: Token,
) -> OriginId {
    if frame.origin_list == OriginListId::EMPTY {
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

    let Some(origins) = stores.origin_list_if_live(frame.origin_list) else {
        return OriginId::UNKNOWN;
    };
    let token_len = stores.tokens(frame.token_list).len();
    assert_eq!(
        origins.len(),
        token_len,
        "token-list replay origin-list length does not match token-list length"
    );
    origins[frame.index - 1]
}

fn load_next_line_readonly<S>(
    source: &mut SourceInputFrame<S>,
    stores: &impl ExpansionState,
) -> Result<bool, LexError>
where
    S: InputSource,
{
    let context = next_line_source_context(source);
    match source
        .lines
        .next_normalized_line(stores)
        .map_err(|error| map_input_source_error(source, error, context))?
    {
        Some(line) => {
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

fn load_next_line<S>(
    source: &mut SourceInputFrame<S>,
    stores: &mut impl ExpansionState,
) -> Result<bool, LexError>
where
    S: InputSource,
{
    let context = next_line_source_context(source);
    match source.lines.next_normalized_line(stores).map_err(|error| {
        map_input_source_error(source, error, context).with_physical_site(stores)
    })? {
        Some(line) => {
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

fn ensure_source_registered<S>(source: &mut SourceInputFrame<S>, stores: &mut impl ExpansionState) {
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

fn map_input_source_error<S>(
    source: &SourceInputFrame<S>,
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

fn next_line_source_context<S>(source: &SourceInputFrame<S>) -> LexSourceContext {
    LexSourceContext {
        source_id: source.source_id,
        input_record: source.input_record,
        byte_offset: u64::try_from(source.next_source_offset).unwrap_or(u64::MAX),
        byte_end: u64::try_from(source.next_source_offset).unwrap_or(u64::MAX),
        line: u32::try_from(source.frame.line_number.saturating_add(1)).unwrap_or(u32::MAX),
        column: 0,
    }
}

fn source_coordinate<S>(source: &SourceInputFrame<S>) -> LexSourceContext {
    source_coordinate_from_frame(source.source_id, source.input_record, &source.frame)
}

fn source_coordinate_from_frame(
    source_id: SourceId,
    input_record: Option<InputRecordId>,
    frame: &SourceFrame,
) -> LexSourceContext {
    let byte_offset = frame
        .synthetic_endline_start
        .filter(|start| frame.byte_offset >= *start)
        .map_or(frame.physical_line_start + frame.byte_offset, |_| {
            frame.normalized_end_anchor
        });
    LexSourceContext {
        source_id,
        input_record,
        byte_offset: u64::try_from(byte_offset).unwrap_or(u64::MAX),
        byte_end: u64::try_from(byte_offset).unwrap_or(u64::MAX),
        line: u32::try_from(frame.line_number).unwrap_or(u32::MAX),
        column: u32::try_from(frame.column).unwrap_or(u32::MAX),
    }
}

fn traced_source_token(
    stores: &mut impl ExpansionState,
    token: Token,
    start: LexSourceContext,
    end: LexSourceContext,
) -> TracedTokenWord {
    let origin = stores.source_range_origin(start.source_id, start.byte_offset, end.byte_offset);
    TracedTokenWord::pack(token, origin)
}

#[inline(always)]
fn traced_ordinary_source_token(
    stores: &mut impl ExpansionState,
    registration: Option<RegisteredSource>,
    token: Token,
    start: LexSourceContext,
    end: LexSourceContext,
    scalar: char,
) -> TracedTokenWord {
    let backed_one_scalar =
        end.byte_offset.checked_sub(start.byte_offset) == u64::try_from(scalar.len_utf8()).ok();
    let origin = if backed_one_scalar {
        if let Some(origin) =
            registration.and_then(|source| source.direct_origin(start.byte_offset, end.byte_offset))
        {
            origin
        } else {
            stores.source_token_origin(start.source_id, start.byte_offset, end.byte_offset)
        }
    } else {
        stores.source_range_origin(start.source_id, start.byte_offset, end.byte_offset)
    };
    TracedTokenWord::pack(token, origin)
}

fn allocate_source_origin(
    stores: &mut impl ExpansionState,
    coordinate: LexSourceContext,
) -> OriginId {
    stores.source_range_origin(
        coordinate.source_id,
        coordinate.byte_offset,
        coordinate.byte_end,
    )
}

fn traced_inserted_token(
    stores: &mut impl ExpansionState,
    kind: InsertedOriginKind,
    token: Token,
    parent: OriginId,
) -> TracedTokenWord {
    let origin = stores.inserted_origin(kind, token, parent);
    TracedTokenWord::pack(token, origin)
}

fn decode_traced_token(token: TracedTokenWord) -> Token {
    token
        .token()
        .expect("input stack must only deliver valid traced tokens")
}

fn next_token_from_line<S>(
    source: &mut SourceInputFrame<S>,
    stores: &mut impl ExpansionState,
    unicode_superscript_notation: bool,
) -> Result<Option<TracedTokenWord>, LexError> {
    let start = source_coordinate(source);
    let ch = read_expanded_char(source, stores, unicode_superscript_notation);
    let cat = stores.catcode(ch);
    match cat {
        Catcode::Ignored => Ok(None),
        Catcode::Invalid => {
            let end = source_coordinate(source);
            let origin =
                stores.source_range_origin(start.source_id, start.byte_offset, end.byte_offset);
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
            let parent = allocate_source_origin(stores, start);
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

fn next_token_from_line_readonly<S>(
    source: &mut SourceInputFrame<S>,
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

fn scan_control_sequence<S>(
    source: &mut SourceInputFrame<S>,
    stores: &mut impl ExpansionState,
    unicode_superscript_notation: bool,
    start: LexSourceContext,
) -> TracedTokenWord {
    if source.frame.byte_offset >= source.frame.line.len() {
        source.frame.state = LexerState::SkippingBlanks;
        let token = Token::Cs(stores.intern("").symbol());
        return traced_source_token(stores, token, start, source_coordinate(source));
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
        return traced_source_token(stores, token, start, source_coordinate(source));
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
    traced_source_token(stores, token, start, source_coordinate(source))
}

fn scan_control_sequence_readonly<S>(
    source: &mut SourceInputFrame<S>,
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

fn read_expanded_char<S>(
    source: &mut SourceInputFrame<S>,
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

fn expand_superscript_notation<S>(
    source: &mut SourceInputFrame<S>,
    ch: char,
    stores: &impl ExpansionState,
    unicode_superscript_notation: bool,
) -> Option<char> {
    if stores.catcode(ch) != Catcode::Superscript {
        return None;
    }
    expand_superscript_after_first(source, stores, unicode_superscript_notation)
}

fn expand_superscript_after_first<S>(
    source: &mut SourceInputFrame<S>,
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

fn chain_superscript_expansion<S>(
    source: &mut SourceInputFrame<S>,
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
