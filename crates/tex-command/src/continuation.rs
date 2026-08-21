//! Handle-free transport for retained command continuations.
//!
//! Live command state first detaches into owned logical recipes connected only
//! by DTO-local indices. A destination validates the complete graph, stages
//! every destination-local value behind a shared borrow, and publishes the
//! finished replacement with one infallible move.

#![allow(dead_code)] // The .6.4 integration installs runtime detachment adapters.

use core::fmt;

#[path = "continuation/detach.rs"]
mod detach;
#[path = "continuation/materialize.rs"]
mod materialize;
#[path = "continuation/schema.rs"]
mod schema;

#[cfg(test)]
pub(crate) use detach::ContinuationRecipeBuilder;
pub(crate) use materialize::{
    CommandContinuationDestination, MaterializationError, ValidatedCommandContinuation,
};
pub(crate) use schema::*;

/// Detached command-continuation schema version.
pub const COMMAND_CONTINUATION_SCHEMA_VERSION: u32 = 1;

/// Explicit admission budgets for one detached continuation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CommandContinuationLimits {
    pub(crate) sources: usize,
    pub(crate) names: usize,
    pub(crate) token_lists: usize,
    pub(crate) tokens: usize,
    pub(crate) origins: usize,
    pub(crate) origin_list_entries: usize,
    pub(crate) macros: usize,
    pub(crate) glue: usize,
    pub(crate) input_frames: usize,
    pub(crate) source_bytes: usize,
    pub(crate) string_bytes: usize,
}

impl Default for CommandContinuationLimits {
    fn default() -> Self {
        Self {
            sources: 4_096,
            names: 1_000_000,
            token_lists: 1_000_000,
            tokens: 16_000_000,
            origins: 16_000_000,
            origin_list_entries: 16_000_000,
            macros: 1_000_000,
            glue: 1_000_000,
            input_frames: 100_000,
            source_bytes: 512 * 1024 * 1024,
            string_bytes: 64 * 1024 * 1024,
        }
    }
}

/// A canonical command continuation containing only recipes and portable
/// scalar state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OwnedCommandContinuation {
    schema: ContinuationSchema,
}

/// A detached continuation could not be validated or published.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandContinuationError {
    InvalidRecipe(&'static str),
    LimitExceeded(&'static str),
    ForeignDestination,
}

impl fmt::Display for CommandContinuationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRecipe(message) => {
                write!(formatter, "invalid command continuation: {message}")
            }
            Self::LimitExceeded(limit) => {
                write!(formatter, "command continuation exceeds {limit} limit")
            }
            Self::ForeignDestination => formatter
                .write_str("staged command continuation belongs to a different destination"),
        }
    }
}

impl std::error::Error for CommandContinuationError {}

impl OwnedCommandContinuation {
    fn from_schema(schema: ContinuationSchema) -> Result<Self, CommandContinuationError> {
        let continuation = Self { schema };
        continuation.validate(CommandContinuationLimits::default())?;
        Ok(continuation)
    }

    pub(crate) fn materialize<Live, Staged, Output, BuildError>(
        &self,
        destination: &mut CommandContinuationDestination<Live>,
        limits: CommandContinuationLimits,
        build: impl FnOnce(&Live, ValidatedCommandContinuation<'_>) -> Result<Staged, BuildError>,
        publish: impl FnOnce(&mut Live, Staged) -> Output,
    ) -> Result<Output, MaterializationError<BuildError>> {
        let staged = destination.stage(self, limits, build)?;
        destination
            .publish(staged, publish)
            .map_err(MaterializationError::Continuation)
    }

    fn validate(&self, limits: CommandContinuationLimits) -> Result<(), CommandContinuationError> {
        let schema = &self.schema;
        if schema.profile.schema != COMMAND_CONTINUATION_SCHEMA_VERSION {
            return invalid("unsupported schema version");
        }
        if schema.profile.dialect > 2 || schema.profile.character_mode > 1 {
            return invalid("command profile recipe is invalid");
        }
        check_limit(schema.sources.len(), limits.sources, "source recipes")?;
        check_limit(schema.names.len(), limits.names, "name recipes")?;
        check_limit(
            schema.token_lists.len(),
            limits.token_lists,
            "token-list recipes",
        )?;
        check_limit(schema.origins.len(), limits.origins, "origin recipes")?;
        check_limit(schema.macros.len(), limits.macros, "macro recipes")?;
        check_limit(schema.glue.len(), limits.glue, "glue recipes")?;
        check_limit(
            schema.summary.input.len(),
            limits.input_frames,
            "input frames",
        )?;

        let mut source_bytes = 0_usize;
        let mut string_bytes = 0_usize;
        for source in &schema.sources {
            source_bytes = checked_total(source_bytes, source.bytes().len(), "source bytes")?;
            match source {
                SourceRecipe::World { path, .. } => {
                    string_bytes = checked_total(string_bytes, path.len(), "string bytes")?;
                }
                SourceRecipe::Generated { logical_path, .. } => {
                    if let Some(path) = logical_path {
                        string_bytes = checked_total(string_bytes, path.len(), "string bytes")?;
                    }
                }
            }
        }
        for name in &schema.names {
            string_bytes = checked_total(string_bytes, name.spelling.len(), "string bytes")?;
            match name.kind {
                DetachedNameKind::Null if !name.spelling.is_empty() => {
                    return invalid("null control sequence has a spelling");
                }
                DetachedNameKind::ActiveCharacter | DetachedNameKind::SingleCharacter
                    if name.spelling.chars().count() != 1 =>
                {
                    return invalid("character control sequence is not one scalar");
                }
                DetachedNameKind::MultiLetter if name.spelling.is_empty() => {
                    return invalid("multiletter control sequence is empty");
                }
                _ => {}
            }
        }

        let mut tokens = 0_usize;
        for list in &schema.token_lists {
            tokens = checked_total(tokens, list.words.len(), "tokens")?;
            for word in &list.words {
                validate_word(schema, word)?;
            }
        }
        for frame in &schema.summary.input {
            if let InputFrameRecipe::Tokens(TokenFrameRecipe {
                payload: InputPayloadRecipe::Inline(words),
                ..
            }) = frame
            {
                tokens = checked_total(tokens, words.len(), "tokens")?;
            }
        }
        check_limit(tokens, limits.tokens, "tokens")?;

        let mut marks = vec![0_u8; schema.origins.len()];
        for index in 0..schema.origins.len() {
            validate_origin(
                schema,
                OriginRecipeIndex::from_len(index).expect("index"),
                &mut marks,
            )?;
        }

        let mut origin_entries = 0_usize;
        for list in &schema.origin_lists {
            origin_entries =
                checked_total(origin_entries, list.origins.len(), "origin-list entries")?;
            if list
                .origins
                .iter()
                .any(|origin| origin.index() >= schema.origins.len())
            {
                return invalid("origin list references a missing origin recipe");
            }
        }
        check_limit(
            origin_entries,
            limits.origin_list_entries,
            "origin-list entries",
        )?;

        for recipe in &schema.macros {
            let Some(parameters) = schema.token_lists.get(recipe.parameter_text.index()) else {
                return invalid("macro parameter text is missing");
            };
            let Some(replacement) = schema.token_lists.get(recipe.replacement_text.index()) else {
                return invalid("macro replacement text is missing");
            };
            if recipe.definition_origin.index() >= schema.origins.len() {
                return invalid("macro definition origin is missing");
            }
            let Some(parameter_origins) = schema.origin_lists.get(recipe.parameter_origins.index())
            else {
                return invalid("macro parameter origins are missing");
            };
            let Some(replacement_origins) =
                schema.origin_lists.get(recipe.replacement_origins.index())
            else {
                return invalid("macro replacement origins are missing");
            };
            if parameter_origins.origins.len() != parameters.words.len()
                || replacement_origins.origins.len() != replacement.words.len()
            {
                return invalid("macro token and origin recipe lengths differ");
            }
        }

        if schema
            .glue
            .iter()
            .any(|glue| glue.stretch_order > 3 || glue.shrink_order > 3)
        {
            return invalid("glue order is outside TeX's four orders");
        }

        validate_summary(schema)?;
        if let Some(attempt) = &schema.attempt {
            validate_attempt(schema, attempt)?;
            string_bytes = checked_total(string_bytes, attempt.request.key.len(), "string bytes")?;
            source_bytes =
                checked_total(source_bytes, attempt.request.payload.len(), "source bytes")?;
        }
        check_limit(source_bytes, limits.source_bytes, "source bytes")?;
        check_limit(string_bytes, limits.string_bytes, "string bytes")?;
        Ok(())
    }
}

fn validate_word(
    schema: &ContinuationSchema,
    word: &DetachedWord,
) -> Result<(), CommandContinuationError> {
    if word.origin.index() >= schema.origins.len() {
        return invalid("token references a missing origin recipe");
    }
    match &word.token {
        DetachedToken::Character { catcode, .. } if *catcode > 15 => {
            invalid("character token has an invalid category code")
        }
        DetachedToken::Parameter(parameter) if !(1..=9).contains(parameter) => {
            invalid("parameter token is outside 1..=9")
        }
        DetachedToken::ControlSequence(name)
        | DetachedToken::Frozen(DetachedFrozenToken::Primitive(name))
            if name.index() >= schema.names.len() =>
        {
            invalid("token references a missing name recipe")
        }
        _ => Ok(()),
    }
}

fn validate_origin(
    schema: &ContinuationSchema,
    index: OriginRecipeIndex,
    marks: &mut [u8],
) -> Result<(), CommandContinuationError> {
    match marks[index.index()] {
        2 => return Ok(()),
        1 => return invalid("origin recipes contain a cycle"),
        _ => marks[index.index()] = 1,
    }
    let recipe = &schema.origins[index.index()];
    match recipe {
        OriginRecipe::Unknown => {}
        OriginRecipe::SourcePoint { source, byte, .. } => {
            let Some(source) = schema.sources.get(source.index()) else {
                return invalid("source point references a missing source recipe");
            };
            if usize::try_from(*byte).map_or(true, |byte| byte > source.bytes().len()) {
                return invalid("source point exceeds its source bytes");
            }
        }
        OriginRecipe::SourceSpan { source, start, end } => {
            let Some(source) = schema.sources.get(source.index()) else {
                return invalid("source span references a missing source recipe");
            };
            if start > end || usize::try_from(*end).map_or(true, |end| end > source.bytes().len()) {
                return invalid("source span exceeds its source bytes");
            }
        }
        OriginRecipe::Derived {
            primary, related, ..
        } => {
            validate_origin_index(schema, *primary, marks)?;
            if let Some(related) = related {
                validate_origin_index(schema, *related, marks)?;
            }
        }
        OriginRecipe::Expansion {
            definition,
            invocation,
            definition_origin,
            parent,
        } => {
            if definition.is_some_and(|definition| definition.index() >= schema.macros.len()) {
                return invalid("expansion references a missing macro recipe");
            }
            validate_origin_index(schema, *invocation, marks)?;
            validate_origin_index(schema, *definition_origin, marks)?;
            if let Some(parent) = parent {
                validate_origin_index(schema, *parent, marks)?;
            }
        }
    }
    marks[index.index()] = 2;
    Ok(())
}

fn validate_origin_index(
    schema: &ContinuationSchema,
    index: OriginRecipeIndex,
    marks: &mut [u8],
) -> Result<(), CommandContinuationError> {
    if index.index() >= schema.origins.len() {
        return invalid("origin recipe references a missing parent");
    }
    validate_origin(schema, index, marks)
}

fn validate_summary(schema: &ContinuationSchema) -> Result<(), CommandContinuationError> {
    for frame in &schema.summary.input {
        match frame {
            InputFrameRecipe::Source(frame) => validate_source_frame(schema, frame)?,
            InputFrameRecipe::Tokens(frame) => validate_token_frame(schema, frame)?,
        }
    }
    if schema
        .summary
        .pending_sources
        .iter()
        .any(|source| source.index() >= schema.sources.len())
    {
        return invalid("pending input references a missing source recipe");
    }
    for activation in &schema.summary.activations {
        if activation.name.index() >= schema.names.len()
            || activation.definition.index() >= schema.macros.len()
            || activation.arguments.index() >= schema.token_lists.len()
            || activation.invocation.index() >= schema.origins.len()
        {
            return invalid("macro activation references a missing recipe");
        }
        let len = schema.token_lists[activation.arguments.index()].words.len();
        validate_ranges(&activation.ranges, len)?;
    }
    Ok(())
}

fn validate_source_frame(
    schema: &ContinuationSchema,
    frame: &SourceFrameRecipe,
) -> Result<(), CommandContinuationError> {
    let Some(source) = schema.sources.get(frame.source.index()) else {
        return invalid("input frame references a missing source recipe");
    };
    if frame.lexer_state > 2 || frame.name_class > 3 || frame.retirement > 3 {
        return invalid("source frame enum value is invalid");
    }
    let source_len = source.bytes().len();
    if usize::try_from(frame.next_physical_byte).map_or(true, |next| next > source_len) {
        return invalid("source cursor exceeds its source bytes");
    }
    if let Some(line) = &frame.line
        && (line.content_start > line.content_end
            || line.content_end > line.terminator_end
            || usize::try_from(line.terminator_end).map_or(true, |end| end > source_len)
            || line.byte_cursor > line.content_end.saturating_sub(line.content_start)
            || line.scalar_cursor > line.content_end.saturating_sub(line.content_start))
    {
        return invalid("source line cursor or range is invalid");
    }
    if frame
        .every_eof
        .is_some_and(|list| list.index() >= schema.token_lists.len())
    {
        return invalid("every-eof input references a missing token-list recipe");
    }
    Ok(())
}

fn validate_token_frame(
    schema: &ContinuationSchema,
    frame: &TokenFrameRecipe,
) -> Result<(), CommandContinuationError> {
    let len = match &frame.payload {
        InputPayloadRecipe::Stored(list) => schema
            .token_lists
            .get(list.index())
            .ok_or(CommandContinuationError::InvalidRecipe(
                "input frame references a missing token-list recipe",
            ))?
            .words
            .len(),
        InputPayloadRecipe::Inline(words) => {
            for word in words {
                validate_word(schema, word)?;
            }
            words.len()
        }
        InputPayloadRecipe::Arguments { words, ranges } => {
            let len = schema
                .token_lists
                .get(words.index())
                .ok_or(CommandContinuationError::InvalidRecipe(
                    "argument frame references a missing token-list recipe",
                ))?
                .words
                .len();
            validate_ranges(ranges, len)?;
            len
        }
    };
    if usize::try_from(frame.index).map_or(true, |index| index > len) {
        return invalid("token-frame cursor exceeds its payload");
    }
    Ok(())
}

fn validate_attempt(
    schema: &ContinuationSchema,
    attempt: &DetachedAttemptRecipe,
) -> Result<(), CommandContinuationError> {
    if attempt
        .token_lists
        .iter()
        .any(|index| index.index() >= schema.token_lists.len())
    {
        return invalid("attempt references a missing token-list recipe");
    }
    if attempt
        .macros
        .iter()
        .any(|index| index.index() >= schema.macros.len())
    {
        return invalid("attempt references a missing macro recipe");
    }
    if attempt
        .glue
        .iter()
        .any(|index| index.index() >= schema.glue.len())
    {
        return invalid("attempt references a missing glue recipe");
    }
    if attempt
        .provenance
        .iter()
        .any(|index| index.index() >= schema.origins.len())
    {
        return invalid("attempt references a missing origin recipe");
    }
    Ok(())
}

fn validate_ranges(
    ranges: &[Option<RecipeRange>; 9],
    len: usize,
) -> Result<(), CommandContinuationError> {
    if ranges
        .iter()
        .flatten()
        .any(|range| range.end().is_none_or(|end| end > len))
    {
        return invalid("macro argument range exceeds its payload");
    }
    Ok(())
}

fn checked_total(
    current: usize,
    added: usize,
    limit: &'static str,
) -> Result<usize, CommandContinuationError> {
    current
        .checked_add(added)
        .ok_or(CommandContinuationError::LimitExceeded(limit))
}

fn check_limit(
    actual: usize,
    maximum: usize,
    name: &'static str,
) -> Result<(), CommandContinuationError> {
    if actual > maximum {
        Err(CommandContinuationError::LimitExceeded(name))
    } else {
        Ok(())
    }
}

fn invalid<T>(message: &'static str) -> Result<T, CommandContinuationError> {
    Err(CommandContinuationError::InvalidRecipe(message))
}

#[cfg(test)]
#[path = "continuation/tests.rs"]
mod tests;
