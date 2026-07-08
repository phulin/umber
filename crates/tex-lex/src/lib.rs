//! TeX input sources and line handling.
//!
//! This crate owns the line-oriented part of TeX's eyes. It normalizes
//! physical input lines before the semantic lexer state machine assigns
//! catcodes and produces tokens.

use std::collections::VecDeque;
use std::fmt;

use tex_state::ids::TokenListId;
use tex_state::token::{Catcode, Token};
use tex_state::{FileContent, Universe, WorldError};

/// Maximum number of macro arguments TeX permits in one macro body.
pub const MACRO_ARGUMENT_SLOTS: usize = 9;

/// Frozen macro arguments carried by a macro-body replay frame.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct MacroArguments {
    slots: [Option<TokenListId>; MACRO_ARGUMENT_SLOTS],
}

impl MacroArguments {
    /// Creates an empty argument-slot frame.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            slots: [None; MACRO_ARGUMENT_SLOTS],
        }
    }

    /// Records one frozen argument token list in a one-based TeX slot.
    pub fn set(&mut self, slot: u8, token_list: TokenListId) {
        let index = argument_index(slot);
        self.slots[index] = Some(token_list);
    }

    /// Reads the frozen argument token list for a one-based TeX slot.
    #[must_use]
    pub fn get(self, slot: u8) -> Option<TokenListId> {
        let index = argument_index(slot);
        self.slots[index]
    }

    /// Returns whether no argument slots are populated.
    #[must_use]
    pub fn is_empty(self) -> bool {
        self.slots.iter().all(Option::is_none)
    }
}

fn argument_index(slot: u8) -> usize {
    assert!(
        (1..=MACRO_ARGUMENT_SLOTS as u8).contains(&slot),
        "macro argument slot must be in 1..=9"
    );
    usize::from(slot - 1)
}

/// Source of physical input lines.
///
/// The trait is local so M3's `World` can implement it without forcing the
/// lexer to know where bytes came from.
pub trait InputSource {
    /// Reads the next physical line without its line terminator.
    fn read_line(&mut self) -> Result<Option<String>, WorldError>;
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
    lines: std::vec::IntoIter<String>,
}

impl WorldInput {
    #[must_use]
    pub fn from_content(content: FileContent) -> Self {
        let input = String::from_utf8_lossy(content.bytes()).into_owned();
        Self {
            lines: split_physical_lines(&input).into_iter(),
        }
    }
}

impl InputSource for WorldInput {
    fn read_line(&mut self) -> Result<Option<String>, WorldError> {
        Ok(self.lines.next())
    }
}

/// A TeX-normalized logical input line.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LineEvent {
    /// A line after trailing spaces were removed and `\endlinechar` was
    /// appended when it names a valid Unicode scalar value.
    Text(String),
    /// A blank/all-space line whose valid appended `\endlinechar` should
    /// produce TeX's `\par` behavior.
    Par,
}

/// Drives TeX line normalization for an input source.
#[derive(Debug)]
pub struct LineReader<S> {
    source: S,
}

/// The semantic lexer state from TeX's `state` variable.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum LexerState {
    /// State N: at the beginning of a line.
    #[default]
    NewLine,
    /// State M: in the middle of a line.
    MidLine,
    /// State S: skipping blanks after a space/control word.
    SkippingBlanks,
}

/// Stable identifier for a source frame.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SourceId(u32);

impl SourceId {
    #[must_use]
    pub const fn new(raw: u32) -> Self {
        Self(raw)
    }

    #[must_use]
    pub const fn raw(self) -> u32 {
        self.0
    }
}

/// Why a frozen token list is being replayed.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum TokenListReplayKind {
    MacroBody,
    MacroArgument,
    NoExpand,
    EveryPar,
    Mark,
    OutputRoutine,
    Inserted,
}

/// The family of TeX conditional represented by an open condition frame.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ConditionKind {
    /// A regular two-limb `\if...` conditional.
    If,
    /// An `\ifcase` conditional whose active limb is selected by `\or` count.
    IfCase,
}

/// The conditional limb currently being evaluated or skipped.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ConditionLimb {
    If,
    Or,
    Else,
}

/// Snapshot-summary state for one open conditional.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ConditionFrameSummary {
    kind: ConditionKind,
    limb: ConditionLimb,
    current_limb_taken: bool,
    any_limb_taken: bool,
    ifcase_or_count: u32,
    skip_nesting: u32,
}

impl ConditionFrameSummary {
    /// Creates a regular `\if...` frame.
    #[must_use]
    pub const fn new_if(current_limb_taken: bool) -> Self {
        Self {
            kind: ConditionKind::If,
            limb: ConditionLimb::If,
            current_limb_taken,
            any_limb_taken: current_limb_taken,
            ifcase_or_count: 0,
            skip_nesting: 0,
        }
    }

    /// Creates an `\ifcase` frame at its initial limb.
    #[must_use]
    pub const fn new_ifcase(current_limb_taken: bool) -> Self {
        Self {
            kind: ConditionKind::IfCase,
            limb: ConditionLimb::If,
            current_limb_taken,
            any_limb_taken: current_limb_taken,
            ifcase_or_count: 0,
            skip_nesting: 0,
        }
    }

    #[must_use]
    pub const fn kind(self) -> ConditionKind {
        self.kind
    }

    #[must_use]
    pub const fn limb(self) -> ConditionLimb {
        self.limb
    }

    #[must_use]
    pub const fn current_limb_taken(self) -> bool {
        self.current_limb_taken
    }

    #[must_use]
    pub const fn any_limb_taken(self) -> bool {
        self.any_limb_taken
    }

    #[must_use]
    pub const fn ifcase_or_count(self) -> u32 {
        self.ifcase_or_count
    }

    #[must_use]
    pub const fn skip_nesting(self) -> u32 {
        self.skip_nesting
    }

    /// Moves the frame to an `\or` limb and records how many `\or` tokens
    /// have been crossed in the current `\ifcase`.
    #[must_use]
    pub const fn with_or_limb(mut self, ifcase_or_count: u32, current_limb_taken: bool) -> Self {
        self.limb = ConditionLimb::Or;
        self.ifcase_or_count = ifcase_or_count;
        self.current_limb_taken = current_limb_taken;
        self.any_limb_taken = self.any_limb_taken || current_limb_taken;
        self
    }

    /// Moves the frame to its `\else` limb.
    #[must_use]
    pub const fn with_else_limb(mut self, current_limb_taken: bool) -> Self {
        self.limb = ConditionLimb::Else;
        self.current_limb_taken = current_limb_taken;
        self.any_limb_taken = self.any_limb_taken || current_limb_taken;
        self
    }

    /// Records nested conditional depth observed while scanning/skipping.
    #[must_use]
    pub const fn with_skip_nesting(mut self, skip_nesting: u32) -> Self {
        self.skip_nesting = skip_nesting;
        self
    }
}

/// Source-frame-local lexer state.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SourceFrame {
    state: LexerState,
    line: Vec<char>,
    offset: usize,
    pending: VecDeque<Token>,
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
        SourceFrameSummary {
            buffer_offset: self.buffer_offset,
            next_source_offset,
            line_number: self.line_number,
            column: self.column,
            lexer_state: self.state,
            normalized_line: self.line.iter().collect(),
            line_char_offset: self.offset,
            line_byte_offset: byte_offset_for_char_offset(&self.line, self.offset),
            pending: self.pending.iter().copied().collect(),
            end_after_current_line: self.end_after_current_line,
        }
    }
}

/// Snapshot summary for the input stack.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct InputSummary {
    frames: Vec<InputFrameSummary>,
    last_source_frame: Option<SourceFrameSummary>,
}

impl InputSummary {
    #[must_use]
    pub fn frames(&self) -> &[InputFrameSummary] {
        &self.frames
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.frames.len()
    }

    /// The most recently popped source frame, retained so a snapshot taken
    /// after source exhaustion can still report the final source coordinates.
    #[must_use]
    pub fn last_source_frame(&self) -> Option<&SourceFrameSummary> {
        self.last_source_frame.as_ref()
    }
}

/// Snapshot summary for one input frame.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InputFrameSummary {
    Source {
        source_id: SourceId,
        source: SourceFrameSummary,
    },
    TokenList {
        token_list: TokenListId,
        replay_kind: TokenListReplayKind,
        index: usize,
        macro_arguments: MacroArguments,
    },
    Condition(ConditionFrameSummary),
}

/// Snapshot summary for one source frame.
///
/// `source_id` belongs to the surrounding `InputFrameSummary`; the durable
/// reopen key is intentionally not stored here because this crate only sees
/// the local `InputSource` trait. M3 `World` snapshots own file/content
/// identity and reopen sources by content hash before applying this frame
/// summary's offsets and normalized-line state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceFrameSummary {
    buffer_offset: usize,
    next_source_offset: usize,
    line_number: usize,
    column: usize,
    lexer_state: LexerState,
    normalized_line: String,
    line_char_offset: usize,
    line_byte_offset: usize,
    pending: Vec<Token>,
    end_after_current_line: bool,
}

impl SourceFrameSummary {
    #[must_use]
    pub fn buffer_offset(&self) -> usize {
        self.buffer_offset
    }

    #[must_use]
    pub fn next_source_offset(&self) -> usize {
        self.next_source_offset
    }

    #[must_use]
    pub fn line_number(&self) -> usize {
        self.line_number
    }

    #[must_use]
    pub fn column(&self) -> usize {
        self.column
    }

    #[must_use]
    pub fn lexer_state(&self) -> LexerState {
        self.lexer_state
    }

    #[must_use]
    pub fn normalized_line(&self) -> &str {
        &self.normalized_line
    }

    #[must_use]
    pub fn line_char_offset(&self) -> usize {
        self.line_char_offset
    }

    #[must_use]
    pub fn line_byte_offset(&self) -> usize {
        self.line_byte_offset
    }

    #[must_use]
    pub fn pending(&self) -> &[Token] {
        &self.pending
    }

    #[must_use]
    pub fn end_after_current_line(&self) -> bool {
        self.end_after_current_line
    }

    /// Returns whether this frame summary contains all lexer-owned state
    /// needed after a source has been reopened by the snapshot owner.
    #[must_use]
    pub fn is_resume_complete(&self) -> bool {
        self.line_char_offset <= self.normalized_line.chars().count()
            && self.line_byte_offset <= self.normalized_line.len()
            && self.normalized_line.is_char_boundary(self.line_byte_offset)
    }
}

#[derive(Debug)]
struct SourceInputFrame<S> {
    source_id: SourceId,
    lines: LineReader<S>,
    frame: SourceFrame,
    next_source_offset: usize,
}

impl<S> SourceInputFrame<S> {
    fn new(source_id: SourceId, source: S) -> Self {
        Self {
            source_id,
            lines: LineReader::new(source),
            frame: SourceFrame::new(),
            next_source_offset: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TokenListInputFrame {
    token_list: TokenListId,
    replay_kind: TokenListReplayKind,
    index: usize,
    macro_arguments: MacroArguments,
}

#[derive(Debug)]
enum InputFrame<S> {
    Source(SourceInputFrame<S>),
    TokenList(TokenListInputFrame),
    Condition(ConditionFrameSummary),
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct LastSourceFrame {
    frame: SourceFrame,
    next_source_offset: usize,
}

enum TokenReplay {
    Deliver(Token),
    DeliverNoExpand(Token),
    PushArgument(TokenListId),
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

/// TeX input stack for source frames and frozen token-list replay.
#[derive(Debug)]
pub struct InputStack<S> {
    frames: Vec<InputFrame<S>>,
    next_source_id: u32,
    unicode_superscript_notation: bool,
    last_source_frame: Option<LastSourceFrame>,
}

impl<S> InputStack<S> {
    #[must_use]
    pub fn new(source: S) -> Self {
        let mut stack = Self {
            frames: Vec::new(),
            next_source_id: 0,
            unicode_superscript_notation: true,
            last_source_frame: None,
        };
        stack.push_source(source);
        stack
    }

    pub fn push_source(&mut self, source: S) -> SourceId {
        let source_id = SourceId::new(self.next_source_id);
        self.next_source_id = self
            .next_source_id
            .checked_add(1)
            .expect("source id counter overflowed");
        self.frames
            .push(InputFrame::Source(SourceInputFrame::new(source_id, source)));
        source_id
    }

    pub fn push_token_list(&mut self, token_list: TokenListId, replay_kind: TokenListReplayKind) {
        self.frames.push(InputFrame::TokenList(TokenListInputFrame {
            token_list,
            replay_kind,
            index: 0,
            macro_arguments: MacroArguments::new(),
        }));
    }

    pub fn push_macro_body(&mut self, token_list: TokenListId, macro_arguments: MacroArguments) {
        self.frames.push(InputFrame::TokenList(TokenListInputFrame {
            token_list,
            replay_kind: TokenListReplayKind::MacroBody,
            index: 0,
            macro_arguments,
        }));
    }

    pub fn push_condition(&mut self, condition: ConditionFrameSummary) {
        self.frames.push(InputFrame::Condition(condition));
    }

    pub fn update_current_condition(
        &mut self,
        condition: ConditionFrameSummary,
    ) -> Option<ConditionFrameSummary> {
        let frame = self.frames.iter_mut().rev().find_map(|frame| match frame {
            InputFrame::Condition(condition) => Some(condition),
            InputFrame::Source(_) | InputFrame::TokenList(_) => None,
        })?;
        Some(std::mem::replace(frame, condition))
    }

    pub fn pop_condition(&mut self) -> Option<ConditionFrameSummary> {
        let index = self
            .frames
            .iter()
            .rposition(|frame| matches!(frame, InputFrame::Condition(_)))?;
        match self.frames.remove(index) {
            InputFrame::Condition(condition) => Some(condition),
            InputFrame::Source(_) | InputFrame::TokenList(_) => unreachable!("rposition matched"),
        }
    }

    #[must_use]
    pub fn summary(&self) -> InputSummary {
        InputSummary {
            frames: self
                .frames
                .iter()
                .map(|frame| match frame {
                    InputFrame::Source(source) => InputFrameSummary::Source {
                        source_id: source.source_id,
                        source: source.frame.summary(source.next_source_offset),
                    },
                    InputFrame::TokenList(token_list) => InputFrameSummary::TokenList {
                        token_list: token_list.token_list,
                        replay_kind: token_list.replay_kind,
                        index: token_list.index,
                        macro_arguments: token_list.macro_arguments,
                    },
                    InputFrame::Condition(condition) => InputFrameSummary::Condition(*condition),
                })
                .collect(),
            last_source_frame: self
                .last_source_frame
                .as_ref()
                .map(|last| last.frame.summary(last.next_source_offset)),
        }
    }

    #[must_use]
    pub fn current_source_frame(&self) -> Option<&SourceFrame> {
        let current = self.frames.iter().rev().find_map(|frame| match frame {
            InputFrame::Source(source) => Some(&source.frame),
            InputFrame::TokenList(_) | InputFrame::Condition(_) => None,
        });
        current.or_else(|| self.last_source_frame.as_ref().map(|last| &last.frame))
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
            InputFrame::TokenList(_) | InputFrame::Condition(_) => None,
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
    pub fn new(source: S) -> Self {
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
            Some(InputFrame::TokenList(_) | InputFrame::Condition(_)) | None => {
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
    pub fn next_token(&mut self, stores: &mut Universe) -> Result<Option<Token>, LexError> {
        self.input.next_token(stores)
    }
}

impl<S> InputStack<S>
where
    S: InputSource,
{
    pub fn next_token(&mut self, stores: &mut Universe) -> Result<Option<Token>, LexError> {
        loop {
            let Some(frame_index) = self.current_token_frame_index() else {
                return Ok(None);
            };
            match &mut self.frames[frame_index] {
                InputFrame::TokenList(token_list) => {
                    match next_token_from_token_list_frame(token_list, stores) {
                        Some(TokenReplay::PushArgument(argument)) => {
                            self.frames.push(InputFrame::TokenList(TokenListInputFrame {
                                token_list: argument,
                                replay_kind: TokenListReplayKind::MacroArgument,
                                index: 0,
                                macro_arguments: MacroArguments::new(),
                            }));
                            continue;
                        }
                        Some(TokenReplay::Deliver(token) | TokenReplay::DeliverNoExpand(token)) => {
                            return Ok(Some(token));
                        }
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
                InputFrame::Condition(_) => {
                    unreachable!("current_token_frame_index skips conditions")
                }
            }
        }
    }

    pub fn next_token_readonly(&mut self, stores: &Universe) -> Result<Option<Token>, LexError> {
        Ok(self
            .next_expansion_token_readonly(stores)?
            .map(ExpansionToken::token))
    }

    pub fn next_expansion_token(
        &mut self,
        stores: &mut Universe,
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
                                token_list: argument,
                                replay_kind: TokenListReplayKind::MacroArgument,
                                index: 0,
                                macro_arguments: MacroArguments::new(),
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
                        return Ok(Some(ExpansionToken::new(token, false)));
                    }

                    if source.frame.offset >= source.frame.line.len() {
                        if source.frame.end_after_current_line {
                            let popped = self.frames.remove(frame_index);
                            if let InputFrame::Source(source) = popped {
                                self.last_source_frame = Some(LastSourceFrame {
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
                    return Ok(Some(ExpansionToken::new(token, false)));
                }
                InputFrame::Condition(_) => {
                    unreachable!("current_token_frame_index skips conditions")
                }
            }
        }
    }

    pub fn next_expansion_token_readonly(
        &mut self,
        stores: &Universe,
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
                                token_list: argument,
                                replay_kind: TokenListReplayKind::MacroArgument,
                                index: 0,
                                macro_arguments: MacroArguments::new(),
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
                        return Ok(Some(ExpansionToken::new(token, false)));
                    }

                    if source.frame.offset >= source.frame.line.len() {
                        if source.frame.end_after_current_line {
                            let popped = self.frames.remove(frame_index);
                            if let InputFrame::Source(source) = popped {
                                self.last_source_frame = Some(LastSourceFrame {
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
                InputFrame::Condition(_) => {
                    unreachable!("current_token_frame_index skips conditions")
                }
            }
        }
    }
}

impl<S> InputStack<S> {
    fn current_token_frame_index(&self) -> Option<usize> {
        self.frames
            .iter()
            .rposition(|frame| matches!(frame, InputFrame::Source(_) | InputFrame::TokenList(_)))
    }
}

fn next_token_from_token_list_frame(
    frame: &mut TokenListInputFrame,
    stores: &Universe,
) -> Option<TokenReplay> {
    let tokens = stores.tokens(frame.token_list);
    let token = tokens.get(frame.index).copied()?;
    frame.index += 1;

    if frame.replay_kind == TokenListReplayKind::MacroBody
        && let Token::Param(slot) = token
        && let Some(argument) = frame.macro_arguments.get(slot)
    {
        return Some(TokenReplay::PushArgument(argument));
    }

    if frame.replay_kind == TokenListReplayKind::NoExpand {
        return Some(TokenReplay::DeliverNoExpand(token));
    }

    Some(TokenReplay::Deliver(token))
}

fn load_next_line_readonly<S>(
    source: &mut SourceInputFrame<S>,
    stores: &Universe,
) -> Result<bool, LexError>
where
    S: InputSource,
{
    match source.lines.next_event(stores)? {
        Some(LineEvent::Text(line)) => {
            source.frame.line = line.chars().collect();
            source.frame.offset = 0;
            source.frame.buffer_offset = source.next_source_offset;
            source.frame.line_number += 1;
            source.frame.column = 0;
            source.next_source_offset += line.len();
            Ok(true)
        }
        Some(LineEvent::Par) => {
            source.frame.state = LexerState::NewLine;
            source.frame.line_number += 1;
            source.frame.column = 0;
            source.frame.buffer_offset = source.next_source_offset;
            source.next_source_offset += 1;
            let Some(par) = stores.symbol("par") else {
                return Err(LexError::MissingControlSequence("par".to_owned()));
            };
            source.frame.pending.push_back(Token::Cs(par));
            Ok(true)
        }
        None => Ok(false),
    }
}

fn load_next_line<S>(
    source: &mut SourceInputFrame<S>,
    stores: &mut Universe,
) -> Result<bool, LexError>
where
    S: InputSource,
{
    match source.lines.next_event(stores)? {
        Some(LineEvent::Text(line)) => {
            source.frame.line = line.chars().collect();
            source.frame.offset = 0;
            source.frame.buffer_offset = source.next_source_offset;
            source.frame.line_number += 1;
            source.frame.column = 0;
            source.next_source_offset += line.len();
            Ok(true)
        }
        Some(LineEvent::Par) => {
            source.frame.state = LexerState::NewLine;
            source.frame.line_number += 1;
            source.frame.column = 0;
            source.frame.buffer_offset = source.next_source_offset;
            source.next_source_offset += 1;
            let par = stores.intern("par");
            source.frame.pending.push_back(Token::Cs(par));
            Ok(true)
        }
        None => Ok(false),
    }
}

fn next_token_from_line<S>(
    source: &mut SourceInputFrame<S>,
    stores: &mut Universe,
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
                    let par = stores.intern("par");
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
        Catcode::Escape => Ok(Some(scan_control_sequence(
            source,
            stores,
            unicode_superscript_notation,
        ))),
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

fn next_token_from_line_readonly<S>(
    source: &mut SourceInputFrame<S>,
    stores: &Universe,
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
    stores: &mut Universe,
    unicode_superscript_notation: bool,
) -> Token {
    if source.frame.offset >= source.frame.line.len() {
        source.frame.state = LexerState::SkippingBlanks;
        return Token::Cs(stores.intern(""));
    }

    let ch = read_expanded_char(source, stores, unicode_superscript_notation);
    if stores.catcode(ch) != Catcode::Letter {
        source.frame.state = LexerState::MidLine;
        return Token::Cs(stores.intern(&ch.to_string()));
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
    Token::Cs(stores.intern(&name))
}

fn scan_control_sequence_readonly<S>(
    source: &mut SourceInputFrame<S>,
    stores: &Universe,
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

fn readonly_cs_token(stores: &Universe, name: &str) -> Result<Token, LexError> {
    stores
        .symbol(name)
        .map(Token::Cs)
        .ok_or_else(|| LexError::MissingControlSequence(name.to_owned()))
}

fn read_expanded_char<S>(
    source: &mut SourceInputFrame<S>,
    stores: &Universe,
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
    stores: &Universe,
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
    pub fn next_event(&mut self, stores: &Universe) -> Result<Option<LineEvent>, WorldError> {
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
        if stripped.is_empty() {
            return LineEvent::Par;
        }

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

fn byte_offset_for_char_offset(line: &[char], char_offset: usize) -> usize {
    line.iter().take(char_offset).map(|ch| ch.len_utf8()).sum()
}

#[cfg(test)]
mod tests {
    use super::{
        ConditionFrameSummary, ConditionKind, ConditionLimb, InputFrame, InputFrameSummary,
        InputStack, LexError, Lexer, LexerState, LineEvent, LineReader, MemoryInput,
        TokenListReplayKind, load_next_line,
    };
    use tex_state::Universe;
    use tex_state::env::banks::IntParam;
    use tex_state::token::{Catcode, Token};

    #[test]
    fn strips_trailing_spaces_and_appends_endlinechar() {
        let mut stores = Universe::new();
        stores.set_int_param(IntParam::END_LINE_CHAR, 13);
        let mut reader = LineReader::new(MemoryInput::new("abc   \n"));

        assert_eq!(
            reader
                .next_event(&stores)
                .expect("memory input should read"),
            Some(LineEvent::Text("abc\r".to_owned()))
        );
        assert_eq!(
            reader
                .next_event(&stores)
                .expect("memory input should read"),
            None
        );
    }

    #[test]
    fn empty_lines_emit_par_event() {
        let mut stores = Universe::new();
        stores.set_int_param(IntParam::END_LINE_CHAR, 13);
        let mut reader = LineReader::new(MemoryInput::new("   \n\nx\n"));

        assert_eq!(
            reader
                .next_event(&stores)
                .expect("memory input should read"),
            Some(LineEvent::Par)
        );
        assert_eq!(
            reader
                .next_event(&stores)
                .expect("memory input should read"),
            Some(LineEvent::Par)
        );
        assert_eq!(
            reader
                .next_event(&stores)
                .expect("memory input should read"),
            Some(LineEvent::Text("x\r".to_owned()))
        );
        assert_eq!(
            reader
                .next_event(&stores)
                .expect("memory input should read"),
            None
        );
    }

    #[test]
    fn suppresses_invalid_endlinechar_values() {
        let mut stores = Universe::new();
        stores.set_int_param(IntParam::END_LINE_CHAR, -1);
        let mut reader = LineReader::new(MemoryInput::new("abc\n"));

        assert_eq!(
            reader
                .next_event(&stores)
                .expect("memory input should read"),
            Some(LineEvent::Text("abc".to_owned()))
        );
    }

    #[test]
    fn letters_spaces_and_endline_state_match_tex_rules() {
        let mut stores = Universe::new();
        stores.set_int_param(IntParam::END_LINE_CHAR, 13);
        let mut lexer = Lexer::new(MemoryInput::new(" a  b\n\n"));

        assert_eq!(
            collect_tokens(&mut lexer, &mut stores),
            vec![
                char_token('a', Catcode::Letter),
                char_token(' ', Catcode::Space),
                char_token('b', Catcode::Letter),
                char_token(' ', Catcode::Space),
                cs_token(&mut stores, "par"),
            ]
        );
        assert_eq!(lexer.frame().state(), LexerState::NewLine);
    }

    #[test]
    fn inactive_endlinechar_blank_line_does_not_emit_par_token() {
        let mut stores = Universe::new();
        stores.set_int_param(IntParam::END_LINE_CHAR, -1);
        let mut lexer = Lexer::new(MemoryInput::new("a\n\nb"));

        assert_eq!(
            collect_tokens(&mut lexer, &mut stores),
            vec![
                char_token('a', Catcode::Letter),
                char_token('b', Catcode::Letter),
            ]
        );
    }

    #[test]
    fn scans_control_words_and_control_symbols() {
        let mut stores = Universe::new();
        stores.set_int_param(IntParam::END_LINE_CHAR, 13);
        let mut lexer = Lexer::new(MemoryInput::new("\\foo   x\\$"));

        assert_eq!(
            collect_tokens(&mut lexer, &mut stores),
            vec![
                cs_token(&mut stores, "foo"),
                char_token('x', Catcode::Letter),
                cs_token(&mut stores, "$"),
                char_token(' ', Catcode::Space),
            ]
        );
    }

    #[test]
    fn control_word_scanning_uses_current_catcodes() {
        let mut stores = Universe::new();
        stores.set_int_param(IntParam::END_LINE_CHAR, 13);
        stores.set_catcode('@', Catcode::Letter);
        let mut lexer = Lexer::new(MemoryInput::new("\\foo@bar"));

        assert_eq!(
            collect_tokens(&mut lexer, &mut stores),
            vec![cs_token(&mut stores, "foo@bar")]
        );
    }

    #[test]
    fn unread_characters_use_catcodes_current_at_token_read() {
        let mut stores = Universe::new();
        stores.set_int_param(IntParam::END_LINE_CHAR, 13);
        let mut lexer = Lexer::new(MemoryInput::new("a@b"));

        assert_eq!(
            lexer.next_token(&mut stores).expect("first token"),
            Some(char_token('a', Catcode::Letter))
        );

        // pdfTeX check: after `a\catcode`\@=11 @b`, the unread `@` is
        // tokenized as a letter while the already-read `a` keeps its token.
        stores.set_catcode('@', Catcode::Letter);

        assert_eq!(
            collect_tokens(&mut lexer, &mut stores),
            vec![
                char_token('@', Catcode::Letter),
                char_token('b', Catcode::Letter),
                char_token(' ', Catcode::Space),
            ]
        );
    }

    #[test]
    fn control_word_scan_rechecks_catcodes_after_escape_token_read() {
        let mut stores = Universe::new();
        stores.set_int_param(IntParam::END_LINE_CHAR, 13);
        stores.set_catcode('@', Catcode::Other);
        let mut lexer = Lexer::new(MemoryInput::new("\\@a"));

        // pdfTeX check: a `\catcode`\@=11` assignment before the following
        // token makes `\@a` scan as the control word `@a`, not control symbol
        // `@` followed by letter `a`.
        stores.set_catcode('@', Catcode::Letter);

        assert_eq!(
            collect_tokens(&mut lexer, &mut stores),
            vec![cs_token(&mut stores, "@a")]
        );
    }

    #[test]
    fn next_physical_line_uses_current_endlinechar() {
        let mut stores = Universe::new();
        stores.set_int_param(IntParam::END_LINE_CHAR, b'!' as i32);
        let mut lexer = Lexer::new(MemoryInput::new("a\nb\nc"));

        assert_eq!(
            lexer.next_token(&mut stores).expect("first token"),
            Some(char_token('a', Catcode::Letter))
        );
        assert_eq!(
            lexer.next_token(&mut stores).expect("first line ending"),
            Some(char_token('!', Catcode::Other))
        );

        // pdfTeX check: `\endlinechar` is read when a physical line is
        // converted to an input line, so changing it here affects the next
        // unread line but cannot rewrite the line already in progress.
        stores.set_int_param(IntParam::END_LINE_CHAR, b'?' as i32);

        assert_eq!(
            lexer
                .next_token(&mut stores)
                .expect("second line first token"),
            Some(char_token('b', Catcode::Letter))
        );
        assert_eq!(
            lexer.next_token(&mut stores).expect("second line ending"),
            Some(char_token('?', Catcode::Other))
        );

        stores.set_int_param(IntParam::END_LINE_CHAR, -1);

        assert_eq!(
            collect_tokens(&mut lexer, &mut stores),
            vec![char_token('c', Catcode::Letter)]
        );
    }

    #[test]
    fn comments_ignore_rest_of_physical_line() {
        let mut stores = Universe::new();
        stores.set_int_param(IntParam::END_LINE_CHAR, 13);
        let mut lexer = Lexer::new(MemoryInput::new("a% ignored\nb"));

        assert_eq!(
            collect_tokens(&mut lexer, &mut stores),
            vec![
                char_token('a', Catcode::Letter),
                char_token('b', Catcode::Letter),
                char_token(' ', Catcode::Space),
            ]
        );
    }

    #[test]
    fn ignored_and_invalid_catcodes_follow_tex_rules() {
        let mut stores = Universe::new();
        stores.set_catcode('!', Catcode::Ignored);
        stores.set_catcode('?', Catcode::Invalid);
        let mut lexer = Lexer::new(MemoryInput::new("a!?"));

        assert_eq!(
            lexer.next_token(&mut stores).expect("valid token"),
            Some(char_token('a', Catcode::Letter))
        );
        match lexer.next_token(&mut stores) {
            Err(LexError::InvalidCharacter('?')) => {}
            other => panic!("expected invalid-character error, got {other:?}"),
        }
    }

    #[test]
    fn superscript_notation_is_expanded_before_catcode_lookup() {
        let mut stores = Universe::new();
        stores.set_int_param(IntParam::END_LINE_CHAR, 13);
        stores.set_catcode('@', Catcode::Letter);
        let mut lexer = Lexer::new(MemoryInput::new("^^40 ^^41 ^^^^00E9"));

        assert_eq!(
            collect_tokens(&mut lexer, &mut stores),
            vec![
                char_token('@', Catcode::Letter),
                char_token(' ', Catcode::Space),
                char_token('A', Catcode::Letter),
                char_token(' ', Catcode::Space),
                char_token('é', Catcode::Other),
                char_token(' ', Catcode::Space),
            ]
        );
    }

    #[test]
    fn every_non_ignored_non_invalid_char_catcode_emits_char_token() {
        let cases = [
            ('{', Catcode::BeginGroup),
            ('}', Catcode::EndGroup),
            ('$', Catcode::MathShift),
            ('&', Catcode::AlignmentTab),
            ('#', Catcode::Parameter),
            ('_', Catcode::Subscript),
            ('~', Catcode::Active),
            ('1', Catcode::Other),
            ('^', Catcode::Superscript),
        ];

        for (ch, cat) in cases {
            let mut stores = Universe::new();
            stores.set_catcode(ch, cat);
            let mut lexer = Lexer::new(MemoryInput::new(ch.to_string()));
            assert_eq!(
                lexer.next_token(&mut stores).expect("valid token"),
                Some(char_token(ch, cat))
            );
        }
    }

    #[test]
    fn token_list_frames_replay_before_sources_and_pop_at_end() {
        let mut stores = Universe::new();
        stores.set_int_param(IntParam::END_LINE_CHAR, 13);
        let list = stores.intern_token_list(&[
            char_token('x', Catcode::Letter),
            char_token('y', Catcode::Letter),
        ]);
        let mut input = InputStack::new(MemoryInput::new("a"));
        input.push_token_list(list, TokenListReplayKind::MacroBody);

        assert!(matches!(
            input.summary().frames(),
            [
                InputFrameSummary::Source { .. },
                InputFrameSummary::TokenList {
                    token_list,
                    replay_kind: TokenListReplayKind::MacroBody,
                    index: 0,
                    macro_arguments
                }
            ] if *token_list == list && macro_arguments.is_empty()
        ));
        assert_eq!(
            input.next_token(&mut stores).expect("token-list replay"),
            Some(char_token('x', Catcode::Letter))
        );
        assert!(matches!(
            input.summary().frames().last(),
            Some(InputFrameSummary::TokenList { index: 1, .. })
        ));
        assert_eq!(
            input.next_token(&mut stores).expect("token-list replay"),
            Some(char_token('y', Catcode::Letter))
        );
        assert_eq!(
            input.next_token(&mut stores).expect("source replay"),
            Some(char_token('a', Catcode::Letter))
        );
    }

    #[test]
    fn source_summaries_track_position_and_eof_pop() {
        let mut stores = Universe::new();
        stores.set_int_param(IntParam::END_LINE_CHAR, 13);
        let mut input = InputStack::new(MemoryInput::new("ab\nc"));

        assert_eq!(
            input.next_token(&mut stores).expect("source token"),
            Some(char_token('a', Catcode::Letter))
        );
        assert!(matches!(
            input.summary().frames(),
            [InputFrameSummary::Source {
                source_id,
                source,
            }] if source_id.raw() == 0
                && source.buffer_offset() == 0
                && source.line_number() == 1
                && source.column() == 1
                && source.lexer_state() == LexerState::MidLine
        ));

        while input
            .next_token(&mut stores)
            .expect("drain input")
            .is_some()
        {}
        assert!(input.summary().is_empty());
        assert!(input.is_empty());
    }

    #[test]
    fn source_summary_is_resume_complete_inside_current_line() {
        let mut stores = Universe::new();
        stores.set_int_param(IntParam::END_LINE_CHAR, 13);
        let mut input = InputStack::new(MemoryInput::new("éa"));

        assert_eq!(
            input.next_token(&mut stores).expect("source token"),
            Some(char_token('é', Catcode::Other))
        );
        let summary = input.summary();
        let [InputFrameSummary::Source { source_id, source }] = summary.frames() else {
            panic!("expected one source frame");
        };

        assert_eq!(source_id.raw(), 0);
        assert_eq!(source.normalized_line(), "éa\r");
        assert_eq!(source.line_char_offset(), 1);
        assert_eq!(source.line_byte_offset(), 2);
        assert_eq!(source.column(), 1);
        assert_eq!(source.lexer_state(), LexerState::MidLine);
        assert!(source.pending().is_empty());
        assert!(source.is_resume_complete());
    }

    #[test]
    fn source_summary_captures_pending_synthetic_par() {
        let mut stores = Universe::new();
        stores.set_int_param(IntParam::END_LINE_CHAR, 13);
        let mut input = InputStack::new(MemoryInput::new("\nnext"));
        let Some(InputFrame::Source(source)) = input.frames.last_mut() else {
            panic!("expected source frame");
        };

        assert!(load_next_line(source, &mut stores).expect("blank line loads"));
        let summary = input.summary();
        let [InputFrameSummary::Source { source, .. }] = summary.frames() else {
            panic!("expected one source frame");
        };

        assert_eq!(source.normalized_line(), "");
        assert_eq!(source.line_char_offset(), 0);
        assert_eq!(source.line_byte_offset(), 0);
        assert_eq!(source.line_number(), 1);
        assert_eq!(source.pending(), &[cs_token(&mut stores, "par")]);
        assert!(source.is_resume_complete());
    }

    #[test]
    fn condition_frames_round_trip_through_input_summary() {
        let mut stores = Universe::new();
        stores.set_int_param(IntParam::END_LINE_CHAR, 13);
        let mut input = InputStack::new(MemoryInput::new("ab"));
        let condition = ConditionFrameSummary::new_ifcase(false)
            .with_or_limb(2, true)
            .with_skip_nesting(1);

        input.push_condition(condition);

        let first = input.summary();
        let round_tripped = first.clone();
        assert_eq!(round_tripped, first);
        assert!(matches!(
            round_tripped.frames(),
            [
                InputFrameSummary::Source { .. },
                InputFrameSummary::Condition(frame),
            ] if frame.kind() == ConditionKind::IfCase
                && frame.limb() == ConditionLimb::Or
                && frame.current_limb_taken()
                && frame.any_limb_taken()
                && frame.ifcase_or_count() == 2
                && frame.skip_nesting() == 1
        ));

        assert_eq!(
            input
                .next_token(&mut stores)
                .expect("condition frame skips"),
            Some(char_token('a', Catcode::Letter))
        );
        assert!(matches!(
            input.summary().frames(),
            [
                InputFrameSummary::Source { source, .. },
                InputFrameSummary::Condition(frame),
            ] if source.column() == 1 && *frame == condition
        ));
    }

    #[test]
    fn open_condition_survives_checkpoint_rollback_resume_summary() {
        let mut stores = Universe::new();
        stores.set_int_param(IntParam::END_LINE_CHAR, 13);
        let mut input = InputStack::new(MemoryInput::new("xy"));
        input.push_condition(ConditionFrameSummary::new_if(true));

        assert_eq!(
            input.next_token(&mut stores).expect("source token"),
            Some(char_token('x', Catcode::Letter))
        );
        let checkpoint = stores.snapshot();
        let resume_summary = input.summary();

        let updated = ConditionFrameSummary::new_if(true).with_else_limb(false);
        assert_eq!(
            input.update_current_condition(updated),
            Some(ConditionFrameSummary::new_if(true))
        );
        stores.set_int_param(IntParam::END_LINE_CHAR, b'!' as i32);
        assert_eq!(
            input.next_token(&mut stores).expect("source token"),
            Some(char_token('y', Catcode::Letter))
        );

        stores.rollback(&checkpoint);

        assert_eq!(stores.endlinechar(), 13);
        assert!(matches!(
            resume_summary.frames(),
            [
                InputFrameSummary::Source { source, .. },
                InputFrameSummary::Condition(frame),
            ] if source.column() == 1
                && frame.kind() == ConditionKind::If
                && frame.limb() == ConditionLimb::If
                && frame.current_limb_taken()
                && frame.any_limb_taken()
                && frame.ifcase_or_count() == 0
                && frame.skip_nesting() == 0
        ));
    }

    fn collect_tokens(lexer: &mut Lexer<MemoryInput>, stores: &mut Universe) -> Vec<Token> {
        let mut tokens = Vec::new();
        while let Some(token) = lexer.next_token(stores).expect("lexing should succeed") {
            tokens.push(token);
        }
        tokens
    }

    fn char_token(ch: char, cat: Catcode) -> Token {
        Token::Char { ch, cat }
    }

    fn cs_token(stores: &mut Universe, name: &str) -> Token {
        Token::Cs(stores.intern(name))
    }
}
