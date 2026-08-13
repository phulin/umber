//! Handle-free transport for retained command continuations.

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;

use tex_state::Universe;
use tex_state::ids::{MacroDefinitionId, OriginListId, TokenListId};
use tex_state::input::SourceId;
use tex_state::interner::{ControlSequenceKind, Symbol};
use tex_state::macro_store::{MacroDefinitionRef, MacroMeaning};
use tex_state::provenance::{
    ExpansionFrameRef, InsertedOriginKind, OriginListRef, OriginRecord, OriginRef,
    SynthesizedOriginKind, SyntheticOriginKind,
};
use tex_state::source_map::{SourceDescriptor, SourceMapError};
use tex_state::token::{OriginId, Token, TracedTokenWord};
use tex_state::token_store::TokenListRef;

use crate::conditionals::{ConditionFrame, ConditionStack, ConditionalKind, IfLimit};
use crate::input::{
    BackedUpToken, InputLevel, InputLevelId, InputState, RegisteredSource, RegisteredSourceKind,
    ReplayTrace, RetirementBehavior, SharedBackedUpBuffer, SharedTokenBuffer, SourceFramingPolicy,
    SourceLevel, SourceNameClass, SourceOpenDepths, SourceProvenance, SourceRetirement,
    TokenBehavior, TokenCursor, TokenPayload,
};
use crate::macro_call::{
    MacroActivation, MacroActivationId, MacroArgumentRange, MacroArguments, ParameterState,
};
use crate::processor::{ConditionId, ExpansionState};
use crate::profile::{CharacterMode, CommandProfile};
use crate::snapshot::CommandSummary;

macro_rules! recipe_id {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
        struct $name(usize);
    };
}

recipe_id!(SourceRecipeId);
recipe_id!(SymbolRecipeId);
recipe_id!(TokenListRecipeId);
recipe_id!(OriginRecipeId);
recipe_id!(OriginListRecipeId);
recipe_id!(MacroRecipeId);

#[derive(Clone, Debug, Eq, PartialEq)]
enum OwnedSourceDescriptor {
    World {
        path: PathBuf,
        bytes: Vec<u8>,
        modification_date: Option<tex_state::FileModificationDate>,
        origin: tex_state::InputOrigin,
    },
    Generated {
        logical_path: Option<String>,
        bytes: Vec<u8>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct OwnedSourceRecipe {
    id: SourceId,
    descriptor: OwnedSourceDescriptor,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct OwnedRegisteredSource {
    source: SourceRecipeId,
    kind: RegisteredSourceKind,
    mode: CharacterMode,
    bytes: Vec<u8>,
    name: Option<String>,
    framing_name: Option<String>,
    framing: SourceFramingPolicy,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct OwnedSymbol {
    kind: ControlSequenceKind,
    spelling: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum OwnedToken {
    Character {
        ch: char,
        cat: tex_state::token::Catcode,
    },
    Parameter(u8),
    Frozen(tex_state::token::FrozenToken),
    ControlSequence(SymbolRecipeId),
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct OwnedWord {
    token: OwnedToken,
    origin: OriginRecipeId,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct OwnedSourceProvenance {
    source: SourceRecipeId,
    start: u64,
    end: u64,
    location: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct OwnedBackedUpToken {
    spelling: OwnedWord,
    source_provenance: Option<OwnedSourceProvenance>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum OwnedOrigin {
    Unknown,
    Source {
        source: SourceRecipeId,
        input_record: Option<tex_state::InputRecordId>,
        byte_offset: u64,
        line: u32,
        column: u32,
    },
    SourceSpan {
        source: SourceRecipeId,
        start: u64,
        end: u64,
    },
    Synthetic(SyntheticOriginKind),
    Synthesized {
        kind: SynthesizedOriginKind,
        parent: OriginRecipeId,
    },
    Inserted {
        kind: InsertedOriginKind,
        token: OwnedToken,
        parent: OriginRecipeId,
    },
    ExpansionFrame {
        definition: Option<MacroRecipeId>,
        detached_operand: u64,
        invocation: OriginRecipeId,
        definition_origin: OriginRecipeId,
        parent: OriginRecipeId,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct OwnedMacro {
    flags: tex_state::meaning::MeaningFlags,
    parameters: TokenListRecipeId,
    replacement: TokenListRecipeId,
    definition_origin: OriginRecipeId,
    parameter_origins: OriginListRecipeId,
    replacement_origins: OriginListRecipeId,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum OwnedTokenPayload {
    Stored {
        tokens: TokenListRecipeId,
        origins: OriginListRecipeId,
    },
    Transient(Vec<OwnedWord>),
    InlineTransient(OwnedWord),
    BackedUp(Vec<OwnedBackedUpToken>),
    InlineBackedUp(OwnedBackedUpToken),
    ArgumentRange {
        buffer: Vec<OwnedWord>,
        range: MacroArgumentRange,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct OwnedTokenCursor {
    payload: OwnedTokenPayload,
    behavior: TokenBehavior,
    retirement: RetirementBehavior,
    trace: ReplayTrace,
    index: usize,
    identity: InputLevelId,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct OwnedSourceCursor {
    backing: OwnedRegisteredSource,
    line_backing: Option<OwnedRegisteredSource>,
    pending_acquired_line: bool,
    next_physical_offset: u64,
    next_line_number: u64,
    line: Option<OwnedSourceLineState>,
    lexer_state: crate::LexerState,
    end_after_line: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct OwnedSourceRange {
    source: SourceRecipeId,
    start: u64,
    end: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct OwnedSourceLineState {
    number: u64,
    content: OwnedSourceRange,
    terminator: OwnedSourceRange,
    terminator_kind: crate::LineTerminator,
    retained_end: u64,
    byte_cursor: u64,
    scalar_cursor: u64,
    endline: Option<crate::CharacterCode>,
    endline_delivered: bool,
    reduced_spellings: Vec<(OwnedSourceRange, crate::CharacterCode)>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct OwnedSourceLevel {
    identity: InputLevelId,
    cursor: OwnedSourceCursor,
    name_class: SourceNameClass,
    retirement: SourceRetirement,
    every_eof: Option<(TokenListRecipeId, OriginListRecipeId)>,
    open_depths: Option<OwnedSourceOpenDepths>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct OwnedSourceOpenDepths {
    group_lineages: Vec<u64>,
    conditional_identities: Vec<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum OwnedInputLevel {
    Source(Box<OwnedSourceLevel>),
    Tokens(OwnedTokenCursor),
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct OwnedInputState {
    levels: Vec<OwnedInputLevel>,
    terminal_context_line: Option<String>,
    pending_sources: BTreeMap<u32, OwnedRegisteredSource>,
    next_level_identity: u64,
    next_source_identity: u64,
    force_eof: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct OwnedActivation {
    identity: MacroActivationId,
    name: SymbolRecipeId,
    definition: MacroRecipeId,
    arguments: Vec<OwnedWord>,
    ranges: [Option<MacroArgumentRange>; 9],
    invocation: OriginRecipeId,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct OwnedParameterState {
    activations: Vec<OwnedActivation>,
    next_activation_identity: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct OwnedConditionState {
    frames: Vec<OwnedConditionFrame>,
    next_identity: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct OwnedConditionFrame {
    identity: ConditionId,
    kind: ConditionalKind,
    limit: IfLimit,
    source_line: u32,
    inverted: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct OwnedExpansionState {
    cumulative_expansions: u64,
    next_resource_resolution: u64,
    pending_diagnostics: Vec<u64>,
    observed_dependencies: Vec<u64>,
    semantic_barriers: Vec<u64>,
    profile: CommandProfile,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct OwnedCommandSummary {
    input: OwnedInputState,
    parameters: OwnedParameterState,
    conditions: OwnedConditionState,
    align_state: i32,
    expansion: OwnedExpansionState,
    next_builder_identity: u64,
}

/// A canonical command continuation containing recipes and portable state.
///
/// Runtime roots and store coordinates exist only in the detacher's temporary
/// maps. The retained value uses dense DTO-local recipe indices.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OwnedCommandContinuation {
    summary: OwnedCommandSummary,
    sources: Vec<OwnedSourceRecipe>,
    symbols: Vec<OwnedSymbol>,
    token_lists: Vec<Vec<OwnedToken>>,
    origins: Vec<OwnedOrigin>,
    origin_lists: Vec<Vec<OriginRecipeId>>,
    macros: Vec<OwnedMacro>,
}

/// A detached continuation could not be validated or installed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandContinuationError {
    InvalidRecipe(&'static str),
    DestinationBusy,
    SourceMap(SourceMapError),
    SourceRegistration(crate::SourceRegistrationError),
}

impl fmt::Display for CommandContinuationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRecipe(message) => {
                write!(formatter, "invalid command continuation: {message}")
            }
            Self::DestinationBusy => formatter
                .write_str("cannot materialize a command continuation during a private revision"),
            Self::SourceMap(error) => {
                write!(formatter, "could not install continuation source: {error}")
            }
            Self::SourceRegistration(error) => {
                write!(formatter, "could not rebuild continuation source: {error}")
            }
        }
    }
}

impl std::error::Error for CommandContinuationError {}

impl OwnedCommandContinuation {
    #[must_use]
    pub fn detach(summary: &CommandSummary, universe: &Universe) -> Self {
        Detacher::new(universe).finish(summary)
    }

    /// Validates every recipe before installing any destination root.
    pub fn materialize(
        &self,
        universe: &mut Universe,
    ) -> Result<CommandSummary, CommandContinuationError> {
        self.validate()?;
        let mut staged = universe
            .stage_detached_import()
            .ok_or(CommandContinuationError::DestinationBusy)?;
        let summary = Materializer::new(self, &mut staged).finish()?;
        *universe = staged;
        Ok(summary)
    }

    fn validate(&self) -> Result<(), CommandContinuationError> {
        let invalid = |message| Err(CommandContinuationError::InvalidRecipe(message));
        for source in &self.sources {
            match &source.descriptor {
                OwnedSourceDescriptor::World { bytes, .. } => {
                    if u64::try_from(bytes.len()).is_err() {
                        return invalid("World source is too large");
                    }
                }
                OwnedSourceDescriptor::Generated { bytes, .. } => {
                    if u64::try_from(bytes.len()).is_err() {
                        return invalid("generated source is too large");
                    }
                }
            }
        }
        for symbol in &self.symbols {
            if symbol.kind == ControlSequenceKind::ActiveCharacter
                && symbol.spelling.chars().count() != 1
            {
                return invalid("active control-sequence recipe is not one scalar");
            }
        }
        for tokens in &self.token_lists {
            for token in tokens {
                self.validate_token(token)?;
            }
        }
        let mut marks = vec![0_u8; self.origins.len()];
        for index in 0..self.origins.len() {
            self.validate_origin(OriginRecipeId(index), &mut marks)?;
        }
        for list in &self.origin_lists {
            if list.iter().any(|id| id.0 >= self.origins.len()) {
                return invalid("origin-list recipe references a missing origin");
            }
        }
        for mac in &self.macros {
            if mac.parameters.0 >= self.token_lists.len()
                || mac.replacement.0 >= self.token_lists.len()
                || mac.definition_origin.0 >= self.origins.len()
                || mac.parameter_origins.0 >= self.origin_lists.len()
                || mac.replacement_origins.0 >= self.origin_lists.len()
            {
                return invalid("macro recipe references missing content");
            }
        }
        for level in &self.summary.input.levels {
            self.validate_level(level)?;
        }
        for source in self.summary.input.pending_sources.values() {
            self.validate_registered_source(source)?;
        }
        let activation_ids = self
            .summary
            .parameters
            .activations
            .iter()
            .map(|activation| activation.identity)
            .collect::<Vec<_>>();
        for activation in &self.summary.parameters.activations {
            if activation.name.0 >= self.symbols.len()
                || activation.definition.0 >= self.macros.len()
                || activation.invocation.0 >= self.origins.len()
            {
                return invalid("macro activation references a missing recipe");
            }
            for word in &activation.arguments {
                self.validate_word(word)?;
            }
            for range in activation.ranges.iter().flatten() {
                if range.end() > activation.arguments.len() {
                    return invalid("macro argument range exceeds its buffer");
                }
            }
        }
        for level in &self.summary.input.levels {
            if let OwnedInputLevel::Tokens(cursor) = level
                && let TokenBehavior::MacroBody(identity) = cursor.behavior
                && !activation_ids.contains(&identity)
            {
                return invalid("macro-body input references a missing activation");
            }
        }
        Ok(())
    }

    fn validate_token(&self, token: &OwnedToken) -> Result<(), CommandContinuationError> {
        if matches!(token, OwnedToken::ControlSequence(id) if id.0 >= self.symbols.len()) {
            return Err(CommandContinuationError::InvalidRecipe(
                "token references a missing symbol",
            ));
        }
        Ok(())
    }

    fn validate_word(&self, word: &OwnedWord) -> Result<(), CommandContinuationError> {
        self.validate_token(&word.token)?;
        if word.origin.0 >= self.origins.len() {
            return Err(CommandContinuationError::InvalidRecipe(
                "word references a missing origin",
            ));
        }
        Ok(())
    }

    fn validate_origin(
        &self,
        id: OriginRecipeId,
        marks: &mut [u8],
    ) -> Result<(), CommandContinuationError> {
        if id.0 >= self.origins.len() {
            return Err(CommandContinuationError::InvalidRecipe(
                "missing origin recipe",
            ));
        }
        if marks[id.0] == 2 {
            return Ok(());
        }
        if marks[id.0] == 1 {
            return Err(CommandContinuationError::InvalidRecipe(
                "cyclic origin recipe",
            ));
        }
        marks[id.0] = 1;
        let mut child = |child| self.validate_origin(child, marks);
        match &self.origins[id.0] {
            OwnedOrigin::Unknown | OwnedOrigin::Synthetic(_) => {}
            OwnedOrigin::Source { source, .. } | OwnedOrigin::SourceSpan { source, .. } => {
                if source.0 >= self.sources.len() {
                    return Err(CommandContinuationError::InvalidRecipe(
                        "origin references a missing source",
                    ));
                }
                let limit = self.source_len(*source);
                match &self.origins[id.0] {
                    OwnedOrigin::Source { byte_offset, .. } if *byte_offset > limit => {
                        return Err(CommandContinuationError::InvalidRecipe(
                            "source origin exceeds its backing",
                        ));
                    }
                    OwnedOrigin::SourceSpan { start, end, .. } if start > end || *end > limit => {
                        return Err(CommandContinuationError::InvalidRecipe(
                            "source-span origin exceeds its backing",
                        ));
                    }
                    _ => {}
                }
            }
            OwnedOrigin::Synthesized { parent, .. } => child(*parent)?,
            OwnedOrigin::Inserted { token, parent, .. } => {
                self.validate_token(token)?;
                child(*parent)?;
            }
            OwnedOrigin::ExpansionFrame {
                definition,
                invocation,
                definition_origin,
                parent,
                ..
            } => {
                if definition.is_some_and(|definition| definition.0 >= self.macros.len()) {
                    return Err(CommandContinuationError::InvalidRecipe(
                        "expansion frame references a missing macro",
                    ));
                }
                child(*invocation)?;
                child(*definition_origin)?;
                child(*parent)?;
            }
        }
        marks[id.0] = 2;
        Ok(())
    }

    fn validate_registered_source(
        &self,
        source: &OwnedRegisteredSource,
    ) -> Result<(), CommandContinuationError> {
        let Some(recipe) = self.sources.get(source.source.0) else {
            return Err(CommandContinuationError::InvalidRecipe(
                "input backing references a missing source",
            ));
        };
        let descriptor_len = match &recipe.descriptor {
            OwnedSourceDescriptor::World { bytes, .. }
            | OwnedSourceDescriptor::Generated { bytes, .. } => bytes.len() as u64,
        };
        if descriptor_len != source.bytes.len() as u64
            || source.mode != self.summary.expansion.profile.character_mode()
        {
            return Err(CommandContinuationError::InvalidRecipe(
                "input backing disagrees with its source recipe",
            ));
        }
        Ok(())
    }

    fn source_len(&self, source: SourceRecipeId) -> u64 {
        match &self.sources[source.0].descriptor {
            OwnedSourceDescriptor::World { bytes, .. }
            | OwnedSourceDescriptor::Generated { bytes, .. } => bytes.len() as u64,
        }
    }

    fn validate_source_range(
        &self,
        range: &OwnedSourceRange,
    ) -> Result<(), CommandContinuationError> {
        if range.source.0 >= self.sources.len()
            || range.start > range.end
            || range.end > self.source_len(range.source)
        {
            return Err(CommandContinuationError::InvalidRecipe(
                "source cursor range is invalid",
            ));
        }
        Ok(())
    }

    fn validate_level(&self, level: &OwnedInputLevel) -> Result<(), CommandContinuationError> {
        match level {
            OwnedInputLevel::Source(source) => {
                self.validate_registered_source(&source.cursor.backing)?;
                if let Some(line) = &source.cursor.line_backing {
                    self.validate_registered_source(line)?;
                }
                if let Some((tokens, origins)) = source.every_eof
                    && (tokens.0 >= self.token_lists.len() || origins.0 >= self.origin_lists.len())
                {
                    return Err(CommandContinuationError::InvalidRecipe(
                        "every-eof references missing content",
                    ));
                }
                if let Some(line) = &source.cursor.line {
                    self.validate_source_range(&line.content)?;
                    self.validate_source_range(&line.terminator)?;
                    for (range, _) in &line.reduced_spellings {
                        self.validate_source_range(range)?;
                    }
                    if line.byte_cursor > self.source_len(line.content.source)
                        || line.retained_end > self.source_len(line.content.source)
                    {
                        return Err(CommandContinuationError::InvalidRecipe(
                            "source line cursor exceeds its backing",
                        ));
                    }
                }
            }
            OwnedInputLevel::Tokens(cursor) => {
                let len = match &cursor.payload {
                    OwnedTokenPayload::Stored { tokens, origins } => {
                        if tokens.0 >= self.token_lists.len()
                            || origins.0 >= self.origin_lists.len()
                        {
                            return Err(CommandContinuationError::InvalidRecipe(
                                "stored input references missing content",
                            ));
                        }
                        self.token_lists[tokens.0].len()
                    }
                    OwnedTokenPayload::Transient(words) => {
                        for word in words {
                            self.validate_word(word)?;
                        }
                        words.len()
                    }
                    OwnedTokenPayload::InlineTransient(word) => {
                        self.validate_word(word)?;
                        1
                    }
                    OwnedTokenPayload::BackedUp(words) => {
                        for word in words {
                            self.validate_word(&word.spelling)?;
                            if let Some(source) = &word.source_provenance
                                && (source.source.0 >= self.sources.len()
                                    || source.start > source.end
                                    || source.end > self.source_len(source.source)
                                    || source.location > self.source_len(source.source))
                            {
                                return Err(CommandContinuationError::InvalidRecipe(
                                    "backup provenance references a missing source",
                                ));
                            }
                        }
                        words.len()
                    }
                    OwnedTokenPayload::InlineBackedUp(word) => {
                        self.validate_word(&word.spelling)?;
                        1
                    }
                    OwnedTokenPayload::ArgumentRange { buffer, range } => {
                        for word in buffer {
                            self.validate_word(word)?;
                        }
                        if range.end() > buffer.len() {
                            return Err(CommandContinuationError::InvalidRecipe(
                                "input argument range exceeds its buffer",
                            ));
                        }
                        range.end().saturating_sub(range.start())
                    }
                };
                if cursor.index > len {
                    return Err(CommandContinuationError::InvalidRecipe(
                        "input cursor exceeds its payload",
                    ));
                }
            }
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn corrupt_first_token_recipe_for_test(&mut self) {
        if let Some(tokens) = self.token_lists.first_mut() {
            tokens.push(OwnedToken::ControlSequence(SymbolRecipeId(usize::MAX)));
        }
    }
}

mod detach;
mod materialize;
mod schema;

use detach::Detacher;
use materialize::Materializer;
