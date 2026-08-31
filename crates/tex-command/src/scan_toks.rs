//! Private canonical token-list scanner.
//!
//! This is deliberately a small, separate state machine rather than a
//! second `get_x_token` interpreter.  TeX.web's `scan_toks` has one crucial
//! exception to ordinary expansion: token-list results from `\the` (and the
//! e-TeX `\unexpanded` family) join the result directly.  In particular, the
//! contents of such a list neither consume the caller's input nor contribute
//! to the brace depth of this collection.
#![allow(dead_code)] // executor scanner callers arrive in the following slice

use tex_state::interner::{ControlSequenceKind, Symbol};
use tex_state::meaning::{ExpandablePrimitive, Meaning, MeaningFlags, ResolvedMeaning};
use tex_state::token::{Catcode, OriginId, Token, TokenWord, TracedTokenWord};

use crate::attempt::{AttemptDefinitionId, AttemptError, AttemptMark, AttemptTokenListId};
use crate::processor::alignment::TEMPLATE_ALIGN_STATE;
use crate::processor::expand::is_expandable_command;
use crate::processor::status::{
    AbsorbingContext, DefinitionContext, ScannerEpisode, ScannerStatus, ScannerStatusVisibility,
    ScannerWarning, TokenBuilderId,
};
use crate::{CommandError, CommandProcessor};
use tex_state::CommandLineSource;

use crate::input::PackedTokenSpanHandle;
use crate::observation::{
    CommandObservation, DiagnosticRecord, InputReason, InputRecord, InputTransition,
    TokenListRecord,
};
use crate::token_collector::{
    PendingParameter, TokenCollector, TokenCollectorDestination, TokenCollectorPhase,
};

/// The two canonical `scan_toks` collection forms.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum ScanToksMode {
    /// Collect balanced general text; parameter characters are ordinary text.
    General { expanded: bool },
    /// General text whose caller supplies TeX82's live `warning_index`.
    GeneralFor { expanded: bool, owner: Symbol },
    /// Collect general text after the caller has validated and backed up the
    /// required opening brace. This is TeX82 §1227's token-list assignment
    /// alone: it reads the right-hand side's first token through `get_x_token`
    /// to tell a braced list from a token register or parameter, then backs
    /// that brace up for `scan_toks`. Every other caller enters §473 directly
    /// and must use `General`, whose absorbing transition precedes the brace.
    GeneralAfterOpening {
        expanded: bool,
        primary: OriginId,
        owner: Option<Symbol>,
    },
    /// e-TeX 2.6 etex.ch §53a's recursive `scan_general_text`.
    ///
    /// It has the same absorbing-state recovery semantics as TeX82
    /// `scan_toks(false, false)`, but is a distinct canonical observation
    /// seam: its caller publishes the extension-specific token-list purpose,
    /// and the reference instrumentation does not publish the internal
    /// scanner-status scope.
    GeneralText { purpose: &'static str },
    /// A standalone general-text result constructed directly in the
    /// generation-owned inserted-input destination which will replay it.
    EscapingGeneralText { purpose: &'static str },
    /// Collect a macro parameter text followed by its replacement text.
    MacroDefinition { expanded: bool },
    /// Production macro definition scan, carrying §479's `warning_index`.
    MacroDefinitionFor { expanded: bool, target: Symbol },
}

/// Parsed grammar of one canonical token-list collection.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum ScanToksGrammar {
    General,
    MacroDefinition,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum ScanToksDestination {
    Attempt,
    ReplayInput,
}

/// How the collector reaches the opening delimiter of its body.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum ScanToksOpening {
    /// Scan TeX82 §403's mandatory opening brace.
    Required,
    /// Consume the already classified and backed-up §1227 opener.
    Prevalidated { primary: OriginId },
    /// Scan a macro's parameter text through its terminating opening brace.
    AfterParameterText,
}

/// Expansion policy inside the replacement collector.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum ScanToksExpansion {
    Expanded,
    Unexpanded,
}

impl ScanToksExpansion {
    const fn is_expanded(self) -> bool {
        matches!(self, Self::Expanded)
    }
}

/// Semantic owner of the scanner status and its runaway warning target.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum ScanToksOwner {
    Absorbed(Option<Symbol>),
    Definition(Option<Symbol>),
}

/// Detached token-list observation emitted when collection completes.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum ScanToksPurpose {
    Balanced,
    ExpandedBalanced,
    MacroReplacement,
    GeneralText(&'static str),
}

impl ScanToksPurpose {
    const fn canonical_name(self) -> &'static str {
        match self {
            Self::Balanced => "scan_toks",
            Self::ExpandedBalanced => "expanded_scan_toks",
            Self::MacroReplacement => "macro_replacement",
            Self::GeneralText(purpose) => purpose,
        }
    }

    fn renders_detokenized_result(self) -> bool {
        matches!(self, Self::GeneralText("detokenize"))
    }
}

/// Fully typed internal configuration parsed once from [`ScanToksMode`].
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct ScanToksConfig {
    grammar: ScanToksGrammar,
    destination: ScanToksDestination,
    opening: ScanToksOpening,
    expansion: ScanToksExpansion,
    owner: ScanToksOwner,
    purpose: ScanToksPurpose,
    status_visibility: ScannerStatusVisibility,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ScannedToksStorage<G> {
    Tokens {
        parameter: AttemptTokenListId,
        replacement: AttemptTokenListId,
    },
    Definition(AttemptDefinitionId),
    ReplayInputBuilder {
        builder: crate::input::ReplayInputBuilderId<G>,
        len: u32,
    },
    ReplayInput {
        replay: crate::input::ReplayPayloadId<G>,
        len: u32,
    },
}

enum ScannedWords<'a, G> {
    Traced(crate::attempt::AttemptTokenListView<'a>),
    Semantic(&'a [TokenWord]),
    ReplayBuilder {
        lane: &'a crate::input::ReplayLane<G>,
        builder: crate::input::ReplayInputBuilderId<G>,
        len: u32,
    },
}

impl<G> ScannedWords<'_, G> {
    fn len(&self) -> usize {
        match self {
            Self::Traced(words) => words.len(),
            Self::Semantic(words) => words.len(),
            Self::ReplayBuilder { len, .. } => *len as usize,
        }
    }

    fn token(&self, index: usize) -> Option<Token> {
        match self {
            Self::Traced(words) => words.get(index).map(|word| word.semantic_token()),
            Self::Semantic(words) => words.get(index).map(|word| word.semantic_token()),
            Self::ReplayBuilder { lane, builder, .. } => lane
                .input_builder_get(*builder, index)
                .map(|word| word.semantic_token()),
        }
    }
}

/// Exact command-owned continuation of one host-suspended token collector.
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct PendingScanToks<G> {
    /// Exact parent suffix before either mutable sink was admitted.
    ///
    /// Successful completion publishes the sinks to the parent operation.
    /// Cancellation or failed continuation publication first closes the live
    /// scanner scope, then truncates through this mark so no unreachable sink
    /// row survives the failed transaction.
    attempt_opening: AttemptMark,
    scope: crate::attempt::OwnedAttemptScope,
    collector: TokenCollector<G>,
    /// First deferred diagnostic which can belong to this scanner episode.
    ///
    /// A resource suspension retains the cursor beside the scanner sinks, so
    /// completion never mistakes an older, still-unpublished runaway report
    /// for recovery produced by the resumed scan.
    diagnostic_start: usize,
    config: ScanToksConfig,
    episode: ScannerEpisode,
    phase: PendingScanToksPhase<G>,
}

impl<G> PendingScanToks<G> {
    pub(crate) fn macro_definition_target(&self, expanded: bool) -> Option<Symbol> {
        let expected = if expanded {
            ScanToksExpansion::Expanded
        } else {
            ScanToksExpansion::Unexpanded
        };
        match (
            self.config.grammar,
            self.config.owner,
            self.config.expansion,
        ) {
            (
                ScanToksGrammar::MacroDefinition,
                ScanToksOwner::Definition(Some(target)),
                expansion,
            ) if expansion == expected => Some(target),
            _ => None,
        }
    }

    fn take_child(&mut self) -> Option<crate::execution_scratch::ScannerFrameKey<G>> {
        match &mut self.phase {
            PendingScanToksPhase::Opening { child } => child.take().map(|child| child.restore().0),
            PendingScanToksPhase::Replacement { progress, .. } => progress
                .pending_expansion
                .as_mut()
                .and_then(|pending| pending.child.take())
                .map(|child| child.restore().0),
        }
    }
}

// Every pending field is a scalar, a typed attempt or generation coordinate,
// or an ephemeral current-command value. The storage owners remain on
// `CommandState`; no continuation borrows its accumulated token buffer.
// Replacement state stays inline so suspension reuses the scratch lane rather
// than allocating a box per scanner frame.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Eq, PartialEq)]
enum PendingScanToksPhase<G> {
    Opening {
        child: Option<crate::execution_scratch::ChildContinuation<G, ScanToksChildDestination>>,
    },
    Replacement {
        macro_parameters: Option<(u8, Option<Symbol>)>,
        hash_brace: Option<TracedTokenWord>,
        primary: OriginId,
        malformed_parameter: bool,
        progress: ReplacementProgress<G>,
    },
}

impl<G> PendingScanToksPhase<G> {
    fn take_child(&mut self) -> Option<crate::execution_scratch::ScannerFrameKey<G>> {
        match self {
            Self::Opening { child } => child.take().map(|child| child.restore().0),
            Self::Replacement { progress, .. } => progress
                .pending_expansion
                .as_mut()
                .and_then(|pending| pending.child.take())
                .map(|child| child.restore().0),
        }
    }

    fn retain_child(
        &mut self,
        baton: &mut Option<crate::execution_scratch::ScannerFrameKey<G>>,
    ) -> Result<(), CommandError> {
        match self {
            Self::Opening {
                child: owner @ None,
            } => {
                *owner = crate::execution_scratch::ChildContinuation::capture(
                    baton,
                    ScanToksChildDestination::Opening,
                );
                Ok(())
            }
            Self::Opening { child: Some(_) } if baton.is_none() => Ok(()),
            Self::Replacement { .. } if baton.is_none() => Ok(()),
            Self::Opening { child: Some(_) } | Self::Replacement { .. } => {
                Err(CommandError::input_invariant())
            }
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
struct ReplacementProgress<G> {
    pending_expansion: Option<PendingCollectorExpansion<G>>,
}

impl<G> ReplacementProgress<G> {
    fn new() -> Self {
        Self {
            pending_expansion: None,
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
struct PendingCollectorExpansion<G> {
    command: Option<crate::CurrentCommand<G>>,
    route: CollectorExpansionRoute,
    operand: Option<crate::CurrentCommand<G>>,
    child: Option<crate::execution_scratch::ChildContinuation<G, CollectorExpansionRoute>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ScanToksChildDestination {
    Opening,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum CollectorExpansionRoute {
    Ordinary,
    The,
    Unexpanded,
    Detokenize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CollectorExpansionOutcome {
    Expanded,
    Retained,
}

#[inline(always)]
fn clear_command_destination<G>(destination: &mut Option<crate::CurrentCommand<G>>) {
    *destination = None;
}

impl ScanToksConfig {
    fn parse(mode: ScanToksMode) -> Self {
        let expansion = |expanded| {
            if expanded {
                ScanToksExpansion::Expanded
            } else {
                ScanToksExpansion::Unexpanded
            }
        };
        let balanced_purpose = |expanded| {
            if expanded {
                ScanToksPurpose::ExpandedBalanced
            } else {
                ScanToksPurpose::Balanced
            }
        };
        match mode {
            ScanToksMode::General { expanded } => Self {
                grammar: ScanToksGrammar::General,
                destination: ScanToksDestination::Attempt,
                opening: ScanToksOpening::Required,
                expansion: expansion(expanded),
                owner: ScanToksOwner::Absorbed(None),
                purpose: balanced_purpose(expanded),
                status_visibility: ScannerStatusVisibility::Observed,
            },
            ScanToksMode::GeneralFor { expanded, owner } => Self {
                grammar: ScanToksGrammar::General,
                destination: ScanToksDestination::Attempt,
                opening: ScanToksOpening::Required,
                expansion: expansion(expanded),
                owner: ScanToksOwner::Absorbed(Some(owner)),
                purpose: balanced_purpose(expanded),
                status_visibility: ScannerStatusVisibility::Observed,
            },
            ScanToksMode::GeneralAfterOpening {
                expanded,
                primary,
                owner,
            } => Self {
                grammar: ScanToksGrammar::General,
                destination: ScanToksDestination::Attempt,
                opening: ScanToksOpening::Prevalidated { primary },
                expansion: expansion(expanded),
                owner: ScanToksOwner::Absorbed(owner),
                purpose: balanced_purpose(expanded),
                status_visibility: ScannerStatusVisibility::Observed,
            },
            ScanToksMode::GeneralText { purpose } => Self {
                grammar: ScanToksGrammar::General,
                destination: ScanToksDestination::Attempt,
                opening: ScanToksOpening::Required,
                expansion: ScanToksExpansion::Unexpanded,
                owner: ScanToksOwner::Absorbed(None),
                purpose: ScanToksPurpose::GeneralText(purpose),
                status_visibility: ScannerStatusVisibility::Hidden,
            },
            ScanToksMode::EscapingGeneralText { purpose } => Self {
                grammar: ScanToksGrammar::General,
                destination: ScanToksDestination::ReplayInput,
                opening: ScanToksOpening::Required,
                expansion: ScanToksExpansion::Unexpanded,
                owner: ScanToksOwner::Absorbed(None),
                purpose: ScanToksPurpose::GeneralText(purpose),
                status_visibility: ScannerStatusVisibility::Hidden,
            },
            ScanToksMode::MacroDefinition { expanded } => Self {
                grammar: ScanToksGrammar::MacroDefinition,
                destination: ScanToksDestination::Attempt,
                opening: ScanToksOpening::AfterParameterText,
                expansion: expansion(expanded),
                owner: ScanToksOwner::Definition(None),
                purpose: ScanToksPurpose::MacroReplacement,
                status_visibility: ScannerStatusVisibility::Observed,
            },
            ScanToksMode::MacroDefinitionFor { expanded, target } => Self {
                grammar: ScanToksGrammar::MacroDefinition,
                destination: ScanToksDestination::Attempt,
                opening: ScanToksOpening::AfterParameterText,
                expansion: expansion(expanded),
                owner: ScanToksOwner::Definition(Some(target)),
                purpose: ScanToksPurpose::MacroReplacement,
                status_visibility: ScannerStatusVisibility::Observed,
            },
        }
    }

    const fn scanner_status(
        self,
        builder: TokenBuilderId,
        warning: ScannerWarning,
    ) -> ScannerStatus {
        match self.owner {
            ScanToksOwner::Absorbed(owner) => ScannerStatus::Absorbing(AbsorbingContext {
                owner,
                builder,
                warning,
            }),
            ScanToksOwner::Definition(target) => ScannerStatus::Defining(DefinitionContext {
                target,
                builder,
                warning,
            }),
        }
    }
}

/// Frozen output of one `scan_toks` episode.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ScannedToks {
    pub(crate) parameter_text: AttemptTokenListId,
    pub(crate) replacement_text: AttemptTokenListId,
    pub(crate) primary: OriginId,
    pub(crate) malformed_parameter: bool,
}

/// Scanner-completed buffers before an owning publication policy is chosen.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ScannedToksBuffers<G> {
    storage: ScannedToksStorage<G>,
    pub(crate) primary: OriginId,
    pub(crate) malformed_parameter: bool,
}

impl<G> ScannedToksBuffers<G> {
    pub(crate) fn definition(self) -> Option<AttemptDefinitionId> {
        match self.storage {
            ScannedToksStorage::Definition(definition) => Some(definition),
            ScannedToksStorage::Tokens { .. }
            | ScannedToksStorage::ReplayInputBuilder { .. }
            | ScannedToksStorage::ReplayInput { .. } => None,
        }
    }
}

/// TeX82 §403's result after a mandatory left-brace scan.
#[derive(Debug)]
// Boxing the delivered command would add a heap allocation to the scanner's
// ordinary left-brace path; the large arm is consumed immediately.
#[allow(clippy::large_enum_variant)]
pub(crate) enum ScannedLeftBrace<G> {
    /// A real source or replay token supplied the brace.
    Consumed(crate::CurrentCommand<G>),
    /// §403 inserted the brace after backing up the offending command.
    Inserted,
}

impl<G> ScannedLeftBrace<G> {
    pub(crate) fn origin(&self) -> OriginId {
        match self {
            Self::Consumed(command) => command.origin(),
            Self::Inserted => OriginId::UNKNOWN,
        }
    }
}

struct ScannedParameterText {
    highest_parameter: u8,
    hash_brace: Option<TracedTokenWord>,
    primary: OriginId,
    malformed_parameter: bool,
    missing_left_brace: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MacroParameterDiagnostic {
    NonconsecutiveNumber,
    TooManyParameters,
    IllegalReplacementNumber { target: Option<Symbol> },
}

const FILE_ENDED_WITHIN_READ_DIAGNOSTIC: u64 = 0x7265_6164_0000_0486;

impl<G> CommandProcessor<'_, '_, G> {
    /// Consumes one structurally owned suspension chain deepest-first.
    ///
    /// Scanner scopes are attempt-arena suffix owners, so their children must
    /// be closed before the caller scope can be discarded. Expansion frames
    /// own no arena scope themselves and only forward that exact child edge.
    pub(crate) fn abort_continuation(
        &mut self,
        key: crate::execution_scratch::ScannerFrameKey<G>,
    ) -> Result<(), CommandError> {
        let frame = self
            .command
            .scratch
            .take_continuation_frame(key)
            .map_err(scratch_command_error)?;
        match frame {
            crate::execution_scratch::ContinuationFrame::Scanner(pending) => {
                self.settle_failed_scan_toks(pending)
            }
            crate::execution_scratch::ContinuationFrame::Scalar(mut pending) => {
                let expression_stack_mark = pending.expression_stack_mark();
                if let Some(child) = pending.take_child() {
                    self.abort_continuation(child)?;
                }
                if let Some(mark) = expression_stack_mark {
                    self.command
                        .scratch
                        .truncate_expression_stack(mark)
                        .map_err(scratch_command_error)?;
                }
                Ok(())
            }
            crate::execution_scratch::ContinuationFrame::Expansion(key) => {
                let mut pending = self
                    .command
                    .scratch
                    .cancel_expansion(key)
                    .map_err(scratch_command_error)?;
                if let Some(child) = pending.take_child() {
                    self.abort_continuation(child)?;
                }
                Ok(())
            }
            crate::execution_scratch::ContinuationFrame::ExpandAfter(mut pending) => {
                if let Some(child) = pending.take_child() {
                    self.abort_continuation(child)?;
                }
                Ok(())
            }
            crate::execution_scratch::ContinuationFrame::PdfStringCompare(mut pending) => {
                if let Some(child) = pending.take_child() {
                    self.abort_continuation(child)?;
                }
                Ok(())
            }
            crate::execution_scratch::ContinuationFrame::AlignmentPreamble(pending) => {
                self.abort_alignment_preamble(pending)
            }
            crate::execution_scratch::ContinuationFrame::StructuredScanner(mut pending) => {
                if let Some(child) = pending.take_child() {
                    self.abort_continuation(child)?;
                }
                Ok(())
            }
        }
    }

    fn allocate_attempt_token_list(
        &mut self,
        words: impl IntoIterator<Item = TracedTokenWord>,
    ) -> Result<AttemptTokenListId, CommandError> {
        self.command
            .attempt
            .arena_mut()
            .allocate_token_list(words)
            .map_err(attempt_command_error)
    }

    fn begin_scan_toks_collector(
        &mut self,
        grammar: ScanToksGrammar,
        destination: ScanToksDestination,
    ) -> Result<TokenCollector<G>, CommandError> {
        let collector = match (grammar, destination) {
            (ScanToksGrammar::General, ScanToksDestination::Attempt) => {
                let parameter = self
                    .command
                    .attempt
                    .arena_mut()
                    .allocate_token_buffer()
                    .map_err(attempt_command_error)?;
                let replacement = self
                    .command
                    .attempt
                    .arena_mut()
                    .allocate_token_buffer()
                    .map_err(attempt_command_error)?;
                TokenCollector::token_buffers(parameter, replacement)
            }
            (ScanToksGrammar::General, ScanToksDestination::ReplayInput) => {
                let builder = self
                    .command
                    .roots
                    .input
                    .replay
                    .begin_input_builder()
                    .map_err(scratch_command_error)?;
                TokenCollector::replay_input(builder)
            }
            (ScanToksGrammar::MacroDefinition, ScanToksDestination::Attempt) => {
                let definition = self
                    .command
                    .attempt
                    .arena_mut()
                    .allocate_definition_builder(self.state.definition_identity_policy())
                    .map_err(attempt_command_error)?;
                TokenCollector::definition(definition)
            }
            (ScanToksGrammar::MacroDefinition, ScanToksDestination::ReplayInput) => {
                return Err(CommandError::input_invariant());
            }
        };
        #[cfg(test)]
        {
            self.command
                .token_collector_path_counters
                .collectors_started += 1;
        }
        Ok(collector)
    }

    fn discard_scan_toks_collector(
        &mut self,
        collector: &TokenCollector<G>,
    ) -> Result<(), CommandError> {
        if let TokenCollectorDestination::ReplayInput { builder } = collector.destination() {
            self.command
                .roots
                .input
                .replay
                .discard_input_builder(*builder)
                .map_err(scratch_command_error)?;
        }
        Ok(())
    }

    fn push_scan_toks_word(
        &mut self,
        collector: &mut TokenCollector<G>,
        word: TracedTokenWord,
    ) -> Result<(), CommandError> {
        if collector.phase() == TokenCollectorPhase::Complete {
            return Err(CommandError::input_invariant());
        }
        let result: Result<(), CommandError> = match collector.destination() {
            TokenCollectorDestination::TokenBuffers { writer, .. } => self
                .command
                .attempt
                .arena_mut()
                .push_buffer_token(*writer, word)
                .map_err(attempt_command_error),
            TokenCollectorDestination::Definition {
                definition,
                writing_replacement: false,
            } => self
                .command
                .attempt
                .arena_mut()
                .push_definition_parameter(*definition, word.token_word())
                .map_err(attempt_command_error),
            TokenCollectorDestination::Definition {
                definition,
                writing_replacement: true,
            } => self
                .command
                .attempt
                .arena_mut()
                .push_definition_replacement(*definition, word.token_word())
                .map_err(attempt_command_error),
            TokenCollectorDestination::ReplayInput { builder } => self
                .command
                .roots
                .input
                .replay
                .push_input_builder_word(*builder, word)
                .map_err(scratch_command_error),
            TokenCollectorDestination::MacroArgument { .. } => {
                return Err(CommandError::input_invariant());
            }
        };
        result?;
        #[cfg(test)]
        {
            self.command.token_collector_path_counters.collector_appends += 1;
        }
        Ok(())
    }

    fn finish_scan_toks_parameters(
        &mut self,
        collector: &mut TokenCollector<G>,
    ) -> Result<(), CommandError> {
        if collector.phase() != TokenCollectorPhase::Parameter {
            return Err(CommandError::input_invariant());
        }
        match collector.destination_mut() {
            TokenCollectorDestination::TokenBuffers {
                writer,
                replacement,
                parameter_result,
            } => {
                *parameter_result = Some(
                    self.command
                        .attempt
                        .arena_mut()
                        .finish_token_buffer(*writer)
                        .map_err(attempt_command_error)?,
                );
                *writer = *replacement;
            }
            TokenCollectorDestination::Definition {
                definition,
                writing_replacement,
            } if !*writing_replacement => {
                self.command
                    .attempt
                    .arena_mut()
                    .finish_definition_parameters(*definition)
                    .map_err(attempt_command_error)?;
                *writing_replacement = true;
            }
            TokenCollectorDestination::ReplayInput { .. } => {}
            _ => return Err(CommandError::input_invariant()),
        }
        collector
            .begin_replacement()
            .map_err(|()| CommandError::input_invariant())?;
        #[cfg(test)]
        {
            self.command.token_collector_path_counters.phase_transitions += 1;
        }
        Ok(())
    }

    fn finish_scan_toks_collector(
        &mut self,
        collector: &mut TokenCollector<G>,
    ) -> Result<ScannedToksStorage<G>, CommandError> {
        if collector.phase() != TokenCollectorPhase::Replacement {
            return Err(CommandError::input_invariant());
        }
        let storage = match collector.destination() {
            TokenCollectorDestination::TokenBuffers {
                writer,
                replacement,
                parameter_result: Some(parameter),
            } if writer == replacement => ScannedToksStorage::Tokens {
                parameter: *parameter,
                replacement: self
                    .command
                    .attempt
                    .arena_mut()
                    .finish_token_buffer(*replacement)
                    .map_err(attempt_command_error)?,
            },
            TokenCollectorDestination::Definition {
                definition,
                writing_replacement: true,
            } => {
                self.command
                    .attempt
                    .arena_mut()
                    .finish_definition(*definition)
                    .map_err(attempt_command_error)?;
                ScannedToksStorage::Definition(*definition)
            }
            TokenCollectorDestination::ReplayInput { builder } => {
                let len = self
                    .command
                    .roots
                    .input
                    .replay
                    .input_builder_len(*builder)
                    .ok_or_else(CommandError::input_invariant)?;
                ScannedToksStorage::ReplayInputBuilder {
                    builder: *builder,
                    len,
                }
            }
            _ => return Err(CommandError::input_invariant()),
        };
        collector
            .complete()
            .map_err(|()| CommandError::input_invariant())?;
        #[cfg(test)]
        {
            self.command.token_collector_path_counters.settlements += 1;
        }
        Ok(storage)
    }

    fn scanned_parameter_words(
        &self,
        scanned: &ScannedToksBuffers<G>,
    ) -> Result<ScannedWords<'_, G>, CommandError> {
        let arena = self.command.attempt.arena();
        match scanned.storage {
            ScannedToksStorage::Tokens { parameter, .. } => arena
                .token_words(parameter)
                .map(ScannedWords::Traced)
                .map_err(attempt_command_error),
            ScannedToksStorage::Definition(definition) => arena
                .definition_parameter_words(definition)
                .map(ScannedWords::Semantic)
                .map_err(attempt_command_error),
            ScannedToksStorage::ReplayInputBuilder { .. }
            | ScannedToksStorage::ReplayInput { .. } => Err(CommandError::input_invariant()),
        }
    }

    fn scanned_replacement_words(
        &self,
        scanned: &ScannedToksBuffers<G>,
    ) -> Result<ScannedWords<'_, G>, CommandError> {
        let arena = self.command.attempt.arena();
        match scanned.storage {
            ScannedToksStorage::Tokens { replacement, .. } => arena
                .token_words(replacement)
                .map(ScannedWords::Traced)
                .map_err(attempt_command_error),
            ScannedToksStorage::Definition(definition) => arena
                .definition_replacement_words(definition)
                .map(ScannedWords::Semantic)
                .map_err(attempt_command_error),
            ScannedToksStorage::ReplayInputBuilder { builder, len } => {
                Ok(ScannedWords::ReplayBuilder {
                    lane: &self.command.roots.input.replay,
                    builder,
                    len,
                })
            }
            ScannedToksStorage::ReplayInput { .. } => Err(CommandError::input_invariant()),
        }
    }

    fn attempt_words(
        &self,
        list: AttemptTokenListId,
    ) -> Result<crate::attempt::AttemptTokenListView<'_>, CommandError> {
        self.command
            .attempt
            .arena()
            .token_words(list)
            .map_err(attempt_command_error)
    }

    /// TeX.web's special token-list collector (parts 26--27).
    ///
    /// `expanded` means one `get_next`/`expand` step per iteration, never a
    /// call to the ordinary expanded delivery loop.  That distinction keeps
    /// the collector's closing brace inaccessible to expansion that happens
    /// to retire an inserted replay level.
    pub(crate) fn scan_toks(&mut self, mode: ScanToksMode) -> Result<ScannedToks, CommandError> {
        let result = self.scan_toks_buffers(mode)?;
        let ScannedToksStorage::Tokens {
            parameter: parameter_text,
            replacement: replacement_text,
        } = result.storage
        else {
            return Err(CommandError::input_invariant());
        };
        Ok(ScannedToks {
            parameter_text,
            replacement_text,
            primary: result.primary,
            malformed_parameter: result.malformed_parameter,
        })
    }

    pub(crate) fn scan_toks_buffers(
        &mut self,
        mode: ScanToksMode,
    ) -> Result<ScannedToksBuffers<G>, CommandError> {
        let config = ScanToksConfig::parse(mode);
        let resumed = match self.scanner_resume.take() {
            Some(key) => Some(
                self.command
                    .scratch
                    .take_scanner_frame(key)
                    .map_err(scratch_command_error)?,
            ),
            None => None,
        };
        let mut pending = match resumed {
            Some(pending) if pending.config == config => pending,
            Some(pending) => {
                self.settle_failed_scan_toks(pending)?;
                return Err(CommandError::input_invariant());
            }
            None => {
                // The result destination belongs to the logical parent, not
                // the scanner's scratch child. Returning a synchronous child
                // to a still-live macro can therefore truncate its suffix
                // without copying or invalidating the completed result.
                let attempt_opening = self.command.attempt.arena().mark();
                let collector =
                    match self.begin_scan_toks_collector(config.grammar, config.destination) {
                        Ok(collector) => collector,
                        Err(error) => {
                            self.command
                                .attempt
                                .arena_mut()
                                .truncate(attempt_opening)
                                .map_err(attempt_command_error)?;
                            return Err(error);
                        }
                    };
                let scope = match self.command.begin_attempt_scanner_scope() {
                    Ok(scope) => scope,
                    Err(error) => {
                        self.discard_scan_toks_collector(&collector)?;
                        self.command
                            .attempt
                            .arena_mut()
                            .truncate(attempt_opening)
                            .map_err(attempt_command_error)?;
                        return Err(attempt_command_error(error));
                    }
                };
                let builder = TokenBuilderId(self.command.transient.next_builder_identity);
                self.command.transient.next_builder_identity =
                    self.command.transient.next_builder_identity.wrapping_add(1);
                let warning = ScannerWarning(builder.0);
                PendingScanToks {
                    attempt_opening,
                    scope,
                    collector,
                    diagnostic_start: self.command.semantic_diagnostics.len(),
                    config,
                    episode: self.begin_scanner_episode(
                        config.scanner_status(builder, warning),
                        config.status_visibility,
                    ),
                    phase: PendingScanToksPhase::Opening { child: None },
                }
            }
        };
        let result = self.scan_toks_inner(
            pending.config,
            &mut pending.collector,
            &pending.episode,
            &mut pending.phase,
        );
        let mut result = match result {
            Ok(result) => result,
            Err(error) if error.is_resource_suspension() => {
                if let Err(error) = pending.phase.retain_child(&mut self.scanner_resume) {
                    self.settle_failed_scan_toks(pending)?;
                    return Err(error);
                }
                if self.scanner_resume.is_some() {
                    self.settle_failed_scan_toks(pending)?;
                    return Err(CommandError::input_invariant());
                }
                let mut pending = Some(pending);
                let key = match self.command.scratch.store_scanner_frame(&mut pending) {
                    Ok(key) => key,
                    Err(error) => {
                        self.settle_failed_scan_toks(
                            pending
                                .take()
                                .expect("failed scratch insertion retains the scanner owner"),
                        )?;
                        return Err(scratch_command_error(error));
                    }
                };
                debug_assert!(pending.is_none());
                #[cfg(test)]
                if self.command.scratch.take_scan_toks_publication_collision() {
                    debug_assert!(self.scanner_resume.is_none());
                    self.scanner_resume = Some(
                        crate::execution_scratch::ScannerFrameKey::injected_scan_toks_publication_collision(),
                    );
                }
                if let Some(displaced) = self.scanner_resume.replace(key) {
                    let parked = self
                        .scanner_resume
                        .replace(displaced)
                        .expect("publication collision installed the new scanner key");
                    let pending = self
                        .command
                        .scratch
                        .take_scanner_frame(parked)
                        .map_err(scratch_command_error)?;
                    self.settle_failed_scan_toks(pending)?;
                    return Err(CommandError::input_invariant());
                }
                return Err(error);
            }
            Err(error) => {
                if let Some(child) = pending.phase.take_child() {
                    self.abort_continuation(child)?;
                }
                self.finish_scanner_episode(pending.episode);
                self.discard_scan_toks_collector(&pending.collector)?;
                self.command
                    .discard_attempt_scope_suffix(pending.scope)
                    .map_err(attempt_command_error)?;
                self.command
                    .attempt
                    .arena_mut()
                    .truncate(pending.attempt_opening)
                    .map_err(attempt_command_error)?;
                return Err(error);
            }
        };
        if let Some(child) = self.scanner_resume.take() {
            self.abort_continuation(child)?;
            self.finish_scanner_episode(pending.episode);
            self.discard_scan_toks_collector(&pending.collector)?;
            self.command
                .discard_attempt_scope_suffix(pending.scope)
                .map_err(attempt_command_error)?;
            self.command
                .attempt
                .arena_mut()
                .truncate(pending.attempt_opening)
                .map_err(attempt_command_error)?;
            return Err(CommandError::input_invariant());
        }
        self.render_scan_toks_runaway_if_recovered(
            pending.config,
            pending.diagnostic_start,
            &result,
        )?;
        self.finish_scanner_episode(pending.episode);
        let completed_tokens = if !self.is_observed() {
            Vec::new()
        } else if pending.config.purpose.renders_detokenized_result() {
            let words = self.scanned_replacement_words(&result)?;
            let mut text = String::new();
            for index in 0..words.len() {
                if let Some(token) = words.token(index) {
                    self.state.append_token_string_text(token, &mut text);
                }
            }
            text.chars()
                .map(|ch| {
                    self.observed_token(TracedTokenWord::pack(
                        Token::Char {
                            ch,
                            cat: if ch == ' ' {
                                Catcode::Space
                            } else {
                                Catcode::Other
                            },
                        },
                        OriginId::UNKNOWN,
                    ))
                })
                .collect()
        } else {
            let words = self.scanned_replacement_words(&result)?;
            (0..words.len())
                .filter_map(|index| words.token(index))
                .map(|token| self.observed_token(TracedTokenWord::pack(token, OriginId::UNKNOWN)))
                .collect()
        };
        observe!(
            self,
            CommandObservation::TokenList(TokenListRecord {
                transition: "complete",
                purpose: pending.config.purpose.canonical_name(),
                tokens: completed_tokens,
            }),
        );
        self.command
            .defer_attempt_scope_retirement(pending.scope)
            .map_err(attempt_command_error)?;
        if let ScannedToksStorage::ReplayInputBuilder { builder, .. } = result.storage {
            let span = match self
                .command
                .roots
                .input
                .replay
                .finish_input_builder(builder)
            {
                Ok(span) => span,
                Err(error) => {
                    self.command
                        .roots
                        .input
                        .replay
                        .discard_input_builder(builder)
                        .map_err(scratch_command_error)?;
                    return Err(scratch_command_error(error));
                }
            };
            let PackedTokenSpanHandle::Replay { replay, len } = span else {
                return Err(CommandError::input_invariant());
            };
            result.storage = ScannedToksStorage::ReplayInput { replay, len };
        }
        Ok(result)
    }

    /// Rejects one unpublished scanner continuation deepest-first.
    ///
    /// The scanner scope opens after its parent-owned result sinks so normal
    /// completion can return them without a copy. Failure therefore closes
    /// the scope first and separately truncates the exact pre-sink suffix.
    fn settle_failed_scan_toks(
        &mut self,
        mut pending: PendingScanToks<G>,
    ) -> Result<(), CommandError> {
        if let Some(child) = pending.take_child() {
            self.abort_continuation(child)?;
        }
        self.finish_scanner_episode(pending.episode);
        self.discard_scan_toks_collector(&pending.collector)?;
        self.command
            .discard_attempt_scope_suffix(pending.scope)
            .map_err(attempt_command_error)?;
        self.command
            .attempt
            .arena_mut()
            .truncate(pending.attempt_opening)
            .map_err(attempt_command_error)
    }

    /// Completes TeX82 §306's partial-list display only when this exact
    /// scanner episode produced a runaway report.
    ///
    /// Ordinary successful scans never enter the renderer. Recovery borrows
    /// the already-collected scanner buffers, walks each word once, and writes
    /// directly into the report's final selector-aware string; it neither
    /// copies the token lists nor creates a diagnostic staging buffer.
    fn render_scan_toks_runaway_if_recovered(
        &mut self,
        config: ScanToksConfig,
        diagnostic_start: usize,
        result: &ScannedToksBuffers<G>,
    ) -> Result<(), CommandError> {
        let expected_heading = match config.grammar {
            ScanToksGrammar::General => "Runaway text?",
            ScanToksGrammar::MacroDefinition => "Runaway definition?",
        };
        let Some(diagnostic_index) = self
            .command
            .semantic_diagnostics
            .get(diagnostic_start..)
            .and_then(|diagnostics| {
                diagnostics.iter().rposition(|diagnostic| {
                    matches!(
                        diagnostic,
                        crate::CommandSemanticDiagnostic::Recoverable {
                            identity: crate::processor::RUNAWAY_SCAN_DIAGNOSTIC,
                            runaway: Some(crate::state::RunawayPrelude { heading, .. }),
                            ..
                        } if *heading == expected_heading
                    )
                })
            })
            .map(|relative| diagnostic_start + relative)
        else {
            return Ok(());
        };

        #[cfg(test)]
        RUNAWAY_RENDER_COUNT.with(|count| count.set(count.get().saturating_add(1)));

        let parameter_text = matches!(config.grammar, ScanToksGrammar::MacroDefinition)
            .then(|| self.scanned_parameter_words(result))
            .transpose()?;
        let replacement_text = self.scanned_replacement_words(result)?;
        let mut partial = String::new();
        let mut match_marker = '#';
        if let Some(parameter_text) = parameter_text {
            append_runaway_words(self.state, &parameter_text, &mut match_marker, &mut partial);
            append_runaway_character(self.state, '-', &mut partial);
            append_runaway_character(self.state, '>', &mut partial);
        }
        append_runaway_words(
            self.state,
            &replacement_text,
            &mut match_marker,
            &mut partial,
        );

        let Some(crate::CommandSemanticDiagnostic::Recoverable {
            runaway: Some(runaway),
            ..
        }) = self.command.semantic_diagnostics.get_mut(diagnostic_index)
        else {
            return Err(CommandError::input_invariant());
        };
        runaway.partial = partial;
        Ok(())
    }

    // The stationary phase moves directly into reusable scratch only when a
    // real resource suspension leaves this synchronous scanner invocation.
    fn scan_toks_inner(
        &mut self,
        config: ScanToksConfig,
        collector: &mut TokenCollector<G>,
        episode: &ScannerEpisode,
        phase: &mut PendingScanToksPhase<G>,
    ) -> Result<ScannedToksBuffers<G>, CommandError> {
        // `macro_parameters` is TeX82 §477's `macro_def` flag carried together
        // with §479's `t`: `Some(highest)` selects the parameter-character
        // rule and bounds a legal parameter number, `None` leaves parameter
        // characters as ordinary text (`\message`, `\write`, `\toks`, ...).
        if let PendingScanToksPhase::Opening { child } = phase {
            if let Some(child) = child.take() {
                let (key, destination) = child.restore();
                if destination != ScanToksChildDestination::Opening {
                    return Err(CommandError::input_invariant());
                }
                self.install_scanner_resume(Some(key));
            }
            let (macro_parameters, hash_brace, primary, malformed_parameter, missing_left_brace) =
                match (config.grammar, config.opening) {
                    (ScanToksGrammar::General, ScanToksOpening::Required) => {
                        // TeX scans the required opening brace through the ordinary
                        // expanded path even when the replacement text itself is
                        // collected unexpanded.
                        let opening = self.scan_left_brace(true)?;
                        let primary = opening.origin();
                        self.finish_scan_toks_parameters(collector)?;
                        (None, None, primary, false, false)
                    }
                    (ScanToksGrammar::General, ScanToksOpening::Prevalidated { primary }) => {
                        // The opening command was already classified through
                        // `get_x_token` by §1227 and backed up solely so the
                        // absorbing scanner status precedes its replay. Preserve
                        // that semantic classification here: a `\let` alias for
                        // `{` is spelled as a control sequence, but it is still
                        // the one opening command this mode is required to
                        // consume. Requiring a literal begin-group spelling
                        // would mistake the following body token for a second
                        // opening delimiter after the alias replay.
                        let mut opening = None;
                        let status = self.get_token_into(&mut opening)?;
                        if status != crate::DeliveryStatus::Command {
                            return Err(CommandError::input_invariant());
                        }
                        let opening = opening.expect("command status initializes destination");
                        if !matches!(
                            opening.meaning(),
                            ResolvedMeaning::Static(Meaning::CharToken {
                                cat: Catcode::BeginGroup,
                                ..
                            })
                        ) {
                            return Err(CommandError::input_invariant());
                        }
                        self.observe_expanded_delivery(&opening);
                        self.finish_scan_toks_parameters(collector)?;
                        (None, None, primary, false, false)
                    }
                    (ScanToksGrammar::MacroDefinition, ScanToksOpening::AfterParameterText) => {
                        let parameters = self.scan_parameter_text(collector)?;
                        (
                            Some((
                                parameters.highest_parameter,
                                match config.owner {
                                    ScanToksOwner::Definition(target) => target,
                                    ScanToksOwner::Absorbed(_) => unreachable!(),
                                },
                            )),
                            parameters.hash_brace,
                            parameters.primary,
                            parameters.malformed_parameter,
                            parameters.missing_left_brace,
                        )
                    }
                    _ => unreachable!("ScanToksConfig admits no other grammar/opening pair"),
                };
            if missing_left_brace {
                return Ok(ScannedToksBuffers {
                    storage: self.finish_scan_toks_collector(collector)?,
                    primary,
                    malformed_parameter,
                });
            }
            *phase = PendingScanToksPhase::Replacement {
                macro_parameters,
                hash_brace,
                primary,
                malformed_parameter,
                progress: ReplacementProgress::new(),
            };
        }
        let PendingScanToksPhase::Replacement {
            macro_parameters,
            hash_brace,
            primary,
            malformed_parameter,
            progress,
        } = phase
        else {
            unreachable!("opening phase is replaced before collection")
        };
        self.collect_replacement(
            config.expansion,
            *macro_parameters,
            episode,
            collector,
            progress,
        )?;
        // TeX's `#{` parameter-text special case treats that left brace as a
        // delimiter and appends the same saved brace after the replacement
        // text (TeX.web §476).
        if let Some(brace) = *hash_brace {
            self.push_scan_toks_word(collector, brace)?;
        }
        Ok(ScannedToksBuffers {
            storage: self.finish_scan_toks_collector(collector)?,
            primary: *primary,
            malformed_parameter: *malformed_parameter,
        })
    }

    /// TeX82 §403's `scan_left_brace`, the one routine every mandatory
    /// opening brace goes through. On a non-brace it reports the exact §403
    /// error, backs the rejected command up, installs the synthetic brace's
    /// `align_state` contribution, and returns normally just as TeX does.
    ///
    /// §403 opens with §404's "Get the next non-blank non-relax non-call
    /// token" -- `repeat get_x_token until (cur_cmd<>spacer)and
    /// (cur_cmd<>relax)` -- because, in §403's own words, "\TeX\ allows
    /// \.{\\relax} to appear before the |left_brace|". Skipping only spaces
    /// rejected the brace in `\message\relax{...}` and every plain-TeX idiom
    /// that parks a `\relax` in front of a mandatory group
    /// (`umber2-johp.209`).
    pub(crate) fn scan_left_brace(
        &mut self,
        expanded: bool,
    ) -> Result<ScannedLeftBrace<G>, CommandError> {
        let mut destination = None;
        loop {
            let status = if expanded {
                self.get_x_token_into(&mut destination)?
            } else {
                self.get_token_into(&mut destination)?
            };
            if status != crate::DeliveryStatus::Command {
                return Err(CommandError::input_invariant());
            }
            let command = destination
                .take()
                .expect("command status initializes destination");
            match command.meaning() {
                ResolvedMeaning::Static(Meaning::CharToken {
                    cat: Catcode::Space,
                    ..
                })
                | ResolvedMeaning::Static(Meaning::Relax) => continue,
                ResolvedMeaning::Static(Meaning::CharToken {
                    cat: Catcode::BeginGroup,
                    ..
                }) => return Ok(ScannedLeftBrace::Consumed(command)),
                _ => {
                    let deferred = {
                        let mut report = self.state.print_err("Missing { inserted");
                        report.help(&[
                            "A left brace was mandatory here, so I've put one in.",
                            "You might want to delete and/or insert some corrections",
                            "so that I will find a matching right brace soon.",
                            "(If you're confused by all this, try typing `I}' now.)",
                        ]);
                        report.defer()
                    };
                    // §403's `back_error` is `back_input; error`: the message
                    // and help are prepared above, then the rejected command
                    // is restored before §82 appends the period and help.
                    // Rendering §310's display only after the backup is what
                    // makes §314 name the rejected token on its own
                    // `<to be read again>␣` line.
                    self.back_input(command)?;
                    let context = self.command.output_open_context(self.state);
                    let mut report = self.state.resume_error_report(deferred);
                    report.context(context);
                    let outcome = report.error();
                    self.finish_error_outcome(outcome)?;
                    // §403 assigns `cur_cmd=left_brace` and increments
                    // `align_state` exactly as raw delivery of that synthetic
                    // brace would have done. The token itself is not pushed:
                    // the caller continues after it while the rejected command
                    // remains first on the backed-up input level.
                    self.command.record_alignment_phase();
                    self.command.alignment.align_state += 1;
                    return Ok(ScannedLeftBrace::Inserted);
                }
            }
        }
    }

    /// Scans the prefix before a macro replacement's compulsory opening
    /// brace.  Compact `Token::Param` values are the stored out-parameter
    /// representation; doubled hashes remain literal parameter characters.
    fn scan_parameter_text(
        &mut self,
        collector: &mut TokenCollector<G>,
    ) -> Result<ScannedParameterText, CommandError> {
        let mut next_parameter = 1_u8;
        let mut primary = OriginId::UNKNOWN;
        let mut malformed_parameter = false;
        let mut destination = None;
        loop {
            if self.get_token_into(&mut destination)? != crate::DeliveryStatus::Command {
                return Err(CommandError::input_invariant());
            }
            let command = destination
                .take()
                .expect("command status initializes destination");
            if primary == OriginId::UNKNOWN {
                primary = command.origin();
            }
            let token = self.classify_collector_token(&command, None);
            if token.spelling_is_begin_group() {
                self.finish_scan_toks_parameters(collector)?;
                return Ok(ScannedParameterText {
                    highest_parameter: next_parameter - 1,
                    hash_brace: None,
                    primary,
                    malformed_parameter,
                    missing_left_brace: false,
                });
            }
            if token.spelling_is_end_group() {
                // TeX82 §§475--476's `done1` branch has already consumed the
                // right brace and decremented `align_state`. It expresses
                // shock, restores that contribution, and finishes the
                // definition immediately with empty replacement text.
                observe!(
                    self,
                    CommandObservation::Diagnostic(DiagnosticRecord {
                        severity: "error",
                        diagnostic: "missing_macro_definition_left_brace",
                        arguments: Vec::new(),
                    }),
                );
                self.command.record_alignment_phase();
                self.command.alignment.align_state += 1;
                let context = self.command.output_open_context(self.state);
                let mut report = self.state.print_err("Missing { inserted");
                report
                    .help(&[
                        "Where was the left brace? You said something like `\\def\\a}',",
                        "which I'm going to interpret as `\\def\\a{}'.",
                    ])
                    .context(context);
                let outcome = report.error();
                self.finish_error_outcome(outcome)?;
                self.finish_scan_toks_parameters(collector)?;
                return Ok(ScannedParameterText {
                    highest_parameter: next_parameter - 1,
                    hash_brace: None,
                    primary,
                    malformed_parameter,
                    missing_left_brace: true,
                });
            }
            if !token.spelling_is_parameter() {
                self.push_scan_toks_word(collector, token.word())?;
                continue;
            }
            if self.get_token_into(&mut destination)? != crate::DeliveryStatus::Command {
                return Err(CommandError::input_invariant());
            }
            let follower = destination
                .take()
                .expect("command status initializes destination");
            let follower_token = self.classify_collector_token(&follower, None);
            if follower_token.spelling_is_begin_group() {
                self.push_scan_toks_word(collector, follower_token.word())?;
                self.finish_scan_toks_parameters(collector)?;
                return Ok(ScannedParameterText {
                    highest_parameter: next_parameter - 1,
                    hash_brace: Some(follower_token.word()),
                    primary,
                    malformed_parameter,
                    missing_left_brace: false,
                });
            }
            if let Some(number) = parameter_number(follower_token.spelling().semantic_token())
                && number == next_parameter
                && number <= 9
            {
                if let Token::Char {
                    ch,
                    cat: Catcode::Parameter,
                } = token.spelling().semantic_token()
                    && ch != '#'
                {
                    // TeX82 §476's match token retains `cur_chr`, i.e. the
                    // actual parameter-character code. Keep that spelling
                    // beside the compact slot token when it is not `#`.
                    self.push_scan_toks_word(collector, token.word())?;
                }
                self.push_scan_toks_word(
                    collector,
                    TracedTokenWord::pack(Token::Param(number), follower.origin()),
                )?;
                next_parameter += 1;
                continue;
            }
            if next_parameter > 9 {
                // TeX82 §476 has already consumed both the parameter
                // character and its follower when `t=#9`; it diagnoses and
                // returns to `continue` without backing either token up.
                self.report_macro_parameter_diagnostic(
                    MacroParameterDiagnostic::TooManyParameters,
                )?;
                malformed_parameter = true;
                continue;
            }
            // Canonical recovery keeps the rejected follower available and
            // supplies the expected parameter number.  The pending outer
            // validity operation remains responsible for all inaccessible
            // token recovery.
            self.back_input(follower)?;
            self.report_macro_parameter_diagnostic(MacroParameterDiagnostic::NonconsecutiveNumber)?;
            malformed_parameter = true;
            if next_parameter <= 9 {
                self.push_scan_toks_word(
                    collector,
                    TracedTokenWord::pack(Token::Param(next_parameter), command.origin()),
                )?;
                next_parameter += 1;
            }
        }
    }

    #[inline(always)]
    fn drive_collector_expansion(
        &mut self,
        route: CollectorExpansionRoute,
        episode: &ScannerEpisode,
        collector: &mut TokenCollector<G>,
        destination: &mut Option<crate::CurrentCommand<G>>,
        expansion_operand: &mut Option<crate::CurrentCommand<G>>,
        pending_expansion: &mut Option<PendingCollectorExpansion<G>>,
    ) -> Result<CollectorExpansionOutcome, CommandError> {
        if route == CollectorExpansionRoute::Ordinary && destination.is_none() {
            // A resumed generic expansion restores its sole parked command
            // into this destination itself. Every outcome either advances the
            // collector or returns its failure.
            return match self.expand_into(destination, true) {
                Ok(()) | Err(CommandError::ParagraphInMacroArgument) => {
                    clear_command_destination(destination);
                    Ok(CollectorExpansionOutcome::Expanded)
                }
                Err(CommandError::OuterInMacroArgument) => {
                    self.resume_scanner_episode_after_recovery(episode);
                    clear_command_destination(destination);
                    Ok(CollectorExpansionOutcome::Expanded)
                }
                Err(error) => {
                    if error.is_resource_suspension() {
                        *pending_expansion = Some(PendingCollectorExpansion {
                            command: destination.take(),
                            route,
                            operand: None,
                            child: crate::execution_scratch::ChildContinuation::capture(
                                &mut self.scanner_resume,
                                route,
                            ),
                        });
                    }
                    Err(error)
                }
            };
        }

        let command = destination
            .as_mut()
            .expect("direct collector expansion retains its command destination");
        if route == CollectorExpansionRoute::The {
            // TeX82 §478 handles `\the` directly in `scan_toks` instead of
            // routing it through §380's ordinary expanded-fetch loop.
            match self.append_direct_the_toks(collector, expansion_operand) {
                Ok(true) => {
                    clear_command_destination(destination);
                    return Ok(CollectorExpansionOutcome::Expanded);
                }
                Ok(false) => {}
                Err(error) => {
                    if error.is_resource_suspension() {
                        let command = destination
                            .take()
                            .expect("collector suspension retains its command");
                        *pending_expansion = Some(PendingCollectorExpansion {
                            command: Some(command),
                            route,
                            operand: expansion_operand.take(),
                            child: crate::execution_scratch::ChildContinuation::capture(
                                &mut self.scanner_resume,
                                route,
                            ),
                        });
                    }
                    return Err(error);
                }
            }
        }
        if route == CollectorExpansionRoute::Unexpanded {
            match self.append_unexpanded(collector) {
                Ok(()) => {
                    clear_command_destination(destination);
                    return Ok(CollectorExpansionOutcome::Expanded);
                }
                Err(error) => {
                    if error.is_resource_suspension() {
                        let command = destination
                            .take()
                            .expect("collector suspension retains its command");
                        *pending_expansion = Some(PendingCollectorExpansion {
                            command: Some(command),
                            route,
                            operand: None,
                            child: crate::execution_scratch::ChildContinuation::capture(
                                &mut self.scanner_resume,
                                route,
                            ),
                        });
                    }
                    return Err(error);
                }
            }
        }
        if route == CollectorExpansionRoute::Detokenize {
            match self.append_detokenize(collector) {
                Ok(()) => {
                    clear_command_destination(destination);
                    return Ok(CollectorExpansionOutcome::Expanded);
                }
                Err(error) => {
                    if error.is_resource_suspension() {
                        let command = destination
                            .take()
                            .expect("collector suspension retains its command");
                        *pending_expansion = Some(PendingCollectorExpansion {
                            command: Some(command),
                            route,
                            operand: None,
                            child: crate::execution_scratch::ChildContinuation::capture(
                                &mut self.scanner_resume,
                                route,
                            ),
                        });
                    }
                    return Err(error);
                }
            }
        }
        let protected = matches!(command.meaning_ref(), ResolvedMeaning::Macro { flags, .. } if flags.contains(MeaningFlags::PROTECTED));
        if protected {
            // e-TeX 2.6 change section [27.465] represents a protected macro
            // as `relax/no_expand_flag` for this collector iteration.
            observe!(
                self,
                CommandObservation::TokenList(TokenListRecord {
                    transition: "splice",
                    purpose: "protected_expansion_suppression",
                    tokens: vec![self.observed_token(command.spelling())],
                }),
            );
            command.suppress_expandable();
            return Ok(CollectorExpansionOutcome::Retained);
        }

        // TeX82 §394 returns from a failed macro call after either an ordinary
        // non-`\long` `\par` or §23's outer-validity recovery. Both return to
        // §380's get_x_token loop, which this collector owns while active.
        match self.expand_into(destination, true) {
            Ok(()) | Err(CommandError::ParagraphInMacroArgument) => {
                clear_command_destination(destination);
                Ok(CollectorExpansionOutcome::Expanded)
            }
            Err(CommandError::OuterInMacroArgument) => {
                self.resume_scanner_episode_after_recovery(episode);
                clear_command_destination(destination);
                Ok(CollectorExpansionOutcome::Expanded)
            }
            Err(error) => {
                if error.is_resource_suspension() {
                    *pending_expansion = Some(PendingCollectorExpansion {
                        command: destination.take(),
                        route,
                        operand: None,
                        child: crate::execution_scratch::ChildContinuation::capture(
                            &mut self.scanner_resume,
                            route,
                        ),
                    });
                }
                Err(error)
            }
        }
    }

    /// TeX82 §477, "Scan and build the body of the token list".
    ///
    /// `macro_parameters` is §477's `macro_def` guard: only a macro
    /// definition's body gives a parameter character its §479 meaning, and
    /// then `Some(highest)` is §479's `t` -- the highest parameter number the
    /// parameter text declared, and so the largest one a `#<digit>` may name.
    /// A body scanned for any other purpose (`\message`, `\write`, `\toks`,
    /// `\mark`, ...) stores parameter characters verbatim.
    fn collect_replacement(
        &mut self,
        expansion: ScanToksExpansion,
        macro_parameters: Option<(u8, Option<Symbol>)>,
        episode: &ScannerEpisode,
        collector: &mut TokenCollector<G>,
        progress: &mut ReplacementProgress<G>,
    ) -> Result<(), CommandError> {
        let ReplacementProgress { pending_expansion } = progress;
        let mut destination = None;

        // A parked expansion exists only after a real resource suspension.
        // Restore it once before entering §477's steady collection loop; the
        // ordinary path below consequently never probes continuation state.
        if let Some(mut pending) = pending_expansion.take() {
            if let Some(command) = pending.command.take() {
                self.resume_current_command(&command);
                destination = Some(command);
            }
            if let Some(child) = pending.child.take() {
                let (key, child_destination) = child.restore();
                if child_destination != pending.route {
                    return Err(CommandError::input_invariant());
                }
                self.scanner_resume = Some(key);
            }
            let mut expansion_operand = pending.operand.take();
            if self.drive_collector_expansion(
                pending.route,
                episode,
                collector,
                &mut destination,
                &mut expansion_operand,
                pending_expansion,
            )? != CollectorExpansionOutcome::Expanded
            {
                return Err(CommandError::input_invariant());
            }
        }

        loop {
            let delivery = if expansion.is_expanded() {
                self.get_next_into(&mut destination)
            } else {
                self.get_token_into(&mut destination)
            };
            match delivery {
                Ok(crate::DeliveryStatus::Command) => {}
                Ok(crate::DeliveryStatus::End) => {
                    return Err(CommandError::input_invariant());
                }
                Ok(_) => unreachable!("ordinary raw delivery has no side event"),
                Err(error) => return Err(error),
            }
            if expansion.is_expanded() && destination.as_ref().is_some_and(is_expandable_command) {
                let route = match destination
                    .as_ref()
                    .expect("command delivery initializes destination")
                    .meaning_ref()
                {
                    ResolvedMeaning::Static(Meaning::ExpandablePrimitive(
                        ExpandablePrimitive::The,
                    )) => CollectorExpansionRoute::The,
                    ResolvedMeaning::Static(Meaning::ExpandablePrimitive(
                        ExpandablePrimitive::Unexpanded,
                    )) => CollectorExpansionRoute::Unexpanded,
                    ResolvedMeaning::Static(Meaning::ExpandablePrimitive(
                        ExpandablePrimitive::Detokenize,
                    )) => CollectorExpansionRoute::Detokenize,
                    _ => CollectorExpansionRoute::Ordinary,
                };
                let mut expansion_operand = None;
                if self.drive_collector_expansion(
                    route,
                    episode,
                    collector,
                    &mut destination,
                    &mut expansion_operand,
                    pending_expansion,
                )? == CollectorExpansionOutcome::Expanded
                {
                    continue;
                }
            }

            // The expanded collector has completed a get_x-style delivery
            // for each retained unexpandable token. Emit that boundary before
            // storing the spelling, while expandable commands above remain
            // represented by their own expansion transitions.
            let command = destination
                .as_ref()
                .expect("command delivery initializes destination");
            if expansion.is_expanded() {
                self.observe_expanded_delivery(command);
            }
            let token = self.classify_collector_token(command, None);
            let spelling = token.word();

            // TeX82 §342 has already replaced a delivered `\cr`/`\span`/tab
            // delimiter by §789's ⟨v_j⟩ template inside `get_next`, so this
            // balanced-text collector never sees one. That matters for a
            // braced group whose matching `}` lives in the ⟨v_j⟩ template
            // (plain.tex's `\eqalign`/`\displaylines` idiom
            // `$\displaystyle{##}$` is the common case): the still-open
            // `depth` continues over the boundary exactly as if no alignment
            // entry had ended.
            //
            // TeX82 §23 backs an inaccessible outer control sequence up,
            // installs the right brace that ends this runaway collector, and
            // changes only the live current command to a space. That space is
            // recovery state, not input: §477 resumes with the inserted brace
            // and must not append the temporary current-command value.
            if command.is_outer_recovery_space() {
                clear_command_destination(&mut destination);
                continue;
            }
            if let Some(PendingParameter {
                hash,
                highest: highest_parameter,
                target,
            }) = collector.take_pending_parameter()
            {
                // §479: a second parameter character stores that character
                // once -- `##` is one parameter token in the body, not two.
                if token.spelling_is_parameter() {
                    self.push_replacement_token(collector, spelling)?;
                    clear_command_destination(&mut destination);
                    continue;
                }
                if let Some(number) = parameter_number(token.spelling().semantic_token())
                    && number <= highest_parameter
                {
                    let converted = TracedTokenWord::pack(Token::Param(number), spelling.origin());
                    self.push_replacement_token(collector, converted)?;
                    observe!(
                        self,
                        CommandObservation::TokenList(TokenListRecord {
                            transition: "splice",
                            purpose: "parameter_conversion",
                            tokens: vec![self.observed_token(converted)],
                        }),
                    );
                    clear_command_destination(&mut destination);
                    continue;
                }
                // §479's text is already rendered by
                // `report_macro_parameter_diagnostic` below.
                let delivered = destination
                    .take()
                    .expect("parameter recovery consumes the delivered command");
                self.back_input(delivered)?;
                self.report_macro_parameter_diagnostic(
                    MacroParameterDiagnostic::IllegalReplacementNumber { target },
                )?;
                self.push_replacement_token(collector, hash)?;
                continue;
            }
            if let Some((highest_parameter, target)) = macro_parameters
                && token.spelling_is_parameter()
            {
                collector
                    .set_pending_parameter(PendingParameter {
                        hash: spelling,
                        highest: highest_parameter,
                        target,
                    })
                    .map_err(|()| CommandError::input_invariant())?;
                clear_command_destination(&mut destination);
                continue;
            }
            if collector
                .settle_balanced_brace(token)
                .map_err(|()| CommandError::input_invariant())?
            {
                #[cfg(test)]
                {
                    self.command.token_collector_path_counters.state_updates += 1;
                }
                clear_command_destination(&mut destination);
                return Ok(());
            }
            #[cfg(test)]
            {
                self.command.token_collector_path_counters.state_updates += 1;
            }
            self.push_replacement_token(collector, spelling)?;
            clear_command_destination(&mut destination);
        }
    }

    fn push_replacement_token(
        &mut self,
        collector: &mut TokenCollector<G>,
        word: TracedTokenWord,
    ) -> Result<(), CommandError> {
        self.push_scan_toks_word(collector, word)
    }

    fn report_macro_parameter_diagnostic(
        &mut self,
        diagnostic: MacroParameterDiagnostic,
    ) -> Result<(), CommandError> {
        observe!(
            self,
            CommandObservation::Diagnostic(DiagnosticRecord {
                severity: "error",
                diagnostic: match diagnostic {
                    MacroParameterDiagnostic::NonconsecutiveNumber =>
                        "nonconsecutive_macro_parameter",
                    MacroParameterDiagnostic::TooManyParameters => "too_many_macro_parameters",
                    MacroParameterDiagnostic::IllegalReplacementNumber { .. } => {
                        "illegal_replacement_parameter"
                    }
                },
                arguments: Vec::new(),
            }),
        );
        // §§476/479 reach `error` only after their own `back_error` has
        // restored the rejected token, which is why §310's display is
        // rendered here rather than at the scanner's decision point.
        let context = self.command.output_open_context(self.state);
        match diagnostic {
            MacroParameterDiagnostic::NonconsecutiveNumber => {
                let mut report = self
                    .state
                    .print_err("Parameters must be numbered consecutively");
                report
                    .help(&[
                        "I've inserted the digit you should have used after the #.",
                        "Type `1' to delete what you did use.",
                    ])
                    .context(context);
                let outcome = report.error();
                self.finish_error_outcome(outcome)?;
            }
            MacroParameterDiagnostic::TooManyParameters => {
                let mut report = self.state.print_err("You already have nine parameters");
                report
                    .help(&[
                        "I'm going to ignore the # sign you just used,",
                        "as well as the token that followed it.",
                    ])
                    .context(context);
                let outcome = report.error();
                self.finish_error_outcome(outcome)?;
            }
            MacroParameterDiagnostic::IllegalReplacementNumber { target } => {
                let rendered_target = target.map(|target| {
                    (
                        self.state.resolve(target).to_owned(),
                        self.state.control_sequence_kind(target),
                    )
                });
                let mut report = self
                    .state
                    .print_err("Illegal parameter number in definition of ");
                if let Some((name, kind)) = rendered_target {
                    if kind != ControlSequenceKind::ActiveCharacter {
                        report.print_esc(&name);
                    } else {
                        report.print(&name);
                    }
                }
                report
                    .help(&[
                        "You meant to type ## instead of #, right?",
                        "Or maybe a } was forgotten somewhere earlier, and things",
                        "are all screwed up? I'm going to assume that you meant ##.",
                    ])
                    .context(context);
                let outcome = report.error();
                self.finish_error_outcome(outcome)?;
            }
        }
        Ok(())
    }

    /// Splices a token-list result of `\the` into the builder directly.
    /// The target alone is read; no input from after that target is examined.
    fn append_direct_the_toks(
        &mut self,
        collector: &mut TokenCollector<G>,
        target: &mut Option<crate::CurrentCommand<G>>,
    ) -> Result<bool, CommandError> {
        if target.is_none() && self.get_x_token_into(target)? != crate::DeliveryStatus::Command {
            return Err(CommandError::input_invariant());
        }
        let retained_target = target.as_ref().expect("target was installed");
        let scan = self.scan_the_internal_value_retained(retained_target);
        let value = match scan {
            crate::RetainedScalarScan::Complete(value) => value,
            crate::RetainedScalarScan::Suspended { error, child } => {
                self.install_scanner_resume(Some(child));
                return Err(error);
            }
            crate::RetainedScalarScan::Failed(error) => return Err(error),
        };
        let Some(value) = value else {
            self.back_input(target.take().expect("target was installed"))?;
            return Ok(false);
        };
        target.take();
        // Built only for an observed episode: an unobserved one leaves this
        // empty, which `Vec::new` does without allocating.
        let mut observed = Vec::new();
        match value {
            crate::InternalValue::Font(symbol) => {
                let word = TracedTokenWord::pack(Token::Cs(symbol), OriginId::UNKNOWN);
                if self.is_observed() {
                    observed.push(self.observed_token(word));
                }
                self.push_scan_toks_word(collector, word)?;
            }
            crate::InternalValue::Tokens { tokens, .. } => {
                let len = self
                    .command
                    .attempt_token_words(tokens)
                    .map_err(crate::scan_toks::attempt_command_error)?
                    .len();
                for index in 0..len {
                    let source = self
                        .command
                        .attempt
                        .arena()
                        .token_word(tokens, index)
                        .map_err(attempt_command_error)?;
                    let word = TracedTokenWord::pack(source.semantic_token(), OriginId::UNKNOWN);
                    if self.is_observed() {
                        observed.push(self.observed_token(word));
                    }
                    self.push_scan_toks_word(collector, word)?;
                }
            }
            value => {
                let text = crate::processor::render_the_value(&value)
                    .expect("non-token internal values render");
                for ch in text.chars() {
                    let word = TracedTokenWord::pack(
                        Token::Char {
                            ch,
                            cat: if ch == ' ' {
                                Catcode::Space
                            } else {
                                Catcode::Other
                            },
                        },
                        OriginId::UNKNOWN,
                    );
                    if self.is_observed() {
                        observed.push(self.observed_token(word));
                    }
                    self.push_scan_toks_word(collector, word)?;
                }
            }
        }
        self.command
            .timeline
            .record_cumulative_expansions(self.command.expansion.cumulative_expansions);
        self.command.expansion.cumulative_expansions = self
            .command
            .expansion
            .cumulative_expansions
            .saturating_add(1);
        // TeX82 §478 attaches `the_toks` only when `link(temp_head)<>null`.
        // Keep the observation on that same semantic boundary: an empty
        // internal token list contributes no splice transition at all.
        if !observed.is_empty() {
            observe!(
                self,
                CommandObservation::TokenList(TokenListRecord {
                    transition: "splice",
                    purpose: "the_toks",
                    tokens: observed,
                }),
            );
        }
        Ok(true)
    }

    /// e-TeX `\unexpanded` uses the same direct-splice rule.  Its balanced
    /// text is scanned raw and attached without parameter conversion or
    /// recursive expansion.
    fn append_unexpanded(&mut self, collector: &mut TokenCollector<G>) -> Result<(), CommandError> {
        let scanned = self.scan_toks(ScanToksMode::GeneralText {
            purpose: "unexpanded",
        })?;
        let direct_destination = match collector.destination() {
            TokenCollectorDestination::TokenBuffers { writer, .. } => Some(*writer),
            TokenCollectorDestination::Definition { .. } => None,
            TokenCollectorDestination::MacroArgument { .. }
            | TokenCollectorDestination::ReplayInput { .. } => {
                return Err(CommandError::input_invariant());
            }
        };
        let len = self.attempt_words(scanned.replacement_text)?.len();
        let mut observed = Vec::new();
        for index in 0..len {
            let word = self
                .command
                .attempt
                .arena()
                .token_word(scanned.replacement_text, index)
                .map_err(attempt_command_error)?;
            if self.is_observed() {
                observed.push(self.observed_token(word));
            }
            if direct_destination.is_none() {
                // A definition must shed attempt-local provenance and publish
                // durable semantic words. That lifetime conversion is the one
                // intentional per-word boundary; token-buffer parents instead
                // adopt the completed child sink below without copying.
                self.push_scan_toks_word(collector, word)?;
            }
        }
        if let Some(destination) = direct_destination {
            let moved = self
                .command
                .attempt
                .arena_mut()
                .consume_token_list_into_buffer(scanned.replacement_text, destination)
                .map_err(attempt_command_error)?;
            if moved as usize != len {
                return Err(CommandError::input_invariant());
            }
        }
        self.command
            .timeline
            .record_cumulative_expansions(self.command.expansion.cumulative_expansions);
        self.command.expansion.cumulative_expansions = self
            .command
            .expansion
            .cumulative_expansions
            .saturating_add(1);
        // TeX82 §478 attaches `the_toks` only when `link(temp_head)<>null`.
        if !observed.is_empty() {
            observe!(
                self,
                CommandObservation::TokenList(TokenListRecord {
                    transition: "splice",
                    purpose: "the_toks",
                    tokens: observed,
                }),
            );
        }
        Ok(())
    }

    /// e-TeX 2.6 etex.ch §53a returns `\detokenize` through `the_toks`.
    ///
    /// Inside §477's expanded collector the converted list is therefore
    /// attached directly, just like §478's ordinary `\the` result. It must
    /// not become a §470 `conv_toks` inserted input level whose characters
    /// are fetched again one by one.
    fn append_detokenize(&mut self, collector: &mut TokenCollector<G>) -> Result<(), CommandError> {
        let scanned = self.scan_toks(ScanToksMode::GeneralText {
            purpose: "detokenize",
        })?;
        let words = self.attempt_words(scanned.replacement_text)?;
        let mut text = String::new();
        for word in words {
            self.state
                .append_token_string_text(word.semantic_token(), &mut text);
        }
        let mut observed = Vec::new();
        for ch in text.chars() {
            let word = TracedTokenWord::pack(
                Token::Char {
                    ch,
                    cat: if ch == ' ' {
                        Catcode::Space
                    } else {
                        Catcode::Other
                    },
                },
                OriginId::UNKNOWN,
            );
            if self.is_observed() {
                observed.push(self.observed_token(word));
            }
            self.push_scan_toks_word(collector, word)?;
        }
        self.command
            .timeline
            .record_cumulative_expansions(self.command.expansion.cumulative_expansions);
        self.command.expansion.cumulative_expansions = self
            .command
            .expansion
            .cumulative_expansions
            .saturating_add(1);
        if !observed.is_empty() {
            observe!(
                self,
                CommandObservation::TokenList(TokenListRecord {
                    transition: "splice",
                    purpose: "the_toks",
                    tokens: observed,
                }),
            );
        }
        Ok(())
    }

    pub(crate) fn expand_unexpanded(&mut self) -> Result<(), CommandError> {
        let scanned = self.scan_toks_buffers(ScanToksMode::EscapingGeneralText {
            purpose: "unexpanded",
        })?;
        // e-TeX change file §27.465 routes `\unexpanded` through `the_toks`
        // and `ins_list`. Unlike a direct splice into an active collector,
        // this standalone input may remain live after the executor operation
        // commits. Its one TokenCollector therefore writes directly into
        // generation-owned replay storage; publication moves that storage's
        // header into the final ordered entry and never promotes or copies an
        // attempt-local list.
        let ScannedToksStorage::ReplayInput { replay, len } = scanned.storage else {
            return Err(CommandError::input_invariant());
        };
        let first = self
            .command
            .roots
            .input
            .replay
            .get(replay, 0)
            .map(|(word, _)| word.semantic_token());
        self.insert_expansion_list(PackedTokenSpanHandle::Replay { replay, len }, first);
        Ok(())
    }
}

fn append_runaway_words<G>(
    state: &tex_state::CommandContext<'_, G>,
    tokens: &ScannedWords<'_, G>,
    match_marker: &mut char,
    partial: &mut String,
) {
    let mut index = 0;
    while index < tokens.len() {
        let token = tokens
            .token(index)
            .expect("runaway index is inside scanned words");
        if let Token::Char {
            ch,
            cat: Catcode::Parameter,
        } = token
            && let Some(Token::Param(slot)) = tokens.token(index + 1)
        {
            *match_marker = ch;
            append_runaway_character(state, ch, partial);
            append_runaway_character(state, char::from(b'0' + slot), partial);
            index += 2;
            continue;
        }
        if let Token::Param(slot) = token {
            append_runaway_character(state, *match_marker, partial);
            append_runaway_character(state, char::from(b'0' + slot), partial);
        } else {
            state.append_token_selector_text(token, partial);
        }
        index += 1;
    }
}

fn append_runaway_character<G>(
    state: &tex_state::CommandContext<'_, G>,
    ch: char,
    partial: &mut String,
) {
    state.append_token_selector_text(
        Token::Char {
            ch,
            cat: Catcode::Other,
        },
        partial,
    );
}

#[cfg(test)]
thread_local! {
    static RUNAWAY_RENDER_COUNT: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
fn runaway_render_count() -> u64 {
    RUNAWAY_RENDER_COUNT.with(std::cell::Cell::get)
}

#[cfg(test)]
fn reset_runaway_render_count() {
    RUNAWAY_RENDER_COUNT.with(|count| count.set(0));
}

pub(crate) fn attempt_command_error(error: AttemptError) -> CommandError {
    match error {
        AttemptError::CapacityOverflow
        | AttemptError::AllocationFailed
        | AttemptError::Definition(
            tex_state::DefinitionBuildError::AllocationFailed
            | tex_state::DefinitionBuildError::CapacityOverflow,
        ) => CommandError::Fatal(crate::FatalError::overflow(
            "scanner token storage",
            i32::MAX,
        )),
        AttemptError::ForeignAttempt
        | AttemptError::InvalidCoordinate
        | AttemptError::Definition(_)
        | AttemptError::Promotion(_) => CommandError::input_invariant(),
    }
}

pub(crate) fn scratch_command_error(error: crate::execution_scratch::ScratchError) -> CommandError {
    match error {
        crate::execution_scratch::ScratchError::CapacityOverflow
        | crate::execution_scratch::ScratchError::AllocationFailed => CommandError::Fatal(
            crate::FatalError::overflow("scanner continuation storage", i32::MAX),
        ),
        crate::execution_scratch::ScratchError::InvalidCoordinate => {
            CommandError::input_invariant()
        }
    }
}

fn is_parameter(token: Token) -> bool {
    matches!(
        token,
        Token::Char {
            cat: Catcode::Parameter,
            ..
        }
    )
}

fn parameter_number(token: Token) -> Option<u8> {
    match token {
        Token::Char {
            ch: '1'..='9',
            cat: Catcode::Other,
        } => Some((token_char(token)? as u8) - b'0'),
        _ => None,
    }
}

fn token_char(token: Token) -> Option<char> {
    match token {
        Token::Char { ch, .. } => Some(ch),
        _ => None,
    }
}

fn is_begin_group(token: Token) -> bool {
    matches!(
        token,
        Token::Char {
            cat: Catcode::BeginGroup,
            ..
        }
    )
}

fn is_end_group(token: Token) -> bool {
    matches!(
        token,
        Token::Char {
            cat: Catcode::EndGroup,
            ..
        }
    )
}

/// TeX82 §482's `read_toks`, the `\read`/`\readline` collector.
///
/// `read_toks` is deliberately not a `scan_toks` mode: it collects whole
/// _lines_ rather than a brace-balanced group, disables alignment delimiters
/// for its whole duration, and continues across a brace imbalance instead of
/// ending at a closing brace. It shares only the frozen-list result.
impl<G> CommandProcessor<'_, '_, G> {
    /// Collects TeX82 §482's `read_toks` list.
    ///
    /// `begin scanner_status:=defining; warning_index:=r; def_ref:=get_avail;
    /// token_ref_count(def_ref):=null; p:=def_ref; store_new_token(
    /// end_match_token); if (n<0)or(n>15) then m:=16 else m:=n; s:=align_state;
    /// align_state:=1000000; repeat <Input and store tokens from the next line
    /// of the file>; until align_state=1000000; cur_val:=def_ref;
    /// scanner_status:=normal; align_state:=s; end`.
    ///
    /// The result is a parameterless macro body: §482 stores `end_match_token`
    /// as the whole parameter text, which is Umber's empty parameter list.
    pub(crate) fn read_toks(
        &mut self,
        stream: i32,
        target: tex_state::interner::Symbol,
        raw_catcodes: bool,
    ) -> Result<AttemptDefinitionId, CommandError> {
        let mut scope = Some(
            self.command
                .begin_attempt_scanner_scope()
                .map_err(attempt_command_error)?,
        );
        let builder = TokenBuilderId(self.command.transient.next_builder_identity);
        self.command.transient.next_builder_identity =
            self.command.transient.next_builder_identity.wrapping_add(1);
        // §482: `scanner_status:=defining; warning_index:=r`.
        let status = ScannerStatus::Defining(DefinitionContext {
            target: Some(target),
            builder,
            warning: ScannerWarning(builder.0),
        });
        let episode = self.begin_scanner_episode(status, ScannerStatusVisibility::Observed);
        // §482: `s:=align_state; align_state:=1000000` disables tab marks and
        // `\cr` for the whole collection, and is restored whatever happens.
        let saved_align_state = self.command.alignment.align_state;
        self.command.record_alignment_phase();
        self.command.alignment.align_state = TEMPLATE_ALIGN_STATE;
        let result = (|| {
            let mut collector = self.begin_scan_toks_collector(
                ScanToksGrammar::MacroDefinition,
                ScanToksDestination::Attempt,
            )?;
            let definition = match collector.destination() {
                TokenCollectorDestination::Definition {
                    definition,
                    writing_replacement: false,
                } => *definition,
                _ => unreachable!(),
            };
            self.finish_scan_toks_parameters(&mut collector)?;
            self.read_toks_lines(stream, target, raw_catcodes, &mut collector)?;
            // §482 leaves the collected list in `cur_val`; §1225 immediately
            // installs it with `define(p,call,cur_val)`. Unlike §473's
            // `scan_toks`, this is not an independently observable completed
            // token-list assignment. The committed observation is §1225's
            // meaning mutation, whose macro body includes §482's leading
            // `end_match_token`.
            match self.finish_scan_toks_collector(&mut collector)? {
                ScannedToksStorage::Definition(completed) if completed == definition => {}
                _ => return Err(CommandError::input_invariant()),
            }
            self.command
                .validate_attempt_scope_retirement(
                    scope.as_ref().expect("read owns its scanner scope"),
                )
                .map_err(attempt_command_error)?;
            self.command
                .defer_attempt_scope_retirement(
                    scope
                        .take()
                        .expect("successful read owns its scanner scope"),
                )
                .expect("validated read scope retires without intervening mutation");
            Ok(definition)
        })();
        self.command.record_alignment_phase();
        self.command.alignment.align_state = saved_align_state;
        self.finish_scanner_episode(episode);
        match result {
            Ok(definition) => Ok(definition),
            Err(error) => {
                let scope = scope.take().ok_or_else(CommandError::input_invariant)?;
                self.command
                    .discard_attempt_scope_suffix(scope)
                    .map_err(attempt_command_error)?;
                Err(error)
            }
        }
    }

    /// §482's `repeat <Input and store tokens from the next line> until
    /// align_state=1000000`.
    fn read_toks_lines(
        &mut self,
        stream: i32,
        target: tex_state::interner::Symbol,
        raw_catcodes: bool,
        collector: &mut TokenCollector<G>,
    ) -> Result<(), CommandError> {
        // §482: `if (n<0)or(n>15) then m:=16 else m:=n`. Stream 16 is never
        // open, so §483 always takes §484's terminal branch for it.
        let slot = u8::try_from(stream)
            .ok()
            .filter(|slot| *slot < tex_state::world::STREAM_SLOT_COUNT as u8)
            .map(tex_state::world::StreamSlot::new);
        // §484's own `n`, which decides whether the user is prompted at all:
        // a negative stream is prompted with the empty string, so `\read-1 to
        // \x` never prints `\x=`. §484 then assigns `n:=-1` after prompting,
        // "so that additional prompts will not be given in the case of
        // multi-line input" -- one variable serving both rules.
        let mut prompt_number = stream;
        loop {
            self.read_toks_line(slot, target, raw_catcodes, &mut prompt_number, collector)?;
            if self.command.alignment.align_state == TEMPLATE_ALIGN_STATE {
                return Ok(());
            }
        }
    }

    /// §483's ⟨Input and store tokens from the next line of the file⟩.
    fn read_toks_line(
        &mut self,
        slot: Option<tex_state::world::StreamSlot>,
        target: tex_state::interner::Symbol,
        raw_catcodes: bool,
        prompt_number: &mut i32,
        collector: &mut TokenCollector<G>,
    ) -> Result<(), CommandError> {
        // §483 calls `begin_file_reading` before §484-§486 acquire the line.
        // §328 establishes that new level with `name:=0`; the selected
        // stream classification is installed only after acquisition chooses
        // the stream rather than §484's terminal fallback.
        let level = self
            .command
            .begin_read_line()
            .map_err(|_| CommandError::input_invariant())?;
        observe!(
            self,
            CommandObservation::Input(InputRecord {
                transition: InputTransition::Push,
                reason: InputReason::Source,
                source_name: Some(crate::input::SourceNameClass::Terminal),
                source: None,
                level: level.0,
                position: 0,
            }),
        );
        let (line, file_ended, name_class) = self.acquire_read_line(slot, target, prompt_number)?;
        self.command
            .finish_read_line(level, name_class, line.into_bytes())
            .map_err(|_| CommandError::input_invariant())?;
        if file_ended && !raw_catcodes && self.command.alignment.align_state != TEMPLATE_ALIGN_STATE
        {
            // TeX82 §486 reports the live `def_ref` before `limit:=0` makes
            // the appended empty line available to §483 and before that
            // line's `end_file_reading`.  Keeping the report at this
            // boundary preserves both §306's `->...` pseudoprint and §82's
            // still-live read-stream context.
            self.acquire_source_line(false)?;
            let mut partial = vec![
                TracedTokenWord::pack(
                    Token::Char {
                        ch: '-',
                        cat: Catcode::Other,
                    },
                    OriginId::UNKNOWN,
                ),
                TracedTokenWord::pack(
                    Token::Char {
                        ch: '>',
                        cat: Catcode::Other,
                    },
                    OriginId::UNKNOWN,
                ),
            ];
            partial.extend(self.read_runaway_words(collector)?);
            let context = self.command.output_open_context(self.state);
            self.command
                .semantic_diagnostics
                .push(crate::CommandSemanticDiagnostic::Recoverable {
                    identity: FILE_ENDED_WITHIN_READ_DIAGNOSTIC,
                    runaway: Some(crate::state::RunawayPrelude {
                        heading: "Runaway definition?",
                        partial: String::new(),
                    }),
                    message: "File ended within \\read".into(),
                    help: &["This \\read has unbalanced braces."],
                    context,
                    integer_error: None,
                });
            self.set_runaway_partial(FILE_ENDED_WITHIN_READ_DIAGNOSTIC, &partial);
            self.command.record_alignment_phase();
            self.command.alignment.align_state = TEMPLATE_ALIGN_STATE;
        }
        if raw_catcodes {
            self.collect_read_line_verbatim(level, collector)?;
            if file_ended {
                self.command.record_alignment_phase();
                self.command.alignment.align_state = TEMPLATE_ALIGN_STATE;
            }
            return Ok(());
        }
        // §483: `loop get_token; if cur_tok=0 then goto done; if
        // align_state<1000000 then {unmatched `}' aborts the line} begin
        // repeat get_token until cur_tok=0; align_state:=1000000; goto done;
        // end; store_new_token(cur_tok); end`.
        let mut destination = None;
        loop {
            let status = self.get_token_into(&mut destination)?;
            if status == crate::DeliveryStatus::End {
                break;
            }
            if status != crate::DeliveryStatus::Command {
                return Err(CommandError::input_invariant());
            }
            let command = destination
                .take()
                .expect("command status initializes destination");
            if self
                .command
                .semantic_diagnostics
                .iter()
                .rev()
                .any(|diagnostic| {
                    matches!(
                        diagnostic,
                        crate::CommandSemanticDiagnostic::Recoverable {
                            runaway: Some(crate::state::RunawayPrelude { partial, .. }),
                            ..
                        } if partial.is_empty()
                    )
                })
            {
                // TeX82 §§482/306 call `runaway` from inside `get_token`,
                // before §23's temporary recovery space returns to §483 and
                // is stored. Snapshot the live `def_ref` at that instant:
                // its leading `end_match_token` prints as `->`, followed by
                // only the body tokens collected before the forbidden outer
                // command.
                let mut runaway = vec![
                    TracedTokenWord::pack(
                        Token::Char {
                            ch: '-',
                            cat: Catcode::Other,
                        },
                        OriginId::UNKNOWN,
                    ),
                    TracedTokenWord::pack(
                        Token::Char {
                            ch: '>',
                            cat: Catcode::Other,
                        },
                        OriginId::UNKNOWN,
                    ),
                ];
                runaway.extend(self.read_runaway_words(collector)?);
                self.set_runaway_partial(crate::processor::RUNAWAY_SCAN_DIAGNOSTIC, &runaway);
            }
            if self.command.alignment.align_state < TEMPLATE_ALIGN_STATE {
                loop {
                    match self.get_token_into(&mut destination)? {
                        crate::DeliveryStatus::Command => {
                            destination
                                .take()
                                .expect("command status initializes destination");
                        }
                        crate::DeliveryStatus::End => break,
                        _ => return Err(CommandError::input_invariant()),
                    }
                }
                self.command.record_alignment_phase();
                self.command.alignment.align_state = TEMPLATE_ALIGN_STATE;
                return Ok(());
            }
            self.push_scan_toks_word(collector, command.spelling())?;
        }
        Ok(())
    }

    /// Collects one e-TeX `\readline` line.
    ///
    /// `\readline` reads the line with every character carrying category 12,
    /// or 10 for a space, whatever the current table says, so no control
    /// sequence, brace, or tab mark can form and §483's alignment and brace
    /// rules have nothing to act on. §483's line still ends at its
    /// `\endlinechar`, which is why the line is loaded and read through the
    /// same cursor rather than out of the acquired string.
    fn collect_read_line_verbatim(
        &mut self,
        level: crate::input::InputLevelId,
        collector: &mut TokenCollector<G>,
    ) -> Result<(), CommandError> {
        self.acquire_source_line(false)?;
        while let Some(character) = self.command.next_source_character() {
            let ch = crate::profile::token_character(character.code());
            let origin = self.state.source_token_origin(
                character.range().source(),
                character.range().start(),
                character.range().end(),
            );
            let cat = if ch == ' ' {
                Catcode::Space
            } else {
                Catcode::Other
            };
            self.push_scan_toks_word(
                collector,
                TracedTokenWord::pack(Token::Char { ch, cat }, origin),
            )?;
        }
        self.retire_read_line_level(level)?;
        Ok(())
    }

    fn read_runaway_words(
        &self,
        collector: &TokenCollector<G>,
    ) -> Result<Vec<TracedTokenWord>, CommandError> {
        let definition = match (collector.destination(), collector.phase()) {
            (
                TokenCollectorDestination::Definition {
                    definition,
                    writing_replacement: true,
                },
                TokenCollectorPhase::Replacement,
            ) => *definition,
            _ => return Err(CommandError::input_invariant()),
        };
        Ok(self
            .command
            .attempt
            .arena()
            .definition_replacement_words(definition)
            .map_err(attempt_command_error)?
            .iter()
            .copied()
            .map(|word| TracedTokenWord::from_parts(word, OriginId::UNKNOWN))
            .collect())
    }

    /// §483's line acquisition: §484's terminal, or §485/§486's stream.
    ///
    /// The flag is §486's `input_ln` returning false: the stream has just
    /// closed, and "if align_state<>1000000 then begin runaway; print_err(
    /// "File ended within \read"); ... align_state:=1000000; limit:=0;
    /// error; end". The line itself is still read and tokenized -- it is
    /// §486's one appended empty line -- so only the brace count is reset.
    fn acquire_read_line(
        &mut self,
        slot: Option<tex_state::world::StreamSlot>,
        target: tex_state::interner::Symbol,
        prompt_number: &mut i32,
    ) -> Result<(String, bool, crate::input::SourceNameClass), CommandError> {
        // §483: `if read_open[m]=closed then <terminal> else <the file>`.
        if let Some(slot) = slot
            && !self.state.read_stream_at_eof(slot)
            && let Some(line) = self.state.input_ln(CommandLineSource::Stream(slot))
        {
            let ended = self.state.read_stream_at_eof(slot);
            return Ok((
                line,
                ended,
                crate::input::SourceNameClass::ReadStream(slot.raw()),
            ));
        }
        // §484: `if interaction>nonstop_mode then if n<0 then
        // prompt_input("") else begin wake_up_terminal; print_ln; sprint_cs(r);
        // prompt_input("="); n:=-1; end else fatal_error(...)`.
        if !self.state.interaction_permits_terminal_input() {
            return Err(CommandError::Fatal(crate::FatalError::emergency_stop(
                "job aborted, file error in nonstop mode",
            )));
        }
        let prompt = if *prompt_number < 0 {
            String::new()
        } else {
            let prompt = format!("\n\\{}=", self.state.resolve(target));
            *prompt_number = -1;
            prompt
        };
        let line = self
            .state
            .input_ln(CommandLineSource::Terminal { prompt: &prompt })
            .ok_or_else(|| CommandError::input_invariant())?;
        Ok((line, false, crate::input::SourceNameClass::Terminal))
    }
}

#[cfg(test)]
mod tests;
