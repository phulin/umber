//! Macro definition token scanning.
//!
//! This module implements the reusable `scan_toks`-style part of `\def` and
//! `\edef`: scan the parameter text, then scan the brace-balanced replacement
//! text. It freezes the resulting token lists through `Universe`, but it does
//! not assign the macro meaning to `Env`.

use std::{fmt, marker::PhantomData};

use tex_lex::{InputSource, InputStack, LexError, MemoryInput, TokenListReplayKind};
use tex_state::ids::{OriginListId, TokenListId};
use tex_state::macro_store::{MacroDefinitionProvenance, MacroMeaning};
use tex_state::meaning::{ExpandablePrimitive, Meaning, MeaningFlags};
use tex_state::provenance::{InsertedOriginKind, OriginListBuilder};
use tex_state::token::{Catcode, Token, TracedTokenWord};
use tex_state::token_store::TokenListBuilder;
use tex_state::{ExpansionState, InputReadState, TracedTokenList};

use crate::{
    DriverExpandNext, ExpandError, ExpandNext, ExpandableOpcode, ExpansionHooks, NoInputExpandNext,
    NoopRecorder, ReadRecorder,
};

/// Result of scanning a macro definition without assigning it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScannedMacro {
    meaning: MacroMeaning,
    provenance: MacroDefinitionProvenance,
}

impl ScannedMacro {
    #[must_use]
    pub const fn meaning(self) -> MacroMeaning {
        self.meaning
    }

    #[must_use]
    pub const fn provenance(self) -> MacroDefinitionProvenance {
        self.provenance
    }

    #[must_use]
    pub const fn with_definition_origin(
        self,
        definition_origin: tex_state::token::OriginId,
    ) -> Self {
        Self {
            provenance: MacroDefinitionProvenance::new(
                definition_origin,
                self.provenance.parameter_origins(),
                self.provenance.replacement_origins(),
            ),
            ..self
        }
    }

    #[must_use]
    pub const fn parameter_text(self) -> TokenListId {
        self.meaning.parameter_text()
    }

    #[must_use]
    pub const fn replacement_text(self) -> TokenListId {
        self.meaning.replacement_text()
    }
}

/// Errors raised while scanning a macro definition.
#[derive(Debug)]
pub enum ScanToksError {
    Lex(LexError),
    Expand(ExpandError),
    EndOfInputInParameterText {
        context: TracedTokenWord,
    },
    EndOfInputInReplacementText {
        context: TracedTokenWord,
    },
    ParameterNumberOutOfOrder {
        expected: u8,
        found: u8,
        context: TracedTokenWord,
    },
    TooManyParameters {
        context: TracedTokenWord,
    },
    InvalidParameterTokenInParameterText {
        context: TracedTokenWord,
    },
    InvalidParameterTokenInReplacementText {
        context: TracedTokenWord,
    },
    MissingGeneralTextBeginGroup {
        context: TracedTokenWord,
    },
}

impl fmt::Display for ScanToksError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Lex(err) => write!(f, "{err}"),
            Self::Expand(err) => write!(f, "{err}"),
            Self::EndOfInputInParameterText { .. } => {
                write!(f, "end of input while scanning macro parameter text")
            }
            Self::EndOfInputInReplacementText { .. } => {
                write!(f, "end of input while scanning macro replacement text")
            }
            Self::ParameterNumberOutOfOrder {
                expected, found, ..
            } => write!(
                f,
                "macro parameter number out of order: expected #{expected}, found #{found}"
            ),
            Self::TooManyParameters { .. } => {
                write!(f, "macro definitions support only #1 through #9")
            }
            Self::InvalidParameterTokenInParameterText { context } => {
                write!(
                    f,
                    "invalid parameter token {:?} in macro parameter text",
                    traced_semantic_token(*context)
                )
            }
            Self::InvalidParameterTokenInReplacementText { context } => {
                write!(
                    f,
                    "invalid parameter token {:?} in macro replacement text",
                    traced_semantic_token(*context)
                )
            }
            Self::MissingGeneralTextBeginGroup { context } => {
                write!(
                    f,
                    "expected begin-group token before general text, got {:?}",
                    traced_semantic_token(*context)
                )
            }
        }
    }
}

impl std::error::Error for ScanToksError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Lex(err) => Some(err),
            Self::Expand(err) => Some(err),
            _ => None,
        }
    }
}

impl From<LexError> for ScanToksError {
    fn from(value: LexError) -> Self {
        Self::Lex(value)
    }
}

impl From<ExpandError> for ScanToksError {
    fn from(value: ExpandError) -> Self {
        Self::Expand(value)
    }
}

impl ScanToksError {
    #[must_use]
    pub fn primary_origin(&self) -> Option<tex_state::token::OriginId> {
        match self {
            Self::Lex(_) => None,
            Self::Expand(error) => error.primary_origin(),
            Self::EndOfInputInParameterText { context }
            | Self::EndOfInputInReplacementText { context }
            | Self::ParameterNumberOutOfOrder { context, .. }
            | Self::TooManyParameters { context }
            | Self::InvalidParameterTokenInParameterText { context }
            | Self::InvalidParameterTokenInReplacementText { context }
            | Self::MissingGeneralTextBeginGroup { context } => Some(context.origin()),
        }
    }
}

/// Scans a macro definition from the current input position.
///
/// The control sequence being defined is already consumed by the caller. This
/// scans tokens up to the opening replacement brace as parameter text, then
/// captures a balanced replacement body. Frozen token-list ids are returned in
/// a `MacroMeaning`; callers decide whether, where, and how to assign it.
pub fn scan_toks<S>(
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
    flags: MeaningFlags,
    context: TracedTokenWord,
) -> Result<ScannedMacro, ScanToksError>
where
    S: InputSource,
{
    let parameter_text = scan_parameter_text(input, stores, context)?;
    let replacement_text = scan_replacement_text(input, stores, context)?;
    Ok(ScannedMacro {
        meaning: MacroMeaning::new(
            flags,
            parameter_text.token_list(),
            replacement_text.token_list(),
        ),
        provenance: MacroDefinitionProvenance::new(
            tex_state::token::OriginId::UNKNOWN,
            parameter_text.origin_list(),
            replacement_text.origin_list(),
        ),
    })
}

/// Scans a macro definition and expands the replacement text as for `\edef`.
pub fn scan_toks_expanded<S, H>(
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
    flags: MeaningFlags,
    context: TracedTokenWord,
    hooks: &mut H,
) -> Result<ScannedMacro, ScanToksError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let scanned = scan_toks(input, stores, flags, context)?;
    let meaning = scanned.meaning();
    let replacement_text = expand_replacement_text(
        stores,
        meaning.replacement_text(),
        scanned.provenance().replacement_origins(),
        hooks,
        &mut NoInputExpandNext,
    )?;
    Ok(ScannedMacro {
        meaning: MacroMeaning::new(
            flags,
            meaning.parameter_text(),
            replacement_text.token_list(),
        ),
        provenance: MacroDefinitionProvenance::new(
            scanned.provenance().definition_origin(),
            scanned.provenance().parameter_origins(),
            replacement_text.origin_list(),
        ),
    })
}

pub fn scan_toks_expanded_with_driver<S, St, H>(
    input: &mut InputStack<S>,
    stores: &mut St,
    flags: MeaningFlags,
    context: TracedTokenWord,
    hooks: &mut H,
) -> Result<ScannedMacro, ScanToksError>
where
    S: InputSource,
    St: ExpansionState + tex_state::InputOpenState,
    H: ExpansionHooks<S>,
{
    let parameter_text = scan_parameter_text(input, stores, context)?;
    let replacement_text = scan_expanded_replacement_with_driver(input, stores, context, hooks)?;
    Ok(ScannedMacro {
        meaning: MacroMeaning::new(
            flags,
            parameter_text.token_list(),
            replacement_text.token_list(),
        ),
        provenance: MacroDefinitionProvenance::new(
            tex_state::token::OriginId::UNKNOWN,
            parameter_text.origin_list(),
            replacement_text.origin_list(),
        ),
    })
}

fn scan_expanded_replacement_with_driver<S, St, H>(
    input: &mut InputStack<S>,
    stores: &mut St,
    context: TracedTokenWord,
    hooks: &mut H,
) -> Result<TracedTokenList, ScanToksError>
where
    S: InputSource,
    St: ExpansionState + tex_state::InputOpenState,
    H: ExpansionHooks<S>,
{
    let mut builder = stores.token_list_builder();
    let mut origins = stores.origin_list_builder();
    let mut recorder = NoopRecorder;
    let mut brace_level = 1_u32;
    let mut pending_parameter = false;

    loop {
        let source_depth = input.source_depth();
        let raw = crate::next_semantic_raw_token(input, stores)?
            .ok_or(ScanToksError::EndOfInputInReplacementText { context })?;
        if input.source_depth() < source_depth {
            unread_token(input, stores, raw);
            return Ok(finish_traced_list(stores, &mut builder, &mut origins));
        }
        if is_outer_macro(stores, traced_semantic_token(raw)) {
            // TeX.web §336 checks outer validity in get_next while the
            // defining scanner is active, before get_x expands the token.
            // Back up the outer token and finish the partial definition as if
            // the error recovery had inserted its missing right brace.
            unread_token(input, stores, raw);
            return Ok(finish_traced_list(stores, &mut builder, &mut origins));
        }
        unread_token(input, stores, raw);
        let expanded =
            crate::get_x_or_protected_with_recorder_and_hooks(input, stores, &mut recorder, hooks)?;
        let Some(traced) = expanded else {
            return Err(ScanToksError::EndOfInputInReplacementText { context });
        };
        if input.source_depth() < source_depth {
            // TeX's defining scanner inserts the missing right brace at the
            // nested-file boundary and leaves the first outer-file token for
            // ordinary input processing.
            unread_token(input, stores, traced);
            return Ok(finish_traced_list(stores, &mut builder, &mut origins));
        }
        let token = traced_semantic_token(traced);

        if pending_parameter {
            pending_parameter = false;
            match token {
                Token::Char {
                    cat: Catcode::Parameter,
                    ..
                } => push_scanned_token(&mut builder, &mut origins, traced, token),
                Token::Char {
                    ch: '1'..='9',
                    cat: Catcode::Other,
                } => push_scanned_token(
                    &mut builder,
                    &mut origins,
                    traced,
                    Token::param(token_digit(token).expect("digit token was matched")),
                ),
                _ => {
                    return Err(ScanToksError::InvalidParameterTokenInReplacementText {
                        context: traced,
                    });
                }
            }
            continue;
        }

        match token {
            Token::Char {
                cat: Catcode::Parameter,
                ..
            } => pending_parameter = true,
            Token::Char {
                cat: Catcode::BeginGroup,
                ..
            } => {
                brace_level = brace_level.saturating_add(1);
                push_scanned_token(&mut builder, &mut origins, traced, token);
            }
            Token::Char {
                cat: Catcode::EndGroup,
                ..
            } => {
                brace_level -= 1;
                if brace_level == 0 {
                    return Ok(finish_traced_list(stores, &mut builder, &mut origins));
                }
                push_scanned_token(&mut builder, &mut origins, traced, token);
            }
            _ => push_scanned_token(&mut builder, &mut origins, traced, token),
        }
    }
}

/// Scans TeX general text as a raw balanced group, then expands it.
///
/// This matches `scan_toks(macro_def = false, xpand = true)` callers such as
/// TeX82 `\mark`: parameter tokens are ordinary tokens while scanning the
/// balanced text, and expansion happens over the frozen raw text.
pub fn scan_general_text_expanded_with_driver<S, St, H>(
    input: &mut InputStack<S>,
    stores: &mut St,
    hooks: &mut H,
    context: TracedTokenWord,
) -> Result<TokenListId, ScanToksError>
where
    S: InputSource,
    St: ExpansionState + tex_state::InputOpenState,
    H: ExpansionHooks<S>,
{
    let raw_text = scan_general_text(input, stores, context)?;
    Ok(expand_replacement_text(
        stores,
        raw_text.token_list(),
        raw_text.origin_list(),
        hooks,
        &mut DriverExpandNext,
    )?
    .token_list())
}

/// Scans e-TeX general text while expanding only the tokens that precede its
/// compulsory opening brace.
///
/// This is the `scan_toks(false, false)` entry behavior used by commands such
/// as `\showtokens`: expansion can expose the opening brace, but the balanced
/// contents themselves are retained without expansion.
pub fn scan_general_text_with_expanded_open_with_driver<S, St, H>(
    input: &mut InputStack<S>,
    stores: &mut St,
    hooks: &mut H,
    context: TracedTokenWord,
) -> Result<TracedTokenList, ScanToksError>
where
    S: InputSource,
    St: ExpansionState + tex_state::InputOpenState,
    H: ExpansionHooks<S>,
{
    scan_general_text_with_expanded_open(
        input,
        stores,
        &mut NoopRecorder,
        hooks,
        &mut DriverExpandNext,
        context,
    )
}

fn expand_replacement_text<'a, S, St, H, E>(
    stores: &mut St,
    replacement_text: TokenListId,
    replacement_origins: OriginListId,
    hooks: &'a mut H,
    expander: &mut E,
) -> Result<TracedTokenList, ScanToksError>
where
    S: InputSource,
    St: ExpansionState,
    H: ExpansionHooks<S> + 'a,
    E: ExpandNext<ReplacementSource<S>, St, NoopRecorder, ReplacementHooks<'a, S, H>>,
{
    let mut input = InputStack::new(ReplacementSource::<S>::empty());
    input.push_token_list_with_origins(
        replacement_text,
        replacement_origins,
        TokenListReplayKind::Inserted,
    );
    let mut builder = stores.token_list_builder();
    let mut origins = stores.origin_list_builder();
    let mut recorder = NoopRecorder;
    let mut hooks = ReplacementHooks::new(hooks);

    loop {
        let Some(read) = input.next_traced_expansion_token(stores)? else {
            break;
        };
        let token = read.token();
        let traced = read.traced_token();
        if read.suppress_expansion() {
            builder.push(token);
            origins.push(read.origin());
            continue;
        }

        let Some(symbol) = crate::expandable_symbol(stores, traced) else {
            builder.push(token);
            origins.push(read.origin());
            continue;
        };
        let meaning = stores.meaning(symbol);
        if matches!(meaning, Meaning::Macro { flags, .. } if flags.contains(MeaningFlags::PROTECTED))
        {
            builder.push(token);
            origins.push(read.origin());
            continue;
        }
        if meaning == Meaning::ExpandablePrimitive(ExpandablePrimitive::NoExpand) {
            let Some(suppressed) = crate::next_suppressed_semantic_raw_token(&mut input, stores)?
            else {
                return Err(ExpandError::MissingTokenAfterPrimitive {
                    opcode: ExpandableOpcode::NoExpand,
                    context: traced,
                }
                .into());
            };
            builder.push(traced_semantic_token(suppressed));
            origins.push(stores.inserted_origin(
                InsertedOriginKind::NoExpand,
                traced_semantic_token(suppressed),
                suppressed.origin(),
            ));
            continue;
        }

        if meaning == Meaning::ExpandablePrimitive(ExpandablePrimitive::ExpandAfter) {
            // In an `\edef`, `\expandafter` performs exactly one expansion
            // step on its target, then returns control to the protected-aware
            // replacement scanner. Calling `get_x_token` here would continue
            // through the saved protected macro and expand it incorrectly.
            let dispatch = expander.dispatch_raw_token(
                traced,
                &mut input,
                stores,
                &mut recorder,
                &mut hooks,
            )?;
            crate::push_dispatch_result(&mut input, stores, dispatch);
            continue;
        }

        unread_token(&mut input, stores, traced);
        if let Some(expanded) =
            expander.next_expanded_token(&mut input, stores, &mut recorder, &mut hooks)?
        {
            builder.push(crate::semantic_token(expanded));
            origins.push(expanded.origin());
        }
    }
    let token_list = stores.finish_token_list(&mut builder);
    let origin_list = stores.finish_origin_list(&mut origins);
    Ok(TracedTokenList::new(token_list, origin_list))
}

fn unread_token<S>(
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
    token: TracedTokenWord,
) where
    S: InputSource,
{
    crate::back_input(input, stores, [token]);
}

fn push_scanned_token(
    builder: &mut TokenListBuilder,
    origins: &mut OriginListBuilder,
    traced: TracedTokenWord,
    token: Token,
) {
    builder.push(token);
    origins.push(traced.origin());
}

fn finish_traced_list(
    stores: &mut impl ExpansionState,
    builder: &mut TokenListBuilder,
    origins: &mut OriginListBuilder,
) -> TracedTokenList {
    let token_list = stores.finish_token_list(builder);
    let origin_list = stores.finish_origin_list(origins);
    TracedTokenList::new(token_list, origin_list)
}

fn traced_semantic_token(token: TracedTokenWord) -> Token {
    token
        .token()
        .expect("macro token scanner received invalid traced token")
}

enum ReplacementSource<S> {
    Empty(MemoryInput),
    Driver(S),
}

impl<S> ReplacementSource<S> {
    fn empty() -> Self {
        Self::Empty(MemoryInput::new(""))
    }
}

impl<S> InputSource for ReplacementSource<S>
where
    S: InputSource,
{
    fn read_line(&mut self) -> Result<Option<tex_lex::PhysicalLine>, tex_lex::InputSourceError> {
        match self {
            Self::Empty(source) => source.read_line(),
            Self::Driver(source) => source.read_line(),
        }
    }
}

struct ReplacementHooks<'a, S, H> {
    inner: &'a mut H,
    _source: PhantomData<fn() -> S>,
}

impl<'a, S, H> ReplacementHooks<'a, S, H> {
    fn new(inner: &'a mut H) -> Self {
        Self {
            inner,
            _source: PhantomData,
        }
    }
}

impl<S, H> ExpansionHooks<ReplacementSource<S>> for ReplacementHooks<'_, S, H>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    fn open_input<C: InputReadState>(
        &mut self,
        input: &mut C,
        name: &str,
    ) -> Result<ReplacementSource<S>, String> {
        self.inner
            .open_input(input, name)
            .map(ReplacementSource::Driver)
    }

    fn open_font<C: InputReadState>(
        &mut self,
        input: &mut C,
        path: &std::path::Path,
    ) -> Result<tex_state::FileContent, String> {
        self.inner.open_font(input, path)
    }

    fn job_name(&self) -> &str {
        self.inner.job_name()
    }

    fn mode(&self) -> crate::EngineMode {
        self.inner.mode()
    }

    fn is_inner_mode(&self) -> bool {
        self.inner.is_inner_mode()
    }

    fn space_factor(&self) -> i32 {
        self.inner.space_factor()
    }

    fn prev_depth(&self) -> tex_state::scaled::Scaled {
        self.inner.prev_depth()
    }

    fn prev_graf(&self) -> i32 {
        self.inner.prev_graf()
    }

    fn last_penalty(&self) -> i32 {
        self.inner.last_penalty()
    }

    fn last_kern(&self) -> tex_state::scaled::Scaled {
        self.inner.last_kern()
    }

    fn last_skip(&self) -> tex_state::glue::GlueSpec {
        self.inner.last_skip()
    }

    fn last_node_type(&self) -> i32 {
        self.inner.last_node_type()
    }

    fn input_stream_eof(&self, stores: &impl ExpansionState, stream: u8) -> bool {
        self.inner.input_stream_eof(stores, stream)
    }

    fn set_engine_state(&mut self, state: crate::EngineStateSnapshot) {
        self.inner.set_engine_state(state);
    }
}

fn scan_parameter_text<S>(
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
    context: TracedTokenWord,
) -> Result<TracedTokenList, ScanToksError>
where
    S: InputSource,
{
    let mut builder = stores.token_list_builder();
    let mut origins = stores.origin_list_builder();
    let mut next_parameter = 1;
    let mut pending_parameter = false;

    loop {
        let traced = crate::next_semantic_raw_token(input, stores)?
            .ok_or(ScanToksError::EndOfInputInParameterText { context })?;
        let token = traced_semantic_token(traced);

        if is_outer_macro(stores, token) {
            // TeX.web §336 backs up a forbidden outer control sequence and
            // inserts a right brace while `scanner_status=defining`.
            unread_token(input, stores, traced);
            return Ok(finish_traced_list(stores, &mut builder, &mut origins));
        }

        if pending_parameter {
            pending_parameter = false;
            match token {
                Token::Char {
                    ch: '1'..='9',
                    cat: Catcode::Other,
                } => {
                    let found = token_digit(token).expect("digit token was matched");
                    if next_parameter == 10 {
                        // TeX.web §476 ignores both the parameter marker and
                        // following token after nine parameters.
                        continue;
                    }
                    if found != next_parameter {
                        // `back_error` replays the wrong digit and inserts the
                        // consecutive digit TeX expected at this position.
                        unread_token(input, stores, traced);
                        let inserted = Token::param(next_parameter);
                        let origin = stores.inserted_origin(
                            InsertedOriginKind::ErrorRecovery,
                            inserted,
                            traced.origin(),
                        );
                        push_scanned_token(
                            &mut builder,
                            &mut origins,
                            TracedTokenWord::pack(inserted, origin),
                            inserted,
                        );
                        next_parameter += 1;
                        continue;
                    }
                    push_scanned_token(&mut builder, &mut origins, traced, Token::param(found));
                    next_parameter += 1;
                }
                Token::Char {
                    cat: Catcode::BeginGroup,
                    ..
                } => {
                    push_scanned_token(&mut builder, &mut origins, traced, token);
                    return Ok(finish_traced_list(stores, &mut builder, &mut origins));
                }
                _ => {
                    return Err(ScanToksError::InvalidParameterTokenInParameterText {
                        context: traced,
                    });
                }
            }
            continue;
        }

        match token {
            Token::Char {
                cat: Catcode::BeginGroup,
                ..
            } => return Ok(finish_traced_list(stores, &mut builder, &mut origins)),
            Token::Char {
                cat: Catcode::Parameter,
                ..
            } => pending_parameter = true,
            _ => push_scanned_token(&mut builder, &mut origins, traced, token),
        }
    }
}

fn scan_replacement_text<S>(
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
    context: TracedTokenWord,
) -> Result<TracedTokenList, ScanToksError>
where
    S: InputSource,
{
    let mut builder = stores.token_list_builder();
    let mut origins = stores.origin_list_builder();
    let mut brace_level = 1_u32;
    let mut pending_parameter = false;

    loop {
        let traced = crate::next_semantic_raw_token(input, stores)?
            .ok_or(ScanToksError::EndOfInputInReplacementText { context })?;
        let token = traced_semantic_token(traced);

        if is_outer_macro(stores, token) {
            unread_token(input, stores, traced);
            return Ok(finish_traced_list(stores, &mut builder, &mut origins));
        }

        if pending_parameter {
            pending_parameter = false;
            match token {
                Token::Char {
                    cat: Catcode::Parameter,
                    ..
                } => push_scanned_token(&mut builder, &mut origins, traced, token),
                Token::Char {
                    ch: '1'..='9',
                    cat: Catcode::Other,
                } => push_scanned_token(
                    &mut builder,
                    &mut origins,
                    traced,
                    Token::param(token_digit(token).expect("digit token was matched")),
                ),
                _ => {
                    return Err(ScanToksError::InvalidParameterTokenInReplacementText {
                        context: traced,
                    });
                }
            }
            continue;
        }

        match token {
            Token::Char {
                cat: Catcode::Parameter,
                ..
            } => pending_parameter = true,
            Token::Char {
                cat: Catcode::BeginGroup,
                ..
            } => {
                brace_level += 1;
                push_scanned_token(&mut builder, &mut origins, traced, token);
            }
            Token::Char {
                cat: Catcode::EndGroup,
                ..
            } => {
                brace_level -= 1;
                if brace_level == 0 {
                    return Ok(finish_traced_list(stores, &mut builder, &mut origins));
                }
                push_scanned_token(&mut builder, &mut origins, traced, token);
            }
            _ => push_scanned_token(&mut builder, &mut origins, traced, token),
        }
    }
}

pub(crate) fn scan_general_text<S>(
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
    context: TracedTokenWord,
) -> Result<TracedTokenList, ScanToksError>
where
    S: InputSource,
{
    let open = next_non_space_token(input, stores)?
        .ok_or(ScanToksError::EndOfInputInReplacementText { context })?;
    if !has_begin_group_meaning(stores, traced_semantic_token(open)) {
        return Err(ScanToksError::MissingGeneralTextBeginGroup { context: open });
    }

    scan_general_text_body(input, stores, context)
}

fn scan_general_text_body<S>(
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
    context: TracedTokenWord,
) -> Result<TracedTokenList, ScanToksError>
where
    S: InputSource,
{
    let mut builder = stores.token_list_builder();
    let mut origins = stores.origin_list_builder();
    let mut brace_level = 1_u32;
    loop {
        let traced = crate::next_semantic_raw_token(input, stores)?
            .ok_or(ScanToksError::EndOfInputInReplacementText { context })?;
        let token = traced_semantic_token(traced);
        if is_outer_macro(stores, token) {
            // The absorbing scanner uses the same inserted-right-brace
            // recovery and leaves the outer token for ordinary dispatch.
            unread_token(input, stores, traced);
            return Ok(finish_traced_list(stores, &mut builder, &mut origins));
        }
        match token {
            Token::Char {
                cat: Catcode::BeginGroup,
                ..
            } => {
                brace_level += 1;
                push_scanned_token(&mut builder, &mut origins, traced, token);
            }
            Token::Char {
                cat: Catcode::EndGroup,
                ..
            } => {
                brace_level -= 1;
                if brace_level == 0 {
                    return Ok(finish_traced_list(stores, &mut builder, &mut origins));
                }
                push_scanned_token(&mut builder, &mut origins, traced, token);
            }
            _ => push_scanned_token(&mut builder, &mut origins, traced, token),
        }
    }
}

/// Scans e-TeX general text after expanding only while looking for its
/// compulsory opening brace; the balanced contents themselves remain raw.
pub(crate) fn scan_general_text_with_expanded_open<S, St, R, H, E>(
    input: &mut InputStack<S>,
    stores: &mut St,
    recorder: &mut R,
    hooks: &mut H,
    expander: &mut E,
    context: TracedTokenWord,
) -> Result<TracedTokenList, ScanToksError>
where
    S: InputSource,
    St: ExpansionState,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
    E: ExpandNext<S, St, R, H>,
{
    let open = loop {
        let token = expander
            .next_expanded_token(input, stores, recorder, hooks)?
            .ok_or(ScanToksError::EndOfInputInReplacementText { context })?;
        if !matches!(
            traced_semantic_token(token),
            Token::Char {
                cat: Catcode::Space,
                ..
            }
        ) {
            break token;
        }
    };
    if !has_begin_group_meaning(stores, traced_semantic_token(open)) {
        return Err(ScanToksError::MissingGeneralTextBeginGroup { context: open });
    }
    scan_general_text_body(input, stores, context)
}

fn has_begin_group_meaning(stores: &impl ExpansionState, token: Token) -> bool {
    match token {
        Token::Char {
            cat: Catcode::BeginGroup,
            ..
        } => true,
        Token::Cs(symbol) => matches!(
            stores.meaning(symbol),
            Meaning::CharToken {
                cat: Catcode::BeginGroup,
                ..
            }
        ),
        Token::Char { .. } | Token::Param(_) | Token::Frozen(_) => false,
    }
}

fn is_outer_macro(stores: &impl ExpansionState, token: Token) -> bool {
    let Token::Cs(symbol) = token else {
        return false;
    };
    matches!(
        stores.meaning(symbol),
        Meaning::Macro { flags, .. } if flags.contains(MeaningFlags::OUTER)
    )
}

fn next_non_space_token<S>(
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
) -> Result<Option<TracedTokenWord>, ScanToksError>
where
    S: InputSource,
{
    loop {
        let Some(token) = crate::next_semantic_raw_token(input, stores)? else {
            return Ok(None);
        };
        if !matches!(
            traced_semantic_token(token),
            Token::Char {
                cat: Catcode::Space,
                ..
            }
        ) {
            return Ok(Some(token));
        }
    }
}

fn token_digit(token: Token) -> Option<u8> {
    let Token::Char {
        ch: '1'..='9',
        cat: Catcode::Other,
    } = token
    else {
        return None;
    };
    Some(match token {
        Token::Char { ch, .. } => ch as u8 - b'0',
        _ => unreachable!("matched token is a char"),
    })
}

#[cfg(test)]
mod tests;
