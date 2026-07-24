//! Independent TeX conditional-stack state and skipped-text delivery.
//!
//! This mirrors TeX.web part 28.  Conditions deliberately are not input
//! levels: recursive expansion can push another condition while an older
//! condition is still evaluating its operands.

use tex_state::meaning::{ExpandablePrimitive, Meaning};

use crate::CommandError;
use crate::processor::CommandProcessor;
use crate::processor::status::{ConditionId, ScannerStatus, ScannerWarning, SkippingContext};

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
    const fn accepts_skipped_delimiter(self, delimiter: ConditionalDelimiter) -> bool {
        match delimiter {
            ConditionalDelimiter::Or => matches!(self, Self::Or),
            ConditionalDelimiter::Else | ConditionalDelimiter::Fi => true,
        }
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
            return Err(CommandError::InputInvariant);
        };
        let kind =
            ConditionalKind::from_primitive(primitive).ok_or(CommandError::InputInvariant)?;
        let condition = self.command.conditions.push(kind, 0);
        match kind {
            ConditionalKind::IfCase => {
                let selected = self.scan_decimal_integer()?;
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
        let next = self.get_token()?.ok_or(CommandError::InputInvariant)?;
        let Meaning::ExpandablePrimitive(primitive) = next.meaning() else {
            return Err(CommandError::InputInvariant);
        };
        let kind =
            ConditionalKind::from_primitive(primitive).ok_or(CommandError::InputInvariant)?;
        if kind == ConditionalKind::IfCase {
            return Err(CommandError::InputInvariant);
        }
        self.expand_conditional(next, true)
    }

    fn complete_boolean(
        &mut self,
        condition: ConditionId,
        result: bool,
    ) -> Result<(), CommandError> {
        if result {
            self.command
                .conditions
                .change_if_limit(condition, IfLimit::Else)
                .then_some(())
                .ok_or(CommandError::InputInvariant)
        } else {
            self.command
                .conditions
                .change_if_limit(condition, IfLimit::Fi)
                .then_some(())
                .ok_or(CommandError::InputInvariant)?;
            self.resume_after_skip(condition)
        }
    }

    fn complete_ifcase(
        &mut self,
        condition: ConditionId,
        selected: i32,
    ) -> Result<(), CommandError> {
        self.command
            .conditions
            .change_if_limit(condition, IfLimit::Or)
            .then_some(())
            .ok_or(CommandError::InputInvariant)?;
        if selected < 0 {
            self.skip_to_else_or_fi(condition)
        } else {
            self.skip_ifcase_limbs(condition, selected)
        }
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
            ConditionalKind::IfOdd => Ok(self.scan_decimal_integer()? & 1 != 0),
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
                let index = self.scan_decimal_integer()?;
                let index = u16::try_from(index).map_err(|_| CommandError::InputInvariant)?;
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
            ConditionalKind::IfEof
            | ConditionalKind::IfDefined
            | ConditionalKind::IfCsName
            | ConditionalKind::IfFontChar
            | ConditionalKind::IfInCsName
            | ConditionalKind::IfCase => {
                Err(CommandError::UnsupportedExpandablePrimitive(match kind {
                    ConditionalKind::IfEof => ExpandablePrimitive::IfEof,
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
        let first = self.get_x_token()?.ok_or(CommandError::InputInvariant)?;
        let second = self.get_x_token()?.ok_or(CommandError::InputInvariant)?;
        Ok(matches!((first.meaning(), second.meaning()),
            (Meaning::CharToken { ch: left, .. }, Meaning::CharToken { ch: right, .. }) if left == right
        ))
    }

    fn evaluate_ifcat(&mut self) -> Result<bool, CommandError> {
        let first = self.get_x_token()?.ok_or(CommandError::InputInvariant)?;
        let second = self.get_x_token()?.ok_or(CommandError::InputInvariant)?;
        Ok(matches!((first.meaning(), second.meaning()),
            (Meaning::CharToken { cat: left, .. }, Meaning::CharToken { cat: right, .. }) if left == right
        ))
    }

    fn evaluate_ifx(&mut self) -> Result<bool, CommandError> {
        let first = self.get_token()?.ok_or(CommandError::InputInvariant)?;
        let second = self.get_token()?.ok_or(CommandError::InputInvariant)?;
        Ok(first.meaning() == second.meaning())
    }

    fn evaluate_numeric_comparison(&mut self) -> Result<bool, CommandError> {
        let left = self.scan_decimal_integer()?;
        let relation = self.get_x_token()?.ok_or(CommandError::InputInvariant)?;
        let right = self.scan_decimal_integer()?;
        match relation.meaning() {
            Meaning::CharToken { ch: '<', .. } => Ok(left < right),
            Meaning::CharToken { ch: '=', .. } => Ok(left == right),
            Meaning::CharToken { ch: '>', .. } => Ok(left > right),
            _ => Err(CommandError::InputInvariant),
        }
    }

    fn evaluate_dimension_comparison(&mut self) -> Result<bool, CommandError> {
        let left = self.scan_dimension()?;
        let relation = self.get_x_token()?.ok_or(CommandError::InputInvariant)?;
        let right = self.scan_dimension()?;
        match relation.meaning() {
            Meaning::CharToken { ch: '<', .. } => Ok(left < right),
            Meaning::CharToken { ch: '=', .. } => Ok(left == right),
            Meaning::CharToken { ch: '>', .. } => Ok(left > right),
            _ => Err(CommandError::InputInvariant),
        }
    }

    fn skip_ifcase_limbs(
        &mut self,
        condition: ConditionId,
        mut remaining: i32,
    ) -> Result<(), CommandError> {
        if remaining == 0 {
            return Ok(());
        }
        loop {
            match self.pass_text(condition, ScannerWarning(0))?.delimiter {
                ConditionalDelimiter::Or => {
                    remaining -= 1;
                    if remaining == 0 {
                        return Ok(());
                    }
                }
                ConditionalDelimiter::Else => {
                    self.command
                        .conditions
                        .change_if_limit(condition, IfLimit::Fi);
                    return Ok(());
                }
                ConditionalDelimiter::Fi => {
                    self.command.conditions.pop();
                    return Ok(());
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
                    self.command.conditions.pop();
                    return Ok(());
                }
            }
        }
    }

    fn resume_after_skip(&mut self, condition: ConditionId) -> Result<(), CommandError> {
        match self.pass_text(condition, ScannerWarning(0))?.delimiter {
            ConditionalDelimiter::Else => {
                self.command
                    .conditions
                    .change_if_limit(condition, IfLimit::Fi);
                Ok(())
            }
            ConditionalDelimiter::Fi => {
                self.command.conditions.pop();
                Ok(())
            }
            ConditionalDelimiter::Or => Err(CommandError::InputInvariant),
        }
    }

    pub(crate) fn expand_conditional_delimiter(
        &mut self,
        _command: crate::CurrentCommand,
        primitive: ExpandablePrimitive,
    ) -> Result<(), CommandError> {
        let delimiter = match primitive {
            ExpandablePrimitive::Else => ConditionalDelimiter::Else,
            ExpandablePrimitive::Or => ConditionalDelimiter::Or,
            ExpandablePrimitive::Fi => ConditionalDelimiter::Fi,
            _ => return Err(CommandError::InputInvariant),
        };
        let Some(frame) = self.command.conditions.current().cloned() else {
            return Ok(());
        };
        if self
            .command
            .conditions
            .evaluating_delimiter_recovery(frame.identity, delimiter)
            .is_some()
        {
            return Err(CommandError::InputInvariant);
        }
        match delimiter {
            ConditionalDelimiter::Fi => {
                self.command.conditions.pop();
                Ok(())
            }
            ConditionalDelimiter::Else if frame.limit == IfLimit::Else => {
                self.command
                    .conditions
                    .change_if_limit(frame.identity, IfLimit::Fi);
                self.resume_after_skip(frame.identity)
            }
            ConditionalDelimiter::Else
                if frame.kind == ConditionalKind::IfCase && frame.limit == IfLimit::Or =>
            {
                self.command
                    .conditions
                    .change_if_limit(frame.identity, IfLimit::Fi);
                self.resume_after_skip(frame.identity)
            }
            ConditionalDelimiter::Or
                if frame.kind == ConditionalKind::IfCase && frame.limit == IfLimit::Or =>
            {
                self.command
                    .conditions
                    .change_if_limit(frame.identity, IfLimit::Fi);
                self.resume_after_skip(frame.identity)
            }
            _ => Ok(()),
        }
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
        let result = self.pass_text_scalar(condition);
        self.command.restore_scanner_status(prior);
        result
    }

    fn pass_text_scalar(&mut self, condition: ConditionId) -> Result<PassTextStop, CommandError> {
        let limit = self
            .command
            .conditions
            .limit(condition)
            .ok_or(CommandError::InputInvariant)?;
        debug_assert_ne!(
            limit,
            IfLimit::Evaluating,
            "pass_text follows conditional evaluation"
        );

        let mut nested_conditions = 0_u32;
        loop {
            let Some(command) = self.get_next()? else {
                return Err(CommandError::InputInvariant);
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
            if limit.accepts_skipped_delimiter(delimiter) {
                return Ok(PassTextStop {
                    delimiter,
                    nested_conditions,
                });
            }
        }
    }
}

#[cfg(test)]
mod tests;
