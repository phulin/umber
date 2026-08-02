//! Private input state machines.

mod backup;
mod levels;
mod lines;
mod source;
mod stack;
mod summary;
mod tokenizer;

#[cfg(test)]
mod tests;

pub(crate) use levels::{
    BackedUpToken, BackupTreatment, InputLevel, InputLevelId, ReplayTrace, RetirementBehavior,
    SharedBackedUpBuffer, SharedTokenBuffer, SourceLevel, SourceOpenDepths, SourceRetirement,
    StoredReplayReason, TokenBehavior, TokenCursor, TokenPayload,
};
pub(crate) use source::{LineBackingRegistry, RegisteredSource, SourceCursor};
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
    FileFramingEvent, MalformedUnicodeRange, RegisteredSourceKind, SourceNameClass,
    SourceRegistration, SourceRegistrationError,
};
pub use tokenizer::{
    CatcodeQueries, InvalidSourceCharacter, LexerState, SourceControlSequenceKind,
    SourceStepQueries, SourceToken, SourceTokenizationStep,
};

/// Persistent input-stack ownership.
///
/// This state owns only future deliveries and semantic identity allocation.
/// Conditions, scanner policy, meanings, and host capabilities belong to
/// other ownership classes.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub(crate) struct InputState {
    pub(crate) levels: Vec<InputLevel>,
    pub(crate) registered_sources: Vec<RegisteredSource>,
    pub(crate) next_level_identity: u64,
    pub(crate) next_source_identity: u64,
    /// TeX82 §362's process-global `force_eof`.
    pub(crate) force_eof: bool,
}

impl InputState {
    /// tex.web §310's `show_context` display for the canonical input stack.
    ///
    /// The two-line pseudoprint arithmetic (§316--§318) and §310's own
    /// `\errorcontextlines` elision are
    /// [`tex_state::print::render_error_context`]; this is §312--§314's
    /// projection of the command core's levels onto it. Every other input
    /// stack in the engine projects onto the same renderer.
    pub(crate) fn output_open_context(
        &self,
        stores: &tex_state::CommandContext<'_>,
        parameters: &crate::macro_call::ParameterState,
    ) -> String {
        self.render_context_for_levels(&self.levels, stores, parameters)
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
        stores: &tex_state::CommandContext<'_>,
        parameters: &crate::macro_call::ParameterState,
    ) -> bool {
        for (index, level) in self.levels.iter().enumerate().rev() {
            let current = index + 1 == self.levels.len();
            match level {
                InputLevel::Source(source) => {
                    let bottom = index == 0
                        || matches!(source.name_class, crate::input::SourceNameClass::File);
                    if Self::source_context_level(source, index == 0).is_some() || bottom {
                        return false;
                    }
                }
                InputLevel::Tokens(tokens) => {
                    if Self::token_context_level(stores, tokens, current, parameters).is_some() {
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
        retiring: &SourceLevel,
        stores: &tex_state::CommandContext<'_>,
        parameters: &crate::macro_call::ParameterState,
    ) -> String {
        let mut levels = self.levels.clone();
        if let Some(InputLevel::Source(source)) = levels.iter_mut().find(|level| {
            matches!(level, InputLevel::Source(source) if source.identity == retiring.identity)
        }) {
            *source = retiring.clone();
            if let Some(line) = source.cursor.line.as_mut() {
                line.physical = line.physical.with_number(source.cursor.next_line_number);
            }
        }
        self.render_context_for_levels(&levels, stores, parameters)
    }

    pub(crate) fn output_close_context(
        &self,
        stores: &tex_state::CommandContext<'_>,
        parameters: &crate::macro_call::ParameterState,
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
        self.render_context_for_levels(levels, stores, parameters)
    }

    fn render_context_for_levels(
        &self,
        levels: &[InputLevel],
        stores: &tex_state::CommandContext<'_>,
        parameters: &crate::macro_call::ParameterState,
    ) -> String {
        tex_state::print::render_error_context(
            &self.error_context_levels_for(levels, stores, parameters),
            stores.error_context_widths(),
            stores.int_param(tex_state::env::banks::IntParam::new(54)),
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
        input_levels: &[InputLevel],
        stores: &tex_state::CommandContext<'_>,
        parameters: &crate::macro_call::ParameterState,
    ) -> Vec<tex_state::print::ErrorContextLevel> {
        let mut levels = Vec::new();
        for (index, level) in input_levels.iter().enumerate().rev() {
            let current = levels.is_empty() && index + 1 == input_levels.len();
            match level {
                InputLevel::Source(source) => {
                    let bottom = index == 0
                        || matches!(source.name_class, crate::input::SourceNameClass::File);
                    let Some(rendered) = Self::source_context_level(source, index == 0) else {
                        // A source level with no live line has nothing to
                        // pseudoprint, but §310 still stops here.
                        if bottom {
                            break;
                        }
                        continue;
                    };
                    levels.push(rendered);
                    if bottom {
                        break;
                    }
                }
                InputLevel::Tokens(tokens) => {
                    if let Some(rendered) =
                        Self::token_context_level(stores, tokens, current, parameters)
                    {
                        levels.push(rendered);
                    }
                }
            }
        }
        levels
    }

    /// §313's `<Print location of current line>` and `<Pseudoprint the line>`.
    fn source_context_level(
        source: &SourceLevel,
        bottom_of_stack: bool,
    ) -> Option<tex_state::print::ErrorContextLevel> {
        use crate::input::SourceNameClass;

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
        Some(tex_state::print::ErrorContextLevel::new(
            label,
            String::from_utf8_lossy(&bytes[start..cursor]),
            String::from_utf8_lossy(&bytes[cursor..end]),
        ))
    }

    /// §314's `<Print type of token list>` and §315's pseudoprint.
    fn token_context_level(
        stores: &tex_state::CommandContext<'_>,
        tokens: &TokenCursor,
        current: bool,
        parameters: &crate::macro_call::ParameterState,
    ) -> Option<tex_state::print::ErrorContextLevel> {
        fn token_text(
            stores: &tex_state::CommandContext<'_>,
            tokens: impl Iterator<Item = tex_state::token::Token>,
        ) -> String {
            tokens
                .map(|token| crate::processor::expand::token_list_token_text(stores, token))
                .collect()
        }

        let (before, after) = match &tokens.payload {
            TokenPayload::Stored { tokens: list, .. } => {
                let words = stores.tokens(*list);
                let split = tokens.index.min(words.len());
                (
                    token_text(stores, words[..split].iter().copied()),
                    token_text(stores, words[split..].iter().copied()),
                )
            }
            TokenPayload::Transient(words) => {
                let split = tokens.index.min(words.len());
                (
                    token_text(
                        stores,
                        (0..split).filter_map(|index| words.get(index).map(|w| w.semantic_token())),
                    ),
                    token_text(
                        stores,
                        (split..words.len())
                            .filter_map(|index| words.get(index).map(|w| w.semantic_token())),
                    ),
                )
            }
            TokenPayload::BackedUp(words) => {
                let before = (0..tokens.index)
                    .filter_map(|index| words.get(index))
                    .map(|word| word.spelling.semantic_token());
                let after = (tokens.index..)
                    .map_while(|index| words.get(index))
                    .map(|word| word.spelling.semantic_token());
                (token_text(stores, before), token_text(stores, after))
            }
            TokenPayload::ArgumentRange { buffer, range } => {
                let start = range.start();
                let end = range.end();
                let split = start.saturating_add(tokens.index).min(end);
                (
                    token_text(
                        stores,
                        (start..split)
                            .filter_map(|index| buffer.get(index).map(|w| w.semantic_token())),
                    ),
                    token_text(
                        stores,
                        (split..end)
                            .filter_map(|index| buffer.get(index).map(|w| w.semantic_token())),
                    ),
                )
            }
        };
        // §314's macro arm is `print_ln; print_cs(name)` -- the control
        // sequence being expanded, not a bracketed type name -- and §319
        // pseudoprints `link(start)`, the whole macro text, so the parameter
        // text and the `->` that §294 renders for `end_match` precede the
        // replacement the cursor is inside.
        if let ReplayTrace::MacroReplacement = tokens.trace {
            let TokenBehavior::MacroBody(activation) = tokens.behavior else {
                return None;
            };
            let activation = parameters
                .activations
                .iter()
                .find(|candidate| candidate.identity == activation)?;
            let label = crate::processor::expand::token_list_token_text(
                stores,
                tex_state::token::Token::Cs(activation.name),
            );
            let parameter_text = token_text(
                stores,
                stores
                    .tokens(
                        stores
                            .macro_definition(activation.definition)
                            .parameter_text(),
                    )
                    .iter()
                    .copied(),
            );
            return Some(tex_state::print::ErrorContextLevel::new(
                label,
                format!("{parameter_text}->{before}"),
                after,
            ));
        }
        // §314's `loc=null` test, which only the `backed_up` family consults.
        let exhausted = after.is_empty();
        let label = match tokens.trace {
            ReplayTrace::MacroParameter { .. } => "<argument> ",
            ReplayTrace::MacroReplacement => unreachable!("handled above"),
            ReplayTrace::BackedUp => {
                // §312 omits a `backed_up` list that has already been read
                // through, unless it is the level the error happened on.
                if exhausted && !current {
                    return None;
                }
                if exhausted {
                    "<recently read> "
                } else {
                    "<to be read again> "
                }
            }
            ReplayTrace::Inserted => "<inserted text> ",
            ReplayTrace::UTemplate | ReplayTrace::VTemplate | ReplayTrace::OmitTemplate => {
                "<template> "
            }
            ReplayTrace::Stored(StoredReplayReason::OutputRoutine) => "<output> ",
            ReplayTrace::Stored(StoredReplayReason::EveryPar) => "<everypar> ",
            ReplayTrace::Stored(StoredReplayReason::EveryMath) => "<everymath> ",
            ReplayTrace::Stored(StoredReplayReason::EveryDisplay) => "<everydisplay> ",
            ReplayTrace::Stored(StoredReplayReason::EveryHBox) => "<everyhbox> ",
            ReplayTrace::Stored(StoredReplayReason::EveryVBox) => "<everyvbox> ",
            ReplayTrace::Stored(StoredReplayReason::EveryJob) => "<everyjob> ",
            ReplayTrace::Stored(StoredReplayReason::EveryCr) => "<everycr> ",
            ReplayTrace::Stored(StoredReplayReason::EveryEof) => "<everyeof> ",
            ReplayTrace::Stored(StoredReplayReason::Mark) => "<mark> ",
            ReplayTrace::Stored(StoredReplayReason::Write) => "<write> ",
            ReplayTrace::Stored(StoredReplayReason::Discretionary) | ReplayTrace::Transient(_) => {
                "<token list> "
            }
        };
        Some(tex_state::print::ErrorContextLevel::new(
            label, before, after,
        ))
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
}
