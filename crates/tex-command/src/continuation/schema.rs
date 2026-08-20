//! Handle-free command-continuation schema.
//!
//! Every relationship uses a dense index into another table in the same DTO.
//! Runtime identities and storage coordinates are deliberately unrepresentable.

macro_rules! recipe_index {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub(crate) struct $name(u32);

        impl $name {
            #[must_use]
            pub(crate) fn from_len(len: usize) -> Option<Self> {
                u32::try_from(len).ok().map(Self)
            }

            #[must_use]
            pub(crate) const fn index(self) -> usize {
                self.0 as usize
            }
        }
    };
}

recipe_index!(SourceRecipeIndex);
recipe_index!(NameRecipeIndex);
recipe_index!(TokenListRecipeIndex);
recipe_index!(OriginRecipeIndex);
recipe_index!(OriginListRecipeIndex);
recipe_index!(MacroRecipeIndex);
recipe_index!(GlueRecipeIndex);

/// Portable command-profile identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct DetachedCommandProfile {
    pub(crate) schema: u32,
    pub(crate) fingerprint: u64,
    pub(crate) dialect: u8,
    pub(crate) character_mode: u8,
}

/// Semantic origin of immutable input bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DetachedInputOrigin {
    User,
    Distribution,
    Generated,
    Terminal,
}

/// Logical source content. Paths and bytes are values, not session handles.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum SourceRecipe {
    World {
        path: String,
        bytes: Vec<u8>,
        modification_time: Option<i64>,
        origin: DetachedInputOrigin,
    },
    Generated {
        logical_path: Option<String>,
        bytes: Vec<u8>,
    },
}

impl SourceRecipe {
    #[must_use]
    pub(crate) fn bytes(&self) -> &[u8] {
        match self {
            Self::World { bytes, .. } | Self::Generated { bytes, .. } => bytes,
        }
    }
}

/// Portable classification of one control-sequence spelling.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DetachedNameKind {
    Null,
    ActiveCharacter,
    SingleCharacter,
    MultiLetter,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct NameRecipe {
    pub(crate) kind: DetachedNameKind,
    pub(crate) spelling: String,
}

/// Portable frozen-token identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum DetachedFrozenToken {
    Relax,
    EndTemplate,
    EndV,
    Primitive(NameRecipeIndex),
}

/// Portable semantic token.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum DetachedToken {
    Character { scalar: char, catcode: u8 },
    Parameter(u8),
    Frozen(DetachedFrozenToken),
    ControlSequence(NameRecipeIndex),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DetachedWord {
    pub(crate) token: DetachedToken,
    pub(crate) origin: OriginRecipeIndex,
}

/// Logical provenance operation without a generation-local provenance key.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DetachedOriginOperation {
    Unknown,
    Inserted,
    Synthesized,
    MacroExpansion,
    ParameterSubstitution,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum OriginRecipe {
    Unknown,
    SourcePoint {
        source: SourceRecipeIndex,
        byte: u64,
        line: u32,
        column: u32,
    },
    SourceSpan {
        source: SourceRecipeIndex,
        start: u64,
        end: u64,
    },
    Derived {
        operation: DetachedOriginOperation,
        primary: OriginRecipeIndex,
        related: Option<OriginRecipeIndex>,
    },
    Expansion {
        definition: Option<MacroRecipeIndex>,
        invocation: OriginRecipeIndex,
        definition_origin: OriginRecipeIndex,
        parent: Option<OriginRecipeIndex>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TokenListRecipe {
    pub(crate) words: Vec<DetachedWord>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct OriginListRecipe {
    pub(crate) origins: Vec<OriginRecipeIndex>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MacroRecipe {
    pub(crate) flags: u16,
    pub(crate) parameter_text: TokenListRecipeIndex,
    pub(crate) replacement_text: TokenListRecipeIndex,
    pub(crate) definition_origin: OriginRecipeIndex,
    pub(crate) parameter_origins: OriginListRecipeIndex,
    pub(crate) replacement_origins: OriginListRecipeIndex,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct GlueRecipe {
    pub(crate) width: i64,
    pub(crate) stretch: i64,
    pub(crate) stretch_order: u8,
    pub(crate) shrink: i64,
    pub(crate) shrink_order: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RecipeRange {
    pub(crate) start: u32,
    pub(crate) len: u32,
}

impl RecipeRange {
    #[must_use]
    pub(crate) fn end(self) -> Option<usize> {
        usize::try_from(self.start)
            .ok()?
            .checked_add(usize::try_from(self.len).ok()?)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DetachedReplayKind {
    MacroBody,
    MacroArgument,
    BackedUp,
    Inserted,
    Named,
    AlignmentTemplate,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum InputPayloadRecipe {
    Stored(TokenListRecipeIndex),
    Inline(Vec<DetachedWord>),
    Arguments {
        words: TokenListRecipeIndex,
        ranges: [Option<RecipeRange>; 9],
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SourceLineRecipe {
    pub(crate) number: u64,
    pub(crate) content_start: u64,
    pub(crate) content_end: u64,
    pub(crate) terminator_end: u64,
    pub(crate) byte_cursor: u64,
    pub(crate) scalar_cursor: u64,
    pub(crate) endline: Option<u32>,
    pub(crate) endline_delivered: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SourceFrameRecipe {
    pub(crate) source: SourceRecipeIndex,
    pub(crate) next_physical_byte: u64,
    pub(crate) next_line: u64,
    pub(crate) line: Option<SourceLineRecipe>,
    pub(crate) lexer_state: u8,
    pub(crate) end_after_line: bool,
    pub(crate) name_class: u8,
    pub(crate) retirement: u8,
    pub(crate) every_eof: Option<TokenListRecipeIndex>,
    pub(crate) group_depth: u32,
    pub(crate) condition_depth: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TokenFrameRecipe {
    pub(crate) payload: InputPayloadRecipe,
    pub(crate) replay: DetachedReplayKind,
    pub(crate) index: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum InputFrameRecipe {
    Source(SourceFrameRecipe),
    Tokens(TokenFrameRecipe),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ActivationRecipe {
    pub(crate) name: NameRecipeIndex,
    pub(crate) definition: MacroRecipeIndex,
    pub(crate) arguments: TokenListRecipeIndex,
    pub(crate) ranges: [Option<RecipeRange>; 9],
    pub(crate) invocation: OriginRecipeIndex,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ConditionRecipe {
    pub(crate) kind: u8,
    pub(crate) limit: u8,
    pub(crate) source_line: u32,
    pub(crate) inverted: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CommandSummaryRecipe {
    pub(crate) input: Vec<InputFrameRecipe>,
    pub(crate) pending_sources: Vec<SourceRecipeIndex>,
    pub(crate) activations: Vec<ActivationRecipe>,
    pub(crate) conditions: Vec<ConditionRecipe>,
    pub(crate) align_state: i32,
    pub(crate) cumulative_expansions: u64,
}

/// Integer-only resume coordinates copied from an in-process attempt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct DetachedResumePoint {
    pub(crate) command: u32,
    pub(crate) scanner: u32,
    pub(crate) expansion: u32,
    pub(crate) subordinate: u32,
}

/// Logical resource request retained without a host capability or callback.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DetachedResourceRecipe {
    pub(crate) kind: u16,
    pub(crate) key: String,
    pub(crate) payload: Vec<u8>,
}

/// Selected attempt roots which must be rebuilt in the destination attempt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DetachedAttemptRecipe {
    pub(crate) token_lists: Vec<TokenListRecipeIndex>,
    pub(crate) macros: Vec<MacroRecipeIndex>,
    pub(crate) glue: Vec<GlueRecipeIndex>,
    pub(crate) provenance: Vec<OriginRecipeIndex>,
    pub(crate) resume: DetachedResumePoint,
    pub(crate) request: DetachedResourceRecipe,
}

/// Complete handle-free command continuation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ContinuationSchema {
    pub(crate) profile: DetachedCommandProfile,
    pub(crate) summary: CommandSummaryRecipe,
    pub(crate) attempt: Option<DetachedAttemptRecipe>,
    pub(crate) sources: Vec<SourceRecipe>,
    pub(crate) names: Vec<NameRecipe>,
    pub(crate) token_lists: Vec<TokenListRecipe>,
    pub(crate) origins: Vec<OriginRecipe>,
    pub(crate) origin_lists: Vec<OriginListRecipe>,
    pub(crate) macros: Vec<MacroRecipe>,
    pub(crate) glue: Vec<GlueRecipe>,
}
