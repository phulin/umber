//! TeX input sources and line handling.
//!
//! This crate owns the line-oriented part of TeX's eyes. It normalizes
//! physical input lines before the semantic lexer state machine assigns
//! catcodes and produces tokens.

use std::collections::VecDeque;
use std::fmt;

use tex_state::ids::{OriginListId, TokenListId};
use tex_state::provenance::{InsertedOriginKind, SyntheticOriginKind};
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
    /// Reads the next physical line without its line terminator.
    fn read_line(&mut self) -> Result<Option<String>, WorldError>;

    /// Returns the durable `World` record for a file-backed source.
    fn input_record(&self) -> Option<InputRecordId> {
        None
    }
}

/// In-memory input source for tests, `\scantokens`, and editor buffers.
#[derive(Debug)]
pub struct MemoryInput {
    lines: std::vec::IntoIter<String>,
}

impl MemoryInput {
    #[must_use]
    pub fn new(input: impl Into<String>) -> Self {
        Self {
            lines: split_physical_lines(&input.into()).into_iter(),
        }
    }
}

impl InputSource for MemoryInput {
    fn read_line(&mut self) -> Result<Option<String>, WorldError> {
        Ok(self.lines.next())
    }
}

/// Content-addressed input source created from `World` file content.
#[derive(Debug)]
pub struct WorldInput {
    input_record: InputRecordId,
    lines: std::vec::IntoIter<String>,
}

impl WorldInput {
    #[must_use]
    pub fn from_content(content: FileContent) -> Self {
        let input_record = content.record();
        let input = String::from_utf8_lossy(content.bytes()).into_owned();
        Self {
            input_record,
            lines: split_physical_lines(&input).into_iter(),
        }
    }

    #[must_use]
    pub fn from_content_after_lines(content: FileContent, lines_read: usize) -> Self {
        let input_record = content.record();
        let input = String::from_utf8_lossy(content.bytes()).into_owned();
        Self {
            input_record,
            lines: split_physical_lines(&input)
                .into_iter()
                .skip(lines_read)
                .collect::<Vec<_>>()
                .into_iter(),
        }
    }
}

impl InputSource for WorldInput {
    fn read_line(&mut self) -> Result<Option<String>, WorldError> {
        Ok(self.lines.next())
    }

    fn input_record(&self) -> Option<InputRecordId> {
        Some(self.input_record)
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
    line: Vec<char>,
    offset: usize,
    pending: VecDeque<TracedTokenWord>,
    buffer_offset: usize,
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
        self.offset
    }

    #[must_use]
    pub fn buffer_offset(&self) -> usize {
        self.buffer_offset
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
        SourceFrameSummary::new(
            self.buffer_offset,
            next_source_offset,
            self.line_number,
            self.column,
            self.state,
            self.line.iter().collect(),
            self.offset,
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
            line: summary.normalized_line().chars().collect(),
            offset: summary.line_char_offset(),
            pending: summary.pending().iter().copied().collect(),
            buffer_offset: summary.buffer_offset(),
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
}

impl<S> SourceInputFrame<S> {
    fn new(source_id: SourceId, source: S) -> Self
    where
        S: InputSource,
    {
        let input_record = source.input_record();
        Self {
            source_id,
            input_record,
            lines: LineReader::new(source),
            frame: SourceFrame::new(),
            next_source_offset: 0,
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

#[derive(Clone, Debug, Eq, PartialEq)]
struct LastSourceFrame {
    source_id: SourceId,
    input_record: Option<InputRecordId>,
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
    frames: Vec<InputFrame<S>>,
    next_source_id: u32,
    unicode_superscript_notation: bool,
    last_source_frame: Option<LastSourceFrame>,
    next_replay_marker: u64,
    next_condition_token: u64,
    alignment_cells: Vec<AlignmentCellInput>,
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

#[derive(Clone, Copy, Debug)]
struct AlignmentCellInput {
    phase: AlignmentCellPhase,
    v_template: TokenListId,
    group_depth: u32,
    terminator: Option<TracedTokenWord>,
}

/// Saved alignment-cell interception state while a nested preamble and body run.
///
/// Like TeX82's alignment-stack node, this value owns the exact outer state;
/// nested input cannot observe or replace it before the matching restore.
#[must_use]
pub struct AlignmentCellSuspension(Option<AlignmentCellInput>);

impl<S> InputStack<S> {
    #[must_use]
    pub fn new(source: S) -> Self
    where
        S: InputSource,
    {
        let mut stack = Self {
            frames: Vec::new(),
            next_source_id: 0,
            unicode_superscript_notation: true,
            last_source_frame: None,
            next_replay_marker: 0,
            next_condition_token: 0,
            alignment_cells: Vec::new(),
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
        self.frames
            .push(InputFrame::Source(SourceInputFrame::new(source_id, source)));
        source_id
    }

    pub fn from_summary<E, F>(summary: &InputSummary, mut reopen_source: F) -> Result<Self, E>
    where
        F: FnMut(SourceId, Option<InputRecordId>, &SourceFrameSummary) -> Result<S, E>,
    {
        let mut max_source_id = None::<u32>;
        let mut frames = Vec::with_capacity(summary.frames().len());
        for frame in summary.frames() {
            match frame {
                InputFrameSummary::Source {
                    source_id,
                    input_record,
                    source,
                } => {
                    max_source_id =
                        Some(max_source_id.map_or(source_id.raw(), |max| max.max(source_id.raw())));
                    frames.push(InputFrame::Source(SourceInputFrame {
                        source_id: *source_id,
                        input_record: *input_record,
                        lines: LineReader::new(reopen_source(*source_id, *input_record, source)?),
                        frame: SourceFrame::from_summary(source),
                        next_source_offset: source.next_source_offset(),
                    }));
                }
                InputFrameSummary::TokenList {
                    token_list,
                    origin_list,
                    replay_kind,
                    index,
                    macro_arguments,
                    macro_invocation,
                } => frames.push(InputFrame::TokenList(TokenListInputFrame {
                    token_list: *token_list,
                    origin_list: *origin_list,
                    replay_kind: *replay_kind,
                    index: *index,
                    macro_arguments: *macro_arguments,
                    macro_invocation: *macro_invocation,
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

        Ok(Self {
            frames,
            next_source_id: max_source_id.map_or(0, |id| {
                id.checked_add(1).expect("source id counter overflowed")
            }),
            unicode_superscript_notation: true,
            last_source_frame: summary.last_source_frame().map(|source| LastSourceFrame {
                source_id: summary
                    .last_source_id()
                    .expect("last source frame must retain its source id"),
                input_record: summary.last_source_record(),
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
            alignment_cells: Vec::new(),
        })
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
        group_depth: u32,
    ) {
        self.alignment_cells.push(AlignmentCellInput {
            phase: u_template.map_or(AlignmentCellPhase::Body, AlignmentCellPhase::UTemplate),
            v_template,
            group_depth,
            terminator: None,
        });
    }

    /// Completes the active cell after its frozen end-v token is delivered.
    pub fn finish_alignment_cell(&mut self) -> Option<TracedTokenWord> {
        let cell = self.alignment_cells.pop()?;
        assert_eq!(cell.phase, AlignmentCellPhase::VTemplate);
        cell.terminator
    }

    #[must_use]
    pub fn alignment_cell_at_group_depth(&self, group_depth: u32) -> bool {
        self.alignment_cells
            .last()
            .is_some_and(|cell| group_depth == cell.group_depth)
    }

    /// Suspends an outer cell while a nested alignment scans its preamble.
    pub fn suspend_alignment_cell(&mut self) -> AlignmentCellSuspension {
        let cell = self.alignment_cells.pop();
        if let Some(cell) = cell.as_ref() {
            assert_eq!(cell.phase, AlignmentCellPhase::Body);
        }
        AlignmentCellSuspension(cell)
    }

    pub fn resume_alignment_cell(&mut self, suspended: AlignmentCellSuspension) {
        assert!(
            self.alignment_cells.is_empty(),
            "nested alignment cell remained active at pop_alignment"
        );
        if let Some(cell) = suspended.0 {
            assert_eq!(cell.phase, AlignmentCellPhase::Body);
            self.alignment_cells.push(cell);
        }
    }

    /// Applies the alignment-sensitive part of TeX82 `get_next`.
    ///
    /// Returns `true` when the token was a cell terminator and has been
    /// replaced in the input by the active v-template.
    pub fn intercept_alignment_token(
        &mut self,
        traced: TracedTokenWord,
        terminator: Option<AlignmentTerminator>,
        group_depth: u32,
    ) -> bool {
        let Some(mut cell) = self.alignment_cells.pop() else {
            return false;
        };
        if let AlignmentCellPhase::UTemplate(marker) = cell.phase
            && !self.contains_token_list_replay_marker(marker)
        {
            cell.phase = AlignmentCellPhase::Body;
            cell.group_depth = group_depth;
        }
        if cell.phase != AlignmentCellPhase::Body {
            self.alignment_cells.push(cell);
            return false;
        }

        let terminates = group_depth == cell.group_depth && terminator.is_some();
        if terminates {
            cell.phase = AlignmentCellPhase::VTemplate;
            cell.terminator = Some(traced);
            self.push_token_list(cell.v_template, TokenListReplayKind::Inserted);
        }
        self.alignment_cells.push(cell);
        terminates
    }

    pub fn push_token_list(
        &mut self,
        token_list: TokenListId,
        replay_kind: TokenListReplayKind,
    ) -> TokenListReplayMarker {
        self.push_token_list_with_origins(token_list, OriginListId::EMPTY, replay_kind)
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
        self.frames.push(InputFrame::TokenList(TokenListInputFrame {
            token_list,
            origin_list,
            replay_kind,
            index: 0,
            macro_arguments: MacroArguments::new(),
            macro_invocation: OriginId::UNKNOWN,
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
        self.frames.push(InputFrame::TokenList(TokenListInputFrame {
            token_list,
            origin_list,
            replay_kind: TokenListReplayKind::MacroBody,
            index: 0,
            macro_arguments,
            macro_invocation,
            replay_marker: None,
        }));
    }

    pub fn push_condition(&mut self, condition: ConditionFrameSummary) -> ConditionFrameToken {
        let token = ConditionFrameToken::new(self.next_condition_token);
        self.next_condition_token = self
            .next_condition_token
            .checked_add(1)
            .expect("condition frame token overflowed");
        self.frames.push(InputFrame::Condition { token, condition });
        token
    }

    pub fn update_condition(
        &mut self,
        token: ConditionFrameToken,
        condition: ConditionFrameSummary,
    ) -> Option<ConditionFrameSummary> {
        let frame = self.frames.iter_mut().rev().find_map(|frame| match frame {
            InputFrame::Condition {
                token: frame_token,
                condition,
            } if *frame_token == token => Some(condition),
            InputFrame::Source(_) | InputFrame::TokenList(_) => None,
            InputFrame::Condition { .. } => None,
        })?;
        Some(std::mem::replace(frame, condition))
    }

    #[must_use]
    pub fn current_condition(&self) -> Option<ConditionFrameSummary> {
        self.frames.iter().rev().find_map(|frame| match frame {
            InputFrame::Condition { condition, .. } => Some(*condition),
            InputFrame::Source(_) | InputFrame::TokenList(_) => None,
        })
    }

    #[must_use]
    pub fn current_condition_token(&self) -> Option<ConditionFrameToken> {
        self.frames.iter().rev().find_map(|frame| match frame {
            InputFrame::Condition { token, .. } => Some(*token),
            InputFrame::Source(_) | InputFrame::TokenList(_) => None,
        })
    }

    pub fn pop_condition(&mut self) -> Option<ConditionFrameSummary> {
        let index = self
            .frames
            .iter()
            .rposition(|frame| matches!(frame, InputFrame::Condition { .. }))?;
        match self.frames.remove(index) {
            InputFrame::Condition { condition, .. } => Some(condition),
            InputFrame::Source(_) | InputFrame::TokenList(_) => unreachable!("rposition matched"),
        }
    }

    #[must_use]
    pub fn summary(&self) -> InputSummary {
        InputSummary::new_with_source_records(
            self.frames
                .iter()
                .map(|frame| match frame {
                    InputFrame::Source(source) => InputFrameSummary::Source {
                        source_id: source.source_id,
                        input_record: source.input_record,
                        source: source.frame.summary(source.next_source_offset),
                    },
                    InputFrame::TokenList(token_list) => InputFrameSummary::TokenList {
                        token_list: token_list.token_list,
                        origin_list: token_list.origin_list,
                        replay_kind: token_list.replay_kind,
                        index: token_list.index,
                        macro_arguments: token_list.macro_arguments,
                        macro_invocation: token_list.macro_invocation,
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
            self.last_source_frame
                .as_ref()
                .map(|last| last.frame.summary(last.next_source_offset)),
        )
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
        if let Some(source) = self.frames.iter().rev().find_map(|frame| match frame {
            InputFrame::Source(source) => Some(source),
            InputFrame::TokenList(_) | InputFrame::Condition { .. } => None,
        }) {
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
}

/// Errors produced while converting characters to TeX tokens.
#[derive(Debug)]
pub enum LexError {
    Input(WorldError),
    InvalidCharacter(char),
    MissingControlSequence(String),
}

impl fmt::Display for LexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Input(err) => write!(f, "input read failed: {err}"),
            Self::InvalidCharacter(ch) => {
                write!(
                    f,
                    "input contains invalid TeX character U+{:04X}",
                    *ch as u32
                )
            }
            Self::MissingControlSequence(name) => {
                write!(f, "control sequence {name:?} is not interned")
            }
        }
    }
}

impl std::error::Error for LexError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Input(err) => Some(err),
            Self::InvalidCharacter(_) => None,
            Self::MissingControlSequence(_) => None,
        }
    }
}

impl From<WorldError> for LexError {
    fn from(value: WorldError) -> Self {
        Self::Input(value)
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
        loop {
            let Some(frame_index) = self.current_token_frame_index() else {
                return Ok(None);
            };
            match &mut self.frames[frame_index] {
                InputFrame::TokenList(token_list) => {
                    match next_traced_token_from_token_list_frame(token_list, stores) {
                        Some(TracedTokenReplay::PushArgument(argument)) => {
                            self.frames.push(InputFrame::TokenList(TokenListInputFrame {
                                token_list: argument.token_list(),
                                origin_list: argument.origin_list(),
                                replay_kind: TokenListReplayKind::MacroArgument,
                                index: 0,
                                macro_arguments: MacroArguments::new(),
                                macro_invocation: OriginId::UNKNOWN,
                                replay_marker: None,
                            }));
                            continue;
                        }
                        Some(
                            TracedTokenReplay::Deliver(token)
                            | TracedTokenReplay::DeliverNoExpand(token),
                        ) => return Ok(Some(token)),
                        None => {
                            self.frames.remove(frame_index);
                        }
                    };
                }
                InputFrame::Source(source) => {
                    if let Some(token) = source.frame.pending.pop_front() {
                        return Ok(Some(token));
                    }

                    if source.frame.offset >= source.frame.line.len() {
                        if source.frame.end_after_current_line {
                            let popped = self.frames.remove(frame_index);
                            if let InputFrame::Source(source) = popped {
                                self.last_source_frame = Some(LastSourceFrame {
                                    source_id: source.source_id,
                                    input_record: source.input_record,
                                    frame: source.frame,
                                    next_source_offset: source.next_source_offset,
                                });
                            }
                            continue;
                        }
                        if !load_next_line(source, stores)? {
                            let popped = self.frames.remove(frame_index);
                            if let InputFrame::Source(source) = popped {
                                self.last_source_frame = Some(LastSourceFrame {
                                    source_id: source.source_id,
                                    input_record: source.input_record,
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

    pub fn next_traced_expansion_token(
        &mut self,
        stores: &mut impl ExpansionState,
    ) -> Result<Option<TracedExpansionToken>, LexError> {
        loop {
            let Some(frame_index) = self.current_token_frame_index() else {
                return Ok(None);
            };
            match &mut self.frames[frame_index] {
                InputFrame::TokenList(token_list) => {
                    match next_traced_token_from_token_list_frame(token_list, stores) {
                        Some(TracedTokenReplay::PushArgument(argument)) => {
                            self.frames.push(InputFrame::TokenList(TokenListInputFrame {
                                token_list: argument.token_list(),
                                origin_list: argument.origin_list(),
                                replay_kind: TokenListReplayKind::MacroArgument,
                                index: 0,
                                macro_arguments: MacroArguments::new(),
                                macro_invocation: OriginId::UNKNOWN,
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
                            self.frames.remove(frame_index);
                        }
                    };
                }
                InputFrame::Source(source) => {
                    if let Some(token) = source.frame.pending.pop_front() {
                        return Ok(Some(TracedExpansionToken::new(token, false)));
                    }

                    if source.frame.offset >= source.frame.line.len() {
                        if source.frame.end_after_current_line {
                            let popped = self.frames.remove(frame_index);
                            if let InputFrame::Source(source) = popped {
                                self.last_source_frame = Some(LastSourceFrame {
                                    source_id: source.source_id,
                                    input_record: source.input_record,
                                    frame: source.frame,
                                    next_source_offset: source.next_source_offset,
                                });
                            }
                            continue;
                        }
                        if !load_next_line(source, stores)? {
                            let popped = self.frames.remove(frame_index);
                            if let InputFrame::Source(source) = popped {
                                self.last_source_frame = Some(LastSourceFrame {
                                    source_id: source.source_id,
                                    input_record: source.input_record,
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
                            self.frames.push(InputFrame::TokenList(TokenListInputFrame {
                                token_list: argument.token_list(),
                                origin_list: argument.origin_list(),
                                replay_kind: TokenListReplayKind::MacroArgument,
                                index: 0,
                                macro_arguments: MacroArguments::new(),
                                macro_invocation: OriginId::UNKNOWN,
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
                            self.frames.remove(frame_index);
                        }
                    };
                }
                InputFrame::Source(source) => {
                    if let Some(token) = source.frame.pending.pop_front() {
                        return Ok(Some(ExpansionToken::new(decode_traced_token(token), false)));
                    }

                    if source.frame.offset >= source.frame.line.len() {
                        if source.frame.end_after_current_line {
                            let popped = self.frames.remove(frame_index);
                            if let InputFrame::Source(source) = popped {
                                self.last_source_frame = Some(LastSourceFrame {
                                    source_id: source.source_id,
                                    input_record: source.input_record,
                                    frame: source.frame,
                                    next_source_offset: source.next_source_offset,
                                });
                            }
                            continue;
                        }
                        if !load_next_line_readonly(source, stores)? {
                            let popped = self.frames.remove(frame_index);
                            if let InputFrame::Source(source) = popped {
                                self.last_source_frame = Some(LastSourceFrame {
                                    source_id: source.source_id,
                                    input_record: source.input_record,
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
            self.frames.remove(frame_index);
        }
        matches
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
        let Some(marked_index) = self.frames.iter().position(|frame| {
            matches!(
                frame,
                InputFrame::TokenList(frame) if frame.replay_marker == Some(marker)
            )
        }) else {
            return true;
        };

        let can_finish = self.frames[marked_index..].iter().all(|frame| match frame {
            InputFrame::TokenList(frame) => frame.index >= stores.tokens(frame.token_list).len(),
            InputFrame::Condition { .. } => true,
            InputFrame::Source(_) => false,
        });
        if !can_finish {
            return false;
        }

        let mut index = 0usize;
        self.frames.retain(|frame| {
            let keep = index < marked_index || !matches!(frame, InputFrame::TokenList(_));
            index += 1;
            keep
        });
        true
    }

    fn current_token_frame_index(&self) -> Option<usize> {
        self.frames
            .iter()
            .rposition(|frame| matches!(frame, InputFrame::Source(_) | InputFrame::TokenList(_)))
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

    let origins = stores.origin_list(frame.origin_list);
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
    match source.lines.next_event(stores)? {
        Some(LineEvent::Text(line)) => {
            source.frame.state = LexerState::NewLine;
            source.frame.line = line.chars().collect();
            source.frame.offset = 0;
            source.frame.buffer_offset = source.next_source_offset;
            source.frame.line_number += 1;
            source.frame.column = 0;
            source.next_source_offset += line.len();
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
    match source.lines.next_event(stores)? {
        Some(LineEvent::Text(line)) => {
            source.frame.state = LexerState::NewLine;
            source.frame.line = line.chars().collect();
            source.frame.offset = 0;
            source.frame.buffer_offset = source.next_source_offset;
            source.frame.line_number += 1;
            source.frame.column = 0;
            source.next_source_offset += line.len();
            Ok(true)
        }
        None => Ok(false),
    }
}

#[derive(Clone, Copy, Debug)]
struct SourceCoordinate {
    source_id: SourceId,
    input_record: Option<InputRecordId>,
    byte_offset: u64,
    line: u32,
    column: u32,
}

fn source_coordinate<S>(source: &SourceInputFrame<S>) -> SourceCoordinate {
    source_coordinate_from_frame(source.source_id, source.input_record, &source.frame)
}

fn source_coordinate_from_frame(
    source_id: SourceId,
    input_record: Option<InputRecordId>,
    frame: &SourceFrame,
) -> SourceCoordinate {
    let byte_offset = frame.buffer_offset + byte_offset_for_char_offset(frame);
    SourceCoordinate {
        source_id,
        input_record,
        byte_offset: u64::try_from(byte_offset).unwrap_or(u64::MAX),
        line: u32::try_from(frame.line_number).unwrap_or(u32::MAX),
        column: u32::try_from(frame.column).unwrap_or(u32::MAX),
    }
}

fn byte_offset_for_char_offset(frame: &SourceFrame) -> usize {
    frame
        .line
        .iter()
        .take(frame.offset)
        .map(|ch| ch.len_utf8())
        .sum()
}

fn traced_source_token(
    stores: &mut impl ExpansionState,
    token: Token,
    coordinate: SourceCoordinate,
) -> TracedTokenWord {
    let origin = allocate_source_origin(stores, coordinate);
    TracedTokenWord::pack(token, origin)
}

fn allocate_source_origin(
    stores: &mut impl ExpansionState,
    coordinate: SourceCoordinate,
) -> OriginId {
    stores.source_origin_with_input_record(
        coordinate.source_id,
        coordinate.input_record,
        coordinate.byte_offset,
        coordinate.line,
        coordinate.column,
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
        Catcode::Invalid => Err(LexError::InvalidCharacter(ch)),
        Catcode::Comment => {
            source.frame.offset = source.frame.line.len();
            Ok(None)
        }
        Catcode::EndLine => {
            let parent = allocate_source_origin(stores, start);
            let (token, kind) = match source.frame.state {
                LexerState::NewLine => {
                    let par = stores.intern("par");
                    (Token::Cs(par), InsertedOriginKind::Paragraph)
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
                Ok(Some(traced_source_token(
                    stores,
                    Token::Char {
                        ch: ' ',
                        cat: Catcode::Space,
                    },
                    start,
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
            Ok(Some(traced_source_token(
                stores,
                Token::Char { ch, cat },
                start,
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
            Ok(Some(traced_source_token(
                stores,
                Token::Char { ch, cat },
                start,
            )))
        }
    }
}

fn next_token_from_line_readonly<S>(
    source: &mut SourceInputFrame<S>,
    stores: &impl ExpansionState,
    unicode_superscript_notation: bool,
) -> Result<Option<Token>, LexError> {
    let ch = read_expanded_char(source, stores, unicode_superscript_notation);
    let cat = stores.catcode(ch);
    match cat {
        Catcode::Ignored => Ok(None),
        Catcode::Invalid => Err(LexError::InvalidCharacter(ch)),
        Catcode::Comment => {
            source.frame.offset = source.frame.line.len();
            Ok(None)
        }
        Catcode::EndLine => {
            let token = match source.frame.state {
                LexerState::NewLine => {
                    let Some(par) = stores.symbol("par") else {
                        return Err(LexError::MissingControlSequence("par".to_owned()));
                    };
                    Token::Cs(par)
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
    start: SourceCoordinate,
) -> TracedTokenWord {
    if source.frame.offset >= source.frame.line.len() {
        source.frame.state = LexerState::SkippingBlanks;
        let token = Token::Cs(stores.intern(""));
        return traced_source_token(stores, token, start);
    }

    let ch = read_expanded_char(source, stores, unicode_superscript_notation);
    if stores.catcode(ch) != Catcode::Letter {
        source.frame.state = LexerState::MidLine;
        let token = Token::Cs(stores.intern(&ch.to_string()));
        return traced_source_token(stores, token, start);
    }

    let mut name = String::from(ch);
    while source.frame.offset < source.frame.line.len() {
        let mark = source.frame.offset;
        let mark_col = source.frame.column;
        let next = read_expanded_char(source, stores, unicode_superscript_notation);
        if stores.catcode(next) == Catcode::Letter {
            name.push(next);
        } else {
            source.frame.offset = mark;
            source.frame.column = mark_col;
            break;
        }
    }
    source.frame.state = LexerState::SkippingBlanks;
    let token = Token::Cs(stores.intern(&name));
    traced_source_token(stores, token, start)
}

fn scan_control_sequence_readonly<S>(
    source: &mut SourceInputFrame<S>,
    stores: &impl ExpansionState,
    unicode_superscript_notation: bool,
) -> Result<Token, LexError> {
    if source.frame.offset >= source.frame.line.len() {
        source.frame.state = LexerState::SkippingBlanks;
        return readonly_cs_token(stores, "");
    }

    let ch = read_expanded_char(source, stores, unicode_superscript_notation);
    if stores.catcode(ch) != Catcode::Letter {
        source.frame.state = LexerState::MidLine;
        return readonly_cs_token(stores, &ch.to_string());
    }

    let mut name = String::from(ch);
    while source.frame.offset < source.frame.line.len() {
        let mark = source.frame.offset;
        let mark_col = source.frame.column;
        let next = read_expanded_char(source, stores, unicode_superscript_notation);
        if stores.catcode(next) == Catcode::Letter {
            name.push(next);
        } else {
            source.frame.offset = mark;
            source.frame.column = mark_col;
            break;
        }
    }
    source.frame.state = LexerState::SkippingBlanks;
    readonly_cs_token(stores, &name)
}

fn readonly_cs_token(stores: &impl ExpansionState, name: &str) -> Result<Token, LexError> {
    stores
        .symbol(name)
        .map(Token::Cs)
        .ok_or_else(|| LexError::MissingControlSequence(name.to_owned()))
}

fn read_expanded_char<S>(
    source: &mut SourceInputFrame<S>,
    stores: &impl ExpansionState,
    unicode_superscript_notation: bool,
) -> char {
    let ch = source.frame.line[source.frame.offset];
    source.frame.offset += 1;
    source.frame.column += 1;
    expand_superscript_notation(source, ch, stores, unicode_superscript_notation).unwrap_or(ch)
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
    let saved = source.frame.offset;
    let saved_col = source.frame.column;
    let second = *source.frame.line.get(source.frame.offset)?;
    if stores.catcode(second) != Catcode::Superscript {
        return None;
    }
    source.frame.offset += 1;
    source.frame.column += 1;

    if unicode_superscript_notation
        && source
            .frame
            .line
            .get(source.frame.offset..source.frame.offset + 6)
            .is_some()
        && stores.catcode(source.frame.line[source.frame.offset]) == Catcode::Superscript
        && stores.catcode(source.frame.line[source.frame.offset + 1]) == Catcode::Superscript
    {
        let digits = &source.frame.line[source.frame.offset + 2..source.frame.offset + 6];
        if digits.iter().all(|ch| ch.is_ascii_hexdigit()) {
            let mut value = 0_u32;
            for digit in digits {
                value = value * 16 + digit.to_digit(16).expect("checked hex digit");
            }
            if let Some(decoded) = char::from_u32(value) {
                source.frame.offset += 6;
                source.frame.column += 6;
                return Some(decoded);
            }
        }
    }

    if source
        .frame
        .line
        .get(source.frame.offset..source.frame.offset + 2)
        .is_some()
    {
        let digits = &source.frame.line[source.frame.offset..source.frame.offset + 2];
        if digits.iter().all(|ch| ch.is_ascii_hexdigit()) {
            let value = digits[0].to_digit(16).expect("checked hex digit") * 16
                + digits[1].to_digit(16).expect("checked hex digit");
            if let Some(decoded) = char::from_u32(value) {
                source.frame.offset += 2;
                source.frame.column += 2;
                return Some(decoded);
            }
        }
    }

    let target = *source.frame.line.get(source.frame.offset)?;
    source.frame.offset += 1;
    source.frame.column += 1;
    let code = target as u32;
    let decoded = if code < 64 { code + 64 } else { code - 64 };
    char::from_u32(decoded).or_else(|| {
        source.frame.offset = saved;
        source.frame.column = saved_col;
        None
    })
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
    ) -> Result<Option<LineEvent>, WorldError> {
        let Some(line) = self.source.read_line()? else {
            return Ok(None);
        };
        Ok(Some(normalize_line(&line, stores.endlinechar())))
    }
}

fn normalize_line(line: &str, endlinechar: i32) -> LineEvent {
    let stripped = line.trim_end_matches(' ');
    if let Ok(value) = u32::try_from(endlinechar)
        && let Some(ch) = char::from_u32(value)
    {
        let mut normalized = stripped.to_owned();
        normalized.push(ch);
        return LineEvent::Text(normalized);
    }
    LineEvent::Text(stripped.to_owned())
}

fn split_physical_lines(input: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut start = 0;
    for (index, ch) in input.char_indices() {
        if ch == '\n' {
            let end = if index > start && input[..index].ends_with('\r') {
                index - 1
            } else {
                index
            };
            lines.push(input[start..end].to_owned());
            start = index + 1;
        }
    }
    if start < input.len() {
        lines.push(input[start..].to_owned());
    }
    lines
}

#[cfg(test)]
mod tests;
