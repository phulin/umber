//! Macro definition token scanning.
//!
//! This module implements the reusable `scan_toks`-style part of `\def` and
//! `\edef`: scan the parameter text, then scan the brace-balanced replacement
//! text. It freezes the resulting token lists through `Universe`, but it does
//! not assign the macro meaning to `Env`.

use std::fmt;

use tex_lex::{
    InputStack, LexError, LiteralSpanPolicy, TokenListReplayKind, TokenListReplayMarker,
};
use tex_state::ids::{OriginListId, TokenListId};
use tex_state::macro_store::{MacroDefinitionProvenance, MacroMeaning};
use tex_state::meaning::{ExpandablePrimitive, Meaning, MeaningFlags};
use tex_state::provenance::{InsertedOriginKind, OriginListBuilder};
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};
use tex_state::token_store::TokenListBuilder;
use tex_state::{ExpansionState, TracedTokenList};

use crate::{
    Dispatch, DriverExpansionMode, ExpandError, ExpandableOpcode, ExpansionContext, ExpansionMode,
    RestrictedExpansionMode,
};

/// Result of scanning a macro definition without assigning it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScannedMacro {
    meaning: MacroMeaning,
    provenance: MacroDefinitionProvenance,
    diagnostics: Vec<MacroScanDiagnostic>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MacroScanDiagnostic {
    UndefinedControlSequence {
        name: String,
        context: TracedTokenWord,
    },
    IllegalParameterNumber {
        context: TracedTokenWord,
    },
}

struct ScannedParameterText {
    text: TracedTokenList,
    hash_brace: Option<TracedTokenWord>,
}

impl ScannedMacro {
    #[must_use]
    pub const fn meaning(&self) -> MacroMeaning {
        self.meaning
    }

    #[must_use]
    pub const fn provenance(&self) -> MacroDefinitionProvenance {
        self.provenance
    }

    #[must_use]
    pub fn diagnostics(&self) -> &[MacroScanDiagnostic] {
        &self.diagnostics
    }

    #[must_use]
    pub fn with_definition_origin(self, definition_origin: tex_state::token::OriginId) -> Self {
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
    pub const fn parameter_text(&self) -> TokenListId {
        self.meaning.parameter_text()
    }

    #[must_use]
    pub const fn replacement_text(&self) -> TokenListId {
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
    TooManyRecoverableErrors {
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
            Self::TooManyRecoverableErrors { .. } => {
                write!(f, "100 errors occurred while scanning a macro definition")
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
    pub fn resource_need(&self) -> Option<crate::ResourceNeed> {
        match self {
            Self::Expand(error) => error.resource_need(),
            _ => None,
        }
    }

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
            | Self::TooManyRecoverableErrors { context }
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
pub fn scan_toks(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    flags: MeaningFlags,
    context: TracedTokenWord,
) -> Result<ScannedMacro, ScanToksError> {
    let mut diagnostics = Vec::new();
    let parameter_text = scan_parameter_text(input, stores, context, &mut diagnostics)?;
    let replacement_text = scan_replacement_text(input, stores, context, &mut diagnostics)?;
    let replacement_text = append_hash_brace(stores, replacement_text, parameter_text.hash_brace);
    Ok(ScannedMacro {
        meaning: MacroMeaning::new(
            flags,
            parameter_text.text.token_list(),
            replacement_text.token_list(),
        ),
        provenance: MacroDefinitionProvenance::new(
            tex_state::token::OriginId::UNKNOWN,
            parameter_text.text.origin_list(),
            replacement_text.origin_list(),
        ),
        diagnostics,
    })
}

/// Scans a macro definition and expands the replacement text as for `\edef`.
pub fn scan_toks_expanded(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    flags: MeaningFlags,
    context: TracedTokenWord,
    expansion: &mut ExpansionContext<'_>,
) -> Result<ScannedMacro, ScanToksError> {
    let scanned = scan_toks(input, stores, flags, context)?;
    let meaning = scanned.meaning();
    let replacement_text = expand_replacement_text(
        input,
        stores,
        meaning.replacement_text(),
        scanned.provenance().replacement_origins(),
        expansion,
        &mut RestrictedExpansionMode,
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
        diagnostics: scanned.diagnostics,
    })
}

pub fn scan_toks_expanded_with_driver(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    flags: MeaningFlags,
    context: TracedTokenWord,
    expansion: &mut ExpansionContext<'_>,
) -> Result<ScannedMacro, ScanToksError>
where
{
    let mut diagnostics = Vec::new();
    let parameter_text = scan_parameter_text(input, stores, context, &mut diagnostics)?;
    let replacement_result = expansion.with_expanded_token_list(|expansion| {
        scan_expanded_replacement_with_driver(input, stores, context, expansion, &mut diagnostics)
    });
    let replacement_text = replacement_result?;
    let replacement_text = append_hash_brace(stores, replacement_text, parameter_text.hash_brace);
    Ok(ScannedMacro {
        meaning: MacroMeaning::new(
            flags,
            parameter_text.text.token_list(),
            replacement_text.token_list(),
        ),
        provenance: MacroDefinitionProvenance::new(
            tex_state::token::OriginId::UNKNOWN,
            parameter_text.text.origin_list(),
            replacement_text.origin_list(),
        ),
        diagnostics,
    })
}

fn scan_expanded_replacement_with_driver(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    context: TracedTokenWord,
    expansion: &mut ExpansionContext<'_>,
    diagnostics: &mut Vec<MacroScanDiagnostic>,
) -> Result<TracedTokenList, ScanToksError>
where
{
    let mut builder = stores.token_list_builder();
    let mut origins = stores.origin_list_builder();
    let mut brace_level = 1_u32;
    let mut pending_parameter: Option<TracedTokenWord> = None;

    loop {
        // Literal spans are safe only while the scanner has no interpretation
        // pending from a previously delivered token. In particular, a
        // parameter character can arrive per-token at the end of one replay
        // segment while its digit arrives from a macro-argument span. The
        // digit must still be interpreted as Param(n), not copied literally.
        // `brace_level` needs no separate gate: begin/end-group tokens are
        // excluded by ExpandedReplacement's lexical span policy.
        if pending_parameter.is_none()
            && input.append_macro_literal_span(
                stores,
                &mut builder,
                &mut origins,
                LiteralSpanPolicy::ExpandedReplacement,
            ) > 0
        {
            continue;
        }
        let source_depth = input.source_depth();
        let prepared = crate::next_prepared_expansion_token(input, stores, expansion)?
            .ok_or(ScanToksError::EndOfInputInReplacementText { context })?;
        let raw = prepared.traced_token();
        if input.source_depth() < source_depth {
            unread_token(input, stores, raw);
            return Ok(finish_traced_list(stores, &mut builder, &mut origins));
        }
        if !prepared.suppress_expansion() && is_outer_macro(stores, traced_semantic_token(raw)) {
            // TeX.web §336 checks outer validity in get_next while the
            // defining scanner is active, before get_x expands the token.
            // Back up the outer token and finish the partial definition as if
            // the error recovery had inserted its missing right brace.
            unread_token(input, stores, raw);
            return Ok(finish_traced_list(stores, &mut builder, &mut origins));
        }
        if !prepared.suppress_expansion()
            && let Some(symbol) = crate::expandable_symbol(stores, raw)
        {
            let meaning = expansion.resolve_meaning(input, stores, symbol);
            if meaning == Meaning::ExpandablePrimitive(ExpandablePrimitive::Unexpanded) {
                expansion.record_meaning(symbol, meaning);
                let dispatch = match crate::dispatch::dispatch_with_context(
                    traced_semantic_token(raw),
                    raw.origin(),
                    input,
                    stores,
                    expansion,
                    meaning,
                ) {
                    Ok(dispatch) => dispatch,
                    Err(error) => {
                        record_undefined_diagnostic(error, diagnostics)?;
                        continue;
                    }
                };
                crate::push_dispatch_result(input, stores, dispatch);
                continue;
            }
            if meaning == Meaning::ExpandablePrimitive(ExpandablePrimitive::Expanded) {
                expansion.record_meaning(symbol, meaning);
                let dispatch = match crate::dispatch::dispatch_with_context(
                    traced_semantic_token(raw),
                    raw.origin(),
                    input,
                    stores,
                    expansion,
                    meaning,
                ) {
                    Ok(dispatch) => dispatch,
                    Err(error) => {
                        record_undefined_diagnostic(error, diagnostics)?;
                        continue;
                    }
                };
                crate::push_dispatch_result(input, stores, dispatch);
                continue;
            }
            if meaning == Meaning::ExpandablePrimitive(ExpandablePrimitive::The) {
                expansion.record_meaning(symbol, meaning);
                let dispatch = match crate::dispatch::dispatch_with_context(
                    traced_semantic_token(raw),
                    raw.origin(),
                    input,
                    stores,
                    expansion,
                    meaning,
                ) {
                    Ok(dispatch) => dispatch,
                    Err(error) => {
                        record_undefined_diagnostic(error, diagnostics)?;
                        continue;
                    }
                };
                if !append_the_toks_output(stores, &mut builder, &mut origins, &dispatch) {
                    crate::push_dispatch_result(input, stores, dispatch);
                }
                continue;
            }
            let needs_suppressed_replay =
                meaning == Meaning::ExpandablePrimitive(ExpandablePrimitive::NoExpand);
            if needs_suppressed_replay
                || matches!(meaning, Meaning::Macro { flags, .. } if !flags.contains(MeaningFlags::PROTECTED))
            {
                expansion.record_meaning(symbol, meaning);
                let dispatched = crate::dispatch::dispatch_with_context(
                    traced_semantic_token(raw),
                    raw.origin(),
                    input,
                    stores,
                    expansion,
                    meaning,
                );
                match dispatched {
                    Ok(dispatch) => crate::push_dispatch_result(input, stores, dispatch),
                    Err(error) => match expansion.recover_macro_mismatch(error) {
                        Ok(()) => continue,
                        Err(ExpandError::MacroCall(crate::args::MacroCallError::EndOfInput {
                            ..
                        })) => {
                            return Err(ScanToksError::EndOfInputInReplacementText { context });
                        }
                        Err(error) => {
                            record_undefined_diagnostic(error, diagnostics)?;
                            continue;
                        }
                    },
                }
                if input.source_depth() < source_depth {
                    // Preserve the defining scanner's nested-source seam: the
                    // first expanded token remains available below the
                    // recovery-inserted closing brace.
                    let traced =
                        match crate::get_x_or_protected_with_context(input, stores, expansion) {
                            Ok(Some(traced)) => traced,
                            Ok(None) => {
                                return Err(ScanToksError::EndOfInputInReplacementText { context });
                            }
                            Err(error) => {
                                record_undefined_diagnostic(error, diagnostics)?;
                                continue;
                            }
                        };
                    unread_token(input, stores, traced);
                    return Ok(finish_traced_list(stores, &mut builder, &mut origins));
                }
                continue;
            }
        }
        let expanded = match crate::get_x_or_protected_from_prepared_with_context(
            prepared, input, stores, expansion,
        ) {
            Ok(expanded) => expanded,
            Err(error) => {
                record_undefined_diagnostic(error, diagnostics)?;
                continue;
            }
        };
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
        let traced = normalize_stored_noexpand_origin(stores, traced, token);
        let stored_unexpanded = prepared.suppress_expansion()
            || stores.origin_is_inserted_kind(traced.origin(), InsertedOriginKind::Unexpanded);

        // e-TeX's `\unexpanded` contributes its token list through TeX's
        // `the_toks` path. Parameter characters from that list are copied
        // verbatim; they are not reinterpreted as definition parameters.
        // `\noexpand` gives a parameter character the same one-token
        // suppression semantics.
        if stored_unexpanded
            && (has_parameter_meaning(stores, token) || matches!(token, Token::Param(_)))
        {
            push_scanned_token(&mut builder, &mut origins, traced, token);
            continue;
        }

        if let Some(parameter) = pending_parameter.take() {
            match token {
                token if has_parameter_meaning(stores, token) => {
                    push_scanned_token(&mut builder, &mut origins, traced, token)
                }
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
                    // TeX.web §479's `back_error; cur_tok:=s` keeps the
                    // invalid follower for the next scanner iteration and
                    // stores the saved parameter character literally.
                    record_scan_diagnostic(
                        diagnostics,
                        MacroScanDiagnostic::IllegalParameterNumber { context: traced },
                        traced,
                    )?;
                    unread_token(input, stores, traced);
                    let token = traced_semantic_token(parameter);
                    push_scanned_token(&mut builder, &mut origins, parameter, token);
                }
            }
            continue;
        }

        match token {
            token if has_parameter_meaning(stores, token) => pending_parameter = Some(traced),
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

fn record_undefined_diagnostic(
    error: ExpandError,
    diagnostics: &mut Vec<MacroScanDiagnostic>,
) -> Result<(), ScanToksError> {
    let (name, context) = take_undefined_control_sequence(error).map_err(ScanToksError::Expand)?;
    record_scan_diagnostic(
        diagnostics,
        MacroScanDiagnostic::UndefinedControlSequence { name, context },
        context,
    )?;
    Ok(())
}

fn record_scan_diagnostic(
    diagnostics: &mut Vec<MacroScanDiagnostic>,
    diagnostic: MacroScanDiagnostic,
    context: TracedTokenWord,
) -> Result<(), ScanToksError> {
    if diagnostics.len() == 99 {
        return Err(ScanToksError::TooManyRecoverableErrors { context });
    }
    diagnostics.push(diagnostic);
    Ok(())
}

fn take_undefined_control_sequence(
    error: ExpandError,
) -> Result<(String, TracedTokenWord), ExpandError> {
    match error {
        ExpandError::UndefinedControlSequence { name, context } => Ok((name, context)),
        ExpandError::Captured { error, site } => match take_undefined_control_sequence(*error) {
            Ok(undefined) => Ok(undefined),
            Err(error) => Err(ExpandError::Captured {
                error: Box::new(error),
                site,
            }),
        },
        error => Err(error),
    }
}

/// Scans TeX general text while expanding its compulsory opening brace and
/// balanced contents.
///
/// This matches `scan_toks(macro_def = false, xpand = true)` callers such as
/// TeX82 `\mark`: parameter tokens are ordinary tokens while scanning the
/// balanced text, and expansion happens over the frozen raw text.
pub fn scan_general_text_expanded_with_driver(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    context: TracedTokenWord,
) -> Result<TokenListId, ScanToksError>
where
{
    Ok(scan_general_text_expanded_with_expanded_open(
        input,
        stores,
        expansion,
        &mut DriverExpansionMode,
        context,
    )?
    .token_list())
}

pub(crate) fn scan_general_text_expanded_with_expanded_open(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    mode: &mut dyn ExpansionMode,
    context: TracedTokenWord,
) -> Result<TracedTokenList, ScanToksError> {
    let open = loop {
        let token = mode
            .next_expanded_token(input, stores, expansion)?
            .ok_or(ScanToksError::EndOfInputInReplacementText { context })?;
        if !is_space_or_relax(stores, traced_semantic_token(token)) {
            break token;
        }
    };
    if !has_begin_group_meaning(stores, traced_semantic_token(open)) {
        return Err(ScanToksError::MissingGeneralTextBeginGroup { context: open });
    }
    collect_expanded_text(
        input,
        stores,
        expansion,
        mode,
        ExpandedTextBoundary::Balanced { depth: 1, context },
    )
}

/// Scans e-TeX general text while expanding only the tokens that precede its
/// compulsory opening brace.
///
/// This is the `scan_toks(false, false)` entry behavior used by commands such
/// as `\showtokens`: expansion can expose the opening brace, but the balanced
/// contents themselves are retained without expansion.
pub fn scan_general_text_with_expanded_open_with_driver(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    context: TracedTokenWord,
) -> Result<TracedTokenList, ScanToksError>
where
{
    scan_general_text_with_expanded_open(
        input,
        stores,
        expansion,
        &mut DriverExpansionMode,
        context,
    )
}

fn expand_replacement_text(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    replacement_text: TokenListId,
    replacement_origins: OriginListId,
    expansion: &mut ExpansionContext<'_>,
    mode: &mut dyn ExpansionMode,
) -> Result<TracedTokenList, ScanToksError> {
    // Keep an inaccessible token below the raw replacement. Expandable
    // primitives commonly read one token ahead while scanning their operands
    // (`\the\count15` is the canonical case). A replay marker alone cannot
    // delimit that read because the raw frame is retired before the primitive
    // pushes its rendered result. A dedicated engine-owned token provides an
    // exact synchronous boundary without impersonating an alignment template
    // sentinel if nested lookahead carries it across replay frames.
    let boundary = TracedTokenWord::pack(stores.expanded_text_boundary_token(), OriginId::UNKNOWN);
    let replay = input.push_transient_tokens(vec![boundary], TokenListReplayKind::Inserted);
    input.push_token_list_with_origins(
        replacement_text,
        replacement_origins,
        TokenListReplayKind::Inserted,
    );
    let result = collect_expanded_text(
        input,
        stores,
        expansion,
        mode,
        ExpandedTextBoundary::Replay(replay),
    );
    if result.is_err() {
        input.abort_token_list_replay(replay);
    }
    result
}

#[derive(Clone, Copy)]
enum ExpandedTextBoundary {
    Replay(TokenListReplayMarker),
    Balanced {
        depth: u32,
        context: TracedTokenWord,
    },
}

fn collect_expanded_text(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    mode: &mut dyn ExpansionMode,
    boundary: ExpandedTextBoundary,
) -> Result<TracedTokenList, ScanToksError> {
    expansion.with_expanded_token_list(|expansion| {
        collect_expanded_text_inner(input, stores, expansion, mode, boundary)
    })
}

fn collect_expanded_text_inner(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    mode: &mut dyn ExpansionMode,
    mut boundary: ExpandedTextBoundary,
) -> Result<TracedTokenList, ScanToksError> {
    let mut builder = stores.token_list_builder();
    let mut origins = stores.origin_list_builder();
    loop {
        if input.append_macro_literal_span(
            stores,
            &mut builder,
            &mut origins,
            LiteralSpanPolicy::ExpandedReplacement,
        ) > 0
        {
            continue;
        }
        let Some(prepared) = crate::next_prepared_expansion_token(input, stores, expansion)? else {
            return match boundary {
                ExpandedTextBoundary::Replay(_) => {
                    Ok(finish_traced_list(stores, &mut builder, &mut origins))
                }
                ExpandedTextBoundary::Balanced { context, .. } => {
                    Err(ScanToksError::EndOfInputInReplacementText { context })
                }
            };
        };
        let read = prepared.expansion_token();
        expansion.observe_read(read);
        let token = read.token();
        let traced = read.traced_token();
        if matches!(boundary, ExpandedTextBoundary::Replay(_)) && token.is_expanded_text_boundary()
        {
            let ExpandedTextBoundary::Replay(replay) = boundary else {
                unreachable!();
            };
            let _ = input.finish_exhausted_token_list_replay(replay, stores);
            break;
        }
        if read.suppress_expansion() {
            append_collected_token(
                &mut boundary,
                &mut builder,
                &mut origins,
                traced,
                token,
                false,
            );
            continue;
        }

        let Some(symbol) = crate::expandable_symbol(stores, traced) else {
            if append_collected_token(
                &mut boundary,
                &mut builder,
                &mut origins,
                traced,
                token,
                true,
            ) {
                break;
            }
            continue;
        };
        let meaning = expansion.resolve_meaning(input, stores, symbol);
        if matches!(meaning, Meaning::Macro { flags, .. } if flags.contains(MeaningFlags::PROTECTED))
        {
            append_collected_token(
                &mut boundary,
                &mut builder,
                &mut origins,
                traced,
                token,
                true,
            );
            continue;
        }
        if meaning == Meaning::ExpandablePrimitive(ExpandablePrimitive::NoExpand) {
            let Some(suppressed_token) = crate::next_suppressed_semantic_raw_token(input, stores)?
            else {
                return Err(ExpandError::MissingTokenAfterPrimitive {
                    opcode: ExpandableOpcode::NoExpand,
                    context: traced,
                }
                .into());
            };
            let suppressed = traced_semantic_token(suppressed_token);
            let origin = stores.inserted_origin(
                InsertedOriginKind::Unexpanded,
                suppressed,
                suppressed_token.origin(),
            );
            append_collected_token(
                &mut boundary,
                &mut builder,
                &mut origins,
                TracedTokenWord::pack(suppressed, origin),
                suppressed,
                false,
            );
            continue;
        }

        if matches!(
            meaning,
            Meaning::ExpandablePrimitive(
                ExpandablePrimitive::Unexpanded | ExpandablePrimitive::Expanded
            )
        ) {
            let dispatch = mode.dispatch_raw_token(traced, input, stores, expansion)?;
            crate::push_dispatch_result(input, stores, dispatch);
            continue;
        }

        if meaning == Meaning::ExpandablePrimitive(ExpandablePrimitive::The) {
            let dispatch = mode.dispatch_raw_token(traced, input, stores, expansion)?;
            if !append_the_toks_output(stores, &mut builder, &mut origins, &dispatch) {
                crate::push_dispatch_result(input, stores, dispatch);
            }
            continue;
        }

        if meaning == Meaning::ExpandablePrimitive(ExpandablePrimitive::ExpandAfter) {
            // In an `\edef`, `\expandafter` performs exactly one expansion
            // step on its target, then returns control to the protected-aware
            // replacement scanner. Calling `get_x_token` here would continue
            // through the saved protected macro and expand it incorrectly.
            let dispatch = mode.dispatch_raw_token(traced, input, stores, expansion)?;
            crate::push_dispatch_result(input, stores, dispatch);
            continue;
        }

        if matches!(meaning, Meaning::Macro { .. }) {
            // Keep macro replacement replay in this collection loop. The next
            // iteration can copy its inert character runs directly; any
            // parameter, cs/active site, or semantic edge naturally re-enters
            // the existing interpreter below.
            let dispatch = mode.dispatch_raw_token(traced, input, stores, expansion)?;
            crate::push_dispatch_result(input, stores, dispatch);
            continue;
        }

        // TeX.web's expanding `scan_toks` loop performs one `get_next` /
        // `expand` step at a time. Returning here after each dispatch is
        // essential: a nested conditional can exhaust this replay, and a
        // general `get_x_token` call would continue into the caller's input.
        match mode.dispatch_raw_token(traced, input, stores, expansion)? {
            Dispatch::Continue => {}
            Dispatch::Deliver(delivered) => {
                if append_collected_token(
                    &mut boundary,
                    &mut builder,
                    &mut origins,
                    delivered,
                    crate::semantic_token(delivered),
                    true,
                ) {
                    break;
                }
            }
            Dispatch::DeliverNoExpand(delivered) => {
                append_collected_token(
                    &mut boundary,
                    &mut builder,
                    &mut origins,
                    delivered,
                    crate::semantic_token(delivered),
                    false,
                );
            }
            push @ (Dispatch::Push { .. } | Dispatch::PushTransient { .. }) => {
                crate::push_dispatch_result(input, stores, push);
            }
        }
    }
    Ok(finish_traced_list(stores, &mut builder, &mut origins))
}

fn append_collected_token(
    boundary: &mut ExpandedTextBoundary,
    builder: &mut TokenListBuilder,
    origins: &mut OriginListBuilder,
    traced: TracedTokenWord,
    token: Token,
    balance: bool,
) -> bool {
    if balance && let ExpandedTextBoundary::Balanced { depth, .. } = boundary {
        match token {
            Token::Char {
                cat: Catcode::BeginGroup,
                ..
            } => *depth = depth.saturating_add(1),
            Token::Char {
                cat: Catcode::EndGroup,
                ..
            } => {
                *depth -= 1;
                if *depth == 0 {
                    return true;
                }
            }
            _ => {}
        }
    }
    push_scanned_token(builder, origins, traced, token);
    false
}

/// Implements TeX.web's `scan_toks` `the_toks` splice.
///
/// Token-register contents are attached directly to an expanding scanner's
/// output, so their control sequences are not expanded and their braces do
/// not participate in the scanner's outer balance. Outside `scan_toks`, the
/// same `\the` result remains ordinary expandable input.
fn append_the_toks_output(
    stores: &impl ExpansionState,
    builder: &mut TokenListBuilder,
    origins: &mut OriginListBuilder,
    dispatch: &Dispatch,
) -> bool {
    let Dispatch::Push {
        replay_kind: crate::ExpansionReplayKind::TheToksOutput,
        token_list,
        origin_list,
        ..
    } = dispatch
    else {
        return false;
    };
    let tokens = stores.tokens(*token_list);
    builder.extend_from_slice(tokens);
    if *origin_list == OriginListId::EMPTY {
        origins.extend_repeated(tex_state::token::OriginId::UNKNOWN, tokens.len());
    } else {
        origins.extend_from_slice(stores.origin_list(*origin_list));
    }
    true
}

fn unread_token(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    token: TracedTokenWord,
) {
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

fn normalize_stored_noexpand_origin(
    stores: &mut tex_state::ExpansionContext<'_>,
    traced: TracedTokenWord,
    token: Token,
) -> TracedTokenWord {
    if stores.origin_is_inserted_kind(traced.origin(), InsertedOriginKind::NoExpand) {
        let origin = stores.inserted_origin(InsertedOriginKind::Unexpanded, token, traced.origin());
        TracedTokenWord::pack(token, origin)
    } else {
        traced
    }
}

fn finish_traced_list(
    stores: &mut tex_state::ExpansionContext<'_>,
    builder: &mut TokenListBuilder,
    origins: &mut OriginListBuilder,
) -> TracedTokenList {
    let token_list = stores.finish_token_list(builder);
    let origin_list = stores.finish_origin_list(origins);
    TracedTokenList::new(token_list, origin_list)
}

fn append_hash_brace(
    stores: &mut tex_state::ExpansionContext<'_>,
    text: TracedTokenList,
    hash_brace: Option<TracedTokenWord>,
) -> TracedTokenList {
    let Some(hash_brace) = hash_brace else {
        return text;
    };
    let mut builder = stores.token_list_builder();
    builder.extend_from_slice(stores.tokens(text.token_list()));
    builder.push(traced_semantic_token(hash_brace));
    if text.origin_list() == OriginListId::EMPTY {
        let token_list = stores.finish_token_list(&mut builder);
        return TracedTokenList::new(token_list, OriginListId::EMPTY);
    }
    let mut origins = stores.origin_list_builder();
    origins.extend_from_slice(stores.origin_list(text.origin_list()));
    origins.push(hash_brace.origin());
    finish_traced_list(stores, &mut builder, &mut origins)
}

fn traced_semantic_token(token: TracedTokenWord) -> Token {
    token
        .token()
        .expect("macro token scanner received invalid traced token")
}

fn scan_parameter_text(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    context: TracedTokenWord,
    diagnostics: &mut Vec<MacroScanDiagnostic>,
) -> Result<ScannedParameterText, ScanToksError> {
    let mut builder = stores.token_list_builder();
    let mut origins = stores.origin_list_builder();
    let mut next_parameter = 1;
    let mut pending_parameter: Option<TracedTokenWord> = None;

    loop {
        let traced = crate::next_semantic_raw_token(input, stores)?
            .ok_or(ScanToksError::EndOfInputInParameterText { context })?;
        let token = traced_semantic_token(traced);

        if is_outer_macro(stores, token) {
            // TeX.web §336 backs up a forbidden outer control sequence and
            // inserts a right brace while `scanner_status=defining`.
            unread_token(input, stores, traced);
            return Ok(ScannedParameterText {
                text: finish_traced_list(stores, &mut builder, &mut origins),
                hash_brace: None,
            });
        }

        if let Some(parameter) = pending_parameter.take() {
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
                    return Ok(ScannedParameterText {
                        text: finish_traced_list(stores, &mut builder, &mut origins),
                        hash_brace: Some(traced),
                    });
                }
                _ => {
                    // TeX.web §476 uses `back_error` for a nonconsecutive
                    // follower: replay it, insert the expected match token,
                    // and continue scanning the parameter delimiter.
                    record_scan_diagnostic(
                        diagnostics,
                        MacroScanDiagnostic::IllegalParameterNumber { context: traced },
                        traced,
                    )?;
                    unread_token(input, stores, traced);
                    if next_parameter <= 9 {
                        let inserted = Token::param(next_parameter);
                        let origin = stores.inserted_origin(
                            InsertedOriginKind::ErrorRecovery,
                            inserted,
                            parameter.origin(),
                        );
                        push_scanned_token(
                            &mut builder,
                            &mut origins,
                            TracedTokenWord::pack(inserted, origin),
                            inserted,
                        );
                        next_parameter += 1;
                    }
                }
            }
            continue;
        }

        match token {
            Token::Char {
                cat: Catcode::BeginGroup,
                ..
            } => {
                return Ok(ScannedParameterText {
                    text: finish_traced_list(stores, &mut builder, &mut origins),
                    hash_brace: None,
                });
            }
            token if has_parameter_meaning(stores, token) => pending_parameter = Some(traced),
            _ => push_scanned_token(&mut builder, &mut origins, traced, token),
        }
    }
}

fn scan_replacement_text(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    context: TracedTokenWord,
    diagnostics: &mut Vec<MacroScanDiagnostic>,
) -> Result<TracedTokenList, ScanToksError> {
    let mut builder = stores.token_list_builder();
    let mut origins = stores.origin_list_builder();
    let mut brace_level = 1_u32;
    let mut pending_parameter: Option<TracedTokenWord> = None;

    loop {
        let traced = crate::next_semantic_raw_token(input, stores)?
            .ok_or(ScanToksError::EndOfInputInReplacementText { context })?;
        let token = traced_semantic_token(traced);

        if is_outer_macro(stores, token) {
            unread_token(input, stores, traced);
            return Ok(finish_traced_list(stores, &mut builder, &mut origins));
        }

        if let Some(parameter) = pending_parameter.take() {
            match token {
                token if has_parameter_meaning(stores, token) => {
                    push_scanned_token(&mut builder, &mut origins, traced, token)
                }
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
                    // TeX.web §479 recovers by backing up the invalid
                    // follower and storing the saved parameter character.
                    record_scan_diagnostic(
                        diagnostics,
                        MacroScanDiagnostic::IllegalParameterNumber { context: traced },
                        traced,
                    )?;
                    unread_token(input, stores, traced);
                    let token = traced_semantic_token(parameter);
                    push_scanned_token(&mut builder, &mut origins, parameter, token);
                }
            }
            continue;
        }

        match token {
            token if has_parameter_meaning(stores, token) => pending_parameter = Some(traced),
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

fn scan_general_text_body(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    context: TracedTokenWord,
) -> Result<TracedTokenList, ScanToksError> {
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
pub(crate) fn scan_general_text_with_expanded_open(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    expansion: &mut ExpansionContext<'_>,
    mode: &mut dyn ExpansionMode,
    context: TracedTokenWord,
) -> Result<TracedTokenList, ScanToksError>
where
{
    let open = loop {
        let token = mode
            .next_expanded_token(input, stores, expansion)?
            .ok_or(ScanToksError::EndOfInputInReplacementText { context })?;
        if !is_space_or_relax(stores, traced_semantic_token(token)) {
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

fn has_parameter_meaning(stores: &impl ExpansionState, token: Token) -> bool {
    match token {
        Token::Char {
            cat: Catcode::Parameter,
            ..
        } => true,
        Token::Char {
            ch,
            cat: Catcode::Active,
        } => stores.active_character_symbol(ch).is_some_and(|symbol| {
            matches!(
                stores.meaning(symbol),
                Meaning::CharToken {
                    cat: Catcode::Parameter,
                    ..
                }
            )
        }),
        Token::Cs(symbol) => matches!(
            stores.meaning(symbol),
            Meaning::CharToken {
                cat: Catcode::Parameter,
                ..
            }
        ),
        Token::Char { .. } | Token::Param(_) | Token::Frozen(_) => false,
    }
}

fn is_space_or_relax(stores: &impl ExpansionState, token: Token) -> bool {
    matches!(
        token,
        Token::Char {
            cat: Catcode::Space,
            ..
        }
    ) || matches!(token, Token::Cs(symbol) if stores.meaning(symbol) == Meaning::Relax)
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
