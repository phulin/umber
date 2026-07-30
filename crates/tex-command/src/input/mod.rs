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
    SharedBackedUpBuffer, SharedTokenBuffer, SourceLevel, SourceRetirement, StoredReplayReason,
    TokenBehavior, TokenCursor, TokenPayload,
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

/// Formats TeX82 §§79--82's two pseudoprinted context lines.
///
/// The descriptive label has already been printed when §82 crops the
/// pseudoprinted prefix. It therefore remains intact while only `before` is
/// replaced by an ellipsis and its fitting suffix.
fn clipped_context(
    label: &str,
    before: &str,
    after: &str,
    widths: tex_state::print::ErrorContextWidths,
) -> String {
    let label_len = label.chars().count();
    let before_len = before.chars().count();
    let (left, indent) = if label_len + before_len > widths.half_error_line() {
        let suffix_len = widths
            .half_error_line()
            .saturating_sub(label_len)
            .saturating_sub(3);
        (
            format!(
                "{label}...{}",
                before
                    .chars()
                    .skip(before_len.saturating_sub(suffix_len))
                    .collect::<String>()
            ),
            widths.half_error_line(),
        )
    } else {
        (
            format!("{label}{before}"),
            label_len.saturating_add(before_len),
        )
    };
    let available = widths.error_line().saturating_sub(indent);
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

impl InputState {
    pub(crate) fn output_open_context(&self, stores: &tex_state::CommandContext<'_>) -> String {
        fn token_text(
            stores: &tex_state::CommandContext<'_>,
            tokens: impl Iterator<Item = tex_state::token::Token>,
        ) -> String {
            tokens
                .map(|token| crate::processor::expand::token_list_token_text(stores, token))
                .collect()
        }

        let max_intermediate_levels = stores
            .int_param(tex_state::env::banks::IntParam::new(54))
            .max(0) as usize;
        let mut contexts = Vec::new();
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
                    contexts.push(clipped_context(
                        &format!("l.{} ", line.physical.number()),
                        &String::from_utf8_lossy(&bytes[start..cursor]),
                        &String::from_utf8_lossy(&bytes[cursor..end]),
                        stores.error_context_widths(),
                    ));
                }
                InputLevel::Tokens(tokens) => {
                    let (before, after, exhausted) = match &tokens.payload {
                        TokenPayload::Stored { tokens: list, .. } => {
                            let words = stores.tokens(*list);
                            let split = tokens.index.min(words.len());
                            (
                                token_text(stores, words[..split].iter().copied()),
                                token_text(stores, words[split..].iter().copied()),
                                tokens.index >= words.len(),
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
                                tokens.index >= words.len(),
                            )
                        }
                        TokenPayload::BackedUp(words) => {
                            let before = (0..tokens.index)
                                .filter_map(|index| words.get(index))
                                .map(|word| word.spelling.semantic_token());
                            let after = (tokens.index..)
                                .map_while(|index| words.get(index))
                                .map(|word| word.spelling.semantic_token());
                            (
                                token_text(stores, before),
                                token_text(stores, after),
                                words.get(tokens.index).is_none(),
                            )
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
                                start.saturating_add(tokens.index) >= end,
                            )
                        }
                    };
                    // TeX82 §530 distinguishes the exhausted one-token
                    // `back_input` level (`loc=null`) from an unread backup.
                    let label = match tokens.trace {
                        ReplayTrace::MacroParameter { .. } => "<argument> ",
                        ReplayTrace::MacroReplacement => "<macro> ",
                        ReplayTrace::BackedUp if exhausted => "<recently read> ",
                        ReplayTrace::BackedUp => "<to be read again> ",
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
                    contexts.push(clipped_context(
                        label,
                        &before,
                        &after,
                        stores.error_context_widths(),
                    ));
                }
            }
        }
        match contexts.as_slice() {
            [] => String::new(),
            [only] => only.clone(),
            [current, rest @ ..] => {
                let (bottom, intermediate) = rest.split_last().expect("rest is nonempty");
                let mut output = current.clone();
                for context in intermediate.iter().take(max_intermediate_levels) {
                    output.push_str(context);
                }
                if intermediate.len() > max_intermediate_levels {
                    output.push_str("\n...");
                }
                output.push_str(bottom);
                output
            }
        }
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
