//! Future-relevant state and discardable runtime ownership.

use tex_state::input::TracedTokenList;
use tex_state::token::TracedTokenWord;

#[cfg(any(test, feature = "instrumentation"))]
use crate::AlignmentRecord;
use crate::conditionals::ConditionStack;
use crate::input::InputState;
use crate::input::{
    InputLevel, InputLevelId, PhysicalLine, RegisteredSource, SourceCharacter, SourceCursor,
    SourceLevel, SourceRegistration, SourceRegistrationError, SourceTokenizationStep,
};
use crate::input::{ReplayTrace, RetirementBehavior, TokenBehavior, TokenPayload};
use crate::macro_call::ParameterState;
#[cfg(any(test, feature = "instrumentation"))]
use crate::processor::CELL_ALIGN_STATE;
use crate::processor::{
    AlignmentCellTemplates, AlignmentDeliveryState, AlignmentIdentity, AlignmentLifecycleError,
    AlignmentRequest, AlignmentRequestResult, ExpansionState, ScannerState,
};
use crate::profile::{
    CommandProfile, CommandProfileBoundary, CommandProfileFingerprint, CommandProfileMismatch,
};

/// Complete future-relevant state owned by the command machine.
///
/// This is the command half of an executor savepoint. It contains semantic
/// and rollback-coupled provenance state only: host capabilities, aggregate
/// engine state, call-local accumulators, and discardable accelerations are
/// deliberately absent.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub struct CommandState {
    pub(crate) input: InputState,
    pub(crate) parameters: ParameterState,
    pub(crate) scanner: ScannerState,
    pub(crate) conditions: ConditionStack,
    pub(crate) alignment: AlignmentDeliveryState,
    pub(crate) expansion: ExpansionState,
    pub(crate) transient: TransientState,
}

impl CommandState {
    /// Returns the committed observation for an executor-applied alignment
    /// begin transition.
    ///
    /// The executor supplies the structural transition, while command state
    /// remains the owner of its align-state and stable alignment identity.
    /// Keeping that projection here prevents replay instrumentation from
    /// reconstructing either value from raw input.
    #[cfg(any(test, feature = "instrumentation"))]
    #[must_use]
    pub fn alignment_begin_observation(&self) -> Option<AlignmentRecord> {
        self.alignment
            .active_alignment
            .map(|alignment| AlignmentRecord {
                transition: "begin",
                alignment: Some(alignment.raw()),
                align_state: self.alignment.align_state,
                delimiter: None,
                previous_align_state: None,
            })
    }

    /// Applies an executor-owned structural alignment request.
    ///
    /// This is the only lifecycle entry point required by `tex-exec`.  It has
    /// no token input, so it cannot duplicate `get_next` delimiter or brace
    /// classification.  Starting a v-template is intentionally absent: that
    /// transition requires an [`crate::AlignmentDeliveryEvent`] and is owned
    /// by [`crate::CommandProcessor`].
    pub fn apply_alignment_request(
        &mut self,
        request: AlignmentRequest,
    ) -> Result<AlignmentRequestResult, AlignmentLifecycleError> {
        match request {
            AlignmentRequest::Begin(alignment) => {
                self.begin_alignment(alignment);
                Ok(AlignmentRequestResult::Applied)
            }
            AlignmentRequest::Preamble(alignment) => {
                self.set_alignment_preamble_phase(alignment)?;
                Ok(AlignmentRequestResult::Applied)
            }
            AlignmentRequest::BeginCell {
                alignment,
                templates,
            } => {
                self.begin_alignment_cell(alignment, templates)?;
                Ok(AlignmentRequestResult::Applied)
            }
            AlignmentRequest::InstallCellTemplate(alignment) => {
                self.install_alignment_cell_template(alignment)?;
                Ok(AlignmentRequestResult::Applied)
            }
            AlignmentRequest::FinishCell(alignment) => Ok(AlignmentRequestResult::FinishedCell(
                self.finish_alignment_cell(alignment)?,
            )),
            AlignmentRequest::Suspend(alignment) => {
                self.suspend_alignment(alignment)?;
                Ok(AlignmentRequestResult::Applied)
            }
            AlignmentRequest::Resume(alignment) => {
                self.resume_alignment(alignment)?;
                Ok(AlignmentRequestResult::Applied)
            }
            AlignmentRequest::Finish(alignment) => {
                self.finish_alignment(alignment)?;
                Ok(AlignmentRequestResult::Applied)
            }
        }
    }

    /// Begins an executor-owned structural alignment at the canonical preamble
    /// sentinel. Delimiter classification remains exclusively in `get_next`.
    pub fn begin_alignment(&mut self, alignment: AlignmentIdentity) {
        self.alignment.begin_alignment(alignment);
    }

    /// Re-enters the preamble sentinel while scanning another alignment column.
    pub fn set_alignment_preamble_phase(
        &mut self,
        alignment: AlignmentIdentity,
    ) -> Result<(), AlignmentLifecycleError> {
        self.alignment.set_preamble_phase(alignment)
    }

    /// Marks one cell's executor-selected templates active and establishes the
    /// body brace-depth base. This operation does not inspect input tokens.
    ///
    /// The source opening brace must be delivered and backed up through a
    /// command processor before [`Self::install_alignment_cell_template`]
    /// installs the optional u-template.
    pub fn begin_alignment_cell(
        &mut self,
        alignment: AlignmentIdentity,
        templates: AlignmentCellTemplates,
    ) -> Result<(), AlignmentLifecycleError> {
        self.alignment.begin_cell(alignment, templates)
    }

    /// Installs the active cell's optional u-template after the executor's
    /// typed opener phase has completed command-owned brace replay.
    pub fn install_alignment_cell_template(
        &mut self,
        alignment: AlignmentIdentity,
    ) -> Result<(), AlignmentLifecycleError> {
        let template = self.alignment.active_cell_template(alignment)?;
        if let Some(template) = template {
            let level = self.push_alignment_template(
                template,
                TokenBehavior::UTemplate,
                RetirementBehavior::Pop,
                ReplayTrace::UTemplate,
            );
            self.alignment.attach_u_template(alignment, level)?;
        } else {
            self.alignment.mark_u_template_installed(alignment)?;
        }
        Ok(())
    }

    /// Returns the committed input push for a just-installed u-template.
    ///
    /// The level identity is allocated by the state transition itself, so
    /// instrumentation can report the canonical input lifecycle without
    /// reconstructing a template push from executor state or token contents.
    #[cfg(any(test, feature = "instrumentation"))]
    #[must_use]
    pub fn alignment_u_template_push_observation(
        &self,
        alignment: AlignmentIdentity,
    ) -> Option<crate::InputRecord> {
        let cell = self.alignment.active_cell.as_ref()?;
        (cell.alignment == alignment).then_some(())?;
        cell.u_level.map(|level| crate::InputRecord {
            transition: crate::InputTransition::Push,
            reason: crate::InputReason::AlignmentUTemplate,
            level: level.0,
            position: 0,
        })
    }

    /// Returns the command-owned alignment transition paired with the
    /// u-template input push.
    #[cfg(any(test, feature = "instrumentation"))]
    #[must_use]
    pub fn alignment_u_template_push_alignment_observation(
        &self,
        alignment: AlignmentIdentity,
    ) -> Option<crate::AlignmentRecord> {
        self.alignment_u_template_push_observation(alignment)
            .map(|_| crate::AlignmentRecord {
                transition: "u_template_push",
                alignment: Some(alignment.raw()),
                align_state: self.alignment.align_state,
                delimiter: None,
                previous_align_state: None,
            })
    }

    /// Transfers one completed raw preamble to the executor for structural
    /// column selection. The returned templates remain frozen command-owned
    /// values; no raw preamble token is exposed.
    pub fn take_completed_alignment_preamble(
        &mut self,
        alignment: AlignmentIdentity,
    ) -> Result<crate::AlignmentPreamble, AlignmentLifecycleError> {
        self.alignment.take_completed_preamble(alignment)
    }

    /// Returns the committed observation for an executor-selected first cell.
    ///
    /// The executor requests the transition, while command state remains the
    /// source of the resulting `align_state`; this avoids deriving an event
    /// from either template contents or raw input.
    #[cfg(any(test, feature = "instrumentation"))]
    #[must_use]
    pub fn alignment_cell_begin_observation(&self) -> Option<AlignmentRecord> {
        self.alignment
            .active_cell
            .as_ref()
            .map(|cell| AlignmentRecord {
                transition: "state_change",
                alignment: Some(cell.alignment.raw()),
                align_state: self.alignment.align_state,
                delimiter: None,
                previous_align_state: None,
            })
    }

    /// Starts the selected cell's v-template after `end_template` main control
    /// has backed up the intercepted delimiter. The suffix is an ordinary
    /// input level, so definitions and macro expansion inside it restart via
    /// the canonical raw-delivery loop.
    pub fn begin_alignment_v_template(
        &mut self,
        alignment: AlignmentIdentity,
    ) -> Result<(), AlignmentLifecycleError> {
        let template = self.alignment.v_template(alignment)?;
        let level = self.push_alignment_template(
            template,
            TokenBehavior::VTemplate,
            RetirementBehavior::RetainExhaustedVTemplate,
            ReplayTrace::VTemplate,
        );
        self.alignment.begin_v_template(alignment, level)
    }

    /// Returns the committed v-template push made after a command-owned
    /// delimiter interception.
    #[cfg(any(test, feature = "instrumentation"))]
    #[must_use]
    pub fn alignment_v_template_push_observation(
        &self,
        alignment: AlignmentIdentity,
    ) -> Option<crate::InputRecord> {
        let cell = self.alignment.active_cell.as_ref()?;
        (cell.alignment == alignment).then_some(())?;
        cell.v_level.map(|level| crate::InputRecord {
            transition: crate::InputTransition::Push,
            reason: crate::InputReason::AlignmentVTemplate,
            level: level.0,
            position: 0,
        })
    }

    /// Returns the template lifecycle transition paired with the v-template
    /// input push, without exposing template tokens to the executor.
    #[cfg(any(test, feature = "instrumentation"))]
    #[must_use]
    pub fn alignment_v_template_push_alignment_observation(
        &self,
        alignment: AlignmentIdentity,
    ) -> Option<AlignmentRecord> {
        self.alignment_v_template_push_observation(alignment)
            .map(|_| AlignmentRecord {
                transition: "v_template_push",
                alignment: Some(alignment.raw()),
                // TeX82's v-template insertion (`init_col`) begins the
                // token list before assigning the post-insertion sentinel.
                // The command state is already guarded against a second
                // delimiter, but the committed lifecycle records that
                // canonical pre-sentinel point.
                align_state: CELL_ALIGN_STATE,
                delimiter: None,
                previous_align_state: None,
            })
    }

    /// Retires the exact exhausted v-template after a delivered frozen end-v.
    /// The caller is the executor's `do_endv` boundary; no token classifier or
    /// template loop exists outside the command input stack.
    pub fn finish_alignment_cell(
        &mut self,
        alignment: AlignmentIdentity,
    ) -> Result<AlignmentCellTemplates, AlignmentLifecycleError> {
        let level = self.alignment.active_v_template_level(alignment)?;
        self.retire_retained_v_template(level)
            .map_err(|_| AlignmentLifecycleError::VTemplateNotExhausted)?;
        self.alignment.finish_cell(alignment, level)
    }

    /// Suspends the complete outer raw-delivery context for a nested alignment.
    pub fn suspend_alignment(
        &mut self,
        alignment: AlignmentIdentity,
    ) -> Result<(), AlignmentLifecycleError> {
        self.alignment.suspend_alignment(alignment)
    }

    /// Restores the exact outer raw-delivery context after a nested alignment.
    pub fn resume_alignment(
        &mut self,
        alignment: AlignmentIdentity,
    ) -> Result<(), AlignmentLifecycleError> {
        self.alignment.resume_alignment(alignment)
    }

    /// Finishes an alignment delivery context after all of its cells retire.
    pub fn finish_alignment(
        &mut self,
        alignment: AlignmentIdentity,
    ) -> Result<(), AlignmentLifecycleError> {
        self.alignment.finish_alignment(alignment)
    }

    /// Creates a fresh command job with an immutable semantic profile.
    ///
    /// No API changes the profile after construction. Snapshot, summary,
    /// format, and checkpoint restoration validate their recorded profile
    /// identity against this value.
    #[must_use]
    pub fn new(profile: CommandProfile) -> Self {
        Self {
            expansion: ExpansionState {
                profile,
                ..ExpansionState::default()
            },
            ..Self::default()
        }
    }

    /// Registers complete immutable backing without consulting host policy.
    ///
    /// Registration validates Unicode before allocating an identity. It does
    /// not open an input level or perform any tokenization.
    pub fn register_source(
        &mut self,
        registration: SourceRegistration,
    ) -> Result<tex_state::SourceId, SourceRegistrationError> {
        let raw = u32::try_from(self.input.next_source_identity)
            .map_err(|_| SourceRegistrationError::SourceIdentityExhausted)?;
        let id = tex_state::SourceId::new(raw);
        let source = RegisteredSource::register(id, self.profile(), registration)?;
        self.input.next_source_identity += 1;
        self.input.registered_sources.push(source);
        Ok(id)
    }

    /// Opens an already registered source as a future input level.
    ///
    /// This operation only clones retained immutable backing. It cannot search
    /// for files, invoke a host callback, or diagnose text encoding.
    pub fn open_registered_source(
        &mut self,
        source: tex_state::SourceId,
    ) -> Result<(), UnknownRegisteredSource> {
        let registered = self
            .input
            .registered_sources
            .iter()
            .find(|registered| registered.id == source)
            .cloned()
            .ok_or(UnknownRegisteredSource(source))?;
        let identity = InputLevelId(self.input.next_level_identity);
        self.input.next_level_identity = self.input.next_level_identity.wrapping_add(1);
        self.input.levels.push(InputLevel::Source(SourceLevel {
            identity,
            cursor: SourceCursor::new(registered),
        }));
        Ok(())
    }

    /// Applies TeX's `\endinput` retirement request to the active physical
    /// source.  The remainder of its current line is still tokenized; no
    /// later physical line may be loaded.
    pub(crate) fn end_current_source_after_current_line(&mut self) -> bool {
        self.input
            .levels
            .iter_mut()
            .rev()
            .find_map(|level| match level {
                InputLevel::Source(level) => Some(level),
                InputLevel::Tokens(_) => None,
            })
            .map(|level| level.cursor.end_after_line = true)
            .is_some()
    }

    fn push_alignment_template(
        &mut self,
        template: TracedTokenList,
        behavior: TokenBehavior,
        retirement: RetirementBehavior,
        trace: ReplayTrace,
    ) -> InputLevelId {
        self.push_token_level(
            TokenPayload::Stored {
                tokens: template.token_list(),
                origins: template.origin_list(),
            },
            behavior,
            retirement,
            trace,
        )
    }

    /// Splits and normalizes the next physical line on the active source.
    ///
    /// LF, CR, and CRLF are retained as distinct physical metadata. TeX
    /// trailing spaces are removed and the current `endlinechar` is captured
    /// for this line without tokenizing any characters.
    pub fn load_next_source_line(&mut self, endlinechar: i32) -> Option<PhysicalLine> {
        let InputLevel::Source(level) = self.input.levels.last_mut()? else {
            return None;
        };
        level
            .cursor
            .load_next_line(endlinechar)
            .map(|line| line.physical)
    }

    /// Reads one byte-domain character or decoded Unicode scalar from the
    /// active normalized line with its exact physical range.
    pub fn next_source_character(&mut self) -> Option<SourceCharacter> {
        let InputLevel::Source(level) = self.input.levels.last_mut()? else {
            return None;
        };
        let mode = level.cursor.backing.mode;
        let bytes = std::sync::Arc::clone(&level.cursor.backing.bytes);
        level.cursor.line.as_mut()?.next_character(mode, &bytes)
    }

    /// Retires the active normalized line so the next physical line may load.
    pub fn finish_source_line(&mut self) {
        if let Some(InputLevel::Source(level)) = self.input.levels.last_mut() {
            level.cursor.finish_line();
        }
    }

    /// Tokenizes one exact-byte source step using the caller's live catcodes.
    ///
    /// The callback is queried independently for every classified character;
    /// it is not retained or cached across tokens. Invalid characters are
    /// returned as recoverable steps after their complete spelling is
    /// consumed.
    ///
    /// # Panics
    ///
    /// Panics when called for the separately implemented Unicode character
    /// profile.
    pub fn next_exact_source_step(
        &mut self,
        endlinechar: i32,
        catcode: impl FnMut(crate::CharacterCode) -> tex_state::token::Catcode,
    ) -> SourceTokenizationStep {
        assert_eq!(
            self.profile().character_mode(),
            crate::CharacterMode::EightBitExact,
            "exact-byte tokenization requires an exact-byte command profile"
        );
        let Some(InputLevel::Source(level)) = self.input.levels.last_mut() else {
            return SourceTokenizationStep::End;
        };
        level.cursor.next_exact_byte_step(endlinechar, catcode)
    }

    /// Tokenizes one Unicode-scalar source step using the caller's live code
    /// table.
    ///
    /// The callback receives only Unicode-domain [`crate::CharacterCode`]
    /// values, including synthetic `endlinechar` and superscript-reduction
    /// results. Sparse-table defaults belong to the aggregate code table, not
    /// this tokenizer.
    ///
    /// # Panics
    ///
    /// Panics when called for an exact-byte command profile.
    pub fn next_unicode_source_step(
        &mut self,
        endlinechar: i32,
        catcode: impl FnMut(crate::CharacterCode) -> tex_state::token::Catcode,
    ) -> SourceTokenizationStep {
        assert_eq!(
            self.profile().character_mode(),
            crate::CharacterMode::UnicodeExtended,
            "Unicode tokenization requires a UnicodeExtended command profile"
        );
        let Some(InputLevel::Source(level)) = self.input.levels.last_mut() else {
            return SourceTokenizationStep::End;
        };
        level.cursor.next_unicode_step(endlinechar, catcode)
    }

    /// Returns the immutable profile selected when this job was created.
    #[must_use]
    pub const fn profile(&self) -> CommandProfile {
        self.expansion.profile
    }

    /// Returns the profile component required in portable format identity.
    #[must_use]
    pub fn format_profile_fingerprint(&self) -> CommandProfileFingerprint {
        self.profile().fingerprint()
    }

    /// Rejects a format image produced for a different command profile.
    pub fn validate_format_profile(
        &self,
        found: CommandProfileFingerprint,
    ) -> Result<(), CommandProfileMismatch> {
        self.profile()
            .validate_fingerprint(CommandProfileBoundary::Format, found)
    }

    /// Returns the profile component required in incremental checkpoint identity.
    #[must_use]
    pub fn checkpoint_profile_fingerprint(&self) -> CommandProfileFingerprint {
        self.profile().fingerprint()
    }

    /// Rejects a checkpoint produced for a different command profile.
    pub fn validate_checkpoint_profile(
        &self,
        found: CommandProfileFingerprint,
    ) -> Result<(), CommandProfileMismatch> {
        self.profile()
            .validate_fingerprint(CommandProfileBoundary::Checkpoint, found)
    }
}

/// An input level referred to a source absent from retained registration.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct UnknownRegisteredSource(tex_state::SourceId);

impl UnknownRegisteredSource {
    /// Returns the missing source identity.
    #[must_use]
    pub const fn source(self) -> tex_state::SourceId {
        self.0
    }
}

impl std::fmt::Display for UnknownRegisteredSource {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "source identity {} is not registered",
            self.0.raw()
        )
    }
}

impl std::error::Error for UnknownRegisteredSource {}

/// Live temporary data referenced by persistent command state.
///
/// Builder contents and rollback roots are semantic while live. Spare
/// capacity and reusable empty buffers instead belong to [`CommandRuntime`].
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub(crate) struct TransientState {
    pub(crate) builders: Vec<LiveTokenBuilder>,
    pub(crate) rollback_roots: Vec<u64>,
    pub(crate) next_builder_identity: u64,
    /// Nesting of the call-local expansion episode currently borrowing the
    /// command machine. This records only quiescence, never a continuation,
    /// accumulator, fuel scope, host capability, or processor borrow.
    pub(crate) active_expansion_depth: u32,
}

/// One semantic token builder named by a scanner-status variant.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct LiveTokenBuilder {
    pub(crate) identity: u64,
    pub(crate) tokens: Vec<TracedTokenWord>,
}

/// Discardable command-processing acceleration and measurements.
///
/// Replacing this value with [`CommandRuntime::default`] at any point cannot
/// change semantic events, diagnostics, effects, output, or `CommandState`.
/// It intentionally implements neither equality nor hashing, preventing it
/// from becoming part of semantic state comparisons by convenience.
#[derive(Debug, Default)]
#[allow(dead_code)] // caches are populated when command semantics are implemented
pub struct CommandRuntime {
    meaning_cache: MeaningCache,
    normalized_lines: LineNormalizationCache,
    transient_pool: TokenBufferPool,
    profiling: CommandProfiling,
}

#[derive(Debug, Default)]
#[allow(dead_code)] // ownership shell
struct MeaningCache {
    entries: Vec<MeaningCacheEntry>,
}

#[derive(Debug)]
#[allow(dead_code)] // ownership shell
struct MeaningCacheEntry {
    identity: u64,
    generation: u64,
}

#[derive(Debug, Default)]
#[allow(dead_code)] // ownership shell
struct LineNormalizationCache {
    entries: Vec<NormalizedLineCacheEntry>,
}

#[derive(Debug)]
#[allow(dead_code)] // ownership shell
struct NormalizedLineCacheEntry {
    content_identity: u64,
    normalized: Vec<u8>,
}

#[derive(Debug, Default)]
#[allow(dead_code)] // ownership shell
struct TokenBufferPool {
    buffers: Vec<Vec<TracedTokenWord>>,
}

#[derive(Debug, Default)]
#[allow(dead_code)] // ownership shell
struct CommandProfiling {
    raw_deliveries: u64,
    cache_hits: u64,
}

#[cfg(test)]
mod tests;
