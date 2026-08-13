use super::*;

// Compile-time schema audit. The allowlist contains only logical scalars and
// recipes. Each struct is exhaustively destructured, so adding a field fails
// compilation until its type is classified. Runtime roots deliberately have
// no implementation of this private trait.
trait HandleFreeSchema {}

macro_rules! schema_leaves {
    ($($ty:ty),+ $(,)?) => { $(impl HandleFreeSchema for $ty {})+ };
}

impl<T: HandleFreeSchema> HandleFreeSchema for Vec<T> {}
impl<T: HandleFreeSchema> HandleFreeSchema for Box<T> {}
impl<T: HandleFreeSchema> HandleFreeSchema for Option<T> {}
impl<T: HandleFreeSchema, const N: usize> HandleFreeSchema for [T; N] {}
impl<K: HandleFreeSchema, V: HandleFreeSchema> HandleFreeSchema for BTreeMap<K, V> {}
impl<A: HandleFreeSchema, B: HandleFreeSchema> HandleFreeSchema for (A, B) {}

schema_leaves!(
    bool,
    char,
    i32,
    u8,
    u32,
    u64,
    usize,
    String,
    std::path::PathBuf,
    SourceId,
    tex_state::FileModificationDate,
    tex_state::InputOrigin,
    ControlSequenceKind,
    RegisteredSourceKind,
    CharacterMode,
    SourceFramingPolicy,
    SyntheticOriginKind,
    SynthesizedOriginKind,
    InsertedOriginKind,
    tex_state::meaning::MeaningFlags,
    tex_state::token::Catcode,
    tex_state::token::FrozenToken,
    crate::CharacterCode,
    crate::LineTerminator,
    crate::LexerState,
    InputLevelId,
    MacroActivationId,
    MacroArgumentRange,
    TokenBehavior,
    RetirementBehavior,
    ReplayTrace,
    SourceNameClass,
    SourceRetirement,
    ConditionId,
    ConditionalKind,
    IfLimit,
    CommandProfile,
    SourceRecipeId,
    SymbolRecipeId,
    TokenListRecipeId,
    OriginRecipeId,
    OriginListRecipeId,
    MacroRecipeId,
);

macro_rules! schema_struct {
    ($ty:ident { $($field:ident : $field_ty:ty),+ $(,)? }) => {
        impl HandleFreeSchema for $ty where $($field_ty: HandleFreeSchema),+ {}
        const _: fn(&$ty) = |value| {
            let $ty { $($field,)+ } = value;
            $(let _: &$field_ty = $field;)+
        };
    };
}

schema_struct!(OwnedSourceRecipe {
    id: SourceId,
    descriptor: OwnedSourceDescriptor,
});
schema_struct!(OwnedRegisteredSource {
    source: SourceRecipeId,
    kind: RegisteredSourceKind,
    mode: CharacterMode,
    bytes: Vec<u8>,
    name: Option<String>,
    framing_name: Option<String>,
    framing: SourceFramingPolicy,
});
schema_struct!(OwnedSymbol {
    kind: ControlSequenceKind,
    spelling: String,
});
schema_struct!(OwnedWord {
    token: OwnedToken,
    origin: OriginRecipeId,
});
schema_struct!(OwnedSourceProvenance {
    source: SourceRecipeId,
    start: u64,
    end: u64,
    location: u64,
});
schema_struct!(OwnedBackedUpToken {
    spelling: OwnedWord,
    source_provenance: Option<OwnedSourceProvenance>,
});
schema_struct!(OwnedMacro {
    flags: tex_state::meaning::MeaningFlags,
    parameters: TokenListRecipeId,
    replacement: TokenListRecipeId,
    definition_origin: OriginRecipeId,
    parameter_origins: OriginListRecipeId,
    replacement_origins: OriginListRecipeId,
});
schema_struct!(OwnedTokenCursor {
    payload: OwnedTokenPayload,
    behavior: TokenBehavior,
    retirement: RetirementBehavior,
    trace: ReplayTrace,
    index: usize,
    identity: InputLevelId,
});
schema_struct!(OwnedSourceCursor {
    backing: OwnedRegisteredSource,
    line_backing: Option<OwnedRegisteredSource>,
    pending_acquired_line: bool,
    next_physical_offset: u64,
    next_line_number: u64,
    line: Option<OwnedSourceLineState>,
    lexer_state: crate::LexerState,
    end_after_line: bool,
});
schema_struct!(OwnedSourceRange {
    source: SourceRecipeId,
    start: u64,
    end: u64,
});
schema_struct!(OwnedSourceLineState {
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
});
schema_struct!(OwnedSourceLevel {
    identity: InputLevelId,
    cursor: OwnedSourceCursor,
    name_class: SourceNameClass,
    retirement: SourceRetirement,
    every_eof: Option<(TokenListRecipeId, OriginListRecipeId)>,
    open_depths: Option<OwnedSourceOpenDepths>,
});
schema_struct!(OwnedSourceOpenDepths {
    group_lineages: Vec<u64>,
    conditional_identities: Vec<u64>,
});
schema_struct!(OwnedInputState {
    levels: Vec<OwnedInputLevel>,
    terminal_context_line: Option<String>,
    pending_sources: BTreeMap<u32, OwnedRegisteredSource>,
    next_level_identity: u64,
    next_source_identity: u64,
    force_eof: bool,
});
schema_struct!(OwnedActivation {
    identity: MacroActivationId,
    name: SymbolRecipeId,
    definition: MacroRecipeId,
    arguments: Vec<OwnedWord>,
    ranges: [Option<MacroArgumentRange>; 9],
    invocation: OriginRecipeId,
});
schema_struct!(OwnedParameterState {
    activations: Vec<OwnedActivation>,
    next_activation_identity: u64,
});
schema_struct!(OwnedConditionState {
    frames: Vec<OwnedConditionFrame>,
    next_identity: u64,
});
schema_struct!(OwnedConditionFrame {
    identity: ConditionId,
    kind: ConditionalKind,
    limit: IfLimit,
    source_line: u32,
    inverted: bool,
});
schema_struct!(OwnedExpansionState {
    cumulative_expansions: u64,
    next_resource_resolution: u64,
    pending_diagnostics: Vec<u64>,
    observed_dependencies: Vec<u64>,
    semantic_barriers: Vec<u64>,
    profile: CommandProfile,
});
schema_struct!(OwnedCommandSummary {
    input: OwnedInputState,
    parameters: OwnedParameterState,
    conditions: OwnedConditionState,
    align_state: i32,
    expansion: OwnedExpansionState,
    next_builder_identity: u64,
});
schema_struct!(OwnedCommandContinuation {
    summary: OwnedCommandSummary,
    sources: Vec<OwnedSourceRecipe>,
    symbols: Vec<OwnedSymbol>,
    token_lists: Vec<Vec<OwnedToken>>,
    origins: Vec<OwnedOrigin>,
    origin_lists: Vec<Vec<OriginRecipeId>>,
    macros: Vec<OwnedMacro>,
});

impl HandleFreeSchema for OwnedSourceDescriptor {}
const _: fn(&OwnedSourceDescriptor) = |value| match value {
    OwnedSourceDescriptor::World {
        path,
        bytes,
        modification_date,
        origin,
    } => {
        let _: &std::path::PathBuf = path;
        let _: &Vec<u8> = bytes;
        let _: &Option<tex_state::FileModificationDate> = modification_date;
        let _: &tex_state::InputOrigin = origin;
    }
    OwnedSourceDescriptor::Generated {
        logical_path,
        bytes,
    } => {
        let _: &Option<String> = logical_path;
        let _: &Vec<u8> = bytes;
    }
};

impl HandleFreeSchema for OwnedToken {}
const _: fn(&OwnedToken) = |value| match value {
    OwnedToken::Character { ch, cat } => {
        let _: &char = ch;
        let _: &tex_state::token::Catcode = cat;
    }
    OwnedToken::Parameter(slot) => {
        let _: &u8 = slot;
    }
    OwnedToken::Frozen(token) => {
        let _: &tex_state::token::FrozenToken = token;
    }
    OwnedToken::ControlSequence(symbol) => {
        let _: &SymbolRecipeId = symbol;
    }
};

impl HandleFreeSchema for OwnedOrigin {}
const _: fn(&OwnedOrigin) = |value| match value {
    OwnedOrigin::Unknown => {}
    OwnedOrigin::Source {
        source,
        input_record,
        byte_offset,
        line,
        column,
    } => {
        let _: &SourceRecipeId = source;
        let _: &Option<tex_state::InputRecordId> = input_record;
        let _: &u64 = byte_offset;
        let _: &u32 = line;
        let _: &u32 = column;
    }
    OwnedOrigin::SourceSpan { source, start, end } => {
        let _: &SourceRecipeId = source;
        let _: &u64 = start;
        let _: &u64 = end;
    }
    OwnedOrigin::Synthetic(kind) => {
        let _: &SyntheticOriginKind = kind;
    }
    OwnedOrigin::Synthesized { kind, parent } => {
        let _: &SynthesizedOriginKind = kind;
        let _: &OriginRecipeId = parent;
    }
    OwnedOrigin::Inserted {
        kind,
        token,
        parent,
    } => {
        let _: &InsertedOriginKind = kind;
        let _: &OwnedToken = token;
        let _: &OriginRecipeId = parent;
    }
    OwnedOrigin::ExpansionFrame {
        definition,
        detached_operand,
        invocation,
        definition_origin,
        parent,
    } => {
        let _: &Option<MacroRecipeId> = definition;
        let _: &u64 = detached_operand;
        let _: &OriginRecipeId = invocation;
        let _: &OriginRecipeId = definition_origin;
        let _: &OriginRecipeId = parent;
    }
};

impl HandleFreeSchema for OwnedTokenPayload {}
const _: fn(&OwnedTokenPayload) = |value| match value {
    OwnedTokenPayload::Stored { tokens, origins } => {
        let _: &TokenListRecipeId = tokens;
        let _: &OriginListRecipeId = origins;
    }
    OwnedTokenPayload::Transient(words) => {
        let _: &Vec<OwnedWord> = words;
    }
    OwnedTokenPayload::InlineTransient(word) => {
        let _: &OwnedWord = word;
    }
    OwnedTokenPayload::BackedUp(words) => {
        let _: &Vec<OwnedBackedUpToken> = words;
    }
    OwnedTokenPayload::InlineBackedUp(word) => {
        let _: &OwnedBackedUpToken = word;
    }
    OwnedTokenPayload::ArgumentRange { buffer, range } => {
        let _: &Vec<OwnedWord> = buffer;
        let _: &MacroArgumentRange = range;
    }
};

impl HandleFreeSchema for OwnedInputLevel {}
#[allow(clippy::borrowed_box)] // Exact field typing is the compile-time schema guard.
const _: fn(&OwnedInputLevel) = |value| match value {
    OwnedInputLevel::Source(source) => {
        let _: &Box<OwnedSourceLevel> = source;
    }
    OwnedInputLevel::Tokens(tokens) => {
        let _: &OwnedTokenCursor = tokens;
    }
};

const _: fn() = || {
    fn assert_schema<T: HandleFreeSchema>() {}
    assert_schema::<OwnedCommandContinuation>();
};
