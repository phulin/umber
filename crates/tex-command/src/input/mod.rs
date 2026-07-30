//! Private input state machines.

mod backup;
mod levels;
mod lines;
mod source;
mod stack;
mod summary;
mod tokenizer;

pub(crate) use levels::{
    BackedUpToken, BackupTreatment, InputLevel, InputLevelId, ReplayTrace, RetirementBehavior,
    SharedBackedUpBuffer, SharedTokenBuffer, SourceLevel, SourceRetirement, StoredReplayReason,
    TokenBehavior, TokenCursor, TokenPayload, TransientReplayReason,
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
    MalformedUnicodeRange, RegisteredSourceKind, SourceNameClass, SourceRegistration,
    SourceRegistrationError,
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
    pub(crate) fn output_open_context(&self, stores: &tex_state::CommandContext<'_>) -> String {
        const ERROR_LINE: usize = 79;
        const HALF_ERROR_LINE: usize = 50;

        fn clipped(label: &str, before: &str, after: &str) -> String {
            let mut left = format!("{label}{before}");
            let len = left.chars().count();
            if len > HALF_ERROR_LINE {
                left = format!(
                    "...{}",
                    left.chars()
                        .skip(len - (HALF_ERROR_LINE - 3))
                        .collect::<String>()
                );
            }
            let indent = left.chars().count();
            let available = ERROR_LINE.saturating_sub(indent);
            let right = if after.chars().count() > available {
                format!(
                    "{}...",
                    after
                        .chars()
                        .take(available.saturating_sub(3))
                        .collect::<String>()
                )
            } else {
                after.to_owned()
            };
            format!("\n{left}\n{}{right}", " ".repeat(indent))
        }

        fn token_text(
            stores: &tex_state::CommandContext<'_>,
            tokens: impl Iterator<Item = tex_state::token::Token>,
        ) -> String {
            tokens
                .map(|token| crate::processor::expand::token_list_token_text(stores, token))
                .collect()
        }

        let max_token_levels = stores
            .int_param(tex_state::env::banks::IntParam::new(54))
            .max(0) as usize;
        let mut shown = 0usize;
        let mut output = String::new();
        for level in self.levels.iter().rev() {
            match level {
                InputLevel::Source(source) => {
                    let Some(line) = source.cursor.line.as_ref() else {
                        continue;
                    };
                    let bytes = &source.cursor.current_backing().bytes;
                    let start = line.physical.content_range().start();
                    let end = line.retained_end;
                    let cursor = line.byte_cursor.clamp(start, end);
                    let (Ok(start), Ok(end), Ok(cursor)) = (
                        usize::try_from(start),
                        usize::try_from(end),
                        usize::try_from(cursor),
                    ) else {
                        continue;
                    };
                    output.push_str(&clipped(
                        &format!("l.{} ", line.physical.number()),
                        &String::from_utf8_lossy(&bytes[start..cursor]),
                        &String::from_utf8_lossy(&bytes[cursor..end]),
                    ));
                    break;
                }
                InputLevel::Tokens(tokens) if shown < max_token_levels => {
                    let label = match tokens.trace {
                        ReplayTrace::MacroParameter { .. } => "<argument> ",
                        ReplayTrace::MacroReplacement => "<macro> ",
                        ReplayTrace::Inserted => "<inserted text> ",
                        ReplayTrace::Stored(StoredReplayReason::OutputRoutine) => "<output> ",
                        ReplayTrace::Stored(StoredReplayReason::EveryPar) => "<everypar> ",
                        ReplayTrace::Stored(StoredReplayReason::EveryHBox) => "<everyhbox> ",
                        ReplayTrace::Stored(StoredReplayReason::EveryVBox) => "<everyvbox> ",
                        ReplayTrace::Stored(StoredReplayReason::EveryJob) => "<everyjob> ",
                        ReplayTrace::Stored(StoredReplayReason::EveryCr) => "<everycr> ",
                        ReplayTrace::Stored(StoredReplayReason::Mark) => "<mark> ",
                        _ => "<token list> ",
                    };
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
                                    (0..split).filter_map(|index| {
                                        words.get(index).map(|w| w.semantic_token())
                                    }),
                                ),
                                token_text(
                                    stores,
                                    (split..words.len()).filter_map(|index| {
                                        words.get(index).map(|w| w.semantic_token())
                                    }),
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
                                    (start..split).filter_map(|index| {
                                        buffer.get(index).map(|w| w.semantic_token())
                                    }),
                                ),
                                token_text(
                                    stores,
                                    (split..end).filter_map(|index| {
                                        buffer.get(index).map(|w| w.semantic_token())
                                    }),
                                ),
                            )
                        }
                    };
                    output.push_str(&clipped(label, &before, &after));
                    shown += 1;
                }
                InputLevel::Tokens(_) => {}
            }
        }
        output
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
                    if matches!(source.name_class, SourceNameClass::File) =>
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
