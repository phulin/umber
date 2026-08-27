//! TeX82's restricted integer classes (§433-§437).
//!
//! tex.web declares five scanners under "Declare procedures that scan
//! restricted classes of integers", and all five have exactly one shape:
//!
//! ```text
//! scan_int;
//! if (cur_val<0)or(cur_val>MAXIMUM) then
//!   begin print_err(MESSAGE); help2(FIRST)(SECOND);
//!   int_error(cur_val); cur_val:=0;
//!   end;
//! ```
//!
//! They differ only in the upper bound and the diagnostic text, so this
//! module carries the enumeration once and every restricted scan in the
//! command core routes through it. Clamping is part of the *scan*, not of the
//! command that consumes the result: `cur_val` is already zero by the time
//! `shorthand_def`, `def_code`, `set_box`, or a math noad reads it, which is
//! what makes the recovered value -- and every observation derived from it --
//! agree with the reference engine.

use crate::profile::{CharacterMode, CommandProfile};
use crate::scanners::scalar::ScalarProvenance;
use crate::{CommandError, processor::CommandProcessor};

const RESTRICTED_INTEGER_DIAGNOSTIC: u64 = 0x7265_7374_0000_0433;

/// One of TeX82's five restricted integer classes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RestrictedIntegerClass {
    /// §433's `scan_eight_bit_int`: a classical register selector.
    EightBit,
    /// e-TeX 2.6 `etex.ch`'s `scan_register_num`: an extended register or
    /// mark-class selector.
    Register,
    /// §434's `scan_char_num`: a character code.
    CharacterCode,
    /// §435's `scan_four_bit_int`: a math family or an input/output stream.
    FourBit,
    /// §436's `scan_fifteen_bit_int`: a math character code.
    FifteenBit,
    /// §437's `scan_twenty_seven_bit_int`: a delimiter code.
    TwentySevenBit,
}

impl RestrictedIntegerClass {
    /// The `print_err` text tex.web reports for an out-of-range value.
    pub const fn message(self) -> &'static str {
        match self {
            Self::EightBit => "Bad register code",
            Self::Register => "Bad register code",
            Self::CharacterCode => "Bad character code",
            Self::FourBit => "Bad number",
            Self::FifteenBit => "Bad mathchar",
            Self::TwentySevenBit => "Bad delimiter code",
        }
    }

    /// The first `help2` line, which names the accepted range.
    pub const fn help(self) -> &'static str {
        match self {
            Self::EightBit => "A register number must be between 0 and 255.",
            Self::Register => "A register number must be between 0 and 32767.",
            Self::CharacterCode => "A character number must be between 0 and 255.",
            Self::FourBit => "Since I expected to read a number between 0 and 15,",
            Self::FifteenBit => "A mathchar number must be between 0 and 32767.",
            Self::TwentySevenBit => "A numeric delimiter code must be between 0 and 2^{27}-1.",
        }
    }

    pub const fn help_lines(self) -> &'static [&'static str] {
        match self {
            Self::EightBit => &[
                "A register number must be between 0 and 255.",
                "I changed this one to zero.",
            ],
            Self::Register => &[
                "A register number must be between 0 and 32767.",
                "I changed this one to zero.",
            ],
            Self::CharacterCode => &[
                "A character number must be between 0 and 255.",
                "I changed this one to zero.",
            ],
            Self::FourBit => &[
                "Since I expected to read a number between 0 and 15,",
                "I changed this one to zero.",
            ],
            Self::FifteenBit => &[
                "A mathchar number must be between 0 and 32767.",
                "I changed this one to zero.",
            ],
            Self::TwentySevenBit => &[
                "A numeric delimiter code must be between 0 and 2^{27}-1.",
                "I changed this one to zero.",
            ],
        }
    }

    /// Whether a scanned integer lies in this class's accepted range.
    ///
    /// Only §434 depends on the job's character mode: TeX82's character
    /// domain is `0..=255`, while Umber's Unicode profile widens the same
    /// bound to the Unicode scalar values. Every other class is a fixed
    /// numeric range in both profiles.
    pub fn accepts(self, profile: CommandProfile, value: i32) -> bool {
        match self {
            Self::EightBit => (0..=255).contains(&value),
            Self::Register => {
                profile.capabilities().supports_etex() && (0..=32_767).contains(&value)
            }
            Self::CharacterCode => match profile.character_mode() {
                CharacterMode::EightBitExact => (0..=255).contains(&value),
                CharacterMode::UnicodeExtended => {
                    u32::try_from(value).ok().and_then(char::from_u32).is_some()
                }
            },
            Self::FourBit => (0..=15).contains(&value),
            Self::FifteenBit => (0..=0x7fff).contains(&value),
            Self::TwentySevenBit => (0..=0x07ff_ffff).contains(&value),
        }
    }
}

/// The completed result of a restricted integer scan.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RestrictedInteger {
    /// `cur_val` after §433-§437's recovery: the scanned value, or zero.
    pub value: i32,
    /// The unrecovered `scan_int` result, which `int_error` reports.
    pub scanned: i32,
    /// Whether recovery replaced an out-of-range value with zero.
    pub recovered: bool,
    pub provenance: ScalarProvenance,
}

impl<G> CommandProcessor<'_, '_, G> {
    /// Performs TeX82 §433-§437's restricted integer scan.
    ///
    /// The ordinary `scan_int` runs first and completes its normal delivery
    /// and backup lifecycle; only then is the result range-checked and, if
    /// necessary, replaced by zero.
    pub(crate) fn finish_restricted_integer(
        &mut self,
        class: RestrictedIntegerClass,
        scanned: crate::ScannedScalar<i32>,
    ) -> Result<RestrictedInteger, CommandError> {
        let accepted = class.accepts(self.command.profile(), scanned.value);
        let recovered = !accepted;
        if recovered {
            // §433-§437 report from inside the scan, before the command that
            // asked for the register ever runs. With no earlier detached
            // output, reporting synchronously keeps this ahead of §362's `)`
            // when the last input line is consumed. A preceding trace instead
            // makes the owned report cross admission so the executor can
            // publish that trace first without exposing World here.
            let context = self.command.output_open_context(self.state);
            if self.has_pending_diagnostic_effects()
                || !self.command.semantic_diagnostics.is_empty()
                || self.command.expanding_deferred_write()
            {
                self.command.semantic_diagnostics.push(
                    crate::CommandSemanticDiagnostic::Recoverable {
                        identity: RESTRICTED_INTEGER_DIAGNOSTIC,
                        runaway: None,
                        message: class.message().into(),
                        help: class.help_lines(),
                        context,
                        integer_error: Some(scanned.value),
                    },
                );
                return Ok(RestrictedInteger {
                    value: 0,
                    scanned: scanned.value,
                    recovered,
                    provenance: scanned.provenance,
                });
            }
            let mut report = self.state.print_err(class.message());
            report.help(class.help_lines()).context(context);
            // §81's `jump_out` never returns to the interrupted scan.
            report.int_error(scanned.value).jump_out()?;
        }
        Ok(RestrictedInteger {
            value: if accepted { scanned.value } else { 0 },
            scanned: scanned.value,
            recovered,
            provenance: scanned.provenance,
        })
    }

    pub fn scan_restricted_integer_retained(
        &mut self,
        class: RestrictedIntegerClass,
    ) -> crate::RetainedScalarScan<G, RestrictedInteger> {
        match self.scan_integer_retained() {
            crate::RetainedScalarScan::Complete(scanned) => {
                match self.finish_restricted_integer(class, scanned) {
                    Ok(value) => crate::RetainedScalarScan::Complete(value),
                    Err(error) => crate::RetainedScalarScan::Failed(error),
                }
            }
            crate::RetainedScalarScan::Suspended { error, child } => {
                crate::RetainedScalarScan::Suspended { error, child }
            }
            crate::RetainedScalarScan::Failed(error) => crate::RetainedScalarScan::Failed(error),
        }
    }
}
