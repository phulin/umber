//! Independent TeX conditional-stack state and skipped-text delivery.
//!
//! This mirrors TeX.web part 28.  Conditions deliberately are not input
//! levels: recursive expansion can push another condition while an older
//! condition is still evaluating its operands.

use tex_state::env::banks::IntParam;
use tex_state::meaning::{ExpandablePrimitive, Meaning};
use tex_state::token::{OriginId, TracedTokenWord};

use crate::input::{ReplayTrace, RetirementBehavior, TokenBehavior, TokenPayload};
use crate::processor::CommandProcessor;
use crate::processor::status::{
    ConditionId, ScannerStatus, ScannerStatusVisibility, ScannerWarning, SkippingContext,
};
use crate::scanners::RestrictedIntegerClass;
use crate::{CommandError, CommandState};

use crate::observation::{
    CommandObservation, ConditionRecord, DiagnosticRecord, InputReason, InputRecord,
    InputTransition, ObservedToken, RecoveryKind, RecoveryRecord,
};

/// Stable pending-diagnostic identities for TeX.web part 28 recovery.
const INCOMPLETE_IF_DIAGNOSTIC: u64 = 0x636f_6e64_0000_0001;
const EXTRA_DELIMITER_DIAGNOSTIC: u64 = 0x636f_6e64_0000_0002;
const MISSING_RELATION_DIAGNOSTIC: u64 = 0x636f_6e64_0000_0003;
const BAD_NUMBER_DIAGNOSTIC: u64 = 0x636f_6e64_0000_0004;
const ILLEGAL_UNLESS_OPERAND_DIAGNOSTIC: u64 = 0x636f_6e64_0000_0005;

/// TeX conditional opcode, kept distinct from delimiter and limit values.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum ConditionalKind {
    IfTrue,
    IfFalse,
    If,
    IfCat,
    IfX,
    IfNum,
    IfDim,
    IfOdd,
    IfCase,
    IfVMode,
    IfHMode,
    IfMMode,
    IfInner,
    IfVoid,
    IfHBox,
    IfVBox,
    IfEof,
    IfDefined,
    IfCsName,
    IfFontChar,
    IfInCsName,
    IfPdfPrimitive,
    IfPdfAbsNum,
    IfPdfAbsDim,
}

#[allow(dead_code)] // used by pass_text now; evaluation uses the same classifier next
impl ConditionalKind {
    pub(crate) const fn canonical_name(self) -> &'static str {
        match self {
            Self::IfTrue => "iftrue",
            Self::IfFalse => "iffalse",
            Self::If => "if",
            Self::IfCat => "ifcat",
            Self::IfX => "ifx",
            Self::IfNum => "ifnum",
            Self::IfDim => "ifdim",
            Self::IfOdd => "ifodd",
            Self::IfCase => "ifcase",
            Self::IfVMode => "ifvmode",
            Self::IfHMode => "ifhmode",
            Self::IfMMode => "ifmmode",
            Self::IfInner => "ifinner",
            Self::IfVoid => "ifvoid",
            Self::IfHBox => "ifhbox",
            Self::IfVBox => "ifvbox",
            Self::IfEof => "ifeof",
            Self::IfDefined => "ifdefined",
            Self::IfCsName => "ifcsname",
            Self::IfFontChar => "iffontchar",
            Self::IfInCsName => "ifincsname",
            Self::IfPdfPrimitive => "ifpdfprimitive",
            Self::IfPdfAbsNum => "ifpdfabsnum",
            Self::IfPdfAbsDim => "ifpdfabsdim",
        }
    }

    pub(crate) const fn from_primitive(primitive: ExpandablePrimitive) -> Option<Self> {
        Some(match primitive {
            ExpandablePrimitive::IfTrue => Self::IfTrue,
            ExpandablePrimitive::IfFalse => Self::IfFalse,
            ExpandablePrimitive::If => Self::If,
            ExpandablePrimitive::IfCat => Self::IfCat,
            ExpandablePrimitive::IfX => Self::IfX,
            ExpandablePrimitive::IfNum => Self::IfNum,
            ExpandablePrimitive::IfDim => Self::IfDim,
            ExpandablePrimitive::IfOdd => Self::IfOdd,
            ExpandablePrimitive::IfCase => Self::IfCase,
            ExpandablePrimitive::IfVMode => Self::IfVMode,
            ExpandablePrimitive::IfHMode => Self::IfHMode,
            ExpandablePrimitive::IfMMode => Self::IfMMode,
            ExpandablePrimitive::IfInner => Self::IfInner,
            ExpandablePrimitive::IfVoid => Self::IfVoid,
            ExpandablePrimitive::IfHBox => Self::IfHBox,
            ExpandablePrimitive::IfVBox => Self::IfVBox,
            ExpandablePrimitive::IfEof => Self::IfEof,
            ExpandablePrimitive::IfDefined => Self::IfDefined,
            ExpandablePrimitive::IfCsName => Self::IfCsName,
            ExpandablePrimitive::IfFontChar => Self::IfFontChar,
            ExpandablePrimitive::IfInCsName => Self::IfInCsName,
            ExpandablePrimitive::IfPdfPrimitive => Self::IfPdfPrimitive,
            ExpandablePrimitive::IfPdfAbsNum => Self::IfPdfAbsNum,
            ExpandablePrimitive::IfPdfAbsDim => Self::IfPdfAbsDim,
            _ => return None,
        })
    }
}

/// The only delimiter commands recognized by `pass_text`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum ConditionalDelimiter {
    Or,
    Else,
    Fi,
}

#[allow(dead_code)] // used by pass_text now; delimiter execution follows next
impl ConditionalDelimiter {
    const fn canonical_branch(self) -> &'static str {
        match self {
            Self::Or => "or",
            Self::Else => "else",
            Self::Fi => "fi",
        }
    }

    const fn from_meaning(meaning: Meaning) -> Option<Self> {
        match meaning {
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Or) => Some(Self::Or),
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Else) => Some(Self::Else),
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Fi) => Some(Self::Fi),
            _ => None,
        }
    }
}

/// TeX's `if_limit`, without an integer encoding shared with command opcodes.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum IfLimit {
    /// Operand evaluation is incomplete; a delimiter is an incomplete-if recovery.
    Evaluating,
    Or,
    Else,
    Fi,
}

impl IfLimit {
    const fn canonical_name(self) -> &'static str {
        match self {
            Self::Evaluating => "evaluating",
            Self::Or => "or",
            Self::Else => "else",
            Self::Fi => "fi",
        }
    }

    /// Whether TeX's `fi_or_else` dispatcher accepts this delimiter for the
    /// live frame.  The ordering mirrors `fi_code`, `else_code`, and
    /// `or_code` without sharing their integer command-code representation.
    const fn accepts_delimiter(self, delimiter: ConditionalDelimiter) -> bool {
        matches!(
            (self, delimiter),
            (_, ConditionalDelimiter::Fi)
                | (Self::Else | Self::Or, ConditionalDelimiter::Else)
                | (Self::Or, ConditionalDelimiter::Or)
        )
    }
}

/// Persistent, stable identity-bearing TeX condition state.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct ConditionFrame {
    pub(crate) identity: ConditionId,
    pub(crate) kind: ConditionalKind,
    pub(crate) limit: IfLimit,
    pub(crate) source_line: u32,
    /// e-TeX's `\unless` negates the current-if type and branch.
    pub(crate) inverted: bool,
}

/// One unfinished conditional retired by TeX82 §1335's final cleanup.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct IncompleteCondition {
    kind: ConditionalKind,
    source_line: u32,
}

/// Read-only e-TeX `\showifs` projection of one active conditional.
///
/// This deliberately omits the stack identity and exposes semantic values,
/// so an executor can detach the diagnostic without retaining command state.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ActiveCondition {
    kind: ConditionalKind,
    source_line: u32,
    inverted: bool,
    else_branch: bool,
}

impl ActiveCondition {
    /// `print_cmd_chr(if_test, cur_if)`'s spelling without the escape.
    #[must_use]
    pub const fn kind_name(self) -> &'static str {
        self.kind.canonical_name()
    }

    /// The saved `if_line`; zero suppresses the `entered on line` clause.
    #[must_use]
    pub const fn source_line(self) -> u32 {
        self.source_line
    }

    /// Whether e-TeX's `\unless` negated this conditional.
    #[must_use]
    pub const fn inverted(self) -> bool {
        self.inverted
    }

    /// Whether `if_limit=fi_code`, rendered by e-TeX as `\else`.
    #[must_use]
    pub const fn else_branch(self) -> bool {
        self.else_branch
    }
}

impl IncompleteCondition {
    /// TeX82's `print_cmd_chr(if_test,cur_if)` spelling without the escape.
    #[must_use]
    pub const fn kind_name(self) -> &'static str {
        self.kind.canonical_name()
    }

    /// The saved `if_line`; zero suppresses the `on line` clause.
    #[must_use]
    pub const fn source_line(self) -> u32 {
        self.source_line
    }
}

/// Independent persistent condition stack.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub(crate) struct ConditionStack {
    pub(crate) frames: Vec<ConditionFrame>,
    pub(crate) next_identity: u64,
}

impl ConditionStack {
    pub(crate) fn tracked_stack_projection(&self) -> u64 {
        let mut hash = 0xcbf2_9ce4_8422_2325_u64 ^ 0x636f_6e64_0000_0001;
        let mut feed = |value: u64| {
            for byte in value.to_le_bytes() {
                hash ^= u64::from(byte);
                hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
            }
        };
        feed(self.frames.len() as u64);
        for frame in &self.frames {
            feed(frame.kind.etex_type() as u64);
            feed(match frame.limit {
                IfLimit::Evaluating => 0,
                IfLimit::Or => 1,
                IfLimit::Else => 2,
                IfLimit::Fi => 3,
            });
            feed(u64::from(frame.source_line));
            feed(frame.inverted.into());
        }
        hash
    }

    #[cfg(test)]
    pub(crate) fn push(&mut self, kind: ConditionalKind, source_line: u32) -> ConditionId {
        self.push_with_inversion(kind, source_line, false)
    }

    pub(crate) fn push_with_inversion(
        &mut self,
        kind: ConditionalKind,
        source_line: u32,
        inverted: bool,
    ) -> ConditionId {
        let identity = ConditionId(self.next_identity);
        self.next_identity = self.next_identity.wrapping_add(1);
        self.frames.push(ConditionFrame {
            identity,
            kind,
            limit: IfLimit::Evaluating,
            source_line,
            inverted,
        });
        identity
    }

    pub(crate) fn current(&self) -> Option<&ConditionFrame> {
        self.frames.last()
    }

    pub(crate) fn pop(&mut self) -> Option<ConditionFrame> {
        self.frames.pop()
    }

    pub(crate) fn drain_incomplete(&mut self) -> Vec<IncompleteCondition> {
        let mut incomplete = Vec::with_capacity(self.frames.len());
        while let Some(frame) = self.pop() {
            incomplete.push(IncompleteCondition {
                kind: frame.kind,
                source_line: frame.source_line,
            });
        }
        incomplete
    }

    pub(crate) fn current_etex_values(&self) -> (i32, i32, i32) {
        let level = i32::try_from(self.frames.len()).unwrap_or(i32::MAX);
        let Some(frame) = self.current() else {
            return (0, 0, 0);
        };
        let ty = frame.kind.etex_type();
        let branch = match frame.limit {
            IfLimit::Evaluating => 0,
            IfLimit::Or | IfLimit::Else => 1,
            IfLimit::Fi => -1,
        };
        if frame.inverted {
            (level, -ty, branch)
        } else {
            (level, ty, branch)
        }
    }

    fn active_conditions(&self) -> Vec<ActiveCondition> {
        self.frames
            .iter()
            .rev()
            .map(|frame| ActiveCondition {
                kind: frame.kind,
                source_line: frame.source_line,
                inverted: frame.inverted,
                else_branch: frame.limit == IfLimit::Fi,
            })
            .collect()
    }

    /// Changes the exact frame selected before recursive operand expansion.
    pub(crate) fn change_if_limit(&mut self, identity: ConditionId, limit: IfLimit) -> bool {
        let Some(frame) = self
            .frames
            .iter_mut()
            .rev()
            .find(|frame| frame.identity == identity)
        else {
            return false;
        };
        frame.limit = limit;
        true
    }

    pub(crate) fn limit(&self, identity: ConditionId) -> Option<IfLimit> {
        self.frames
            .iter()
            .rev()
            .find(|frame| frame.identity == identity)
            .map(|frame| frame.limit)
    }

    pub(crate) fn frame(&self, identity: ConditionId) -> Option<&ConditionFrame> {
        self.frames
            .iter()
            .rev()
            .find(|frame| frame.identity == identity)
    }

    /// Detects the TeX `Incomplete \if` recovery case without popping an
    /// arbitrary frame. The evaluator owns the actual diagnostic/insertion.
    pub(crate) fn evaluating_delimiter_recovery(
        &self,
        identity: ConditionId,
        delimiter: ConditionalDelimiter,
    ) -> Option<EvaluatingDelimiterRecovery> {
        (self.limit(identity) == Some(IfLimit::Evaluating)).then_some(EvaluatingDelimiterRecovery {
            condition: identity,
            delimiter,
        })
    }
}

impl CommandProcessor<'_> {
    /// Detaches the active stack from innermost to outermost for `\showifs`.
    #[must_use]
    pub fn active_conditions(&self) -> Vec<ActiveCondition> {
        self.command.conditions.active_conditions()
    }
}

impl CommandState {
    /// Number of condition frames whose delimiters remain future input.
    #[must_use]
    pub fn active_condition_depth(&self) -> usize {
        self.conditions.frames.len()
    }
}

impl ConditionalKind {
    /// e-TeX 2.6 `etex.ch`'s `\currentiftype` result.
    ///
    /// The enquiry returns `cur_if+1`, not the zero-based `if_test` operand.
    /// e-TeX's later predicates leave opcode 12 unused by inserting `\ifx`
    /// at 13, so spell the complete one-based result table explicitly.
    const fn etex_type(self) -> i32 {
        match self {
            Self::If => 1,
            Self::IfCat => 2,
            Self::IfNum => 3,
            Self::IfDim => 4,
            Self::IfOdd => 5,
            Self::IfVMode => 6,
            Self::IfHMode => 7,
            Self::IfMMode => 8,
            Self::IfInner => 9,
            Self::IfVoid => 10,
            Self::IfHBox => 11,
            Self::IfVBox => 12,
            Self::IfX => 13,
            Self::IfEof => 14,
            Self::IfTrue => 15,
            Self::IfFalse => 16,
            Self::IfCase => 17,
            Self::IfDefined => 18,
            Self::IfCsName => 19,
            Self::IfFontChar => 20,
            Self::IfInCsName => 21,
            Self::IfPdfPrimitive => 22,
            Self::IfPdfAbsNum => 23,
            Self::IfPdfAbsDim => 24,
        }
    }
}

/// A delimiter interrupted operand evaluation of this exact condition frame.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct EvaluatingDelimiterRecovery {
    pub(crate) condition: ConditionId,
    pub(crate) delimiter: ConditionalDelimiter,
}

/// Result of canonical skipped-text delivery.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct PassTextStop {
    pub(crate) delimiter: ConditionalDelimiter,
    pub(crate) nested_conditions: u32,
}

/// The classified `<`, `=`, or `>` relation token TeX.web §503 requires
/// between an `\ifnum`/`\ifdim` pair of operands.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum IfRelation {
    Less,
    Equal,
    Greater,
}

impl IfRelation {
    fn compare<T: PartialOrd>(self, left: T, right: T) -> bool {
        match self {
            Self::Less => left < right,
            Self::Equal => left == right,
            Self::Greater => left > right,
        }
    }
}

impl CommandProcessor<'_> {
    /// TeX.web part 28's `conditional`, entered after delivery of an `if`
    /// primitive.  The frame is installed before any operand scan because
    /// those scans may recursively expand another conditional.
    pub(crate) fn expand_conditional(
        &mut self,
        command: &crate::CurrentCommand,
        inverted: bool,
    ) -> Result<(), CommandError> {
        let Meaning::ExpandablePrimitive(primitive) = command.meaning() else {
            return Err(CommandError::input_invariant());
        };
        let kind =
            ConditionalKind::from_primitive(primitive).ok_or(CommandError::input_invariant())?;
        let source_line = u32::try_from(self.command.input.current_file_line_number()).unwrap_or(0);
        let condition = self
            .command
            .conditions
            .push_with_inversion(kind, source_line, inverted);
        let frame = self
            .command
            .conditions
            .frame(condition)
            .cloned()
            .ok_or(CommandError::input_invariant())?;
        self.trace_conditional_enter(&frame);
        self.observe_condition("push", &frame, None);
        match kind {
            ConditionalKind::IfCase => {
                let selected = self.scan_integer()?.value;
                self.complete_ifcase(condition, selected)
            }
            _ => {
                let result = self.evaluate_boolean(kind)?;
                self.complete_boolean(condition, result ^ inverted)
            }
        }
    }

    /// e-TeX's `\\unless` has no independent condition state: it consumes
    /// precisely one following conditional and flips only boolean results.
    /// A non-conditional or `\\ifcase` operand follows e-TeX 2.6's merged
    /// change [28.498]: `back_error` restores that command, reports the exact
    /// prefix diagnostic, and leaves the conditional stack untouched.
    pub(crate) fn expand_unless(
        &mut self,
        _command: &crate::CurrentCommand,
    ) -> Result<(), CommandError> {
        // The following conditional is an operand of `\unless`, not an
        // ordinary expansion result: preserve its primitive command for the
        // shared evaluator to install the one inverted frame.
        let next = self.get_token()?.ok_or(CommandError::input_invariant())?;
        let kind = match next.meaning() {
            Meaning::ExpandablePrimitive(primitive) => ConditionalKind::from_primitive(primitive),
            _ => None,
        };
        let Some(_kind) = kind.filter(|kind| *kind != ConditionalKind::IfCase) else {
            let mut message = String::from("You can't use `");
            crate::processor::expand::append_print_esc_text(&self.state, "unless", &mut message);
            message.push_str("' before `");
            crate::processor::expand::append_print_cmd_chr_text(
                &self.state,
                crate::processor::expand::PrintCommand::from_current(&next),
                &mut message,
            );
            message.push_str("'.");
            self.observe_command_diagnostic("illegal_unless_operand", &next);
            self.back_error_reporting(
                next,
                ILLEGAL_UNLESS_OPERAND_DIAGNOSTIC,
                message,
                &["I'll pretend you didn't say \\unless."],
            )?;
            return Ok(());
        };
        if self.state.int_param(IntParam::TRACING_COMMANDS) > 1
            && self.state.int_param(IntParam::TRACING_IFS) <= 0
        {
            self.print_unless_command_trace(crate::processor::expand::PrintCommand::from_current(
                &next,
            ));
        }
        self.expand_conditional(&next, true)
    }

    fn complete_boolean(
        &mut self,
        condition: ConditionId,
        result: bool,
    ) -> Result<(), CommandError> {
        // TeX.web §498 records the evaluated value while `if_limit` still
        // says the operands were being scanned, then changes the limit.
        let evaluating = self
            .command
            .conditions
            .frame(condition)
            .cloned()
            .ok_or(CommandError::input_invariant())?;
        let branch = if result { "true" } else { "false" };
        self.trace_boolean_result(branch);
        self.observe_condition("branch", &evaluating, Some(branch));
        if !result {
            return self.resume_after_skip(condition);
        }
        self.command
            .conditions
            .change_if_limit(condition, IfLimit::Else)
            .then_some(())
            .ok_or(CommandError::input_invariant())?;
        let frame = self
            .command
            .conditions
            .frame(condition)
            .cloned()
            .ok_or(CommandError::input_invariant())?;
        self.observe_condition("limit", &frame, None);
        Ok(())
    }

    /// TeX82 §502's diagnostic immediately after a boolean predicate has
    /// been evaluated and before its selected or skipped limb is entered.
    fn trace_boolean_result(&mut self, result: &'static str) {
        if self.state.int_param(IntParam::TRACING_COMMANDS) <= 1 {
            return;
        }
        let text = format!("{{{result}}}");
        // A queued recoverable diagnostic represents a synchronous §82
        // report. Any trace discovered later in this same expansion episode
        // must cross the executor boundary behind it, just as command traces
        // do in `print_command_trace_text`.
        if self.command.expanding_deferred_write() || !self.command.semantic_diagnostics.is_empty()
        {
            self.command
                .semantic_diagnostics
                .push(crate::CommandSemanticDiagnostic::Trace {
                    text,
                    force_newline: false,
                });
            return;
        }
        let mut diagnostic = self.state.begin_diagnostic();
        diagnostic.print_nl(&text);
        diagnostic.end(false);
    }

    fn complete_ifcase(
        &mut self,
        condition: ConditionId,
        selected: i32,
    ) -> Result<(), CommandError> {
        self.trace_ifcase_selection(selected);
        if self.skip_ifcase_limbs(condition, selected)? {
            self.command
                .conditions
                .change_if_limit(condition, IfLimit::Or)
                .then_some(())
                .ok_or(CommandError::input_invariant())?;
            let frame = self
                .command
                .conditions
                .frame(condition)
                .cloned()
                .ok_or(CommandError::input_invariant())?;
            self.observe_condition("limit", &frame, None);
            self.observe_condition("branch", &frame, Some("case"));
        }
        Ok(())
    }

    /// TeX82 §509's diagnostic after scanning the case number and before
    /// skipping any unselected limbs.
    fn trace_ifcase_selection(&mut self, selected: i32) {
        if self.state.int_param(IntParam::TRACING_COMMANDS) <= 1 {
            return;
        }
        let text = format!("{{case {selected}}}");
        if self.command.expanding_deferred_write() || !self.command.semantic_diagnostics.is_empty()
        {
            self.command
                .semantic_diagnostics
                .push(crate::CommandSemanticDiagnostic::Trace {
                    text,
                    force_newline: false,
                });
            return;
        }
        let mut diagnostic = self.state.begin_diagnostic();
        diagnostic.print_nl(&text);
        diagnostic.end(false);
    }

    fn evaluate_boolean(&mut self, kind: ConditionalKind) -> Result<bool, CommandError> {
        match kind {
            ConditionalKind::IfTrue => Ok(true),
            ConditionalKind::IfFalse => Ok(false),
            ConditionalKind::If => self.evaluate_if(),
            ConditionalKind::IfCat => self.evaluate_ifcat(),
            // TeX.web deliberately uses get_token, not get_x_token, here:
            // macro meanings are compared as raw operands rather than expanded.
            ConditionalKind::IfX => self.evaluate_ifx(),
            ConditionalKind::IfNum => self.evaluate_numeric_comparison(),
            ConditionalKind::IfDim => self.evaluate_dimension_comparison(),
            ConditionalKind::IfOdd => Ok(self.scan_integer()?.value & 1 != 0),
            ConditionalKind::IfVMode => Ok(matches!(
                self.host.conditional_state().mode(),
                crate::ConditionalMode::Vertical
            )),
            ConditionalKind::IfHMode => Ok(matches!(
                self.host.conditional_state().mode(),
                crate::ConditionalMode::Horizontal
            )),
            ConditionalKind::IfMMode => Ok(matches!(
                self.host.conditional_state().mode(),
                crate::ConditionalMode::Math
            )),
            ConditionalKind::IfInner => Ok(self.host.conditional_state().is_inner()),
            // TeX.web §505 uses `scan_eight_bit_int; p:=box(cur_val)`, while
            // e-TeX 2.6 [28.505] widens that exact selector to
            // `scan_register_num; fetch_box(p)`. The shared profile scan keeps
            // TeX82's recover-to-zero behavior and reads e-TeX's sparse bank.
            ConditionalKind::IfVoid | ConditionalKind::IfHBox | ConditionalKind::IfVBox => {
                let index = self.scan_profile_register_index()?;
                let box_kind = self.state.box_kind(index);
                Ok(match kind {
                    ConditionalKind::IfVoid => box_kind.is_none(),
                    ConditionalKind::IfHBox => {
                        box_kind == Some(tex_state::CommandBoxKind::Horizontal)
                    }
                    ConditionalKind::IfVBox => {
                        box_kind == Some(tex_state::CommandBoxKind::Vertical)
                    }
                    _ => unreachable!(),
                })
            }
            // TeX.web §501: `scan_four_bit_int; b:=(read_open[cur_val]=closed)`.
            ConditionalKind::IfEof => {
                let stream = self.scan_four_bit_int()?;
                Ok(self
                    .state
                    .read_stream_at_eof(tex_state::world::StreamSlot::new(stream)))
            }
            ConditionalKind::IfDefined => self.evaluate_ifdefined(),
            // e-TeX 2.6 etex.ch [17.4765--4779] expands the same character-name
            // stream as TeX82 §372's `\csname`, but performs §259's lookup
            // with `no_new_control_sequence` set. An absent spelling
            // therefore answers false without entering the hash table.
            ConditionalKind::IfCsName => {
                let name = self.scan_csname_characters()?;
                Ok(self
                    .state
                    .known_control_sequence(&name)
                    .is_some_and(|symbol| self.state.meaning(symbol) != Meaning::Undefined))
            }
            // e-TeX 2.6 etex.ch [17.4797--4805]: `\iffontchar` uses the
            // ordinary §577 font-identifier scanner followed by §434's
            // character-number scanner, then tests the TFM character-info
            // existence bit. The restricted scan owns out-of-range recovery,
            // and the immutable metric lookup works identically for fonts
            // restored from a format and fonts loaded in this session.
            ConditionalKind::IfFontChar => {
                let font = self.scan_font_selector()?;
                let character = self.scan_character_number()?;
                Ok(u8::try_from(u32::from(character))
                    .ok()
                    .is_some_and(|code| self.state.font_char_metrics(font, code).is_some()))
            }
            // pdfTeX §57.2 compares the live meaning of the following raw
            // control sequence with the immutable primitive-table entry of
            // the same spelling. Aliases and character tokens are false.
            ConditionalKind::IfPdfPrimitive => {
                let operand = self.get_next()?.ok_or(CommandError::input_invariant())?;
                let Some(symbol) = operand.control_sequence() else {
                    return Ok(false);
                };
                let name = self.state.resolve(symbol);
                Ok(operand.meaning() != Meaning::Undefined
                    && self.state.primitive_meaning(name) == Some(operand.meaning()))
            }
            // pdfTeX §57.3 applies comparison to mathematical magnitudes;
            // widening first preserves abs(INT_MIN) without overflow.
            ConditionalKind::IfPdfAbsNum => {
                let left = i64::from(self.scan_integer()?.value).abs();
                let relation = self.scan_if_relation("ifpdfabsnum")?;
                let right = i64::from(self.scan_integer()?.value).abs();
                Ok(relation.compare(left, right))
            }
            ConditionalKind::IfPdfAbsDim => {
                let left = i64::from(self.scan_dimension()?.value.raw()).abs();
                let relation = self.scan_if_relation("ifpdfabsdim")?;
                let right = i64::from(self.scan_dimension()?.value.raw()).abs();
                Ok(relation.compare(left, right))
            }
            // pdfTeX section 57.1 reads the dynamically scoped flag maintained
            // by both canonical control-sequence-name scanners.
            ConditionalKind::IfInCsName => Ok(self.is_in_csname),
            ConditionalKind::IfCase => unreachable!(),
        }
    }

    /// e-TeX 2.6 etex.ch [17.4712--4763] tests one raw command with
    /// `get_next`, temporarily setting `scanner_status := normal` so an outer
    /// control sequence is a legal operand even inside a definition or
    /// preamble. Unlike `get_token`, this does not enter a previously unseen
    /// control-sequence spelling; both that dummy command and an existing
    /// undefined meaning nevertheless carry `undefined_cs`.
    fn evaluate_ifdefined(&mut self) -> Result<bool, CommandError> {
        let episode =
            self.begin_scanner_episode(ScannerStatus::Normal, ScannerStatusVisibility::Observed);
        let operand = self.get_next();
        self.finish_scanner_episode(episode);
        Ok(operand?.ok_or(CommandError::input_invariant())?.meaning() != Meaning::Undefined)
    }

    fn evaluate_if(&mut self) -> Result<bool, CommandError> {
        let first = self.get_x_token_or_active_char()?;
        let second = self.get_x_token_or_active_char()?;
        Ok(Self::if_character_code(first) == Self::if_character_code(second))
    }

    fn evaluate_ifcat(&mut self) -> Result<bool, CommandError> {
        let first = self.get_x_token_or_active_char()?;
        let second = self.get_x_token_or_active_char()?;
        Ok(Self::if_category_code(first) == Self::if_category_code(second))
    }

    /// TeX.web §506's `get_x_token_or_active_char`, the operand fetch used by
    /// `\\if` and `\\ifcat` alone.
    ///
    /// An active character replayed by `\\noexpand` arrives as the generic
    /// frozen-`\\relax`/`no_expand_flag` command, which would otherwise
    /// compare as "not a character". §506 restores `cur_cmd:=active_char` and
    /// `cur_chr` from the retained `cur_tok`, so `\\if\\noexpand~` compares
    /// against the active character's own code and `\\ifcat\\noexpand~`
    /// against category 13.
    fn get_x_token_or_active_char(&mut self) -> Result<Meaning, CommandError> {
        let command = self.get_x_token()?.ok_or(CommandError::input_invariant())?;
        Ok(match command.no_expand_active_character() {
            Some(ch) => Meaning::CharToken {
                ch,
                cat: tex_state::token::Catcode::Active,
            },
            None => command.meaning(),
        })
    }

    /// TeX.web part 28 maps every non-character `\\if` operand to the
    /// shared sentinel 256 before comparing character codes.
    fn if_character_code(meaning: Meaning) -> u32 {
        match meaning {
            Meaning::CharToken { ch, .. } if (ch as u32) <= u32::from(u8::MAX) => ch as u32,
            _ => 256,
        }
    }

    /// TeX.web part 28 maps every non-character `\\ifcat` operand to the
    /// shared `relax` command sentinel before comparing category commands.
    fn if_category_code(meaning: Meaning) -> Option<tex_state::token::Catcode> {
        match meaning {
            Meaning::CharToken { cat, .. } => Some(cat),
            _ => None,
        }
    }

    /// TeX82 §507 reads both `\\ifx` operands with `get_next`, not
    /// `get_token`: `no_new_control_sequence` stays set (§365), so an operand
    /// naming a control sequence the hash table has never held is §259's
    /// dummy `undefined_control_sequence` and is not entered. Two such
    /// operands still compare equal, because §222 gives the dummy the
    /// `undefined_cs` command every fresh hash entry also starts with.
    /// Section 507 also makes outer operands legal by holding
    /// `scanner_status := normal` across both deliveries, then restoring the
    /// complete prior scanner state.
    fn evaluate_ifx(&mut self) -> Result<bool, CommandError> {
        let episode =
            self.begin_scanner_episode(ScannerStatus::Normal, ScannerStatusVisibility::Observed);
        let operands = (|| {
            let first = self.get_next()?.ok_or(CommandError::input_invariant())?;
            let second = self.get_next()?.ok_or(CommandError::input_invariant())?;
            Ok::<_, CommandError>((first, second))
        })();
        self.finish_scanner_episode(episode);
        let (first, second) = operands?;
        Ok(self.ifx_meaning_eq(first.meaning(), second.meaning()))
    }

    /// TeX compares macro meanings by their defining token lists, not by the
    /// engine's allocation identity for the macro definition. All other
    /// meanings retain their direct raw-meaning equality.
    fn ifx_meaning_eq(&self, first: Meaning, second: Meaning) -> bool {
        let (
            Meaning::Macro {
                flags: first_flags,
                definition: first_definition,
            },
            Meaning::Macro {
                flags: second_flags,
                definition: second_definition,
            },
        ) = (first, second)
        else {
            return first == second;
        };

        if first_flags != second_flags {
            return false;
        }
        let first = self.state.macro_definition(first_definition).meaning();
        let second = self.state.macro_definition(second_definition).meaning();
        let first_parameters = self.state.tokens(first.parameter_text());
        let second_parameters = self.state.tokens(second.parameter_text());
        let first_replacement = self.state.tokens(first.replacement_text());
        let second_replacement = self.state.tokens(second.replacement_text());
        first.flags() == second.flags()
            && *first_parameters == *second_parameters
            && *first_replacement == *second_replacement
    }

    fn evaluate_numeric_comparison(&mut self) -> Result<bool, CommandError> {
        let left = self.scan_integer()?.value;
        let relation = self.scan_if_relation("ifnum")?;
        let right = self.scan_integer()?.value;
        Ok(relation.compare(left, right))
    }

    fn evaluate_dimension_comparison(&mut self) -> Result<bool, CommandError> {
        let left = self.scan_dimension()?.value;
        let relation = self.scan_if_relation("ifdim")?;
        let right = self.scan_dimension()?.value;
        Ok(relation.compare(left, right))
    }

    /// TeX.web §503's relation lookahead for `\ifnum`/`\ifdim`: fetches the
    /// expanded token after the first operand and classifies it as `<`, `=`,
    /// or `>`. A token outside that set is not a scan failure: §503 reports
    /// "Missing = inserted for \ifnum"/"\ifdim" and calls `back_error` (back
    /// up the offending token, then continue as though `=` had been found),
    /// so the second operand is still scanned and the comparison completes.
    fn scan_if_relation(&mut self, conditional: &str) -> Result<IfRelation, CommandError> {
        let relation = self.get_x_token()?.ok_or(CommandError::input_invariant())?;
        match relation.meaning() {
            Meaning::CharToken { ch: '<', .. } => Ok(IfRelation::Less),
            Meaning::CharToken { ch: '=', .. } => Ok(IfRelation::Equal),
            Meaning::CharToken { ch: '>', .. } => Ok(IfRelation::Greater),
            _ => {
                // §503's `print_cmd_chr(if_test,this_if)` names the
                // conditional whose relation is missing, so the message ends
                // in the escaped primitive rather than a bare word.
                let name = crate::processor::expand::print_esc_text(&self.state, conditional);
                let message = format!("Missing = inserted for {name}");
                self.back_error_reporting(
                    relation,
                    MISSING_RELATION_DIAGNOSTIC,
                    message,
                    &["I was expecting to see `<', `=', or `>'. Didn't."],
                )?;
                Ok(IfRelation::Equal)
            }
        }
    }

    /// TeX.web §435's `scan_four_bit_int`: an ordinary integer scan whose
    /// result must name one of TeX's sixteen streams. Anything outside
    /// `0..=15` reports "Bad number" and recovers as stream zero rather than
    /// truncating; the scan itself has already completed normally.
    fn scan_four_bit_int(&mut self) -> Result<u8, CommandError> {
        let scanned = self.scan_restricted_integer(RestrictedIntegerClass::FourBit)?;
        if scanned.recovered {
            self.record_bad_number();
        }
        Ok(scanned.value as u8)
    }

    fn record_bad_number(&mut self) {
        self.command
            .expansion
            .pending_diagnostics
            .push(BAD_NUMBER_DIAGNOSTIC);
        observe!(
            self,
            CommandObservation::Diagnostic(DiagnosticRecord {
                severity: "error",
                diagnostic: "conditional_bad_stream_number",
                arguments: Vec::new(),
            }),
        );
    }

    /// TeX.web §509's limb skip, spelled `while n<>0 do begin pass_text; if
    /// cur_chr=or_code then decr(n) else goto common_ending end`.
    ///
    /// A negative case index is not a separate path: `n` only ever decreases,
    /// so it never reaches zero and the same loop skips through every limb to
    /// `\else` or `\fi`. Returns whether a limb was actually selected.
    fn skip_ifcase_limbs(
        &mut self,
        condition: ConditionId,
        mut remaining: i32,
    ) -> Result<bool, CommandError> {
        while remaining != 0 {
            let delimiter = self.pass_text(condition, ScannerWarning(0))?.delimiter;
            // TeX82 §509 compares `cond_ptr` with the frame saved before
            // scanning the case number.  Operand expansion can have pushed a
            // newer conditional, in which case only its `\fi` is acted on;
            // its `\or` or `\else` is passed and the limb scan continues.
            // This is deliberately different from `common_ending`: the
            // delimiter does not belong to this `\ifcase` until its frame is
            // current again.
            if self
                .command
                .conditions
                .current()
                .is_some_and(|frame| frame.identity != condition)
            {
                if delimiter == ConditionalDelimiter::Fi {
                    let frame = self
                        .command
                        .conditions
                        .pop()
                        .ok_or(CommandError::input_invariant())?;
                    self.observe_condition("pop", &frame, None);
                }
                continue;
            }
            if delimiter == ConditionalDelimiter::Or {
                remaining = remaining.saturating_sub(1);
                continue;
            }
            self.common_ending(condition, delimiter)?;
            return Ok(false);
        }
        Ok(true)
    }

    /// TeX.web §500's false-branch skip, including the `cond_ptr=p` test.
    ///
    /// Operand expansion may leave a completed inner condition above the
    /// condition whose false branch is being skipped. A delimiter belonging
    /// to that inner frame cannot end the saved condition: its `\fi` pops the
    /// inner frame and scanning continues until the saved frame is current.
    /// An `\or` reached for the saved boolean condition matches no `\ifcase`
    /// limb and is diagnosed rather than accepted.
    fn resume_after_skip(&mut self, condition: ConditionId) -> Result<(), CommandError> {
        loop {
            let delimiter = self.pass_text(condition, ScannerWarning(0))?.delimiter;
            if self
                .command
                .conditions
                .current()
                .is_some_and(|frame| frame.identity != condition)
            {
                if delimiter == ConditionalDelimiter::Fi {
                    let frame = self
                        .command
                        .conditions
                        .pop()
                        .ok_or(CommandError::input_invariant())?;
                    self.observe_condition("pop", &frame, None);
                }
                continue;
            }
            if delimiter == ConditionalDelimiter::Or {
                self.record_extra_delimiter(delimiter);
                continue;
            }
            return self.common_ending(condition, delimiter);
        }
    }

    /// TeX.web §498's `common_ending`: `if cur_chr=fi_code then <Pop the
    /// condition stack> else if_limit:=fi_code`. Shared verbatim by §498's
    /// false boolean branch and §509's exhausted `\ifcase` limb count.
    fn common_ending(
        &mut self,
        condition: ConditionId,
        delimiter: ConditionalDelimiter,
    ) -> Result<(), CommandError> {
        if delimiter == ConditionalDelimiter::Fi {
            if let Some(frame) = self.command.conditions.current().cloned() {
                self.warn_cross_file_conditional_close(&frame);
            }
            let frame = self
                .command
                .conditions
                .pop()
                .ok_or(CommandError::input_invariant())?;
            self.observe_condition("pop", &frame, None);
            return Ok(());
        }
        self.command
            .conditions
            .change_if_limit(condition, IfLimit::Fi)
            .then_some(())
            .ok_or(CommandError::input_invariant())?;
        let frame = self
            .command
            .conditions
            .frame(condition)
            .cloned()
            .ok_or(CommandError::input_invariant())?;
        self.observe_condition("limit", &frame, None);
        Ok(())
    }

    pub(crate) fn expand_conditional_delimiter(
        &mut self,
        command: &crate::CurrentCommand,
        primitive: ExpandablePrimitive,
    ) -> Result<(), CommandError> {
        let delimiter = match primitive {
            ExpandablePrimitive::Else => ConditionalDelimiter::Else,
            ExpandablePrimitive::Or => ConditionalDelimiter::Or,
            ExpandablePrimitive::Fi => ConditionalDelimiter::Fi,
            _ => return Err(CommandError::input_invariant()),
        };
        let Some(frame) = self.command.conditions.current().cloned() else {
            self.record_extra_delimiter(delimiter);
            return Ok(());
        };
        self.trace_conditional_close(delimiter, &frame, false);
        if self
            .command
            .conditions
            .evaluating_delimiter_recovery(frame.identity, delimiter)
            .is_some()
        {
            // TeX.web's incomplete-conditional path uses `back_error`: the
            // delimiter is replayed below the inserted frozen `\relax`.
            // That ordering matters because the resumed operand scanner must
            // see the delimiter through ordinary raw delivery after recovery.
            self.back_input(command.copy_for_backup())?;
            self.recover_incomplete_if()?;
            return Ok(());
        }
        if !frame.limit.accepts_delimiter(delimiter) {
            self.record_extra_delimiter(delimiter);
            return Ok(());
        }
        self.skip_to_fi_after_delimiter(frame, delimiter)
    }

    /// TeX.web §510's accepted-delimiter tail, spelled `while cur_chr<>fi_code
    /// do pass_text; <Pop the condition stack>`.
    ///
    /// The loop tests the delimiter already in hand first, so `\fi` skips
    /// nothing and pops immediately — §510 has no separate `\fi` case. Any
    /// `\or`/`\else` closing an unselected remaining limb is swallowed by the
    /// loop and is never a diagnostic: the ordinary multi-arm `\ifcase` with a
    /// non-final limb selected reaches this loop once per remaining limb.
    ///
    /// §510 deliberately leaves `if_limit` alone: the frame is popped at the
    /// end regardless, and the limit it still carries is what §494's branch
    /// records name for every limb skipped here.
    fn skip_to_fi_after_delimiter(
        &mut self,
        frame: ConditionFrame,
        delimiter: ConditionalDelimiter,
    ) -> Result<(), CommandError> {
        let mut stopped = delimiter;
        while stopped != ConditionalDelimiter::Fi {
            stopped = self.pass_text(frame.identity, ScannerWarning(0))?.delimiter;
        }
        self.warn_cross_file_conditional_close(&frame);
        let popped = self
            .command
            .conditions
            .pop()
            .ok_or(CommandError::input_invariant())?;
        self.observe_condition("pop", &popped, None);
        Ok(())
    }

    fn record_extra_delimiter(&mut self, delimiter: ConditionalDelimiter) {
        // §510's `print_cmd_chr(fi_or_else,cur_chr)` names the delimiter that
        // matched nothing, so the message ends in the escaped primitive.
        let name = crate::processor::expand::print_esc_text(
            &self.state,
            match delimiter {
                ConditionalDelimiter::Or => "or",
                ConditionalDelimiter::Else => "else",
                ConditionalDelimiter::Fi => "fi",
            },
        );
        let context = self.command.output_open_context(&self.state);
        self.command
            .semantic_diagnostics
            .push(crate::CommandSemanticDiagnostic::Recoverable {
                identity: EXTRA_DELIMITER_DIAGNOSTIC,
                runaway: None,
                message: format!("Extra {name}"),
                help: &["I'm ignoring this; it doesn't match any \\if."],
                context,
            });
        self.command
            .expansion
            .pending_diagnostics
            .push(EXTRA_DELIMITER_DIAGNOSTIC);
        // TeX82 §509 diagnoses a delimiter which exceeds the current
        // `if_limit` at the delimiter transition itself.  Keep the pending
        // diagnostic for engine-facing recovery, but publish the detached
        // command event here so it remains ordered after raw delivery and
        // before the following token.
        observe!(
            self,
            CommandObservation::Diagnostic(DiagnosticRecord {
                severity: "error",
                diagnostic: "conditional_extra_delimiter",
                arguments: Vec::new(),
            }),
        );
    }

    /// TeX inserts its inaccessible frozen `\\relax` when a delimiter is
    /// encountered before the current conditional has consumed its operands.
    fn recover_incomplete_if(&mut self) -> Result<(), CommandError> {
        let relax = self
            .state
            .primitive_token("relax")
            .ok_or(CommandError::input_invariant())?;
        self.command
            .expansion
            .pending_diagnostics
            .push(INCOMPLETE_IF_DIAGNOSTIC);
        let level = self.command.push_token_level(
            TokenPayload::transient([TracedTokenWord::pack(relax, OriginId::UNKNOWN)]),
            TokenBehavior::Recovery,
            RetirementBehavior::Pop,
            ReplayTrace::Inserted,
        );
        if self.is_observed() {
            self.observe(CommandObservation::Input(InputRecord {
                transition: InputTransition::Recovery,
                reason: InputReason::Recovery,
                source_name: None,
                source: None,
                level: level.0,
                position: 0,
            }));
            self.observe(CommandObservation::Recovery(RecoveryRecord {
                kind: RecoveryKind::InsertedToken,
                // The frozen token is deliberately opaque to TeX input, but
                // the canonical observer reports its primitive spelling.
                tokens: vec![ObservedToken::ControlSequence("relax".into())],
            }));
            self.observe(CommandObservation::Diagnostic(DiagnosticRecord {
                severity: "error",
                diagnostic: "conditional_limit_recovery",
                arguments: Vec::new(),
            }));
        }
        Ok(())
    }
    /// TeX.web §494's `pass_text`: skip through the sole canonical raw
    /// delivery path, never by peeking at input levels or retokenizing source.
    ///
    /// §494's `done:` label is the one place TeX resolves a skipped limb, so
    /// it is also the one place the canonical observer records which
    /// delimiter ended it. Every caller — §498's false branch, §509's
    /// `\ifcase` limb count, §510's skip to `\fi` — reaches that same label,
    /// so none of them records a branch of its own. The record names the live
    /// top-of-stack frame, TeX's `cur_if`/`if_limit`, which is not
    /// necessarily the frame this skip was started for: §500's
    /// `\if\iftrue...` case leaves an inner frame on top.
    pub(crate) fn pass_text(
        &mut self,
        condition: ConditionId,
        warning: ScannerWarning,
    ) -> Result<PassTextStop, CommandError> {
        // §494's `skip_line:=line`, taken before any token of the skipped
        // text is read.
        let skip_line = self.command.current_file_line_number();
        // §336's `cur_if` is the live top-of-stack conditional, which §494's
        // own comment above notes need not be the frame this skip was
        // started for: §500's `\if\iftrue...` leaves an inner frame on top.
        let conditional = self
            .command
            .conditions
            .current()
            .map_or(ConditionalKind::IfTrue, |frame| frame.kind);
        let episode = self.begin_scanner_episode(
            ScannerStatus::Skipping(SkippingContext {
                condition,
                warning,
                skip_line,
                conditional,
            }),
            ScannerStatusVisibility::Observed,
        );
        let result = self.pass_text_scalar(condition);
        // `check_outer_validity` clears a live skipping episode before it
        // inserts frozen `\\fi`.  The lexical recovery is still the end of
        // this `pass_text` invocation, so retain its canonical
        // skipping-to-prior transition instead of publishing a spurious
        // normal-to-normal restoration after nested-source EOF.
        self.finish_scanner_episode(episode);
        if let Ok(stop) = &result {
            self.observe_pass_text_branch(stop.delimiter);
        }
        result
    }

    /// TeX.web §494's `done:` branch record, published after the
    /// scanner-status restoration it follows in the same label.
    #[allow(unused_variables)]
    fn observe_pass_text_branch(&mut self, delimiter: ConditionalDelimiter) {
        if let Some(frame) = self.command.conditions.current().cloned() {
            self.trace_conditional_close(delimiter, &frame, true);
            self.observe_condition("branch", &frame, Some(delimiter.canonical_branch()));
        }
    }

    /// e-TeX 2.6 [28.498]'s extra `show_cur_cmd_chr` fired at conditional
    /// entry, before the operand scan that may recursively expand another
    /// conditional: `if tracing_ifs>0 then if tracing_commands<=1 then
    /// show_cur_cmd_chr`. `frame` is already the pushed top-of-stack entry,
    /// so its own depth is the level e-TeX displays.
    ///
    fn trace_conditional_enter(&mut self, frame: &ConditionFrame) {
        if self.state.int_param(IntParam::TRACING_IFS) <= 0
            || (self.state.int_param(IntParam::TRACING_COMMANDS) > 1 && !frame.inverted)
        {
            return;
        }
        let level = self.command.conditions.frames.len();
        let name = self.conditional_kind_text(frame);
        let mode_prefix = self.claim_command_trace_mode_prefix();
        if !self.command.semantic_diagnostics.is_empty() {
            let mut text = String::from("{");
            if let Some(mode_prefix) = mode_prefix {
                text.push_str(&mode_prefix);
                text.push_str(": ");
            }
            text.push_str(&name);
            text.push_str(": (level ");
            text.push_str(&level.to_string());
            text.push(')');
            append_if_line(&mut text, frame.source_line);
            text.push('}');
            self.command
                .semantic_diagnostics
                .push(crate::CommandSemanticDiagnostic::Trace {
                    text,
                    force_newline: false,
                });
            return;
        }
        let mut diagnostic = self.state.begin_diagnostic();
        diagnostic.print_char('{');
        if let Some(mode_prefix) = mode_prefix {
            diagnostic.print(&mode_prefix).print(": ");
        }
        diagnostic
            .print(&name)
            .print(": (level ")
            .print_int(i32::try_from(level).unwrap_or(i32::MAX))
            .print_char(')');
        print_if_line(&mut diagnostic, frame.source_line);
        diagnostic.print_char('}');
        diagnostic.end(false);
    }

    /// e-TeX 2.6 [28.494/28.510]'s extra `show_cur_cmd_chr` fired wherever a
    /// `\or`/`\else`/`\fi` delimiter resolves a conditional's current limb --
    /// whether it arrives through ordinary expansion (§510, this file's
    /// `expand_conditional_delimiter`) or is found while skipping unselected
    /// material (§494's `pass_text` `done:` label, this file's
    /// `observe_pass_text_branch`). `frame` is the live top-of-stack entry,
    /// which need not be the frame the enclosing skip was started for.
    fn trace_conditional_close(
        &mut self,
        delimiter: ConditionalDelimiter,
        frame: &ConditionFrame,
        found_by_pass_text: bool,
    ) {
        if self.state.int_param(IntParam::TRACING_IFS) <= 0
            || (self.state.int_param(IntParam::TRACING_COMMANDS) > 1 && !found_by_pass_text)
        {
            return;
        }
        let level = self.command.conditions.frames.len();
        let delimiter_name =
            crate::processor::expand::print_esc_text(&self.state, delimiter.canonical_branch());
        let condition_name = self.conditional_kind_text(frame);
        if !self.command.semantic_diagnostics.is_empty() {
            let mut text = format!("{{{delimiter_name}: {condition_name} (level {level})");
            append_if_line(&mut text, frame.source_line);
            text.push('}');
            self.command
                .semantic_diagnostics
                .push(crate::CommandSemanticDiagnostic::Trace {
                    text,
                    force_newline: false,
                });
            return;
        }
        let mut diagnostic = self.state.begin_diagnostic();
        diagnostic
            .print_char('{')
            .print(&delimiter_name)
            .print(": ")
            .print(&condition_name)
            .print(" (level ")
            .print_int(i32::try_from(level).unwrap_or(i32::MAX))
            .print_char(')');
        print_if_line(&mut diagnostic, frame.source_line);
        diagnostic.print_char('}');
        diagnostic.end(false);
    }

    /// e-TeX's additions to TeX82 §299 when §367 traces an expandable
    /// conditional or delimiter before its expansion routine runs.
    pub(crate) fn command_trace_conditional_suffix(&self, meaning: Meaning) -> String {
        if self.state.untracked_int_param(IntParam::TRACING_IFS) <= 0 {
            return String::new();
        }
        if let Meaning::ExpandablePrimitive(primitive) = meaning
            && ConditionalKind::from_primitive(primitive).is_some()
        {
            let level = self.command.conditions.frames.len() + 1;
            let line = u32::try_from(self.command.input.current_file_line_number()).unwrap_or(0);
            return conditional_trace_suffix(level, None, line);
        }
        if ConditionalDelimiter::from_meaning(meaning).is_none() {
            return String::new();
        }
        let Some(frame) = self.command.conditions.current() else {
            return String::new();
        };
        let mut condition = String::new();
        if frame.inverted {
            crate::processor::expand::append_print_esc_text(&self.state, "unless", &mut condition);
        }
        crate::processor::expand::append_print_esc_text(
            &self.state,
            frame.kind.canonical_name(),
            &mut condition,
        );
        conditional_trace_suffix(
            self.command.conditions.frames.len(),
            Some(condition),
            frame.source_line,
        )
    }

    /// e-TeX's `\unless`-prefixed `print_cmd_chr(if_test,cur_if)` spelling.
    pub(crate) fn conditional_kind_text(&self, frame: &ConditionFrame) -> String {
        let mut name = String::new();
        if frame.inverted {
            crate::processor::expand::append_print_esc_text(&self.state, "unless", &mut name);
        }
        crate::processor::expand::append_print_esc_text(
            &self.state,
            frame.kind.canonical_name(),
            &mut name,
        );
        name
    }

    #[allow(unused_variables)]
    fn observe_condition(
        &mut self,
        transition: &'static str,
        frame: &ConditionFrame,
        branch: Option<&'static str>,
    ) {
        // e-TeX 2.6 etex.ch [17.4713--4751] stores `\unless` by adding
        // `unless_code` to `cur_if`. The immediate boolean-result observation
        // is deliberately about `this_if` (the unprefixed predicate), while
        // the pushed and subsequently retained frame observations use
        // `cur_if` and therefore retain the prefix.
        let evaluating_boolean_result = transition == "branch"
            && frame.limit == IfLimit::Evaluating
            && matches!(branch, Some("true" | "false"));
        observe!(
            self,
            CommandObservation::Condition(ConditionRecord {
                transition,
                identity: frame.identity.0,
                condition: if frame.inverted && !evaluating_boolean_result {
                    format!("unless_{}", frame.kind.canonical_name())
                } else {
                    frame.kind.canonical_name().into()
                },
                limit: frame.limit.canonical_name(),
                branch: branch.map(str::to_owned),
            }),
        );
    }

    fn pass_text_scalar(&mut self, condition: ConditionId) -> Result<PassTextStop, CommandError> {
        self.command
            .conditions
            .limit(condition)
            .ok_or(CommandError::input_invariant())?;

        let mut nested_conditions = 0_u32;
        loop {
            let Some(command) = self.get_next()? else {
                return Err(CommandError::input_invariant());
            };
            if let Meaning::ExpandablePrimitive(primitive) = command.meaning()
                && ConditionalKind::from_primitive(primitive).is_some()
            {
                nested_conditions = nested_conditions.saturating_add(1);
                continue;
            }
            let Some(delimiter) = ConditionalDelimiter::from_meaning(command.meaning()) else {
                continue;
            };
            if nested_conditions != 0 {
                if delimiter == ConditionalDelimiter::Fi {
                    nested_conditions -= 1;
                }
                continue;
            }
            return Ok(PassTextStop {
                delimiter,
                nested_conditions,
            });
        }
    }
}

fn conditional_trace_suffix(level: usize, condition: Option<String>, source_line: u32) -> String {
    let mut text = String::from(": ");
    if let Some(condition) = condition {
        text.push_str(&condition);
        text.push(' ');
    }
    text.push_str("(level ");
    text.push_str(&level.to_string());
    text.push(')');
    if source_line != 0 {
        text.push_str(" entered on line ");
        text.push_str(&source_line.to_string());
    }
    text
}

/// e-TeX 2.6 [49.3715]'s `print_if_line`: `if #<>0 then begin print(" entered
/// on line "); print_int(#); end`, shared by `\tracingifs` and `\showifs`.
fn print_if_line(diagnostic: &mut tex_state::diagnostic::Diagnostic<'_>, source_line: u32) {
    if source_line != 0 {
        diagnostic
            .print(" entered on line ")
            .print_int(i32::try_from(source_line).unwrap_or(i32::MAX));
    }
}

fn append_if_line(text: &mut String, source_line: u32) {
    if source_line != 0 {
        text.push_str(" entered on line ");
        text.push_str(&source_line.to_string());
    }
}

#[cfg(test)]
mod tests;
