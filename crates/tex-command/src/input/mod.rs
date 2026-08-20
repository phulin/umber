//! Private input state machines.

use std::collections::BTreeMap;
use std::collections::VecDeque;

#[cfg(test)]
use std::cell::Cell;

mod levels;
mod lines;
mod source;
mod stack;
mod tokenizer;

#[cfg(test)]
mod tests;

#[cfg(test)]
pub(crate) use levels::SharedTokenBuffer;
pub(crate) use levels::{
    BackedUpToken, BackupTreatment, InputLevel, InputLevelId, PackedInputFrame, ReplayTrace,
    RetirementBehavior, SourceLevel, SourceOpenDepths, SourceRetirement, StoredReplayReason,
    TokenBehavior, TokenCursor, TokenPayload, packed_token_frame,
};
pub(crate) use lines::{ReducedSourceSpelling, SourceLineState};
pub(crate) use source::{
    DetachedRegisteredSourceParts, LineBackingRegistry, RegisteredSource, SourceCursor,
    source_line_buffer_high_water,
};
#[allow(unused_imports)] // consumed by the ordered raw-delivery implementation issues
pub(crate) use stack::{
    InputRetirement, InputRetirementAction, InputRetirementError, InputRetirementReason,
    OutParameterReplay, ParameterReplayError, input_level_identity,
};

pub use lines::{
    LineTerminator, PhysicalLine, SourceCharacter, SourceLocation, SourceProvenance, SourceRange,
    SourceScalarRange,
};
pub use source::{
    FileFramingEvent, MalformedUnicodeRange, RegisteredSourceKind, SourceFramingPolicy,
    SourceNameClass, SourceRegistration, SourceRegistrationError,
};
pub use tokenizer::{
    CONTROL_SEQUENCE_NAME_INLINE_CAPACITY, CatcodeQueries, ControlSequenceName,
    InvalidSourceCharacter, LexerState, SourceControlSequenceKind, SourceStepQueries, SourceToken,
    SourceTokenizationStep,
};

/// Persistent input-stack ownership.
///
/// This state owns only future deliveries and semantic identity allocation.
/// Conditions, scanner policy, meanings, and host capabilities belong to
/// other ownership classes.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub(crate) struct InputState<G> {
    pub(crate) levels: Vec<InputLevel<G>>,
    /// TeX82 §331's bottom terminal buffer after the startup line has been
    /// consumed. Umber retires that acquisition level before opening the root
    /// file, but §310 must still reach its `<*>` context after the last file
    /// retires and while EOF recovery token lists remain live.
    pub(crate) terminal_context_line: Option<String>,
    /// Backing registered for a future one-shot open, keyed by `SourceId`.
    ///
    /// Opening removes the entry and moves its backing into the source level,
    /// so retired sources leave neither bytes nor registry state behind.
    pub(crate) pending_sources: BTreeMap<u32, RegisteredSource>,
    pub(crate) next_level_identity: u64,
    pub(crate) next_source_identity: u64,
    /// TeX82 §362's process-global `force_eof`.
    pub(crate) force_eof: bool,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct TokenContextProjectionStats {
    pub(crate) visited_tokens: usize,
    pub(crate) peak_temporary_string_capacity: usize,
}

#[cfg(test)]
thread_local! {
    static TOKEN_CONTEXT_PROJECTION_STATS: Cell<TokenContextProjectionStats> =
        const { Cell::new(TokenContextProjectionStats {
            visited_tokens: 0,
            peak_temporary_string_capacity: 0,
        }) };
}

#[cfg(test)]
fn note_token_context_projection(raw: &String, rendered: &String) {
    TOKEN_CONTEXT_PROJECTION_STATS.with(|stats| {
        let mut current = stats.get();
        current.visited_tokens = current.visited_tokens.saturating_add(1);
        current.peak_temporary_string_capacity = current
            .peak_temporary_string_capacity
            .max(raw.capacity())
            .max(rendered.capacity());
        stats.set(current);
    });
}

#[cfg(test)]
fn note_token_context_capacity(capacity: usize) {
    TOKEN_CONTEXT_PROJECTION_STATS.with(|stats| {
        let mut current = stats.get();
        current.peak_temporary_string_capacity =
            current.peak_temporary_string_capacity.max(capacity);
        stats.set(current);
    });
}

#[cfg(test)]
pub(crate) fn reset_token_context_projection_stats() {
    TOKEN_CONTEXT_PROJECTION_STATS.set(TokenContextProjectionStats::default());
}

#[cfg(test)]
#[must_use]
pub(crate) fn token_context_projection_stats() -> TokenContextProjectionStats {
    TOKEN_CONTEXT_PROJECTION_STATS.get()
}

struct ContextTail {
    chars: VecDeque<char>,
    total: usize,
    limit: usize,
}

impl ContextTail {
    fn new(limit: usize) -> Self {
        Self {
            chars: VecDeque::with_capacity(limit),
            total: 0,
            limit,
        }
    }

    fn push_str(&mut self, text: &str) {
        for ch in text.chars() {
            self.total = self.total.saturating_add(1);
            if self.limit == 0 {
                continue;
            }
            if self.chars.len() == self.limit {
                let _ = self.chars.pop_front();
            }
            self.chars.push_back(ch);
        }
    }

    fn prepend_str(&mut self, text: &str) {
        for ch in text.chars().rev() {
            self.total = self
                .total
                .saturating_add(1)
                .min(self.limit.saturating_add(1));
            if self.total > self.limit {
                break;
            }
            self.chars.push_front(ch);
        }
    }

    fn is_complete(&self) -> bool {
        self.total > self.limit
    }

    fn finish(self) -> (String, usize) {
        (self.chars.into_iter().collect(), self.total)
    }
}

struct ContextHead {
    text: String,
    chars: usize,
    limit: usize,
}

enum ContextSink<'a> {
    Tail(&'a mut ContextTail),
    Head(&'a mut ContextHead),
}

impl ContextSink<'_> {
    fn push_str(&mut self, text: &str) {
        match self {
            Self::Tail(tail) => tail.push_str(text),
            Self::Head(head) => head.push_str(text),
        }
    }

    fn is_complete(&self) -> bool {
        matches!(self, Self::Head(head) if head.is_complete())
    }
}

impl ContextHead {
    fn new(limit: usize) -> Self {
        Self {
            text: String::with_capacity(limit.min(256)),
            chars: 0,
            limit,
        }
    }

    fn push_str(&mut self, text: &str) {
        for ch in text.chars() {
            self.chars = self.chars.saturating_add(1);
            if self.chars <= self.limit {
                self.text.push(ch);
            }
            if self.is_complete() {
                break;
            }
        }
    }

    fn is_complete(&self) -> bool {
        self.chars > self.limit
    }

    fn finish(self) -> (String, usize) {
        (self.text, self.chars)
    }
}

/// Versioned, allocation-independent projection of command input state.
/// Runtime source, level, token-list, and provenance ids are deliberately
/// translated to immutable content and stack position before hashing.
pub(crate) fn tracked_input_projection<G>(
    input: &InputState<G>,
    state: &mut tex_state::CommandContext<'_, G>,
) -> Option<(u64, u64)> {
    let mut stack = ProjectionHasher::new(0x696e_7075_745f_0001);
    stack.bytes(
        input
            .terminal_context_line
            .as_deref()
            .unwrap_or_default()
            .as_bytes(),
    );
    stack.byte(input.force_eof.into());
    // Pending ids are referenced by future host/resource operations. Until
    // their stable request identity is admitted here, fail closed.
    if !input.pending_sources.is_empty() {
        return None;
    }
    stack.u64(input.levels.len() as u64);
    let mut line = ProjectionHasher::new(0x696e_6c69_6e65_0001);
    for level in &input.levels {
        match level {
            InputLevel::Source(source) => {
                stack.byte(0);
                observe_immutable_source(state, source);
                project_source(&mut stack, source);
                if let Some(current) = &source.cursor.line {
                    project_line(&mut line, &source.cursor, current);
                }
            }
            InputLevel::Tokens(cursor) => {
                stack.byte(1);
                // Macro-activation and alignment identities need stack-relative
                // translation, which is deliberately fail-closed for now.
                if matches!(
                    cursor.behavior,
                    TokenBehavior::MacroBody(_)
                        | TokenBehavior::UTemplate
                        | TokenBehavior::VTemplate
                ) {
                    return None;
                }
                project_token_cursor(&mut stack, cursor, state)?;
            }
        }
    }
    Some((line.finish(), stack.finish()))
}

pub(crate) fn observe_immutable_source<G>(
    state: &mut tex_state::CommandContext<'_, G>,
    source: &SourceLevel<G>,
) {
    let backing = source.cursor.current_backing();
    let record = tex_state::world::ContentHash::from_bytes(&backing.bytes);
    state.observe_command_projection(
        tex_state::DependencyKey::InputRecord(record),
        tex_state::DependencyValue::Content(record),
    );
    let Some(line) = &source.cursor.line else {
        return;
    };
    let range = line.physical.content_range();
    let start = usize::try_from(range.start()).expect("registered source offsets fit usize");
    let end = usize::try_from(range.end()).expect("registered source offsets fit usize");
    let content = tex_state::world::ContentHash::from_bytes(&backing.bytes[start..end]);
    let terminator = match line.physical.terminator() {
        LineTerminator::Missing => 0,
        LineTerminator::Lf => 1,
        LineTerminator::Cr => 2,
        LineTerminator::CrLf => 3,
    };
    state.observe_command_projection(
        tex_state::DependencyKey::PhysicalLine {
            content,
            terminator,
        },
        tex_state::DependencyValue::Content(content),
    );
}

fn project_source<G>(hash: &mut ProjectionHasher, source: &SourceLevel<G>) {
    hash.bytes(&source.cursor.backing.bytes);
    hash.byte(source.cursor.backing.mode as u8);
    hash.bytes(
        source
            .cursor
            .backing
            .name
            .as_deref()
            .unwrap_or_default()
            .as_bytes(),
    );
    hash.u64(source.cursor.next_physical_offset);
    hash.u64(source.cursor.next_line_number);
    hash.byte(source.cursor.pending_acquired_line.into());
    hash.byte(source.cursor.end_after_line.into());
    hash.byte(match source.cursor.lexer_state {
        LexerState::MidLine => 0,
        LexerState::SkipBlanks => 1,
        LexerState::NewLine => 2,
    });
    hash.byte(match source.retirement {
        SourceRetirement::Pop => 0,
        SourceRetirement::EndReadLine => 1,
    });
}

fn project_line(hash: &mut ProjectionHasher, cursor: &SourceCursor, line: &lines::SourceLineState) {
    let backing = cursor.current_backing();
    hash.bytes(&backing.bytes);
    hash.u64(line.physical.number());
    hash.u64(line.physical.content_range().start());
    hash.u64(line.physical.content_range().end());
    hash.u64(line.retained_end);
    hash.u64(line.byte_cursor);
    hash.u64(line.scalar_cursor);
    hash.byte(line.endline_delivered.into());
    if let Some(endline) = line.endline {
        hash.byte(1);
        hash.bytes(&endline.to_stable_bytes());
    } else {
        hash.byte(0);
    }
    hash.u64(line.reduced_spellings.len() as u64);
    for spelling in &line.reduced_spellings {
        hash.u64(spelling.range.start());
        hash.u64(spelling.range.end());
        hash.bytes(&spelling.code.to_stable_bytes());
    }
}

fn project_token_cursor<G>(
    hash: &mut ProjectionHasher,
    cursor: &TokenCursor<G>,
    state: &tex_state::CommandContext<'_, G>,
) -> Option<()> {
    hash.u64(u64::from(cursor.frame.position()));
    hash.byte(match cursor.retirement {
        RetirementBehavior::Pop => 0,
        RetirementBehavior::StopAtEnd => 1,
        RetirementBehavior::RetainExhaustedVTemplate => 2,
        RetirementBehavior::AwaitingVTemplateRetirement => 3,
    });
    hash.byte(match cursor.behavior {
        TokenBehavior::Ordinary => 0,
        TokenBehavior::Recovery => 1,
        TokenBehavior::Parameter => 2,
        TokenBehavior::BackedUp(BackupTreatment::Ordinary) => 3,
        TokenBehavior::BackedUp(BackupTreatment::SuppressExpandableControlSequence) => 4,
        TokenBehavior::MacroBody(_) | TokenBehavior::UTemplate | TokenBehavior::VTemplate => {
            return None;
        }
    });
    if matches!(
        cursor.payload,
        TokenPayload::MacroReplacement { .. } | TokenPayload::Argument { .. }
    ) {
        return None;
    }
    match &cursor.payload {
        TokenPayload::Packed(chunk) => {
            for index in 0..chunk.len() {
                project_token(hash, chunk.word(index)?.token()?, state)?;
            }
        }
        TokenPayload::MacroReplacement { .. } | TokenPayload::Argument { .. } => {
            unreachable!("packed macro payloads fail closed above")
        }
    }
    Some(())
}

fn project_token<G>(
    hash: &mut ProjectionHasher,
    token: tex_state::token::Token,
    state: &tex_state::CommandContext<'_, G>,
) -> Option<()> {
    use tex_state::token::Token;
    match token {
        Token::Char { ch, cat } => {
            hash.byte(0);
            hash.u64(ch as u64);
            hash.byte(cat as u8);
        }
        Token::Cs(symbol) => {
            hash.byte(1);
            hash.bytes(state.resolve(symbol).as_bytes());
        }
        Token::Param(slot) => {
            hash.byte(2);
            hash.byte(slot);
        }
        Token::Frozen(_) => {
            hash.byte(3);
            hash.bytes(state.frozen_primitive_name(token)?.as_bytes());
        }
    }
    Some(())
}

struct ProjectionHasher(u64);

impl ProjectionHasher {
    const fn new(domain: u64) -> Self {
        Self(0xcbf2_9ce4_8422_2325 ^ domain)
    }

    fn byte(&mut self, byte: u8) {
        self.0 ^= u64::from(byte);
        self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
    }

    fn bytes(&mut self, bytes: &[u8]) {
        self.u64(bytes.len() as u64);
        for &byte in bytes {
            self.byte(byte);
        }
    }

    fn u64(&mut self, value: u64) {
        for byte in value.to_le_bytes() {
            self.byte(byte);
        }
    }

    const fn finish(self) -> u64 {
        self.0
    }
}

impl<G> InputState<G> {
    /// tex.web §310's `show_context` display for the canonical input stack.
    ///
    /// The two-line pseudoprint arithmetic (§316--§318) and §310's own
    /// `\errorcontextlines` elision are
    /// [`tex_state::print::render_error_context`]; this is §312--§314's
    /// projection of the command core's levels onto it. Every other input
    /// stack in the engine projects onto the same renderer.
    pub(crate) fn output_open_context(
        &self,
        stores: &tex_state::CommandContext<'_, G>,
        parameters: &crate::macro_call::ParameterState<G>,
        attempt: &crate::attempt::AttemptArena<G>,
    ) -> String {
        self.render_context_for_levels(&self.levels, stores, parameters, attempt)
    }

    /// Whether §312's first displayed level enters §314's unconditional
    /// `print_ln` arm rather than §313's/§314's conditional `print_nl` arm.
    ///
    /// Most callers append the rendered context while a diagnostic line is
    /// open, where both arms contribute one newline. e-TeX's nesting warnings
    /// first finish their own line, so the distinction becomes observable as
    /// a blank separator before an ordinary token-list level.
    pub(crate) fn open_context_starts_with_print_ln(
        &self,
        stores: &tex_state::CommandContext<'_, G>,
        parameters: &crate::macro_call::ParameterState<G>,
        attempt: &crate::attempt::AttemptArena<G>,
    ) -> bool {
        let widths = stores.error_context_widths();
        for (index, level) in self.levels.iter().enumerate().rev() {
            let current = index + 1 == self.levels.len();
            match level {
                InputLevel::Source(source) => {
                    let bottom = index == 0
                        || matches!(source.name_class, crate::input::SourceNameClass::File);
                    if Self::source_context_level(source, index == 0, None, None, widths).is_some()
                        || bottom
                    {
                        return false;
                    }
                }
                InputLevel::Tokens(tokens) => {
                    if Self::token_context_level(
                        stores, tokens, current, parameters, attempt, widths,
                    )
                    .is_some()
                    {
                        // §314's backed-up family uses `print_nl` for both
                        // `<recently read>` and `<to be read again>`; every
                        // other token-list kind begins with `print_ln`.
                        return !matches!(tokens.trace, ReplayTrace::BackedUp);
                    }
                }
            }
        }
        false
    }

    /// `show_context` projection for e-TeX `file_warning`, whose retiring
    /// source has completed its last line but has not yet left `input_stack`.
    pub(crate) fn output_retiring_source_context(
        &self,
        retiring: &SourceLevel<G>,
        stores: &tex_state::CommandContext<'_, G>,
        parameters: &crate::macro_call::ParameterState<G>,
        attempt: &crate::attempt::AttemptArena<G>,
    ) -> String {
        let mut levels = self.levels.clone();
        if let Some(InputLevel::Source(source)) = levels.iter_mut().find(|level| {
            matches!(level, InputLevel::Source(source) if source.identity() == retiring.identity())
        }) {
            *source = retiring.clone();
            if let Some(line) = source.cursor.line.as_mut() {
                line.physical = line.physical.with_number(source.cursor.next_line_number);
            }
        }
        self.render_context_for_levels(&levels, stores, parameters, attempt)
    }

    pub(crate) fn output_close_context(
        &self,
        stores: &tex_state::CommandContext<'_, G>,
        parameters: &crate::macro_call::ParameterState<G>,
        attempt: &crate::attempt::AttemptArena<G>,
    ) -> String {
        let output_index = self.levels.iter().position(|level| {
            matches!(
                level,
                InputLevel::Tokens(TokenCursor {
                    trace: ReplayTrace::Stored(StoredReplayReason::OutputRoutine),
                    ..
                })
            )
        });
        let levels = output_index.map_or(self.levels.as_slice(), |index| &self.levels[..index]);
        self.render_context_for_levels(levels, stores, parameters, attempt)
    }

    fn render_context_for_levels(
        &self,
        levels: &[InputLevel<G>],
        stores: &tex_state::CommandContext<'_, G>,
        parameters: &crate::macro_call::ParameterState<G>,
        attempt: &crate::attempt::AttemptArena<G>,
    ) -> String {
        let widths = stores.error_context_widths();
        tex_state::print::render_error_context(
            &self.error_context_levels_for(levels, stores, parameters, attempt, widths),
            widths,
            stores.untracked_int_param(tex_state::env::banks::IntParam::new(54)),
        )
    }

    /// §312's `<Display the current context>`, innermost level first.
    ///
    /// §310 stops at the first level that sets `bottom_line` -- a non-token
    /// level that is either a real file (`name>19` in e-TeX) or the bottom of
    /// the stack -- so a scantokens pseudo-file (`name=18` or `19`) keeps
    /// traversing while nothing below an `\input`ed file is projected.
    fn error_context_levels_for(
        &self,
        input_levels: &[InputLevel<G>],
        stores: &tex_state::CommandContext<'_, G>,
        parameters: &crate::macro_call::ParameterState<G>,
        attempt: &crate::attempt::AttemptArena<G>,
        widths: tex_state::print::ErrorContextWidths,
    ) -> Vec<tex_state::print::ErrorContextLevel> {
        let mut levels = Vec::new();
        let newlinechar = char::from_u32(
            stores.untracked_int_param(tex_state::env::banks::IntParam::NEWLINE_CHAR) as u32,
        );
        let live_endlinechar = char::from_u32(
            stores.untracked_int_param(tex_state::env::banks::IntParam::END_LINE_CHAR) as u32,
        );
        let mut reached_bottom_source = false;
        for (index, level) in input_levels.iter().enumerate().rev() {
            let current = levels.is_empty() && index + 1 == input_levels.len();
            match level {
                InputLevel::Source(source) => {
                    let bottom = index == 0
                        || matches!(source.name_class, crate::input::SourceNameClass::File);
                    let Some(rendered) = Self::source_context_level(
                        source,
                        index == 0,
                        live_endlinechar,
                        newlinechar,
                        widths,
                    ) else {
                        // A source level with no live line has nothing to
                        // pseudoprint, but §310 still stops here.
                        if bottom {
                            reached_bottom_source = true;
                            break;
                        }
                        continue;
                    };
                    levels.push(rendered);
                    if bottom {
                        reached_bottom_source = true;
                        break;
                    }
                }
                InputLevel::Tokens(tokens) => {
                    if let Some(rendered) = Self::token_context_level(
                        stores, tokens, current, parameters, attempt, widths,
                    ) {
                        levels.push(rendered);
                    }
                }
            }
        }
        if !reached_bottom_source && let Some(line) = &self.terminal_context_line {
            let mut before = ContextTail::new(widths.half_error_line());
            let mut raw = String::new();
            let mut rendered = String::new();
            for ch in line.chars() {
                raw.clear();
                raw.push(ch);
                rendered.clear();
                stores.append_selector_string_text(&raw, &mut rendered);
                before.push_str(&rendered);
            }
            let (before, before_chars) = before.finish();
            levels.push(
                tex_state::print::ErrorContextLevel::from_bounded_projection(
                    "<*> ",
                    before,
                    before_chars,
                    "",
                    0,
                ),
            );
        }
        levels
    }

    /// §313's `<Print location of current line>` and `<Pseudoprint the line>`.
    fn source_context_level(
        source: &SourceLevel<G>,
        bottom_of_stack: bool,
        live_endlinechar: Option<char>,
        newlinechar: Option<char>,
        widths: tex_state::print::ErrorContextWidths,
    ) -> Option<tex_state::print::ErrorContextLevel> {
        use crate::input::SourceNameClass;

        fn append_source_text(
            text: &str,
            newlinechar: Option<char>,
            sink: &mut ContextSink<'_>,
            scratch: &mut String,
        ) {
            for character in text.chars() {
                if Some(character) == newlinechar {
                    sink.push_str("\n");
                } else {
                    scratch.clear();
                    tex_state::token_show::append_tex_print_char(character, scratch);
                    sink.push_str(scratch);
                }
                if sink.is_complete() {
                    break;
                }
            }
        }

        fn append_source_code(
            code: crate::profile::CharacterCode,
            newlinechar: Option<char>,
            sink: &mut ContextSink<'_>,
            scratch: &mut String,
        ) -> Option<()> {
            let character = if code.is_byte() {
                char::from(code.to_byte().ok()?)
            } else {
                code.to_char().ok()?
            };
            scratch.clear();
            if Some(character) == newlinechar {
                scratch.push('\n');
            } else {
                tex_state::token_show::append_tex_print_char(character, scratch);
            }
            sink.push_str(scratch);
            Some(())
        }

        let line = source.cursor.line.as_ref()?;
        let bytes = &source.cursor.current_backing().bytes;
        let start = line.physical.content_range().start();
        let end = line.retained_end;
        let cursor = line.byte_cursor.clamp(start, end);
        let (Ok(start), Ok(end), Ok(cursor)) = (
            usize::try_from(start),
            usize::try_from(end),
            usize::try_from(cursor),
        ) else {
            return None;
        };
        // §313 ends every one of its branches with the same `print_char(" ")`,
        // including the `<insert> ` arm that already carries a space.
        let label = match source.name_class {
            SourceNameClass::Terminal if bottom_of_stack => "<*> ".to_owned(),
            SourceNameClass::Terminal => "<insert>  ".to_owned(),
            // §303's stream 16 is the invalid stream number `\read` reads from
            // the terminal under `read_toks` control, and §313 spells it `*`.
            SourceNameClass::ReadStream(16) => "<read *> ".to_owned(),
            SourceNameClass::ReadStream(stream) => format!("<read {stream}> "),
            SourceNameClass::Scantokens(_) | SourceNameClass::File => {
                format!("l.{} ", line.physical.number())
            }
        };
        // §313 pseudoprints each buffer character through §59's `print`, so
        // both the live `new_line_char` and TeX's printable character-string
        // spelling apply to physical source text just as they do to tokens.
        let render_range = |range_start: usize,
                            range_end: usize,
                            sink: &mut ContextSink<'_>,
                            scratch: &mut String|
         -> Option<()> {
            let mut position = range_start;
            for spelling in &line.reduced_spellings {
                if sink.is_complete() {
                    break;
                }
                let spelling_start = usize::try_from(spelling.range.start()).ok()?;
                let spelling_end = usize::try_from(spelling.range.end()).ok()?;
                if spelling_end <= range_start || spelling_start >= range_end {
                    continue;
                }
                if spelling_start < position || spelling_end > range_end {
                    return None;
                }
                append_source_text(
                    &String::from_utf8_lossy(&bytes[position..spelling_start]),
                    newlinechar,
                    sink,
                    scratch,
                );
                append_source_code(spelling.code, newlinechar, sink, scratch)?;
                position = spelling_end;
            }
            if !sink.is_complete() {
                append_source_text(
                    &String::from_utf8_lossy(&bytes[position..range_end]),
                    newlinechar,
                    sink,
                    scratch,
                );
            }
            Some(())
        };
        let mut scratch = String::new();
        let mut before = ContextTail::new(widths.half_error_line());
        render_range(
            start,
            cursor,
            &mut ContextSink::Tail(&mut before),
            &mut scratch,
        )?;
        if let Some(endline) = line.endline {
            let character = if endline.is_byte() {
                char::from(endline.to_byte().ok()?)
            } else {
                endline.to_char().ok()?
            };
            // §313 sets `j:=limit` when the stored buffer sentinel still
            // equals the live `end_line_char`, excluding it from pseudoprint;
            // otherwise `j:=limit+1` and the stale character is visible.
            if Some(character) != live_endlinechar && line.endline_delivered {
                scratch.clear();
                if Some(character) == newlinechar {
                    scratch.push('\n');
                } else {
                    tex_state::token_show::append_tex_print_char(character, &mut scratch);
                }
                before.push_str(&scratch);
            }
        }
        let (before, before_chars) = before.finish();
        let label_chars = label.chars().count();
        let indent = if label_chars.saturating_add(before_chars) <= widths.half_error_line() {
            label_chars.saturating_add(before_chars)
        } else {
            widths.half_error_line()
        };
        let mut after = ContextHead::new(widths.error_line().saturating_sub(indent));
        render_range(
            cursor,
            end,
            &mut ContextSink::Head(&mut after),
            &mut scratch,
        )?;
        if !after.is_complete()
            && let Some(endline) = line.endline
            && !line.endline_delivered
        {
            let character = if endline.is_byte() {
                char::from(endline.to_byte().ok()?)
            } else {
                endline.to_char().ok()?
            };
            if Some(character) != live_endlinechar {
                scratch.clear();
                if Some(character) == newlinechar {
                    scratch.push('\n');
                } else {
                    tex_state::token_show::append_tex_print_char(character, &mut scratch);
                }
                after.push_str(&scratch);
            }
        }
        let (after, after_chars) = after.finish();
        Some(
            tex_state::print::ErrorContextLevel::from_bounded_projection(
                label,
                before,
                before_chars,
                after,
                after_chars,
            ),
        )
    }

    /// §314's `<Print type of token list>` and §315's pseudoprint.
    fn token_context_level(
        stores: &tex_state::CommandContext<'_, G>,
        tokens: &TokenCursor<G>,
        current: bool,
        parameters: &crate::macro_call::ParameterState<G>,
        attempt: &crate::attempt::AttemptArena<G>,
        widths: tex_state::print::ErrorContextWidths,
    ) -> Option<tex_state::print::ErrorContextLevel> {
        fn payload_len<G>(
            _stores: &tex_state::CommandContext<'_, G>,
            tokens: &TokenCursor<G>,
            _parameters: &crate::macro_call::ParameterState<G>,
        ) -> usize {
            match &tokens.payload {
                TokenPayload::Packed(chunk) => chunk.len(),
                TokenPayload::MacroReplacement { len, .. } => *len as usize,
                TokenPayload::Argument { len, .. } => *len as usize,
            }
        }

        fn payload_token<G>(
            stores: &tex_state::CommandContext<'_, G>,
            tokens: &TokenCursor<G>,
            index: usize,
            _parameters: &crate::macro_call::ParameterState<G>,
            attempt: &crate::attempt::AttemptArena<G>,
        ) -> Option<tex_state::token::Token> {
            match &tokens.payload {
                TokenPayload::Packed(chunk) => chunk.word(index).map(|word| word.semantic_token()),
                TokenPayload::MacroReplacement { definition, .. } => stores
                    .definition(*definition)
                    .replacement_text()
                    .get(index)
                    .map(|word| word.semantic_token()),
                TokenPayload::Argument { list, .. } => attempt
                    .token_word(*list, index)
                    .ok()
                    .map(|word| word.semantic_token()),
            }
        }

        fn render_token(
            stores: &tex_state::CommandContext<'_, G>,
            token: tex_state::token::Token,
            raw: &mut String,
            rendered: &mut String,
        ) {
            raw.clear();
            rendered.clear();
            crate::processor::expand::append_token_list_token_text(stores, token, raw);
            stores.append_selector_string_text(raw, rendered);
            #[cfg(test)]
            note_token_context_projection(raw, rendered);
        }

        fn render_selector_text(
            stores: &tex_state::CommandContext<'_, G>,
            text: &str,
            rendered: &mut String,
        ) {
            rendered.clear();
            stores.append_selector_string_text(text, rendered);
        }

        let macro_context = if let ReplayTrace::MacroReplacement = tokens.trace {
            let TokenBehavior::MacroBody(activation) = tokens.behavior else {
                return None;
            };
            let TokenPayload::MacroReplacement { definition, .. } = &tokens.payload else {
                return None;
            };
            let activation = parameters
                .activations
                .iter()
                .find(|candidate| candidate.identity == activation)?;
            if activation.definition != *definition {
                return None;
            }
            Some((
                crate::processor::expand::token_list_token_text(
                    stores,
                    tex_state::token::Token::Cs(activation.name),
                ),
                *definition,
            ))
        } else {
            None
        };
        let count = payload_len(stores, tokens, parameters);
        let split = tokens.position().min(count);
        let noexpand_marker = matches!(
            tokens.behavior,
            TokenBehavior::BackedUp(BackupTreatment::SuppressExpandableControlSequence)
        )
        .then(|| {
            format!(
                "{} ",
                crate::processor::expand::print_esc_text(stores, "notexpanded:")
            )
        });
        let v_sentinel = matches!(tokens.behavior, TokenBehavior::VTemplate).then(|| {
            crate::processor::expand::token_list_token_text(
                stores,
                stores.frozen_end_template_token(),
            )
        });
        let mut raw = String::new();
        let mut rendered = String::new();
        let mut before = ContextTail::new(widths.half_error_line());
        if matches!(
            tokens.retirement,
            RetirementBehavior::AwaitingVTemplateRetirement
        ) && let Some(sentinel) = v_sentinel.as_deref()
        {
            before.prepend_str(sentinel);
        }
        for index in (0..split).rev() {
            if before.is_complete() {
                break;
            }
            if let Some(token) = payload_token(stores, tokens, index, parameters, attempt) {
                render_token(stores, token, &mut raw, &mut rendered);
                before.prepend_str(&rendered);
            }
        }
        if let Some((_, definition)) = &macro_context {
            if !before.is_complete() {
                before.prepend_str("->");
            }
            let owner = stores.definition(*definition);
            for index in (0..owner.parameter_text().len()).rev() {
                if before.is_complete() {
                    break;
                }
                let token = owner.parameter_text().get(index)?.semantic_token();
                render_token(stores, token, &mut raw, &mut rendered);
                before.prepend_str(&rendered);
            }
        }
        if split > 0
            && !before.is_complete()
            && let Some(marker) = noexpand_marker.as_deref()
        {
            render_selector_text(stores, marker, &mut rendered);
            before.prepend_str(&rendered);
        }
        let (before, before_chars) = before.finish();
        let label = if let Some((label, _)) = &macro_context {
            label.clone()
        } else {
            match tokens.trace {
                ReplayTrace::MacroParameter { .. } => "<argument> ".to_owned(),
                ReplayTrace::MacroReplacement => unreachable!("handled above"),
                ReplayTrace::BackedUp => String::new(),
                ReplayTrace::Inserted => "<inserted text> ".to_owned(),
                ReplayTrace::UTemplate | ReplayTrace::VTemplate | ReplayTrace::OmitTemplate => {
                    "<template> ".to_owned()
                }
                ReplayTrace::Stored(StoredReplayReason::OutputRoutine) => "<output> ".to_owned(),
                ReplayTrace::Stored(StoredReplayReason::EveryPar) => "<everypar> ".to_owned(),
                ReplayTrace::Stored(StoredReplayReason::EveryMath) => "<everymath> ".to_owned(),
                ReplayTrace::Stored(StoredReplayReason::EveryDisplay) => {
                    "<everydisplay> ".to_owned()
                }
                ReplayTrace::Stored(StoredReplayReason::EveryHBox) => "<everyhbox> ".to_owned(),
                ReplayTrace::Stored(StoredReplayReason::EveryVBox) => "<everyvbox> ".to_owned(),
                ReplayTrace::Stored(StoredReplayReason::EveryJob) => "<everyjob> ".to_owned(),
                ReplayTrace::Stored(StoredReplayReason::EveryCr) => "<everycr> ".to_owned(),
                ReplayTrace::Stored(StoredReplayReason::EveryEof) => "<everyeof> ".to_owned(),
                ReplayTrace::Stored(StoredReplayReason::Mark) => "<mark> ".to_owned(),
                ReplayTrace::Stored(StoredReplayReason::Write) => "<write> ".to_owned(),
                ReplayTrace::Stored(StoredReplayReason::Discretionary)
                | ReplayTrace::Transient(_) => "<token list> ".to_owned(),
            }
        };
        let indent =
            if label.chars().count().saturating_add(before_chars) <= widths.half_error_line() {
                label.chars().count().saturating_add(before_chars)
            } else {
                widths.half_error_line()
            };
        let available = widths.error_line().saturating_sub(indent);
        let mut after = ContextHead::new(available);
        if split == 0
            && let Some(marker) = noexpand_marker.as_deref()
        {
            render_selector_text(stores, marker, &mut rendered);
            after.push_str(&rendered);
        }
        for index in split..count {
            if after.is_complete() {
                break;
            }
            if let Some(token) = payload_token(stores, tokens, index, parameters, attempt) {
                render_token(stores, token, &mut raw, &mut rendered);
                after.push_str(&rendered);
            }
        }
        if !after.is_complete()
            && matches!(
                tokens.retirement,
                RetirementBehavior::RetainExhaustedVTemplate
            )
            && let Some(sentinel) = v_sentinel.as_deref()
        {
            after.push_str(sentinel);
        }
        let (after, after_chars) = after.finish();
        if matches!(tokens.trace, ReplayTrace::BackedUp) && after_chars == 0 && !current {
            return None;
        }
        let label = if matches!(tokens.trace, ReplayTrace::BackedUp) {
            if after_chars == 0 {
                "<recently read> ".to_owned()
            } else {
                "<to be read again> ".to_owned()
            }
        } else {
            label
        };
        #[cfg(test)]
        note_token_context_capacity(
            before
                .capacity()
                .max(after.capacity())
                .max(raw.capacity())
                .max(rendered.capacity()),
        );
        Some(
            tex_state::print::ErrorContextLevel::from_bounded_projection(
                label,
                before,
                before_chars,
                after,
                after_chars,
            ),
        )
    }

    /// TeX82's current `line` value for e-TeX's `\inputlineno`.
    ///
    /// Token-list levels retain the source line they interrupted; terminal and
    /// `\read` levels have no file line number and therefore expose zero.
    pub(crate) fn current_file_line_number(&self) -> i32 {
        self.levels
            .iter()
            .rev()
            .find_map(|level| match level {
                InputLevel::Source(source)
                    if matches!(
                        source.name_class,
                        SourceNameClass::Scantokens(_) | SourceNameClass::File
                    ) =>
                {
                    source
                        .cursor
                        .line
                        .as_ref()
                        .map(|line| line.physical.number().min(i32::MAX as u64) as i32)
                }
                InputLevel::Source(_) => Some(0),
                InputLevel::Tokens(_) => None,
            })
            .unwrap_or(0)
    }

    pub(crate) fn current_file_source_id(&self) -> Option<tex_state::SourceId> {
        self.levels.iter().rev().find_map(|level| match level {
            InputLevel::Source(source)
                if matches!(
                    source.name_class,
                    SourceNameClass::Scantokens(_) | SourceNameClass::File
                ) =>
            {
                Some(source.cursor.current_backing().id)
            }
            InputLevel::Source(_) => None,
            InputLevel::Tokens(_) => None,
        })
    }
}
