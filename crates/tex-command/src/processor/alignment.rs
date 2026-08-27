//! Alignment-template delivery state.
//!
//! TeX.web §789--§790 makes `align_state` a property of raw token delivery,
//! not of the executor's mode nest.  Consequently the executor may request
//! lifecycle transitions here, but only `get_next` classifies a delivered tab,
//! `\span`, or row terminator.

#[cfg(test)]
mod tests;

use tex_state::TokenListId;
use tex_state::glue::GlueSpec;
use tex_state::meaning::{Meaning, UnexpandablePrimitive};
use tex_state::token::{Catcode, Token, TracedTokenWord};

use crate::CurrentCommand;
use crate::attempt::AttemptTokenListId;
use crate::input::InputLevelId;

/// Whether TeX82 §24 has resolved this delivery to the given character
/// command code.
///
/// This deliberately examines `cur_cmd` (the effective [`Meaning`]), not the
/// token spelling. Control-sequence aliases therefore participate in §§342 and
/// 760's command-code tests, while the separate spelling match in
/// [`AlignmentDeliveryState::classify_delivery`] keeps literal-brace
/// `align_state` accounting physical.
pub(crate) fn is_character_command<G>(command: &CurrentCommand<G>, cat: Catcode) -> bool {
    matches!(
        command.meaning(),
        tex_state::ResolvedMeaning::Static(Meaning::CharToken { cat: actual, .. }) if actual == cat
    )
}

pub(crate) const PREAMBLE_ALIGN_STATE: i32 = -1_000_000;
pub(crate) const TEMPLATE_ALIGN_STATE: i32 = 1_000_000;
pub(crate) const CELL_ALIGN_STATE: i32 = 0;
/// TeX82 §331's `align_state:=1000000` in `@<Initialize the input routines@>`.
///
/// `align_state` is a single running brace count that `get_next` maintains for
/// the whole run, not a per-alignment counter.  Starting it far above zero is
/// what makes §1127's `abs(align_state)>2` test mean "no alignment entry is in
/// progress here": only material that has actually descended into an alignment
/// cell can bring it near zero.
pub(crate) const TOP_LEVEL_ALIGN_STATE: i32 = 1_000_000;

/// Stable identity supplied by the executor for one structural alignment.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct AlignmentIdentity(u64);

impl AlignmentIdentity {
    /// Creates an executor-owned identity for a structurally active alignment.
    #[must_use]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    #[allow(dead_code)]
    pub(crate) const fn raw(self) -> u64 {
        self.0
    }
}

/// Exact identities of the templates selected for one cell.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct AlignmentCellTemplates {
    /// Prefix replayed before the cell body. `None` is TeX's `\omit` path.
    pub u_template: Option<AttemptTokenListId>,
    /// Suffix replayed between the intercepted delimiter and its re-delivery.
    pub v_template: AttemptTokenListId,
}

/// Generation-durable templates selected for an active alignment cell.
///
/// Raw preamble scanning produces [`AlignmentCellTemplates`] in the current
/// attempt. The executor must atomically promote the complete preamble before
/// constructing this value; active delivery therefore retains no attempt
/// coordinate.
#[derive(Debug)]
pub struct PreparedAlignmentCellTemplates<G> {
    pub u_template: Option<TokenListId<G>>,
    pub v_template: TokenListId<G>,
}

impl<G> Clone for PreparedAlignmentCellTemplates<G> {
    fn clone(&self) -> Self {
        Self {
            u_template: self.u_template.clone(),
            v_template: self.v_template.clone(),
        }
    }
}

impl<G> PartialEq for PreparedAlignmentCellTemplates<G> {
    fn eq(&self, other: &Self) -> bool {
        self.u_template == other.u_template && self.v_template == other.v_template
    }
}

impl<G> Eq for PreparedAlignmentCellTemplates<G> {}

/// The delimiter saved when `get_next` enters a cell's v-template.
///
/// TeX82 keeps this as `extra_info(cur_align)` until `fin_col` consumes it
/// after `do_endv`; it is never ordinary input on the far side of the
/// template.  The executor receives only this structural outcome.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum AlignmentCellDelimiter {
    Tab,
    Span,
    Row,
}

/// Template data and the saved `fin_col` delimiter returned by `do_endv`.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct FinishedAlignmentCell {
    pub delimiter: AlignmentCellDelimiter,
    /// TeX82's live `line` captured when `get_next` intercepted the saved
    /// delimiter, before the v-template and its cold executor boundary.
    pub delimiter_line: u32,
}

/// Frozen template pairs collected during one raw alignment preamble scan.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct AlignmentPreamble {
    pub columns: Vec<AlignmentCellTemplates>,
    /// TeX82 §759's `\tabskip` value at each preamble boundary.
    pub tabskips: Vec<GlueSpec>,
    /// Value left by the last preamble `\tabskip` assignment.
    pub default_tabskip: GlueSpec,
    /// First column of TeX82's periodic `&&` preamble suffix, if present.
    ///
    /// TeX82 §760 consumes the second tab while scanning an otherwise empty
    /// u-template and records `cur_loop`; it is not an empty column. Later
    /// §772 extensions select from this frozen suffix.
    pub repeat_start: Option<usize>,
}

/// A structural transition requested by the executor.
///
/// These requests deliberately contain no token spelling, command meaning,
/// or delimiter kind.  `tex-exec` owns row/cell packaging and asks for these
/// lifecycle changes; canonical raw delivery remains the only place that can
/// decide that a delivered command is an alignment delimiter.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum AlignmentRequest {
    /// Start scanning an alignment preamble.
    Begin(AlignmentIdentity),
    /// Restart preamble scanning for a repeated preamble column.
    Preamble(AlignmentIdentity),
    /// Restore `align_peek`/`init_col`'s lookahead sentinel before a typed
    /// already-delivered `\\omit` result is installed.
    PrepareCellLookahead(AlignmentIdentity),
    /// Install the selected cell's optional u-template after its source
    /// opening brace has been delivered and backed up by command processing.
    InstallCellTemplate(AlignmentIdentity),
    /// Complete `init_col`'s typed `\omit` branch without replaying a
    /// u-template.
    InstallOmitCellTemplate(AlignmentIdentity),
    /// Retire the exhausted v-template for one finished cell.
    FinishCell(AlignmentIdentity),
    /// Convert TeX82 `fin_col`'s saved overrun tab or span to its row-ending
    /// outcome after the executor has established that no preamble column can
    /// be selected. Raw delimiter identity and the recovery observation stay
    /// command-owned.
    RecoverExtraTab(AlignmentIdentity),
    /// Preserve the outer raw-delivery context before entering a nested alignment.
    Suspend(AlignmentIdentity),
    /// Restore the outer raw-delivery context after a nested alignment.
    Resume(AlignmentIdentity),
    /// Tear down a completed alignment delivery context.
    Finish(AlignmentIdentity),
}

/// Result material returned by a structural alignment request.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum AlignmentRequestResult {
    /// The request changed lifecycle state without returning template data.
    Applied,
    /// A finished cell returned the exact templates that were active for it.
    FinishedCell(FinishedAlignmentCell),
    /// TeX82 `fin_col` changed the saved tab/span to `\\cr`.
    ExtraTabRecovered,
}

/// A command-core event that the executor must handle at an alignment boundary.
///
/// The contained command is intentionally opaque.  It can only be handed back
/// to [`crate::CommandProcessor::begin_alignment_v_template`], which backs up
/// the exact delivery before installing the v-template.
#[derive(Debug, Eq, PartialEq)]
pub enum AlignmentDeliveryEvent<G> {
    /// `get_next` intercepted an active-cell delimiter and delivered frozen
    /// `end_template` instead.
    EndTemplate(crate::CurrentCommand<G>),
    /// A right brace reached the active entry `align_group` at depth `-1`.
    /// The executor selects the structural branch; command recovery owns the
    /// exact backup correction and frozen-row-terminator insertion.
    ClosingBrace(crate::CurrentCommand<G>),
}

/// One expanded delivery while the executor is running an alignment cell.
///
/// Ordinary commands continue to main control. An intercepted delimiter is
/// represented separately so that only the command processor decides when to
/// enter the v-template transition.
#[derive(Debug, Eq, PartialEq)]
pub enum AlignmentDelivery<G> {
    /// An ordinary expanded command for executor main control.
    Command(crate::CurrentCommand<G>),
    /// A command-core alignment event.
    Event(AlignmentDeliveryEvent<G>),
    /// An executor-requested stored replay episode (a math field, math
    /// group/choice branch, or discretionary part) retired during this
    /// delivery. This must be surfaced rather than silently continuing to
    /// fetch the next token: the retiring level may sit arbitrarily deep
    /// below several other exhausted-but-unpopped levels (nested macro
    /// expansions, parameter substitutions, `\box`-style operand scans with
    /// no trailing lookahead of their own), so the *next* real token found
    /// by the cascade can belong to the enclosing context, not to the
    /// episode the caller is replaying. See TeX82's `get_next`/`get_x_token`
    /// (§§24-25) uninterrupted-boundary discussion in
    /// `docs/tex_command_core.md` §2.1: ordinary `get_x_token` (and this
    /// alignment-delivery twin, before this fix) consumes that boundary
    /// internally, but a driving loop that owns an episode must observe it.
    Completed(crate::CommandReplayEpisode),
}

/// A lifecycle request did not match the currently active alignment.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum AlignmentLifecycleError {
    /// A requested transition needs an active alignment, but none is live.
    NoActiveAlignment,
    /// A transition named a different structural alignment.
    WrongAlignment,
    /// The requested operation is invalid while a cell is already active.
    CellAlreadyActive,
    /// The requested operation needs a current active cell.
    NoActiveCell,
    /// Resume was requested without a suspended outer alignment.
    NoSuspendedAlignment,
    /// The cell has not reached the point where its v-template may start.
    UTemplateStillActive,
    /// The cell's selected u-template has already been installed.
    UTemplateAlreadyInstalled,
    /// The cell's source opener has not reached template installation yet.
    CellTemplateNotInstalled,
    /// The cell has no retained, exhausted v-template to retire.
    VTemplateNotExhausted,
    /// The raw preamble scanner has not finished its `\\cr`-terminated input.
    PreambleNotComplete,
}

impl std::fmt::Display for AlignmentLifecycleError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::NoActiveAlignment => "no alignment delivery context is active",
            Self::WrongAlignment => "alignment delivery identity does not match",
            Self::CellAlreadyActive => "an alignment cell delivery is already active",
            Self::NoActiveCell => "no alignment cell delivery is active",
            Self::NoSuspendedAlignment => "no outer alignment delivery context is suspended",
            Self::UTemplateStillActive => "the alignment u-template is still active",
            Self::UTemplateAlreadyInstalled => {
                "the alignment u-template has already been installed"
            }
            Self::CellTemplateNotInstalled => "the alignment cell template has not been installed",
            Self::VTemplateNotExhausted => "the alignment v-template is not exhausted",
            Self::PreambleNotComplete => "the alignment preamble is not complete",
        })
    }
}

impl std::error::Error for AlignmentLifecycleError {}

/// Persistent ownership for alignment-sensitive raw delivery.
#[derive(Debug)]
pub(crate) struct AlignmentDeliveryState<G> {
    pub(crate) align_state: i32,
    /// TeX82 §772's `align_stack`, restricted to the one field this layer
    /// owns: the `align_state` that `push_alignment` saves and
    /// `pop_alignment` restores.  Every alignment pushes an entry, nested or
    /// not, so `fin_align` always returns `align_state` to the running brace
    /// count that was live when `\\halign`/`\\valign` was read.
    pub(crate) align_stack: crate::timeline::LogicalStack<i32>,
    pub(crate) active_alignment: Option<AlignmentIdentity>,
    pub(crate) suspended: crate::timeline::LogicalStack<SuspendedAlignment<G>>,
    pub(crate) active_cell: Option<ActiveCellDelivery<G>>,
    pub(crate) completed_preamble: Option<AlignmentPreamble>,
    /// The delimiter just returned from `do_endv`, retained until `fin_col`
    /// either starts its next cell or changes an exhausted tab/span to `\\cr`.
    pub(crate) pending_fin_col_delimiter: Option<(AlignmentIdentity, AlignmentCellDelimiter)>,
    /// A one-shot observer marker for TeX82 `fin_col` extra-tab recovery.
    pub(crate) extra_tab_recovery: Option<AlignmentIdentity>,
    /// Frozen `\cr` retained by §336 until the follow-up inserted brace has
    /// restored the preamble sentinel and the delimiter can be replayed.
    pub(crate) pending_outer_recovery_cr: Option<TracedTokenWord>,
}

impl<G> Clone for AlignmentDeliveryState<G> {
    fn clone(&self) -> Self {
        Self {
            align_state: self.align_state,
            align_stack: self.align_stack.clone(),
            active_alignment: self.active_alignment,
            suspended: self.suspended.clone(),
            active_cell: self.active_cell.clone(),
            completed_preamble: self.completed_preamble.clone(),
            pending_fin_col_delimiter: self.pending_fin_col_delimiter,
            extra_tab_recovery: self.extra_tab_recovery,
            pending_outer_recovery_cr: self.pending_outer_recovery_cr,
        }
    }
}

impl<G> PartialEq for AlignmentDeliveryState<G> {
    fn eq(&self, other: &Self) -> bool {
        self.align_state == other.align_state
            && self.align_stack == other.align_stack
            && self.active_alignment == other.active_alignment
            && self.suspended == other.suspended
            && self.active_cell == other.active_cell
            && self.completed_preamble == other.completed_preamble
            && self.pending_fin_col_delimiter == other.pending_fin_col_delimiter
            && self.extra_tab_recovery == other.extra_tab_recovery
            && self.pending_outer_recovery_cr == other.pending_outer_recovery_cr
    }
}

impl<G> Default for AlignmentDeliveryState<G> {
    fn default() -> Self {
        Self {
            align_state: TOP_LEVEL_ALIGN_STATE,
            align_stack: crate::timeline::LogicalStack::default(),
            active_alignment: None,
            suspended: crate::timeline::LogicalStack::default(),
            active_cell: None,
            completed_preamble: None,
            pending_fin_col_delimiter: None,
            extra_tab_recovery: None,
            pending_outer_recovery_cr: None,
        }
    }
}

#[derive(Debug)]
pub(crate) struct SuspendedAlignment<G> {
    pub(crate) alignment: AlignmentIdentity,
    pub(crate) active_cell: Option<ActiveCellDelivery<G>>,
}

impl<G> Clone for SuspendedAlignment<G> {
    fn clone(&self) -> Self {
        Self {
            alignment: self.alignment,
            active_cell: self.active_cell.clone(),
        }
    }
}

impl<G> PartialEq for SuspendedAlignment<G> {
    fn eq(&self, other: &Self) -> bool {
        self.alignment == other.alignment && self.active_cell == other.active_cell
    }
}

#[derive(Debug)]
pub(crate) struct ActiveCellDelivery<G> {
    pub(crate) alignment: AlignmentIdentity,
    pub(crate) templates: PreparedAlignmentCellTemplates<G>,
    pub(crate) u_template_installed: bool,
    pub(crate) u_level: Option<InputLevelId>,
    pub(crate) v_level: Option<InputLevelId>,
    pub(crate) delimiter: Option<AlignmentCellDelimiter>,
    pub(crate) delimiter_line: Option<u32>,
    /// `init_col` consumed `\omit`, so its delimiter suffix is the canonical
    /// one-token `omit_template`, rather than this column's v-template.
    pub(crate) omit: bool,
    pub(crate) omit_previous_align_state: Option<i32>,
}

impl<G> Clone for ActiveCellDelivery<G> {
    fn clone(&self) -> Self {
        Self {
            alignment: self.alignment,
            templates: self.templates.clone(),
            u_template_installed: self.u_template_installed,
            u_level: self.u_level,
            v_level: self.v_level,
            delimiter: self.delimiter,
            delimiter_line: self.delimiter_line,
            omit: self.omit,
            omit_previous_align_state: self.omit_previous_align_state,
        }
    }
}

impl<G> PartialEq for ActiveCellDelivery<G> {
    fn eq(&self, other: &Self) -> bool {
        self.alignment == other.alignment
            && self.templates == other.templates
            && self.u_template_installed == other.u_template_installed
            && self.u_level == other.u_level
            && self.v_level == other.v_level
            && self.delimiter == other.delimiter
            && self.delimiter_line == other.delimiter_line
            && self.omit == other.omit
            && self.omit_previous_align_state == other.omit_previous_align_state
    }
}

/// The one semantic alignment adjustment made by a raw delivery.
///
/// It is stored on `CurrentCommand<G>`, so `back_input` can reverse exactly that
/// transition without inspecting the replayed spelling.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub(crate) enum AlignmentDeliveryAdjustment {
    #[default]
    None,
    BeginGroup,
    EndGroup,
    Delimiter(AlignmentDelimiter),
}

/// TeX82 identity of a delimiter intercepted by `get_next`.
///
/// This is captured before `get_next` changes the delivered command to the
/// inaccessible `end_template` meaning.  The executor deliberately receives
/// no copy of it: it is command-owned observation data, while `fin_col`
/// receives its separately stored structural outcome after `do_endv`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum AlignmentDelimiter {
    Tab,
    Span,
    Cr,
    CrCr,
}

impl AlignmentDelimiter {
    /// The observer spelling of this delimiter.
    ///
    /// Its only caller is `get_next`'s alignment observation, which is itself
    /// compiled only for crate tests and instrumentation builds, so the
    /// spelling table carries the same `cfg` rather than being dead weight in
    /// a production build.
    pub(crate) const fn observation_name(self) -> &'static str {
        match self {
            Self::Tab => "tab",
            Self::Span => "span",
            Self::Cr => "cr",
            Self::CrCr => "crcr",
        }
    }
}

impl<G> AlignmentDeliveryState<G> {
    /// Applies TeX82 §1127 `align_error`'s pre-insertion correction.
    ///
    /// `None` selects §1128's `@<Express consternation over the fact that no
    /// alignment is in progress@>` branch, which prints `Misplaced ...` and
    /// drops the delimiter without backing anything up.  `Some(brace)` is
    /// §1127's `abs(align_state)<=2` branch, which has already applied the
    /// matching `incr`/`decr`; the caller owns the `back_input`/`ins_error`
    /// pair.  The later inserted-brace backup correction is deliberately
    /// separate: it is observable input ownership, and raw delivery will then
    /// consume that correction normally.
    ///
    /// §1127 tests only `align_state`.  There is deliberately no active-cell
    /// gate here: `align_state` is a whole-run brace count (§331, §772), so a
    /// delimiter that reaches main control below base depth gets the same
    /// recovery whether or not a cell happens to be open.
    pub(crate) fn correct_unbalanced_delimiter(&mut self) -> Option<Token> {
        if self.align_state.unsigned_abs() > 2 {
            return None;
        }
        if self.align_state < 0 {
            self.align_state += 1;
            Some(Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            })
        } else {
            self.align_state -= 1;
            Some(Token::Char {
                ch: '}',
                cat: Catcode::EndGroup,
            })
        }
    }

    /// Mirrors `back_input` for the recovery brace that §1127 installs with
    /// `ins_error`, before that brace is replayed through `get_next`.
    pub(crate) fn correct_inserted_brace_backup(&mut self, token: Token) {
        match token {
            Token::Char {
                cat: Catcode::BeginGroup,
                ..
            } => self.align_state -= 1,
            Token::Char {
                cat: Catcode::EndGroup,
                ..
            } => self.align_state += 1,
            _ => {}
        }
    }

    pub(crate) fn needs_closing_brace_recovery(&self, command: &CurrentCommand<G>) -> bool {
        self.active_cell.is_some()
            && self.align_state == -1
            && matches!(
                command.meaning(),
                tex_state::ResolvedMeaning::Static(Meaning::CharToken {
                    cat: Catcode::EndGroup,
                    ..
                })
            )
    }
    /// TeX82 §774's `init_align` prologue: `push_alignment` (§772) saves the
    /// running `align_state`, then `align_state:=-1000000` enters the new
    /// alignment level.  The save is unconditional in tex.web, so it happens
    /// for the outermost alignment exactly as for a nested one.
    pub(crate) fn begin_alignment(&mut self, alignment: AlignmentIdentity) {
        self.align_stack.push(self.align_state);
        self.active_alignment = Some(alignment);
        self.active_cell = None;
        self.completed_preamble = None;
        self.pending_fin_col_delimiter = None;
        self.extra_tab_recovery = None;
        self.align_state = PREAMBLE_ALIGN_STATE;
    }

    pub(crate) fn set_preamble_phase(
        &mut self,
        alignment: AlignmentIdentity,
    ) -> Result<(), AlignmentLifecycleError> {
        self.require_alignment(alignment)?;
        self.active_cell = None;
        self.completed_preamble = None;
        self.align_state = PREAMBLE_ALIGN_STATE;
        Ok(())
    }

    pub(crate) fn complete_preamble(
        &mut self,
        alignment: AlignmentIdentity,
        preamble: AlignmentPreamble,
    ) -> Result<(), AlignmentLifecycleError> {
        self.require_alignment(alignment)?;
        self.completed_preamble = Some(preamble);
        Ok(())
    }

    /// Transfers the frozen preamble to the executor after raw collection
    /// has completed. This is a one-shot structural handoff; token delivery,
    /// template installation, and `align_state` remain command-owned.
    pub(crate) fn take_completed_preamble(
        &mut self,
        alignment: AlignmentIdentity,
    ) -> Result<AlignmentPreamble, AlignmentLifecycleError> {
        self.require_alignment(alignment)?;
        self.completed_preamble
            .take()
            .ok_or(AlignmentLifecycleError::PreambleNotComplete)
    }

    pub(crate) fn begin_cell(
        &mut self,
        alignment: AlignmentIdentity,
        templates: PreparedAlignmentCellTemplates<G>,
    ) -> Result<(), AlignmentLifecycleError> {
        self.require_alignment(alignment)?;
        if self.active_cell.is_some() {
            return Err(AlignmentLifecycleError::CellAlreadyActive);
        }
        let has_u_template = templates.u_template.is_some();
        self.active_cell = Some(ActiveCellDelivery {
            alignment,
            templates,
            u_template_installed: false,
            u_level: None,
            v_level: None,
            delimiter: None,
            delimiter_line: None,
            omit: false,
            omit_previous_align_state: None,
        });
        self.align_state = if has_u_template {
            TEMPLATE_ALIGN_STATE
        } else {
            CELL_ALIGN_STATE
        };
        Ok(())
    }

    pub(crate) fn attach_u_template(
        &mut self,
        alignment: AlignmentIdentity,
        level: InputLevelId,
    ) -> Result<(), AlignmentLifecycleError> {
        let cell = self.active_cell_mut(alignment)?;
        if cell.u_template_installed {
            return Err(AlignmentLifecycleError::UTemplateAlreadyInstalled);
        }
        cell.u_template_installed = true;
        cell.u_level = Some(level);
        Ok(())
    }

    pub(crate) fn active_cell_template(
        &self,
        alignment: AlignmentIdentity,
    ) -> Result<Option<TokenListId<G>>, AlignmentLifecycleError> {
        let cell = self.active_cell_ref(alignment)?;
        if cell.u_template_installed {
            return Err(AlignmentLifecycleError::UTemplateAlreadyInstalled);
        }
        Ok(cell.templates.u_template.clone())
    }

    pub(crate) fn mark_u_template_installed(
        &mut self,
        alignment: AlignmentIdentity,
    ) -> Result<(), AlignmentLifecycleError> {
        let cell = self.active_cell_mut(alignment)?;
        if cell.u_template_installed {
            return Err(AlignmentLifecycleError::UTemplateAlreadyInstalled);
        }
        cell.u_template_installed = true;
        Ok(())
    }

    pub(crate) fn finish_u_template(&mut self, level: InputLevelId) -> bool {
        let Some(cell) = self.active_cell.as_mut() else {
            return false;
        };
        if cell.u_level != Some(level) {
            return false;
        }
        cell.u_level = None;
        self.align_state = CELL_ALIGN_STATE;
        true
    }

    pub(crate) fn begin_v_template(
        &mut self,
        alignment: AlignmentIdentity,
        level: InputLevelId,
        delimiter: AlignmentCellDelimiter,
        delimiter_line: u32,
    ) -> Result<(), AlignmentLifecycleError> {
        let cell = self.active_cell_mut(alignment)?;
        if !cell.u_template_installed {
            return Err(AlignmentLifecycleError::CellTemplateNotInstalled);
        }
        if cell.u_level.is_some() {
            return Err(AlignmentLifecycleError::UTemplateStillActive);
        }
        cell.v_level = Some(level);
        cell.delimiter = Some(delimiter);
        cell.delimiter_line = Some(delimiter_line);
        self.align_state = TEMPLATE_ALIGN_STATE;
        Ok(())
    }

    pub(crate) fn v_template(
        &self,
        alignment: AlignmentIdentity,
    ) -> Result<Option<TokenListId<G>>, AlignmentLifecycleError> {
        let cell = self.active_cell_ref(alignment)?;
        if cell.u_level.is_some() {
            return Err(AlignmentLifecycleError::UTemplateStillActive);
        }
        // TeX82 §37 installs `omit_template` at this boundary, not the
        // selected column's v-part. Its only effective delivery is the
        // retained end-template boundary, represented by an empty replay
        // level in the command machine.
        Ok((!cell.omit).then(|| cell.templates.v_template.clone()))
    }

    pub(crate) fn active_v_template_level(
        &self,
        alignment: AlignmentIdentity,
    ) -> Result<InputLevelId, AlignmentLifecycleError> {
        self.active_cell_ref(alignment)?
            .v_level
            .ok_or(AlignmentLifecycleError::VTemplateNotExhausted)
    }

    pub(crate) fn finish_cell(
        &mut self,
        alignment: AlignmentIdentity,
        v_level: InputLevelId,
    ) -> Result<FinishedAlignmentCell, AlignmentLifecycleError> {
        self.require_alignment(alignment)?;
        let cell = self.active_cell_ref(alignment)?;
        if cell.v_level != Some(v_level) {
            return Err(AlignmentLifecycleError::VTemplateNotExhausted);
        }
        let cell = self
            .active_cell
            .take()
            .expect("active cell was just checked");
        self.align_state = TEMPLATE_ALIGN_STATE;
        let delimiter = cell
            .delimiter
            .ok_or(AlignmentLifecycleError::VTemplateNotExhausted)?;
        let delimiter_line = cell
            .delimiter_line
            .ok_or(AlignmentLifecycleError::VTemplateNotExhausted)?;
        self.pending_fin_col_delimiter = Some((alignment, delimiter));
        Ok(FinishedAlignmentCell {
            delimiter,
            delimiter_line,
        })
    }

    /// Performs TeX82 §772's exhausted-preamble arm of `fin_col`.
    pub(crate) fn recover_extra_tab(
        &mut self,
        alignment: AlignmentIdentity,
    ) -> Result<(), AlignmentLifecycleError> {
        self.require_alignment(alignment)?;
        match self.pending_fin_col_delimiter {
            Some((saved_alignment, AlignmentCellDelimiter::Tab | AlignmentCellDelimiter::Span))
                if saved_alignment == alignment =>
            {
                // `extra_info(cur_align):=cr_code`: retain the structural
                // replacement in command state; the executor will finish
                // the row through its typed result.
                self.pending_fin_col_delimiter = Some((alignment, AlignmentCellDelimiter::Row));
                self.extra_tab_recovery = Some(alignment);
                Ok(())
            }
            _ => Err(AlignmentLifecycleError::NoActiveCell),
        }
    }

    pub(crate) fn suspend_alignment(
        &mut self,
        alignment: AlignmentIdentity,
    ) -> Result<(), AlignmentLifecycleError> {
        self.require_alignment(alignment)?;
        self.suspended.push(SuspendedAlignment {
            alignment,
            active_cell: self.active_cell.take(),
        });
        self.active_alignment = None;
        Ok(())
    }

    pub(crate) fn resume_alignment(
        &mut self,
        alignment: AlignmentIdentity,
    ) -> Result<(), AlignmentLifecycleError> {
        let suspended = self
            .suspended
            .pop()
            .ok_or(AlignmentLifecycleError::NoSuspendedAlignment)?;
        if suspended.alignment != alignment {
            self.suspended.push(suspended);
            return Err(AlignmentLifecycleError::WrongAlignment);
        }
        self.active_alignment = Some(alignment);
        self.active_cell = suspended.active_cell;
        Ok(())
    }

    /// TeX82 §800's `fin_align` epilogue: `pop_alignment` (§772) restores the
    /// `align_state` that `push_alignment` saved when this alignment started.
    /// The outer running brace count therefore survives the alignment, which
    /// is what makes §1127's `abs(align_state)>2` test still report "no
    /// alignment is in progress" for material after `fin_align`.
    pub(crate) fn finish_alignment(
        &mut self,
        alignment: AlignmentIdentity,
    ) -> Result<(), AlignmentLifecycleError> {
        self.require_alignment(alignment)?;
        self.active_alignment = None;
        self.active_cell = None;
        self.completed_preamble = None;
        self.align_state = self.align_stack.pop().unwrap_or(TOP_LEVEL_ALIGN_STATE);
        Ok(())
    }

    pub(crate) fn classify_delivery(&mut self, command: &mut CurrentCommand<G>) {
        let adjustment = match command.spelling().semantic_token() {
            Token::Char {
                cat: Catcode::BeginGroup,
                ..
            } => {
                self.align_state += 1;
                AlignmentDeliveryAdjustment::BeginGroup
            }
            Token::Char {
                cat: Catcode::EndGroup,
                ..
            } => {
                self.align_state -= 1;
                AlignmentDeliveryAdjustment::EndGroup
            }
            _ if self.active_cell.is_some()
                && self.align_state == CELL_ALIGN_STATE
                && is_character_command(command, Catcode::AlignmentTab) =>
            {
                self.intercept_delimiter(command, AlignmentDelimiter::Tab)
            }
            _ if self.active_cell.is_some()
                && self.align_state == CELL_ALIGN_STATE
                && matches!(
                    command.meaning(),
                    tex_state::ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                        UnexpandablePrimitive::Cr
                            | UnexpandablePrimitive::CrCr
                            | UnexpandablePrimitive::Span
                    ))
                ) =>
            {
                let delimiter = match command.meaning() {
                    tex_state::ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                        UnexpandablePrimitive::Span,
                    )) => AlignmentDelimiter::Span,
                    tex_state::ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                        UnexpandablePrimitive::Cr,
                    )) => AlignmentDelimiter::Cr,
                    tex_state::ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                        UnexpandablePrimitive::CrCr,
                    )) => AlignmentDelimiter::CrCr,
                    _ => unreachable!("alignment delimiter was classified above"),
                };
                self.intercept_delimiter(command, delimiter)
            }
            _ => AlignmentDeliveryAdjustment::None,
        };
        command.set_alignment_adjustment(adjustment);
    }

    /// TeX82 §325's own brace rule, stated over `cur_tok` alone:
    /// `if cur_tok<right_brace_limit then if cur_tok<left_brace_limit then
    /// decr(align_state) else incr(align_state)`.
    ///
    /// `back_input` never consults the delivery that produced `cur_tok`; it
    /// reads the token's own category. §326 (`cur_tok:=p; back_input`) is
    /// exactly why that matters -- its token was saved long before, or, as
    /// with §372's `\\csname`, was never delivered at all -- so a caller
    /// without a live [`CurrentCommand<G>`] classifies here instead of recalling
    /// an adjustment. Control-sequence and active tokens carry
    /// `cs_token_flag`, so they exceed `right_brace_limit` and never adjust.
    ///
    /// The result is fed to [`Self::undo_delivery`] because §325's sign is
    /// the reverse of delivery's: a left brace that incremented on the way
    /// out decrements on the way back in, and is incremented again when the
    /// backup level redelivers it.
    pub(crate) const fn back_input_adjustment(token: Token) -> AlignmentDeliveryAdjustment {
        match token {
            Token::Char {
                cat: Catcode::BeginGroup,
                ..
            } => AlignmentDeliveryAdjustment::BeginGroup,
            Token::Char {
                cat: Catcode::EndGroup,
                ..
            } => AlignmentDeliveryAdjustment::EndGroup,
            Token::Char { .. } | Token::Cs(_) | Token::Param(_) | Token::Frozen(_) => {
                AlignmentDeliveryAdjustment::None
            }
        }
    }

    pub(crate) fn undo_delivery(&mut self, adjustment: AlignmentDeliveryAdjustment) {
        match adjustment {
            AlignmentDeliveryAdjustment::None => {}
            AlignmentDeliveryAdjustment::BeginGroup => self.align_state -= 1,
            AlignmentDeliveryAdjustment::EndGroup => self.align_state += 1,
            AlignmentDeliveryAdjustment::Delimiter(_) => self.align_state = CELL_ALIGN_STATE,
        }
    }

    pub(crate) fn undo_delimiter_begin_group_delivery(&mut self) {
        self.align_state -= 1;
    }

    fn intercept_delimiter(
        &mut self,
        command: &mut CurrentCommand<G>,
        delimiter: AlignmentDelimiter,
    ) -> AlignmentDeliveryAdjustment {
        self.align_state = TEMPLATE_ALIGN_STATE;
        command.convert_to_end_template();
        AlignmentDeliveryAdjustment::Delimiter(delimiter)
    }

    fn require_alignment(
        &self,
        alignment: AlignmentIdentity,
    ) -> Result<(), AlignmentLifecycleError> {
        match self.active_alignment {
            None => Err(AlignmentLifecycleError::NoActiveAlignment),
            Some(active) if active != alignment => Err(AlignmentLifecycleError::WrongAlignment),
            Some(_) => Ok(()),
        }
    }

    fn active_cell_ref(
        &self,
        alignment: AlignmentIdentity,
    ) -> Result<&ActiveCellDelivery<G>, AlignmentLifecycleError> {
        self.require_alignment(alignment)?;
        let cell = self
            .active_cell
            .as_ref()
            .ok_or(AlignmentLifecycleError::NoActiveCell)?;
        if cell.alignment != alignment {
            return Err(AlignmentLifecycleError::WrongAlignment);
        }
        Ok(cell)
    }

    pub(crate) fn active_cell_mut(
        &mut self,
        alignment: AlignmentIdentity,
    ) -> Result<&mut ActiveCellDelivery<G>, AlignmentLifecycleError> {
        self.require_alignment(alignment)?;
        let cell = self
            .active_cell
            .as_mut()
            .ok_or(AlignmentLifecycleError::NoActiveCell)?;
        if cell.alignment != alignment {
            return Err(AlignmentLifecycleError::WrongAlignment);
        }
        Ok(cell)
    }
}
