//! Independent TeX conditional-stack state and skipped-text delivery.
//!
//! This mirrors TeX.web part 28.  Conditions deliberately are not input
//! levels: recursive expansion can push another condition while an older
//! condition is still evaluating its operands.

use tex_state::meaning::{ExpandablePrimitive, Meaning};
use tex_state::token::{OriginId, TracedTokenWord};

use crate::CommandError;
use crate::input::{
    ReplayTrace, RetirementBehavior, SharedTokenBuffer, TokenBehavior, TokenPayload,
};
use crate::processor::CommandProcessor;
use crate::processor::status::{ConditionId, ScannerStatus, ScannerWarning, SkippingContext};

#[cfg(any(test, feature = "instrumentation"))]
use crate::observation::{
    CommandObservation, ConditionRecord, DiagnosticRecord, InputReason, InputRecord,
    InputTransition, ObservedToken, RecoveryKind, RecoveryRecord,
};

/// Stable pending-diagnostic identities for TeX.web part 28 recovery.
const INCOMPLETE_IF_DIAGNOSTIC: u64 = 0x636f_6e64_0000_0001;
const EXTRA_DELIMITER_DIAGNOSTIC: u64 = 0x636f_6e64_0000_0002;
const MISSING_RELATION_DIAGNOSTIC: u64 = 0x636f_6e64_0000_0003;
const BAD_NUMBER_DIAGNOSTIC: u64 = 0x636f_6e64_0000_0004;

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
}

#[allow(dead_code)] // used by pass_text now; evaluation uses the same classifier next
impl ConditionalKind {
    #[cfg(any(test, feature = "instrumentation"))]
    const fn canonical_name(self) -> &'static str {
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
    #[cfg(any(test, feature = "instrumentation"))]
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
}

/// Independent persistent condition stack.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub(crate) struct ConditionStack {
    pub(crate) frames: Vec<ConditionFrame>,
    pub(crate) next_identity: u64,
}

impl ConditionStack {
    pub(crate) fn push(&mut self, kind: ConditionalKind, source_line: u32) -> ConditionId {
        let identity = ConditionId(self.next_identity);
        self.next_identity = self.next_identity.wrapping_add(1);
        self.frames.push(ConditionFrame {
            identity,
            kind,
            limit: IfLimit::Evaluating,
            source_line,
        });
        identity
    }

    pub(crate) fn current(&self) -> Option<&ConditionFrame> {
        self.frames.last()
    }

    pub(crate) fn pop(&mut self) -> Option<ConditionFrame> {
        self.frames.pop()
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
        command: crate::CurrentCommand,
        inverted: bool,
    ) -> Result<(), CommandError> {
        let Meaning::ExpandablePrimitive(primitive) = command.meaning() else {
            return Err(CommandError::input_invariant());
        };
        let kind =
            ConditionalKind::from_primitive(primitive).ok_or(CommandError::input_invariant())?;
        let condition = self.command.conditions.push(kind, 0);
        let frame = self
            .command
            .conditions
            .frame(condition)
            .cloned()
            .ok_or(CommandError::input_invariant())?;
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
    /// `\\ifcase` is deliberately rejected here, matching e-TeX's separate
    /// diagnostic path rather than silently inverting a case index.
    pub(crate) fn expand_unless(
        &mut self,
        _command: crate::CurrentCommand,
    ) -> Result<(), CommandError> {
        // The following conditional is an operand of `\unless`, not an
        // ordinary expansion result: preserve its primitive command for the
        // shared evaluator to install the one inverted frame.
        let next = self.get_token()?.ok_or(CommandError::input_invariant())?;
        let Meaning::ExpandablePrimitive(primitive) = next.meaning() else {
            return Err(CommandError::input_invariant());
        };
        let kind =
            ConditionalKind::from_primitive(primitive).ok_or(CommandError::input_invariant())?;
        if kind == ConditionalKind::IfCase {
            return Err(CommandError::input_invariant());
        }
        self.expand_conditional(next, true)
    }

    fn complete_boolean(
        &mut self,
        condition: ConditionId,
        result: bool,
    ) -> Result<(), CommandError> {
        if result {
            let evaluating = self
                .command
                .conditions
                .frame(condition)
                .cloned()
                .ok_or(CommandError::input_invariant())?;
            self.command
                .conditions
                .change_if_limit(condition, IfLimit::Else)
                .then_some(())
                .ok_or(CommandError::input_invariant())?;
            self.observe_condition("branch", &evaluating, Some("true".into()));
            let frame = self
                .command
                .conditions
                .frame(condition)
                .cloned()
                .ok_or(CommandError::input_invariant())?;
            self.observe_condition("limit", &frame, None);
            Ok(())
        } else {
            let evaluating = self
                .command
                .conditions
                .frame(condition)
                .cloned()
                .ok_or(CommandError::input_invariant())?;
            self.observe_condition("branch", &evaluating, Some("false".into()));
            self.resume_after_skip(condition)
        }
    }

    fn complete_ifcase(
        &mut self,
        condition: ConditionId,
        selected: i32,
    ) -> Result<(), CommandError> {
        let selected_limb = if selected < 0 {
            self.skip_to_else_or_fi(condition)?;
            false
        } else {
            self.skip_ifcase_limbs(condition, selected)?
        };
        if selected_limb {
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
            self.observe_condition("branch", &frame, Some("case".into()));
        }
        Ok(())
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
            ConditionalKind::IfVoid | ConditionalKind::IfHBox | ConditionalKind::IfVBox => {
                let index = self.scan_integer()?.value;
                let index = u16::try_from(index).map_err(|_| CommandError::input_invariant())?;
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
            ConditionalKind::IfDefined
            | ConditionalKind::IfCsName
            | ConditionalKind::IfFontChar
            | ConditionalKind::IfInCsName
            | ConditionalKind::IfCase => {
                Err(CommandError::UnsupportedExpandablePrimitive(match kind {
                    ConditionalKind::IfDefined => ExpandablePrimitive::IfDefined,
                    ConditionalKind::IfCsName => ExpandablePrimitive::IfCsName,
                    ConditionalKind::IfFontChar => ExpandablePrimitive::IfFontChar,
                    ConditionalKind::IfInCsName => ExpandablePrimitive::IfInCsName,
                    _ => unreachable!(),
                }))
            }
        }
    }

    fn evaluate_if(&mut self) -> Result<bool, CommandError> {
        let first = self.get_x_token()?.ok_or(CommandError::input_invariant())?;
        let second = self.get_x_token()?.ok_or(CommandError::input_invariant())?;
        Ok(Self::if_character_code(first.meaning()) == Self::if_character_code(second.meaning()))
    }

    fn evaluate_ifcat(&mut self) -> Result<bool, CommandError> {
        let first = self.get_x_token()?.ok_or(CommandError::input_invariant())?;
        let second = self.get_x_token()?.ok_or(CommandError::input_invariant())?;
        Ok(Self::if_category_code(first.meaning()) == Self::if_category_code(second.meaning()))
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

    fn evaluate_ifx(&mut self) -> Result<bool, CommandError> {
        let first = self.get_token()?.ok_or(CommandError::input_invariant())?;
        let second = self.get_token()?.ok_or(CommandError::input_invariant())?;
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
        let first = self.state.macro_definition(first_definition);
        let second = self.state.macro_definition(second_definition);
        first.flags() == second.flags()
            && self.state.tokens(first.parameter_text())
                == self.state.tokens(second.parameter_text())
            && self.state.tokens(first.replacement_text())
                == self.state.tokens(second.replacement_text())
    }

    fn evaluate_numeric_comparison(&mut self) -> Result<bool, CommandError> {
        let left = self.scan_integer()?.value;
        let relation = self.scan_if_relation()?;
        let right = self.scan_integer()?.value;
        Ok(relation.compare(left, right))
    }

    fn evaluate_dimension_comparison(&mut self) -> Result<bool, CommandError> {
        let left = self.scan_dimension()?.value;
        let relation = self.scan_if_relation()?;
        let right = self.scan_dimension()?.value;
        Ok(relation.compare(left, right))
    }

    /// TeX.web §503's relation lookahead for `\ifnum`/`\ifdim`: fetches the
    /// expanded token after the first operand and classifies it as `<`, `=`,
    /// or `>`. A token outside that set is not a scan failure: §503 reports
    /// "Missing = inserted for \ifnum"/"\ifdim" and calls `back_error` (back
    /// up the offending token, then continue as though `=` had been found),
    /// so the second operand is still scanned and the comparison completes.
    fn scan_if_relation(&mut self) -> Result<IfRelation, CommandError> {
        let relation = self.get_x_token()?.ok_or(CommandError::input_invariant())?;
        match relation.meaning() {
            Meaning::CharToken { ch: '<', .. } => Ok(IfRelation::Less),
            Meaning::CharToken { ch: '=', .. } => Ok(IfRelation::Equal),
            Meaning::CharToken { ch: '>', .. } => Ok(IfRelation::Greater),
            _ => {
                self.back_error(relation, MISSING_RELATION_DIAGNOSTIC)?;
                Ok(IfRelation::Equal)
            }
        }
    }

    /// TeX.web §433's `scan_four_bit_int`: an ordinary integer scan whose
    /// result must name one of TeX's sixteen streams. Anything outside
    /// `0..=15` reports "Bad number" and recovers as stream zero rather than
    /// truncating; the scan itself has already completed normally.
    fn scan_four_bit_int(&mut self) -> Result<u8, CommandError> {
        let value = self.scan_integer()?.value;
        match u8::try_from(value) {
            Ok(stream) if stream <= 15 => Ok(stream),
            _ => {
                self.record_bad_number();
                Ok(0)
            }
        }
    }

    fn record_bad_number(&mut self) {
        self.command
            .expansion
            .pending_diagnostics
            .push(BAD_NUMBER_DIAGNOSTIC);
        #[cfg(any(test, feature = "instrumentation"))]
        self.observe(CommandObservation::Diagnostic(DiagnosticRecord {
            severity: "error",
            diagnostic: "conditional_bad_stream_number",
            arguments: Vec::new(),
        }));
    }

    fn skip_ifcase_limbs(
        &mut self,
        condition: ConditionId,
        mut remaining: i32,
    ) -> Result<bool, CommandError> {
        if remaining == 0 {
            return Ok(true);
        }
        loop {
            match self.pass_text(condition, ScannerWarning(0))?.delimiter {
                ConditionalDelimiter::Or => {
                    let evaluating = self
                        .command
                        .conditions
                        .frame(condition)
                        .cloned()
                        .ok_or(CommandError::input_invariant())?;
                    self.observe_condition("branch", &evaluating, Some("or".into()));
                    remaining -= 1;
                    if remaining == 0 {
                        return Ok(true);
                    }
                }
                ConditionalDelimiter::Else => {
                    self.command
                        .conditions
                        .change_if_limit(condition, IfLimit::Fi);
                    return Ok(false);
                }
                ConditionalDelimiter::Fi => {
                    let frame = self
                        .command
                        .conditions
                        .pop()
                        .ok_or(CommandError::input_invariant())?;
                    self.observe_condition("pop", &frame, None);
                    return Ok(false);
                }
            }
        }
    }

    fn skip_to_else_or_fi(&mut self, condition: ConditionId) -> Result<(), CommandError> {
        loop {
            match self.pass_text(condition, ScannerWarning(0))?.delimiter {
                ConditionalDelimiter::Or => {}
                ConditionalDelimiter::Else => {
                    self.command
                        .conditions
                        .change_if_limit(condition, IfLimit::Fi);
                    return Ok(());
                }
                ConditionalDelimiter::Fi => {
                    let frame = self
                        .command
                        .conditions
                        .pop()
                        .ok_or(CommandError::input_invariant())?;
                    self.observe_condition("pop", &frame, None);
                    return Ok(());
                }
            }
        }
    }

    fn resume_after_skip(&mut self, condition: ConditionId) -> Result<(), CommandError> {
        loop {
            match self.pass_text(condition, ScannerWarning(0))?.delimiter {
                ConditionalDelimiter::Else => {
                    let evaluating = self
                        .command
                        .conditions
                        .frame(condition)
                        .cloned()
                        .ok_or(CommandError::input_invariant())?;
                    self.observe_condition("branch", &evaluating, Some("else".into()));
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
                    return Ok(());
                }
                ConditionalDelimiter::Fi => {
                    let frame = self
                        .command
                        .conditions
                        .pop()
                        .ok_or(CommandError::input_invariant())?;
                    self.observe_condition("branch", &frame, Some("fi".into()));
                    self.observe_condition("pop", &frame, None);
                    return Ok(());
                }
                ConditionalDelimiter::Or => self.record_extra_delimiter(),
            }
        }
    }

    pub(crate) fn expand_conditional_delimiter(
        &mut self,
        command: crate::CurrentCommand,
        primitive: ExpandablePrimitive,
    ) -> Result<(), CommandError> {
        let delimiter = match primitive {
            ExpandablePrimitive::Else => ConditionalDelimiter::Else,
            ExpandablePrimitive::Or => ConditionalDelimiter::Or,
            ExpandablePrimitive::Fi => ConditionalDelimiter::Fi,
            _ => return Err(CommandError::input_invariant()),
        };
        let Some(frame) = self.command.conditions.current().cloned() else {
            self.record_extra_delimiter();
            return Ok(());
        };
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
            self.back_input(command)?;
            self.recover_incomplete_if()?;
            return Ok(());
        }
        if !frame.limit.accepts_delimiter(delimiter) {
            self.record_extra_delimiter();
            return Ok(());
        }
        match delimiter {
            ConditionalDelimiter::Fi => {
                let frame = self
                    .command
                    .conditions
                    .pop()
                    .ok_or(CommandError::input_invariant())?;
                self.observe_condition("pop", &frame, None);
                Ok(())
            }
            // Every other accepted delimiter terminates the selected limb.
            // TeX.web §510's `else` branch is unconditional in the delimiter:
            // once `cur_chr <= if_limit`, the remaining text up to the
            // matching `\fi` is skipped and the frame is popped.
            ConditionalDelimiter::Else | ConditionalDelimiter::Or => {
                self.command
                    .conditions
                    .change_if_limit(frame.identity, IfLimit::Fi);
                self.skip_to_fi_after_delimiter(frame)
            }
        }
    }

    /// Skips the remainder of a selected limb after its `\else` or `\or`.
    /// The pre-change frame remains the canonical branch-observation seam;
    /// the independent stack is then unlinked only after `pass_text` reaches
    /// its matching `\fi` (TeX.web §510).
    ///
    /// TeX.web §510 spells this skip `while cur_chr<>fi_code do pass_text`:
    /// the loop inspects only whether the delimiter `pass_text` stopped at is
    /// `\fi`. Any `\or`/`\else` closing an unselected remaining limb is
    /// silently swallowed here, and is never a diagnostic — the ordinary
    /// multi-arm `\ifcase` with a non-final limb selected reaches this loop
    /// once per remaining limb.
    fn skip_to_fi_after_delimiter(&mut self, frame: ConditionFrame) -> Result<(), CommandError> {
        loop {
            if self.pass_text(frame.identity, ScannerWarning(0))?.delimiter
                == ConditionalDelimiter::Fi
            {
                self.observe_condition("branch", &frame, Some("fi".into()));
                let _popped = self
                    .command
                    .conditions
                    .pop()
                    .ok_or(CommandError::input_invariant())?;
                self.observe_condition("pop", &frame, None);
                return Ok(());
            }
        }
    }

    fn record_extra_delimiter(&mut self) {
        self.command
            .expansion
            .pending_diagnostics
            .push(EXTRA_DELIMITER_DIAGNOSTIC);
        // TeX82 §509 diagnoses a delimiter which exceeds the current
        // `if_limit` at the delimiter transition itself.  Keep the pending
        // diagnostic for engine-facing recovery, but publish the detached
        // command event here so it remains ordered after raw delivery and
        // before the following token.
        #[cfg(any(test, feature = "instrumentation"))]
        self.observe(CommandObservation::Diagnostic(DiagnosticRecord {
            severity: "error",
            diagnostic: "conditional_extra_delimiter",
            arguments: Vec::new(),
        }));
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
            TokenPayload::Transient(SharedTokenBuffer::new(vec![TracedTokenWord::pack(
                relax,
                OriginId::UNKNOWN,
            )])),
            TokenBehavior::Recovery,
            RetirementBehavior::Pop,
            ReplayTrace::Transient(crate::input::TransientReplayReason::Inserted),
        );
        #[cfg(not(any(test, feature = "instrumentation")))]
        let _ = level;
        #[cfg(any(test, feature = "instrumentation"))]
        {
            self.observe(CommandObservation::Input(InputRecord {
                transition: InputTransition::Recovery,
                reason: InputReason::Recovery,
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
    /// TeX.web's `pass_text`: skip through the sole canonical raw delivery
    /// path, never by peeking at input levels or retokenizing source.
    pub(crate) fn pass_text(
        &mut self,
        condition: ConditionId,
        warning: ScannerWarning,
    ) -> Result<PassTextStop, CommandError> {
        let prior = self
            .command
            .begin_scanner_status(ScannerStatus::Skipping(SkippingContext {
                condition,
                warning,
            }));
        self.observe_scanner_status_transition(
            prior.status().clone(),
            self.command.scanner.status().clone(),
        );
        let skipping = self.command.scanner.status().clone();
        let result = self.pass_text_scalar(condition);
        // `check_outer_validity` clears a live skipping episode before it
        // inserts frozen `\\fi`.  The lexical recovery is still the end of
        // this `pass_text` invocation, so retain its canonical
        // skipping-to-prior transition instead of publishing a spurious
        // normal-to-normal restoration after nested-source EOF.
        self.restore_scanner_status_with_observation(skipping, prior);
        result
    }

    #[allow(unused_variables)]
    fn observe_condition(
        &mut self,
        transition: &'static str,
        frame: &ConditionFrame,
        branch: Option<String>,
    ) {
        #[cfg(any(test, feature = "instrumentation"))]
        self.observe(CommandObservation::Condition(ConditionRecord {
            transition,
            identity: frame.identity.0,
            condition: frame.kind.canonical_name(),
            limit: frame.limit.canonical_name(),
            branch,
        }));
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

#[cfg(test)]
mod tests;
