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
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};

use crate::attempt::{AttemptError, AttemptMark, AttemptTokenBufferId, AttemptTokenListId};
use crate::processor::alignment::TEMPLATE_ALIGN_STATE;
use crate::processor::expand::is_expandable_command;
use crate::processor::status::{
    AbsorbingContext, DefinitionContext, ScannerEpisode, ScannerStatus, ScannerStatusVisibility,
    ScannerWarning, TokenBuilderId,
};
use crate::{CommandError, CommandProcessor};
use tex_state::CommandLineSource;

use crate::input::TokenPayload;
use crate::observation::{
    CommandObservation, DiagnosticRecord, InputReason, InputRecord, InputTransition,
    TokenListRecord,
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
    opening: ScanToksOpening,
    expansion: ScanToksExpansion,
    owner: ScanToksOwner,
    purpose: ScanToksPurpose,
    status_visibility: ScannerStatusVisibility,
}

/// Exact command-owned continuation of one host-suspended token collector.
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct PendingScanToks<G> {
    mark: AttemptMark,
    config: ScanToksConfig,
    episode: ScannerEpisode,
    phase: PendingScanToksPhase<G>,
}

impl<G> Clone for PendingScanToks<G> {
    fn clone(&self) -> Self {
        Self {
            mark: self.mark,
            config: self.config,
            episode: self.episode.clone(),
            phase: self.phase.clone(),
        }
    }
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

    pub(crate) fn retain_attempt_coordinates(
        &self,
        arena: &crate::attempt::AttemptArena<G>,
        cursor: &mut crate::attempt::AttemptLiveCursor,
    ) -> Result<(), AttemptError> {
        arena.retain_mark(cursor, self.mark)?;
        let PendingScanToksPhase::Replacement {
            parameter_text,
            progress,
            ..
        } = &self.phase
        else {
            return Ok(());
        };
        arena.retain_token_list(cursor, *parameter_text)?;
        arena.retain_token_buffer(cursor, progress.output)
    }
}

// Every pending field is a scalar, a typed attempt coordinate, or an
// ephemeral current-command value. The arena owner remains on `CommandState`;
// no continuation borrows its accumulated token buffer.
#[derive(Debug, Eq, PartialEq)]
enum PendingScanToksPhase<G> {
    Opening,
    Replacement {
        parameter_text: AttemptTokenListId,
        macro_parameters: Option<(u8, Option<Symbol>)>,
        hash_brace: Option<TracedTokenWord>,
        primary: OriginId,
        malformed_parameter: bool,
        progress: Box<ReplacementProgress<G>>,
    },
}

impl<G> Clone for PendingScanToksPhase<G> {
    fn clone(&self) -> Self {
        match self {
            Self::Opening => Self::Opening,
            Self::Replacement {
                parameter_text,
                macro_parameters,
                hash_brace,
                primary,
                malformed_parameter,
                progress,
            } => Self::Replacement {
                parameter_text: *parameter_text,
                macro_parameters: *macro_parameters,
                hash_brace: *hash_brace,
                primary: *primary,
                malformed_parameter: *malformed_parameter,
                progress: progress.clone(),
            },
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
struct ReplacementProgress<G> {
    output: AttemptTokenBufferId,
    depth: u32,
    pending_parameter: Option<(TracedTokenWord, u8, Option<Symbol>)>,
    pending_expansion: Option<PendingCollectorExpansion<G>>,
}

impl<G> Clone for ReplacementProgress<G> {
    fn clone(&self) -> Self {
        Self {
            output: self.output,
            depth: self.depth,
            pending_parameter: self.pending_parameter,
            pending_expansion: self.pending_expansion,
        }
    }
}

impl<G> ReplacementProgress<G> {
    fn new(output: AttemptTokenBufferId) -> Self {
        Self {
            output,
            depth: 1,
            pending_parameter: None,
            pending_expansion: None,
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
struct PendingCollectorExpansion<G> {
    command: crate::CurrentCommand<G>,
    route: CollectorExpansionRoute,
}

impl<G> Clone for PendingCollectorExpansion<G> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<G> Copy for PendingCollectorExpansion<G> {}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum CollectorExpansionRoute {
    Ordinary,
    The,
    Unexpanded,
    Detokenize,
}

struct ScanToksFailure<G> {
    error: CommandError,
    continuation: Box<PendingScanToksPhase<G>>,
}

struct ReplacementFailure<G> {
    error: CommandError,
    progress: Box<ReplacementProgress<G>>,
}

impl<G> From<CommandError> for ScanToksFailure<G> {
    fn from(error: CommandError) -> Self {
        Self {
            error,
            continuation: Box::new(PendingScanToksPhase::Opening),
        }
    }
}

fn replacement_failure<G>(
    error: CommandError,
    output: AttemptTokenBufferId,
    depth: u32,
    pending_parameter: &mut Option<(TracedTokenWord, u8, Option<Symbol>)>,
    pending_expansion: Option<PendingCollectorExpansion<G>>,
) -> ReplacementFailure<G> {
    ReplacementFailure {
        error,
        progress: Box::new(ReplacementProgress {
            output,
            depth,
            pending_parameter: pending_parameter.take(),
            pending_expansion,
        }),
    }
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
                opening: ScanToksOpening::Required,
                expansion: expansion(expanded),
                owner: ScanToksOwner::Absorbed(None),
                purpose: balanced_purpose(expanded),
                status_visibility: ScannerStatusVisibility::Observed,
            },
            ScanToksMode::GeneralFor { expanded, owner } => Self {
                grammar: ScanToksGrammar::General,
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
                opening: ScanToksOpening::Prevalidated { primary },
                expansion: expansion(expanded),
                owner: ScanToksOwner::Absorbed(owner),
                purpose: balanced_purpose(expanded),
                status_visibility: ScannerStatusVisibility::Observed,
            },
            ScanToksMode::GeneralText { purpose } => Self {
                grammar: ScanToksGrammar::General,
                opening: ScanToksOpening::Required,
                expansion: ScanToksExpansion::Unexpanded,
                owner: ScanToksOwner::Absorbed(None),
                purpose: ScanToksPurpose::GeneralText(purpose),
                status_visibility: ScannerStatusVisibility::Hidden,
            },
            ScanToksMode::MacroDefinition { expanded } => Self {
                grammar: ScanToksGrammar::MacroDefinition,
                opening: ScanToksOpening::AfterParameterText,
                expansion: expansion(expanded),
                owner: ScanToksOwner::Definition(None),
                purpose: ScanToksPurpose::MacroReplacement,
                status_visibility: ScannerStatusVisibility::Observed,
            },
            ScanToksMode::MacroDefinitionFor { expanded, target } => Self {
                grammar: ScanToksGrammar::MacroDefinition,
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
pub(crate) struct ScannedToksBuffers {
    pub(crate) parameter_text: AttemptTokenListId,
    pub(crate) replacement_text: AttemptTokenListId,
    pub(crate) primary: OriginId,
    pub(crate) malformed_parameter: bool,
}

/// TeX82 §403's result after a mandatory left-brace scan.
#[derive(Debug)]
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
    tokens: AttemptTokenListId,
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

const NONCONSECUTIVE_PARAMETER_DIAGNOSTIC: u64 = 0x6465_6600_0000_0476;
const ILLEGAL_REPLACEMENT_PARAMETER_DIAGNOSTIC: u64 = 0x6465_6600_0000_0479;
const FILE_ENDED_WITHIN_READ_DIAGNOSTIC: u64 = 0x7265_6164_0000_0486;

impl<G> CommandProcessor<'_, '_, G> {
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

    fn begin_attempt_token_list(&mut self) -> Result<AttemptTokenBufferId, CommandError> {
        self.command
            .attempt
            .arena_mut()
            .allocate_token_buffer()
            .map_err(attempt_command_error)
    }

    fn push_attempt_token(
        &mut self,
        buffer: AttemptTokenBufferId,
        word: TracedTokenWord,
    ) -> Result<(), CommandError> {
        self.command
            .attempt
            .arena_mut()
            .push_buffer_token(buffer, word)
            .map_err(attempt_command_error)
    }

    fn finish_attempt_token_list(
        &mut self,
        buffer: AttemptTokenBufferId,
    ) -> Result<AttemptTokenListId, CommandError> {
        self.command
            .attempt
            .arena_mut()
            .finish_token_buffer(buffer)
            .map_err(attempt_command_error)
    }

    fn attempt_words(&self, list: AttemptTokenListId) -> Result<&[TracedTokenWord], CommandError> {
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
        Ok(ScannedToks {
            parameter_text: result.parameter_text,
            replacement_text: result.replacement_text,
            primary: result.primary,
            malformed_parameter: result.malformed_parameter,
        })
    }

    pub(crate) fn scan_toks_buffers(
        &mut self,
        mode: ScanToksMode,
    ) -> Result<ScannedToksBuffers, CommandError> {
        let config = ScanToksConfig::parse(mode);
        let (mark, episode, phase) = match self.command.pending_scan_toks.pop() {
            Some(pending) if pending.config == config => {
                (pending.mark, pending.episode, pending.phase)
            }
            Some(pending) => {
                self.command.pending_scan_toks.push(pending);
                return Err(CommandError::input_invariant());
            }
            None => {
                let mark = self.command.attempt.arena().mark();
                let builder = TokenBuilderId(self.command.transient.next_builder_identity);
                self.command.transient.next_builder_identity =
                    self.command.transient.next_builder_identity.wrapping_add(1);
                let warning = ScannerWarning(builder.0);
                (
                    mark,
                    self.begin_scanner_episode(
                        config.scanner_status(builder, warning),
                        config.status_visibility,
                    ),
                    PendingScanToksPhase::Opening,
                )
            }
        };
        let result = self.scan_toks_inner(config, &episode, phase);
        let result = match result {
            Ok(result) => result,
            Err(failure) if failure.error.is_resource_suspension() => {
                self.command.pending_scan_toks.push(PendingScanToks {
                    mark,
                    config,
                    episode,
                    phase: *failure.continuation,
                });
                return Err(failure.error);
            }
            Err(failure) => {
                self.finish_scanner_episode(episode);
                self.command
                    .attempt
                    .arena_mut()
                    .truncate(mark)
                    .map_err(attempt_command_error)?;
                return Err(failure.error);
            }
        };
        let mut partial = if matches!(config.grammar, ScanToksGrammar::MacroDefinition) {
            parameter_text_for_runaway(self.command.attempt.arena(), &result)?
        } else {
            Vec::new()
        };
        partial.extend(
            self.attempt_words(result.replacement_text)?
                .iter()
                .map(|word| TracedTokenWord::pack(word.semantic_token(), OriginId::UNKNOWN)),
        );
        self.set_runaway_partial(crate::processor::RUNAWAY_SCAN_DIAGNOSTIC, &partial);
        self.finish_scanner_episode(episode);
        let completed_tokens = if !self.is_observed() {
            Vec::new()
        } else if config.purpose.renders_detokenized_result() {
            let semantic_tokens = self
                .attempt_words(result.replacement_text)?
                .iter()
                .map(|word| word.semantic_token())
                .collect::<Vec<_>>();
            crate::processor::expand::token_slice_string_text(&mut self.state, &semantic_tokens)
                .chars()
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
            self.attempt_words(result.replacement_text)?
                .iter()
                .map(|word| {
                    self.observed_token(TracedTokenWord::pack(
                        word.semantic_token(),
                        OriginId::UNKNOWN,
                    ))
                })
                .collect()
        };
        observe!(
            self,
            CommandObservation::TokenList(TokenListRecord {
                transition: "complete",
                purpose: config.purpose.canonical_name(),
                tokens: completed_tokens,
            }),
        );
        Ok(result)
    }

    fn scan_toks_inner(
        &mut self,
        config: ScanToksConfig,
        episode: &ScannerEpisode,
        phase: PendingScanToksPhase<G>,
    ) -> Result<ScannedToksBuffers, ScanToksFailure<G>> {
        // `macro_parameters` is TeX82 §477's `macro_def` flag carried together
        // with §479's `t`: `Some(highest)` selects the parameter-character
        // rule and bounds a legal parameter number, `None` leaves parameter
        // characters as ordinary text (`\message`, `\write`, `\toks`, ...).
        let (
            parameter_text,
            macro_parameters,
            hash_brace,
            primary,
            malformed_parameter,
            missing_left_brace,
            replacement_progress,
        ) = match phase {
            PendingScanToksPhase::Replacement {
                parameter_text,
                macro_parameters,
                hash_brace,
                primary,
                malformed_parameter,
                progress,
            } => (
                parameter_text,
                macro_parameters,
                hash_brace,
                primary,
                malformed_parameter,
                false,
                *progress,
            ),
            PendingScanToksPhase::Opening => match (config.grammar, config.opening) {
                (ScanToksGrammar::General, ScanToksOpening::Required) => {
                    // TeX scans the required opening brace through the ordinary
                    // expanded path even when the replacement text itself is
                    // collected unexpanded.
                    let opening = self
                        .scan_left_brace(true)
                        .map_err(|error| ScanToksFailure {
                            error,
                            continuation: Box::new(PendingScanToksPhase::Opening),
                        })?;
                    let primary = opening.origin();
                    let parameter_text = self.allocate_attempt_token_list([])?;
                    let output = self.begin_attempt_token_list()?;
                    (
                        parameter_text,
                        None,
                        None,
                        primary,
                        false,
                        false,
                        ReplacementProgress::new(output),
                    )
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
                    let opening = self
                        .get_token()
                        .map_err(|error| ScanToksFailure {
                            error,
                            continuation: Box::new(PendingScanToksPhase::Opening),
                        })?
                        .ok_or_else(CommandError::input_invariant)
                        .map_err(|error| ScanToksFailure {
                            error,
                            continuation: Box::new(PendingScanToksPhase::Opening),
                        })?;
                    if !matches!(
                        opening.meaning(),
                        ResolvedMeaning::Static(Meaning::CharToken {
                            cat: Catcode::BeginGroup,
                            ..
                        })
                    ) {
                        return Err(ScanToksFailure {
                            error: CommandError::input_invariant(),
                            continuation: Box::new(PendingScanToksPhase::Opening),
                        });
                    }
                    self.observe_expanded_delivery(&opening);
                    let parameter_text = self.allocate_attempt_token_list([])?;
                    let output = self.begin_attempt_token_list()?;
                    (
                        parameter_text,
                        None,
                        None,
                        primary,
                        false,
                        false,
                        ReplacementProgress::new(output),
                    )
                }
                (ScanToksGrammar::MacroDefinition, ScanToksOpening::AfterParameterText) => {
                    let parameters =
                        self.scan_parameter_text()
                            .map_err(|error| ScanToksFailure {
                                error,
                                continuation: Box::new(PendingScanToksPhase::Opening),
                            })?;
                    (
                        parameters.tokens,
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
                        ReplacementProgress::new(self.begin_attempt_token_list()?),
                    )
                }
                _ => unreachable!("ScanToksConfig admits no other grammar/opening pair"),
            },
        };
        let replacement = if missing_left_brace {
            replacement_progress.output
        } else {
            match self.collect_replacement(
                config.expansion,
                macro_parameters,
                episode,
                replacement_progress,
            ) {
                Ok(replacement) => replacement,
                Err(failure) => {
                    return Err(ScanToksFailure {
                        error: failure.error,
                        continuation: Box::new(PendingScanToksPhase::Replacement {
                            parameter_text,
                            macro_parameters,
                            hash_brace,
                            primary,
                            malformed_parameter,
                            progress: failure.progress,
                        }),
                    });
                }
            }
        };
        // TeX's `#{` parameter-text special case treats that left brace as a
        // delimiter and appends the same saved brace after the replacement
        // text (TeX.web §476).
        if let Some(brace) = hash_brace {
            self.push_attempt_token(replacement, brace)?;
        }
        let replacement_text = self.finish_attempt_token_list(replacement)?;
        Ok(ScannedToksBuffers {
            parameter_text,
            replacement_text,
            primary,
            malformed_parameter,
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
        loop {
            let command = if expanded {
                self.get_x_token()?
            } else {
                self.get_token()?
            }
            .ok_or(CommandError::input_invariant())?;
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
                    let context = self.command.output_open_context(&self.state);
                    let mut report = self.state.resume_error_report(deferred);
                    report.context(context);
                    report.error().jump_out()?;
                    // §403 assigns `cur_cmd=left_brace` and increments
                    // `align_state` exactly as raw delivery of that synthetic
                    // brace would have done. The token itself is not pushed:
                    // the caller continues after it while the rejected command
                    // remains first on the backed-up input level.
                    self.command.alignment.align_state += 1;
                    return Ok(ScannedLeftBrace::Inserted);
                }
            }
        }
    }

    /// Scans the prefix before a macro replacement's compulsory opening
    /// brace.  Compact `Token::Param` values are the stored out-parameter
    /// representation; doubled hashes remain literal parameter characters.
    fn scan_parameter_text(&mut self) -> Result<ScannedParameterText, CommandError> {
        let output = self.begin_attempt_token_list()?;
        let mut next_parameter = 1_u8;
        let mut primary = OriginId::UNKNOWN;
        let mut malformed_parameter = false;
        loop {
            let command = self.get_token()?.ok_or(CommandError::input_invariant())?;
            if primary == OriginId::UNKNOWN {
                primary = command.origin();
            }
            let token = command.spelling().semantic_token();
            if is_begin_group(token) {
                return Ok(ScannedParameterText {
                    tokens: self.finish_attempt_token_list(output)?,
                    highest_parameter: next_parameter - 1,
                    hash_brace: None,
                    primary,
                    malformed_parameter,
                    missing_left_brace: false,
                });
            }
            if is_end_group(token) {
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
                self.command.alignment.align_state += 1;
                let context = self.command.output_open_context(&self.state);
                let mut report = self.state.print_err("Missing { inserted");
                report
                    .help(&[
                        "Where was the left brace? You said something like `\\def\\a}',",
                        "which I'm going to interpret as `\\def\\a{}'.",
                    ])
                    .context(context);
                report.error().jump_out()?;
                return Ok(ScannedParameterText {
                    tokens: self.finish_attempt_token_list(output)?,
                    highest_parameter: next_parameter - 1,
                    hash_brace: None,
                    primary,
                    malformed_parameter,
                    missing_left_brace: true,
                });
            }
            if !is_parameter(token) {
                self.push_attempt_token(output, command.spelling())?;
                continue;
            }
            let follower = self.get_token()?.ok_or(CommandError::input_invariant())?;
            let follower_token = follower.spelling().semantic_token();
            if is_begin_group(follower_token) {
                self.push_attempt_token(output, follower.spelling())?;
                return Ok(ScannedParameterText {
                    tokens: self.finish_attempt_token_list(output)?,
                    highest_parameter: next_parameter - 1,
                    hash_brace: Some(follower.spelling()),
                    primary,
                    malformed_parameter,
                    missing_left_brace: false,
                });
            }
            if let Some(number) = parameter_number(follower_token)
                && number == next_parameter
                && number <= 9
            {
                if let Token::Char {
                    ch,
                    cat: Catcode::Parameter,
                } = token
                    && ch != '#'
                {
                    // TeX82 §476's match token retains `cur_chr`, i.e. the
                    // actual parameter-character code. Keep that spelling
                    // beside the compact slot token when it is not `#`.
                    self.push_attempt_token(output, command.spelling())?;
                }
                self.push_attempt_token(
                    output,
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
            // §476's text is already rendered by
            // `report_macro_parameter_diagnostic` below; this records only
            // the recovery identity.
            self.back_error(follower, NONCONSECUTIVE_PARAMETER_DIAGNOSTIC)?;
            self.report_macro_parameter_diagnostic(MacroParameterDiagnostic::NonconsecutiveNumber)?;
            malformed_parameter = true;
            if next_parameter <= 9 {
                self.push_attempt_token(
                    output,
                    TracedTokenWord::pack(Token::Param(next_parameter), command.origin()),
                )?;
                next_parameter += 1;
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
        progress: ReplacementProgress<G>,
    ) -> Result<AttemptTokenBufferId, ReplacementFailure<G>> {
        let ReplacementProgress {
            output,
            mut depth,
            mut pending_parameter,
            mut pending_expansion,
        } = progress;
        loop {
            let delivered;
            let spelling = {
                let resumed_expansion = pending_expansion.take();
                let command = if let Some(pending) = resumed_expansion.as_ref() {
                    self.resume_current_command(&pending.command);
                    Some(pending.command)
                } else if expansion.is_expanded() {
                    match self.get_next() {
                        Ok(command) => command,
                        Err(error) => {
                            return Err(replacement_failure(
                                error,
                                output,
                                depth,
                                &mut pending_parameter,
                                None,
                            ));
                        }
                    }
                } else {
                    match self.get_token() {
                        Ok(command) => command,
                        Err(error) => {
                            return Err(replacement_failure(
                                error,
                                output,
                                depth,
                                &mut pending_parameter,
                                None,
                            ));
                        }
                    }
                };
                let mut command =
                    command
                        .ok_or_else(CommandError::input_invariant)
                        .map_err(|error| {
                            replacement_failure(error, output, depth, &mut pending_parameter, None)
                        })?;
                if expansion.is_expanded() && is_expandable_command(&command) {
                    let route = resumed_expansion
                        .as_ref()
                        .map(|pending| pending.route)
                        .unwrap_or_else(|| match command.meaning() {
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
                        });
                    if route == CollectorExpansionRoute::The {
                        // TeX82 §478 handles `\the` directly in `scan_toks`
                        // instead of routing it through §380's ordinary
                        // expanded-fetch loop. It therefore has only the raw
                        // delivery produced by `get_next`; the resulting
                        // `the_toks` splice is the canonical expansion event.
                        match self.append_direct_the_toks(output) {
                            Ok(true) => continue,
                            Ok(false) => {}
                            Err(error) => {
                                return Err(replacement_failure(
                                    error,
                                    output,
                                    depth,
                                    &mut pending_parameter,
                                    Some(PendingCollectorExpansion { command, route }),
                                ));
                            }
                        }
                    }
                    if route == CollectorExpansionRoute::Unexpanded {
                        match self.append_unexpanded(output) {
                            Ok(()) => continue,
                            Err(error) => {
                                return Err(replacement_failure(
                                    error,
                                    output,
                                    depth,
                                    &mut pending_parameter,
                                    Some(PendingCollectorExpansion { command, route }),
                                ));
                            }
                        }
                    }
                    if route == CollectorExpansionRoute::Detokenize {
                        match self.append_detokenize(output) {
                            Ok(()) => continue,
                            Err(error) => {
                                return Err(replacement_failure(
                                    error,
                                    output,
                                    depth,
                                    &mut pending_parameter,
                                    Some(PendingCollectorExpansion { command, route }),
                                ));
                            }
                        }
                    }
                    if matches!(command.meaning(), ResolvedMeaning::Macro { flags, .. } if flags.contains(MeaningFlags::PROTECTED))
                    {
                        // e-TeX 2.6 change section [27.465] represents a
                        // protected macro as `relax/no_expand_flag` for this
                        // collector iteration. The spelling is retained, and
                        // the reference instrumentation observes that exact
                        // one-token suppression splice before the terminal
                        // expanded delivery.
                        observe!(
                            self,
                            CommandObservation::TokenList(TokenListRecord {
                                transition: "splice",
                                purpose: "protected_expansion_suppression",
                                tokens: vec![self.observed_token(command.spelling())],
                            }),
                        );
                        command.suppress_expandable();
                    } else {
                        // TeX82 §394 returns from a failed macro call after
                        // either an ordinary non-`\long` `\par` or §23's
                        // outer-validity recovery. Both return to §380's
                        // get_x_token loop; this inlined expanded collector is
                        // that loop's owner while scan_toks is active.
                        match self.expand(&command) {
                            Ok(()) | Err(CommandError::ParagraphInMacroArgument) => continue,
                            Err(CommandError::OuterInMacroArgument) => {
                                self.resume_scanner_episode_after_recovery(episode);
                                continue;
                            }
                            Err(error) => {
                                return Err(replacement_failure(
                                    error,
                                    output,
                                    depth,
                                    &mut pending_parameter,
                                    Some(PendingCollectorExpansion { command, route }),
                                ));
                            }
                        }
                    }
                }

                // The expanded collector has completed a get_x-style delivery
                // for each retained unexpandable token. Emit that boundary before
                // storing the spelling, while expandable commands above remain
                // represented by their own expansion transitions.
                if expansion.is_expanded() {
                    self.observe_expanded_delivery(&command);
                }
                let spelling = command.spelling();
                delivered = command;
                spelling
            };

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
            if delivered.is_outer_recovery_space() {
                continue;
            }
            let token = spelling.semantic_token();
            if let Some((hash, highest_parameter, target)) = pending_parameter.take() {
                // §479: a second parameter character stores that character
                // once -- `##` is one parameter token in the body, not two.
                if is_parameter(token) {
                    self.push_replacement_token(output, spelling, depth, &mut pending_parameter)?;
                    continue;
                }
                if let Some(number) = parameter_number(token)
                    && number <= highest_parameter
                {
                    let converted = TracedTokenWord::pack(Token::Param(number), spelling.origin());
                    self.push_replacement_token(output, converted, depth, &mut pending_parameter)?;
                    observe!(
                        self,
                        CommandObservation::TokenList(TokenListRecord {
                            transition: "splice",
                            purpose: "parameter_conversion",
                            tokens: vec![self.observed_token(converted)],
                        }),
                    );
                    continue;
                }
                // §479's text is already rendered by
                // `report_macro_parameter_diagnostic` below.
                if let Err(error) =
                    self.back_error(delivered, ILLEGAL_REPLACEMENT_PARAMETER_DIAGNOSTIC)
                {
                    return Err(replacement_failure(
                        error,
                        output,
                        depth,
                        &mut pending_parameter,
                        None,
                    ));
                }
                if let Err(error) = self.report_macro_parameter_diagnostic(
                    MacroParameterDiagnostic::IllegalReplacementNumber { target },
                ) {
                    return Err(replacement_failure(
                        error,
                        output,
                        depth,
                        &mut pending_parameter,
                        None,
                    ));
                }
                self.push_replacement_token(output, hash, depth, &mut pending_parameter)?;
                continue;
            }
            if let Some((highest_parameter, target)) = macro_parameters
                && is_parameter(token)
            {
                pending_parameter = Some((spelling, highest_parameter, target));
                continue;
            }
            if is_begin_group(token) {
                depth = depth.saturating_add(1);
                self.push_replacement_token(output, spelling, depth, &mut pending_parameter)?;
            } else if is_end_group(token) {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Ok(output);
                }
                self.push_replacement_token(output, spelling, depth, &mut pending_parameter)?;
            } else {
                self.push_replacement_token(output, spelling, depth, &mut pending_parameter)?;
            }
        }
    }

    fn push_replacement_token(
        &mut self,
        output: AttemptTokenBufferId,
        word: TracedTokenWord,
        depth: u32,
        pending_parameter: &mut Option<(TracedTokenWord, u8, Option<Symbol>)>,
    ) -> Result<(), ReplacementFailure<G>> {
        self.push_attempt_token(output, word)
            .map_err(|error| replacement_failure(error, output, depth, pending_parameter, None))
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
        let context = self.command.output_open_context(&self.state);
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
                report.error().jump_out()?;
            }
            MacroParameterDiagnostic::TooManyParameters => {
                let mut report = self.state.print_err("You already have nine parameters");
                report
                    .help(&[
                        "I'm going to ignore the # sign you just used,",
                        "as well as the token that followed it.",
                    ])
                    .context(context);
                report.error().jump_out()?;
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
                report.error().jump_out()?;
            }
        }
        Ok(())
    }

    /// Splices a token-list result of `\the` into the builder directly.
    /// The target alone is read; no input from after that target is examined.
    fn append_direct_the_toks(
        &mut self,
        output: AttemptTokenBufferId,
    ) -> Result<bool, CommandError> {
        let target = self.get_x_token()?.ok_or(CommandError::input_invariant())?;
        let Some(value) = self.scan_the_internal_value(&target)? else {
            self.back_input(target)?;
            return Ok(false);
        };
        let tokens = match value {
            crate::InternalValue::Font(symbol) => {
                vec![TracedTokenWord::pack(Token::Cs(symbol), OriginId::UNKNOWN)]
            }
            crate::InternalValue::Tokens { tokens, .. } => self
                .command
                .attempt_token_words(tokens)
                .map_err(crate::scan_toks::attempt_command_error)?
                .iter()
                .copied()
                .map(|token| TracedTokenWord::pack(token.semantic_token(), OriginId::UNKNOWN))
                .collect::<Vec<_>>(),
            value => crate::processor::render_the_value(&value)
                .expect("non-token internal values render")
                .chars()
                .map(|ch| {
                    TracedTokenWord::pack(
                        Token::Char {
                            ch,
                            cat: if ch == ' ' {
                                Catcode::Space
                            } else {
                                Catcode::Other
                            },
                        },
                        OriginId::UNKNOWN,
                    )
                })
                .collect(),
        };
        // Built only for an observed episode: an unobserved one leaves this
        // empty, which `Vec::new` does without allocating.
        let observed: Vec<_> = if self.is_observed() {
            tokens
                .iter()
                .copied()
                .map(|token| self.observed_token(token))
                .collect()
        } else {
            Vec::new()
        };
        for token in tokens {
            self.push_attempt_token(output, token)?;
        }
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
    fn append_unexpanded(&mut self, output: AttemptTokenBufferId) -> Result<(), CommandError> {
        let scanned = self.scan_toks(ScanToksMode::GeneralText {
            purpose: "unexpanded",
        })?;
        let raw = self.attempt_words(scanned.replacement_text)?.to_vec();
        let observed = raw
            .iter()
            .copied()
            .map(|token| self.observed_token(token))
            .collect::<Vec<_>>();
        for token in raw {
            self.push_attempt_token(output, token)?;
        }
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
    fn append_detokenize(&mut self, output: AttemptTokenBufferId) -> Result<(), CommandError> {
        let scanned = self.scan_toks(ScanToksMode::GeneralText {
            purpose: "detokenize",
        })?;
        let semantic_tokens = self
            .attempt_words(scanned.replacement_text)?
            .iter()
            .map(|word| word.semantic_token())
            .collect::<Vec<_>>();
        let text =
            crate::processor::expand::token_slice_string_text(&mut self.state, &semantic_tokens);
        let tokens = text
            .chars()
            .map(|ch| {
                TracedTokenWord::pack(
                    Token::Char {
                        ch,
                        cat: if ch == ' ' {
                            Catcode::Space
                        } else {
                            Catcode::Other
                        },
                    },
                    OriginId::UNKNOWN,
                )
            })
            .collect::<Vec<_>>();
        let observed = tokens
            .iter()
            .copied()
            .map(|token| self.observed_token(token))
            .collect::<Vec<_>>();
        for token in tokens {
            self.push_attempt_token(output, token)?;
        }
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
        let scanned = self.scan_toks(ScanToksMode::GeneralText {
            purpose: "unexpanded",
        })?;
        let words = self.attempt_words(scanned.replacement_text)?.to_vec();
        let first = words.first().map(|word| word.semantic_token());
        self.insert_expansion_list(TokenPayload::transient(words), first);
        Ok(())
    }
}

fn parameter_text_for_runaway<G>(
    arena: &crate::attempt::AttemptArena<G>,
    result: &ScannedToksBuffers,
) -> Result<Vec<TracedTokenWord>, CommandError> {
    let mut tokens: Vec<_> = arena
        .token_words(result.parameter_text)
        .map_err(attempt_command_error)?
        .iter()
        .map(|word| TracedTokenWord::pack(word.semantic_token(), OriginId::UNKNOWN))
        .collect();
    // TeX82 §§294/306/473 store one `def_ref` list whose `end_match_token`
    // separates parameter text from replacement text and prints as `->`.
    // Umber owns those halves as separate immutable lists, so reconstruct the
    // sentinel's diagnostic spelling before appending the replacement below.
    tokens.extend(['-', '>'].map(|ch| {
        TracedTokenWord::pack(
            Token::Char {
                ch,
                cat: Catcode::Other,
            },
            OriginId::UNKNOWN,
        )
    }));
    Ok(tokens)
}

pub(crate) fn attempt_command_error(error: AttemptError) -> CommandError {
    match error {
        AttemptError::CapacityOverflow | AttemptError::AllocationFailed => CommandError::Fatal(
            crate::FatalError::overflow("scanner token storage", i32::MAX),
        ),
        AttemptError::ForeignAttempt
        | AttemptError::InvalidCoordinate
        | AttemptError::Promotion(_) => CommandError::input_invariant(),
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
    ) -> Result<AttemptTokenListId, CommandError> {
        let mark = self.command.attempt.arena().mark();
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
        self.command.alignment.align_state = TEMPLATE_ALIGN_STATE;
        let result = self.read_toks_lines(stream, target, raw_catcodes);
        self.command.alignment.align_state = saved_align_state;
        self.finish_scanner_episode(episode);
        let tokens = match result {
            Ok(tokens) => tokens,
            Err(error) => {
                self.command
                    .attempt
                    .arena_mut()
                    .truncate(mark)
                    .map_err(attempt_command_error)?;
                return Err(error);
            }
        };
        // §482 leaves the collected list in `cur_val`; §1225 immediately
        // installs it with `define(p,call,cur_val)`. Unlike §473's
        // `scan_toks`, this is not an independently observable completed
        // token-list assignment. The committed observation is §1225's
        // meaning mutation, whose macro body includes §482's leading
        // `end_match_token`.
        match self.finish_attempt_token_list(tokens) {
            Ok(tokens) => Ok(tokens),
            Err(error) => {
                self.command
                    .attempt
                    .arena_mut()
                    .truncate(mark)
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
    ) -> Result<AttemptTokenBufferId, CommandError> {
        // §482: `if (n<0)or(n>15) then m:=16 else m:=n`. Stream 16 is never
        // open, so §483 always takes §484's terminal branch for it.
        let slot = u8::try_from(stream)
            .ok()
            .filter(|slot| *slot < tex_state::world::STREAM_SLOT_COUNT as u8)
            .map(tex_state::world::StreamSlot::new);
        let tokens = self.begin_attempt_token_list()?;
        // §484's own `n`, which decides whether the user is prompted at all:
        // a negative stream is prompted with the empty string, so `\read-1 to
        // \x` never prints `\x=`. §484 then assigns `n:=-1` after prompting,
        // "so that additional prompts will not be given in the case of
        // multi-line input" -- one variable serving both rules.
        let mut prompt_number = stream;
        loop {
            self.read_toks_line(slot, target, raw_catcodes, &mut prompt_number, tokens)?;
            if self.command.alignment.align_state == TEMPLATE_ALIGN_STATE {
                return Ok(tokens);
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
        tokens: AttemptTokenBufferId,
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
            let endlinechar = self
                .state
                .int_param(tex_state::env::banks::IntParam::END_LINE_CHAR);
            self.command.load_next_source_line(endlinechar);
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
            partial.extend(
                self.command
                    .attempt
                    .arena()
                    .token_buffer(tokens)
                    .map_err(attempt_command_error)?
                    .iter()
                    .copied(),
            );
            let context = self.command.output_open_context(&self.state);
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
            self.command.alignment.align_state = TEMPLATE_ALIGN_STATE;
        }
        if raw_catcodes {
            self.collect_read_line_verbatim(level, tokens)?;
            if file_ended {
                self.command.alignment.align_state = TEMPLATE_ALIGN_STATE;
            }
            return Ok(());
        }
        // §483: `loop get_token; if cur_tok=0 then goto done; if
        // align_state<1000000 then {unmatched `}' aborts the line} begin
        // repeat get_token until cur_tok=0; align_state:=1000000; goto done;
        // end; store_new_token(cur_tok); end`.
        while let Some(command) = self.get_token()? {
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
                runaway.extend(
                    self.command
                        .attempt
                        .arena()
                        .token_buffer(tokens)
                        .map_err(attempt_command_error)?
                        .iter()
                        .copied(),
                );
                self.set_runaway_partial(crate::processor::RUNAWAY_SCAN_DIAGNOSTIC, &runaway);
            }
            if self.command.alignment.align_state < TEMPLATE_ALIGN_STATE {
                while self.get_token()?.is_some() {}
                self.command.alignment.align_state = TEMPLATE_ALIGN_STATE;
                return Ok(());
            }
            self.push_attempt_token(tokens, command.spelling())?;
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
        tokens: AttemptTokenBufferId,
    ) -> Result<(), CommandError> {
        let endlinechar = self
            .state
            .int_param(tex_state::env::banks::IntParam::END_LINE_CHAR);
        self.command.load_next_source_line(endlinechar);
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
            self.push_attempt_token(
                tokens,
                TracedTokenWord::pack(Token::Char { ch, cat }, origin),
            )?;
        }
        self.retire_read_line_level(level)?;
        Ok(())
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
            .ok_or_else(CommandError::input_invariant)?;
        Ok((line, false, crate::input::SourceNameClass::Terminal))
    }
}

#[cfg(test)]
mod tests;
