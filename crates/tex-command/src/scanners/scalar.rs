//! Executor-facing canonical scalar scanners.

use tex_state::glue::{GlueSpec, Order};
use tex_state::ids::TokenListId;
use tex_state::meaning::{Meaning, UnexpandablePrimitive};
use tex_state::scaled::{PhysicalUnit, Scaled, round_decimal_fraction, scaled_from_decimal_parts};
use tex_state::token::{OriginId, Token};

use crate::{CommandError, CurrentCommand, processor::CommandProcessor};
#[cfg(any(test, feature = "instrumentation"))]
use crate::{CommandObservation, ScannerRecord};

/// Recovery performed by a canonical scalar scan.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScalarRecovery {
    /// The operand was present and valid.
    None,
    /// TeX's missing-number recovery supplied a zero value.
    InsertedZero,
}

/// Provenance retained for a scalar scan without exposing input frames.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScalarProvenance {
    /// Origin of the first token considered by the scan.
    pub primary: OriginId,
}

/// A typed scalar result plus canonical recovery and provenance information.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScannedScalar<T> {
    pub value: T,
    pub recovery: ScalarRecovery,
    pub provenance: ScalarProvenance,
}

/// A value accepted by TeX's internal-quantity scanner.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InternalValue {
    Integer(i32),
    Dimension(Scaled),
    Glue(GlueSpec),
    MuGlue(GlueSpec),
    Tokens {
        tokens: TokenListId,
        index: u16,
        parameter: bool,
    },
}

enum DimensionUnit {
    Physical(PhysicalUnit),
    Infinite(Order),
}

impl CommandProcessor<'_> {
    /// Scans TeX's optional signs from expanded command-owned input.
    pub fn scan_optional_sign(&mut self) -> Result<ScannedScalar<bool>, CommandError> {
        let mut negative = false;
        let mut provenance = OriginId::UNKNOWN;
        loop {
            let Some(command) = self.get_x_token()? else {
                break;
            };
            if provenance == OriginId::UNKNOWN {
                provenance = command.origin();
            }
            match command.meaning() {
                Meaning::CharToken { ch: ' ', .. } | Meaning::CharToken { ch: '+', .. } => {}
                Meaning::CharToken { ch: '-', .. } => negative = !negative,
                _ => {
                    self.retire_exhausted_backup_before_scalar_replay(command.delivery_stamp())?;
                    self.back_input(command)?;
                    break;
                }
            }
        }
        Ok(ScannedScalar {
            value: negative,
            recovery: ScalarRecovery::None,
            provenance: ScalarProvenance {
                primary: provenance,
            },
        })
    }

    /// Consumes one optional equals sign, after optional spaces.
    pub fn scan_optional_equals(&mut self) -> Result<ScannedScalar<bool>, CommandError> {
        let mut provenance = OriginId::UNKNOWN;
        loop {
            let Some(command) = self.get_x_token()? else {
                return Ok(ScannedScalar {
                    value: false,
                    recovery: ScalarRecovery::None,
                    provenance: ScalarProvenance {
                        primary: provenance,
                    },
                });
            };
            if provenance == OriginId::UNKNOWN {
                provenance = command.origin();
            }
            match command.meaning() {
                Meaning::CharToken { ch: ' ', .. } => continue,
                Meaning::CharToken { ch: '=', .. } => {
                    return Ok(ScannedScalar {
                        value: true,
                        recovery: ScalarRecovery::None,
                        provenance: ScalarProvenance {
                            primary: provenance,
                        },
                    });
                }
                _ => {
                    self.back_input(command)?;
                    return Ok(ScannedScalar {
                        value: false,
                        recovery: ScalarRecovery::None,
                        provenance: ScalarProvenance {
                            primary: provenance,
                        },
                    });
                }
            }
        }
    }

    /// Scans an optional expanded keyword. A failed match is replayed through
    /// the sole canonical raw-delivery path.
    pub fn scan_keyword(&mut self, keyword: &str) -> Result<ScannedScalar<bool>, CommandError> {
        let mut consumed = Vec::new();
        let mut provenance = OriginId::UNKNOWN;
        let first = loop {
            let Some(command) = self.get_x_token()? else {
                return Ok(ScannedScalar {
                    value: false,
                    recovery: ScalarRecovery::None,
                    provenance: ScalarProvenance {
                        primary: provenance,
                    },
                });
            };
            if provenance == OriginId::UNKNOWN {
                provenance = command.origin();
            }
            if matches!(command.meaning(), Meaning::CharToken { ch: ' ', .. }) {
                consumed.push(command);
                continue;
            }
            break command;
        };
        let mut first = Some(first);
        for expected in keyword.chars() {
            let command = if let Some(command) = first.take() {
                command
            } else {
                let Some(command) = self.get_x_token()? else {
                    self.replay_scalar_commands(consumed)?;
                    return Ok(ScannedScalar {
                        value: false,
                        recovery: ScalarRecovery::None,
                        provenance: ScalarProvenance {
                            primary: provenance,
                        },
                    });
                };
                command
            };
            if !matches!(command.meaning(), Meaning::CharToken { ch, .. } if ch.eq_ignore_ascii_case(&expected))
            {
                consumed.push(command);
                self.replay_scalar_commands(consumed)?;
                return Ok(ScannedScalar {
                    value: false,
                    recovery: ScalarRecovery::None,
                    provenance: ScalarProvenance {
                        primary: provenance,
                    },
                });
            }
            consumed.push(command);
        }
        Ok(ScannedScalar {
            value: true,
            recovery: ScalarRecovery::None,
            provenance: ScalarProvenance {
                primary: provenance,
            },
        })
    }

    /// Scans an integer or an internal integer quantity.
    pub fn scan_integer(&mut self) -> Result<ScannedScalar<i32>, CommandError> {
        let mut negative = false;
        let mut provenance = OriginId::UNKNOWN;
        let first = loop {
            let Some(command) = self.get_x_token()? else {
                return Ok(ScannedScalar {
                    value: 0,
                    recovery: ScalarRecovery::InsertedZero,
                    provenance: ScalarProvenance {
                        primary: provenance,
                    },
                });
            };
            if provenance == OriginId::UNKNOWN {
                provenance = command.origin();
            }
            match command.meaning() {
                Meaning::CharToken { ch: ' ', .. } | Meaning::CharToken { ch: '+', .. } => {}
                Meaning::CharToken { ch: '-', .. } => negative = !negative,
                _ => break command,
            }
        };
        self.complete_integer(first, negative, provenance)
    }

    fn complete_integer(
        &mut self,
        first: CurrentCommand,
        negative: bool,
        provenance: OriginId,
    ) -> Result<ScannedScalar<i32>, CommandError> {
        let value = match self.internal_value_from_command(&first)? {
            Some(InternalValue::Integer(value)) => value,
            Some(_) => {
                self.retire_exhausted_backup_before_scalar_replay(first.delivery_stamp())?;
                self.back_input(first)?;
                let scanned = ScannedScalar {
                    value: 0,
                    recovery: ScalarRecovery::InsertedZero,
                    provenance: ScalarProvenance {
                        primary: provenance,
                    },
                };
                #[cfg(any(test, feature = "instrumentation"))]
                self.observe(CommandObservation::Scanner(ScannerRecord {
                    kind: "integer",
                    value: scanned.value.to_string(),
                    tokens: None,
                }));
                return Ok(scanned);
            }
            None => match first.meaning() {
                Meaning::CharToken { ch, .. } if ch.is_ascii_digit() => {
                    self.scan_radix_tail(ch, 10)?
                }
                // TeX.web `scan_int` treats an apostrophe or double quote as
                // an octal or hexadecimal introducer. The following digits
                // still travel through `get_x_token`, so their deliveries are
                // observable before the completed scanner result.
                Meaning::CharToken { ch: '\'', .. } => self.scan_radix_tail('0', 8)?,
                Meaning::CharToken { ch: '"', .. } => self.scan_radix_tail('0', 16)?,
                // TeX's `\` character-code form consumes its following token
                // through raw delivery: that token supplies a character code,
                // rather than participating in ordinary expansion.  The
                // optional following space remains an expanded scanner token.
                Meaning::CharToken { ch: '`', .. } => self.scan_character_code()?,
                _ => {
                    self.retire_exhausted_backup_before_scalar_replay(first.delivery_stamp())?;
                    self.back_input(first)?;
                    let scanned = ScannedScalar {
                        value: 0,
                        recovery: ScalarRecovery::InsertedZero,
                        provenance: ScalarProvenance {
                            primary: provenance,
                        },
                    };
                    #[cfg(any(test, feature = "instrumentation"))]
                    self.observe(CommandObservation::Scanner(ScannerRecord {
                        kind: "integer",
                        value: scanned.value.to_string(),
                        tokens: None,
                    }));
                    return Ok(scanned);
                }
            },
        };
        let scanned = ScannedScalar {
            value: if negative {
                value.saturating_neg()
            } else {
                value
            },
            recovery: ScalarRecovery::None,
            provenance: ScalarProvenance {
                primary: provenance,
            },
        };
        #[cfg(any(test, feature = "instrumentation"))]
        self.observe(CommandObservation::Scanner(ScannerRecord {
            kind: "integer",
            value: scanned.value.to_string(),
            tokens: None,
        }));
        Ok(scanned)
    }

    /// Scans a dimension or an internal dimension quantity.
    pub fn scan_dimension(&mut self) -> Result<ScannedScalar<Scaled>, CommandError> {
        Ok(self.scan_dimension_with_order(false)?.0)
    }

    fn scan_dimension_with_order(
        &mut self,
        allow_infinite: bool,
    ) -> Result<(ScannedScalar<Scaled>, Order), CommandError> {
        // TeX82 §455's signed lookahead normally backs up its first non-sign
        // token before the integer scanner owns it. An internal dimension is
        // the exception: it goes directly to `scan_something_internal`.
        let mut negative = false;
        let mut provenance = OriginId::UNKNOWN;
        let retained_internal_dimension = loop {
            let Some(command) = self.get_x_token()? else {
                return Ok((
                    ScannedScalar {
                        value: Scaled::from_raw(0),
                        recovery: ScalarRecovery::InsertedZero,
                        provenance: ScalarProvenance {
                            primary: provenance,
                        },
                    },
                    Order::Normal,
                ));
            };
            if provenance == OriginId::UNKNOWN {
                provenance = command.origin();
            }
            match command.meaning() {
                Meaning::CharToken { ch: ' ', .. } | Meaning::CharToken { ch: '+', .. } => {}
                Meaning::CharToken { ch: '-', .. } => negative = !negative,
                Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Dimen) => {
                    break Some(command);
                }
                _ => {
                    self.retire_exhausted_backup_before_scalar_replay(command.delivery_stamp())?;
                    self.back_input(command)?;
                    break None;
                }
            }
        };
        let provenance = ScalarProvenance {
            primary: provenance,
        };
        let first = match retained_internal_dimension {
            Some(command) => command,
            None => self.get_x_token()?.ok_or(CommandError::InputInvariant)?,
        };
        let (value, order, internal_dimension) = match self.internal_value_from_command(&first)? {
            Some(InternalValue::Dimension(value)) => (value, Order::Normal, true),
            Some(_) => {
                self.back_input(first)?;
                return Ok((
                    ScannedScalar {
                        value: Scaled::from_raw(0),
                        recovery: ScalarRecovery::InsertedZero,
                        provenance,
                    },
                    Order::Normal,
                ));
            }
            None => match first.meaning() {
                Meaning::CharToken { ch, .. } if ch.is_ascii_digit() || ch == '.' => {
                    // TeX82 `scan_dimen` delegates the integral prefix to
                    // `scan_int`.  Besides sharing radix and recovery rules,
                    // that ownership is observable: `scan_int` backs up the
                    // decimal point before completing, then `scan_dimen`
                    // consumes it raw before scanning fractional digits.
                    // Keep that hand-off inside the command scanner rather
                    // than collapsing it into a private decimal parser.
                    self.last_integer_terminator = None;
                    let integer = self
                        .complete_integer(first, false, provenance.primary)?
                        .value;
                    let decimal = self
                        .last_integer_terminator
                        .as_ref()
                        .is_some_and(|command| {
                            matches!(command.meaning(), Meaning::CharToken { ch: '.', .. })
                        });
                    if decimal {
                        // The decimal point is replayed raw after `scan_int`
                        // backed it up.  A unit stays in the backed-up input
                        // for the keyword scanner instead.
                        let _ = self.get_next()?;
                    }
                    let (value, order) =
                        self.scan_decimal_dimension(integer, decimal, allow_infinite)?;
                    (value, order, false)
                }
                _ => {
                    self.back_input(first)?;
                    return Ok((
                        ScannedScalar {
                            value: Scaled::from_raw(0),
                            recovery: ScalarRecovery::InsertedZero,
                            provenance,
                        },
                        Order::Normal,
                    ));
                }
            },
        };
        if !internal_dimension {
            self.scan_optional_space()?;
        }
        let scanned = ScannedScalar {
            value: if negative {
                Scaled::from_raw(-value.raw())
            } else {
                value
            },
            recovery: ScalarRecovery::None,
            provenance,
        };
        #[cfg(any(test, feature = "instrumentation"))]
        self.observe(CommandObservation::Scanner(ScannerRecord {
            kind: "dimension",
            value: scanned.value.raw().to_string(),
            tokens: None,
        }));
        Ok((scanned, order))
    }

    /// Scans a normal or mu glue specification.
    pub fn scan_glue(&mut self, mu: bool) -> Result<ScannedScalar<GlueSpec>, CommandError> {
        // TeX82 probes an internal glue quantity before treating the input as
        // a width dimension.  An ordinary number therefore travels through
        // one canonical backup/replay cycle before `scan_dimen` owns its
        // integer prefix; collapsing the probe loses that input lifecycle and
        // also prevents a direct `\skip` RHS from being accepted as glue.
        let (mut value, mut recovery, provenance, internal_glue) = match self.get_x_token()? {
            Some(command) => match self.internal_value_from_command(&command)? {
                Some(InternalValue::Glue(value)) => (
                    value,
                    ScalarRecovery::None,
                    ScalarProvenance {
                        primary: command.origin(),
                    },
                    true,
                ),
                Some(_) | None => {
                    self.back_input(command)?;
                    let width = self.scan_dimension()?;
                    (
                        GlueSpec {
                            width: width.value,
                            ..GlueSpec::ZERO
                        },
                        width.recovery,
                        width.provenance,
                        false,
                    )
                }
            },
            None => {
                let width = self.scan_dimension()?;
                (
                    GlueSpec {
                        width: width.value,
                        ..GlueSpec::ZERO
                    },
                    width.recovery,
                    width.provenance,
                    false,
                )
            }
        };
        if !internal_glue {
            if self.scan_keyword("plus")?.value {
                let (stretch, order) = self.scan_dimension_with_order(true)?;
                value.stretch = stretch.value;
                recovery = stretch.recovery;
                value.stretch_order = order;
            }
            if self.scan_keyword("minus")?.value {
                let (shrink, order) = self.scan_dimension_with_order(true)?;
                value.shrink = shrink.value;
                recovery = shrink.recovery;
                value.shrink_order = order;
            }
        }
        let _ = mu;
        let scanned = ScannedScalar {
            value,
            recovery,
            provenance,
        };
        #[cfg(any(test, feature = "instrumentation"))]
        self.observe(CommandObservation::Scanner(ScannerRecord {
            kind: "glue",
            value: format!(
                "width={};stretch={};stretch_order={:?};shrink={};shrink_order={:?}",
                scanned.value.width.raw(),
                scanned.value.stretch.raw(),
                scanned.value.stretch_order,
                scanned.value.shrink.raw(),
                scanned.value.shrink_order,
            ),
            tokens: None,
        }));
        Ok(scanned)
    }

    /// Scans one internal value without treating other syntax as an error.
    pub fn scan_internal_value(
        &mut self,
    ) -> Result<Option<ScannedScalar<InternalValue>>, CommandError> {
        let Some(command) = self.get_x_token()? else {
            return Ok(None);
        };
        let provenance = ScalarProvenance {
            primary: command.origin(),
        };
        match self.internal_value_from_command(&command)? {
            Some(value) => Ok(Some(ScannedScalar {
                value,
                recovery: ScalarRecovery::None,
                provenance,
            })),
            None => {
                self.back_input(command)?;
                Ok(None)
            }
        }
    }

    fn scan_radix_tail(&mut self, first: char, radix: u8) -> Result<i32, CommandError> {
        let mut value = i32::from(Self::radix_digit(first).expect("radix introducer is valid"));
        loop {
            let Some(command) = self.get_x_token()? else {
                break;
            };
            match command.meaning() {
                Meaning::CharToken { ch, .. }
                    if Self::radix_digit(ch).is_some_and(|digit| digit < radix) =>
                {
                    value = value
                        .saturating_mul(i32::from(radix))
                        .saturating_add(i32::from(Self::radix_digit(ch).expect("digit checked")))
                }
                // TeX's numeric scanner absorbs one trailing space after a
                // decimal constant; replay must not manufacture a backup
                // transition before publishing the completed integer.
                Meaning::CharToken { ch: ' ', .. } => break,
                _ => {
                    self.last_integer_terminator = Some(command.copy_for_backup());
                    self.back_input(command)?;
                    break;
                }
            }
        }
        Ok(value)
    }

    fn radix_digit(ch: char) -> Option<u8> {
        match ch {
            '0'..='9' => Some(ch as u8 - b'0'),
            'a'..='f' => Some(ch as u8 - b'a' + 10),
            'A'..='F' => Some(ch as u8 - b'A' + 10),
            _ => None,
        }
    }

    fn scan_character_code(&mut self) -> Result<i32, CommandError> {
        let Some(command) = self.get_next_character_code()? else {
            return Ok(0);
        };
        // TeX82 §442 tests `cur_tok`, not `cur_cmd`: an active character is
        // represented by a control-sequence meaning, but remains a valid
        // alphabetic character constant. Likewise a one-character control
        // sequence denotes that character. Only multi-character (or null)
        // control sequences take the improper-constant recovery path.
        let value = match command.spelling().semantic_token() {
            Token::Char { ch, .. } => i32::try_from(u32::from(ch)).unwrap_or(0),
            Token::Cs(symbol) => {
                let mut name = self.state.resolve(symbol).chars();
                match (name.next(), name.next()) {
                    (Some(ch), None) => i32::try_from(u32::from(ch)).unwrap_or(0),
                    _ => {
                        self.back_input(command)?;
                        return Ok(0);
                    }
                }
            }
            _ => {
                self.back_input(command)?;
                return Ok(0);
            }
        };
        let Some(command) = self.get_x_token()? else {
            return Ok(value);
        };
        if !matches!(command.meaning(), Meaning::CharToken { ch: ' ', .. }) {
            self.back_input(command)?;
        }
        Ok(value)
    }

    fn scan_decimal_dimension(
        &mut self,
        integer: i32,
        decimal: bool,
        allow_infinite: bool,
    ) -> Result<(Scaled, Order), CommandError> {
        let mut fraction = String::new();
        if decimal {
            loop {
                let Some(command) = self.get_x_token()? else {
                    break;
                };
                match command.meaning() {
                    Meaning::CharToken { ch, .. } if ch.is_ascii_digit() => fraction.push(ch),
                    _ => {
                        self.back_input(command)?;
                        break;
                    }
                }
            }
        }
        let unit = if allow_infinite
            && !decimal
            && self
                .last_integer_terminator
                .as_ref()
                .is_some_and(|command| {
                    matches!(command.meaning(), Meaning::CharToken { ch: 'f', .. })
                }) {
            // `scan_int` has already observed and backed up the leading `f`.
            // Replay it once, then finish the `fil` suffix without routing the
            // same candidate through unrelated physical-unit keywords.
            let Some(first) = self.get_x_token()? else {
                return Err(CommandError::InputInvariant);
            };
            if !matches!(first.meaning(), Meaning::CharToken { ch: 'f', .. }) {
                return Err(CommandError::InputInvariant);
            }
            self.scan_infinite_unit_from_fil()?
        } else {
            self.scan_dimension_unit(allow_infinite)?
        };
        let digits = fraction.bytes().map(|byte| byte - b'0').collect::<Vec<_>>();
        let (unit, order) = match unit {
            DimensionUnit::Physical(unit) => (unit, Order::Normal),
            // TeX stores an infinite glue component's finite coefficient as a
            // scaled value, while its order is carried separately.
            DimensionUnit::Infinite(order) => (PhysicalUnit::Pt, order),
        };
        scaled_from_decimal_parts(integer, round_decimal_fraction(&digits), unit)
            .map(|value| (value, order))
            .map_err(|_| CommandError::InputInvariant)
    }

    fn scan_dimension_unit(&mut self, allow_infinite: bool) -> Result<DimensionUnit, CommandError> {
        // TeX82 §455 first looks for an internal dimension, then probes `em`
        // and `ex`, before accepting `true`, `pt`, or a physical unit.  Each
        // unsuccessful probe owns one `back_input` hand-off.  In particular,
        // an `in` following a fraction must be replayed through precisely the
        // internal/`em`/`ex`/`true`/`pt` probes before `scan_keyword("in")`
        // consumes its `i` and its following `n` directly.
        if allow_infinite && self.scan_keyword("fil")?.value {
            return self.scan_infinite_unit(Order::Fil);
        }
        self.probe_dimension_unit()?;
        for keyword in ["em", "ex"] {
            let _ = self.scan_keyword(keyword)?;
        }
        let _true_dimension = self.scan_keyword("true")?.value;
        if self.scan_keyword("pt")?.value {
            return Ok(DimensionUnit::Physical(PhysicalUnit::Pt));
        }
        for (keyword, unit) in [
            ("in", PhysicalUnit::In),
            ("pc", PhysicalUnit::Pc),
            ("cm", PhysicalUnit::Cm),
            ("mm", PhysicalUnit::Mm),
            ("bp", PhysicalUnit::Bp),
            ("dd", PhysicalUnit::Dd),
            ("cc", PhysicalUnit::Cc),
            ("sp", PhysicalUnit::Sp),
        ] {
            if self.scan_keyword(keyword)?.value {
                return Ok(DimensionUnit::Physical(unit));
            }
        }
        Err(CommandError::InputInvariant)
    }

    /// Performs TeX82 §455's internal-dimension unit lookahead.
    ///
    /// This scanner does not yet materialize internal units, but the failed
    /// lookahead is still a real command-owned operation: when it consumes a
    /// token replayed by the fractional scanner, TeX's `back_input` first
    /// retires that exhausted backup before installing the new one.
    fn probe_dimension_unit(&mut self) -> Result<(), CommandError> {
        let Some(command) = self.get_x_token()? else {
            return Ok(());
        };
        self.retire_exhausted_backup_before_scalar_replay(command.delivery_stamp())?;
        self.back_input(command)
    }

    fn scan_optional_space(&mut self) -> Result<(), CommandError> {
        let Some(command) = self.get_x_token()? else {
            return Ok(());
        };
        if !matches!(command.meaning(), Meaning::CharToken { ch: ' ', .. }) {
            self.retire_exhausted_backup_before_scalar_replay(command.delivery_stamp())?;
            self.back_input(command)?;
        }
        Ok(())
    }

    fn scan_infinite_unit(&mut self, mut order: Order) -> Result<DimensionUnit, CommandError> {
        loop {
            let Some(command) = self.get_x_token()? else {
                break;
            };
            match command.meaning() {
                Meaning::CharToken { ch: 'l', .. } if order != Order::Filll => {
                    order = match order {
                        Order::Normal => Order::Fil,
                        Order::Fil => Order::Fill,
                        Order::Fill => Order::Filll,
                        _ => unreachable!("infinite glue order is bounded"),
                    };
                }
                _ => {
                    self.back_input(command)?;
                    break;
                }
            }
        }
        Ok(DimensionUnit::Infinite(order))
    }

    fn scan_infinite_unit_from_fil(&mut self) -> Result<DimensionUnit, CommandError> {
        for expected in ['i', 'l'] {
            let command = self.get_x_token()?.ok_or(CommandError::InputInvariant)?;
            if !matches!(command.meaning(), Meaning::CharToken { ch, .. } if ch == expected) {
                return Err(CommandError::InputInvariant);
            }
        }
        let mut order = Order::Fil;
        loop {
            let Some(command) = self.get_x_token()? else {
                break;
            };
            match command.meaning() {
                Meaning::CharToken { ch: 'l', .. } if order != Order::Filll => {
                    order = match order {
                        Order::Fil => Order::Fill,
                        Order::Fill => Order::Filll,
                        _ => unreachable!("infinite glue order is bounded"),
                    };
                }
                Meaning::CharToken { ch: ' ', .. } => {
                    if let Some(next) = self.get_x_token()? {
                        self.back_input(next)?;
                    }
                    break;
                }
                _ => {
                    self.back_input(command)?;
                    break;
                }
            }
        }
        Ok(DimensionUnit::Infinite(order))
    }

    pub(crate) fn internal_value_from_command(
        &mut self,
        command: &CurrentCommand,
    ) -> Result<Option<InternalValue>, CommandError> {
        let value = match command.meaning() {
            // TeX82 `scan_something_internal` owns a register primitive's
            // restricted (`scan_eight_bit_int`) index scan. Keeping every
            // register family here means its index deliveries, nested integer
            // result, and internal-value observer precede the outer scalar
            // scanner's result (TeX.web `scan_something_internal`/`scan_int`).
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Count) => {
                let index = self.scan_eight_bit_register_index()?;
                let value = self.state.count(index);
                #[cfg(any(test, feature = "instrumentation"))]
                self.observe(CommandObservation::Scanner(ScannerRecord {
                    kind: "internal",
                    value: value.to_string(),
                    tokens: None,
                }));
                InternalValue::Integer(value)
            }
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Dimen) => {
                let index = self.scan_eight_bit_register_index()?;
                let value = self.state.dimen(index);
                #[cfg(any(test, feature = "instrumentation"))]
                self.observe(CommandObservation::Scanner(ScannerRecord {
                    kind: "internal",
                    value: format!("scaled:{}", value.raw()),
                    tokens: None,
                }));
                InternalValue::Dimension(value)
            }
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Skip) => {
                let index = self.scan_eight_bit_register_index()?;
                let value = self.state.glue(self.state.skip(index));
                #[cfg(any(test, feature = "instrumentation"))]
                self.observe(CommandObservation::Scanner(ScannerRecord {
                    kind: "internal",
                    value: format!(
                        "glue:width={};stretch={};stretch_order={:?};shrink={};shrink_order={:?}",
                        value.width.raw(),
                        value.stretch.raw(),
                        value.stretch_order,
                        value.shrink.raw(),
                        value.shrink_order,
                    ),
                    tokens: None,
                }));
                InternalValue::Glue(value)
            }
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Muskip) => {
                let index = self.scan_eight_bit_register_index()?;
                let value = self.state.glue(self.state.muskip(index));
                #[cfg(any(test, feature = "instrumentation"))]
                self.observe(CommandObservation::Scanner(ScannerRecord {
                    kind: "internal",
                    value: format!(
                        "glue:width={};stretch={};stretch_order={:?};shrink={};shrink_order={:?}",
                        value.width.raw(),
                        value.stretch.raw(),
                        value.stretch_order,
                        value.shrink.raw(),
                        value.shrink_order,
                    ),
                    tokens: None,
                }));
                InternalValue::MuGlue(value)
            }
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Toks) => {
                let index = self.scan_eight_bit_register_index()?;
                let tokens = self.state.toks(index);
                #[cfg(any(test, feature = "instrumentation"))]
                self.observe(CommandObservation::Scanner(ScannerRecord {
                    kind: "internal",
                    value: "tokens".into(),
                    tokens: Some(
                        self.state
                            .tokens(tokens)
                            .iter()
                            .copied()
                            .map(|token| {
                                self.observed_token(tex_state::token::TracedTokenWord::pack(
                                    token,
                                    OriginId::UNKNOWN,
                                ))
                            })
                            .collect(),
                    ),
                }));
                InternalValue::Tokens {
                    tokens,
                    index,
                    parameter: false,
                }
            }
            Meaning::CountRegister(index) => InternalValue::Integer(self.state.count(index)),
            Meaning::IntParam(index) => InternalValue::Integer(
                self.state
                    .int_param(tex_state::env::banks::IntParam::new(index)),
            ),
            Meaning::PageInteger(integer) => {
                InternalValue::Integer(self.state.page_integer(integer))
            }
            Meaning::DimenRegister(index) => InternalValue::Dimension(self.state.dimen(index)),
            Meaning::DimenParam(index) => InternalValue::Dimension(self.state.dimen_param(index)),
            Meaning::PageDimension(dimension) => {
                InternalValue::Dimension(self.state.page_dimension(dimension))
            }
            Meaning::SkipRegister(index) => {
                InternalValue::Glue(self.state.glue(self.state.skip(index)))
            }
            Meaning::MuskipRegister(index) => {
                InternalValue::MuGlue(self.state.glue(self.state.muskip(index)))
            }
            Meaning::GlueParam(index) => {
                InternalValue::Glue(self.state.glue(self.state.glue_param(index)))
            }
            Meaning::MuGlueParam(index) => {
                InternalValue::MuGlue(self.state.glue(self.state.glue_param(index)))
            }
            Meaning::ToksRegister(index) => InternalValue::Tokens {
                tokens: self.state.toks(index),
                index,
                parameter: false,
            },
            Meaning::TokParam(index) => InternalValue::Tokens {
                tokens: self
                    .state
                    .tok_param(tex_state::env::banks::TokParam::new(index)),
                index,
                parameter: true,
            },
            Meaning::InternalInteger(integer) => {
                let Some(value) = self.state.internal_integer(integer) else {
                    return Ok(None);
                };
                InternalValue::Integer(value)
            }
            _ => return Ok(None),
        };
        Ok(Some(value))
    }

    /// Scans TeX82's `scan_eight_bit_int` register index.
    ///
    /// `scan_something_internal` uses this bounded scan for `\count`,
    /// `\dimen`, `\skip`, and `\muskip`; an out-of-range value recovers as
    /// register zero rather than truncating or addressing an extended bank.
    pub(crate) fn scan_eight_bit_register_index(&mut self) -> Result<u16, CommandError> {
        let value = self.scan_integer()?.value;
        Ok(u16::try_from(value)
            .ok()
            .filter(|index| *index <= 255)
            .unwrap_or(0))
    }

    fn replay_scalar_commands(
        &mut self,
        commands: Vec<CurrentCommand>,
    ) -> Result<(), CommandError> {
        if commands.is_empty() {
            return Ok(());
        }
        if commands.len() == 1 {
            let command = commands.into_iter().next().expect("checked singleton");
            self.retire_exhausted_backup_before_scalar_replay(command.delivery_stamp())?;
            return self.back_input(command);
        }
        for command in &commands {
            self.undo_alignment_delivery(command);
        }
        self.command.push_token_level(
            crate::input::TokenPayload::BackedUp(crate::input::SharedBackedUpBuffer::new(
                commands
                    .into_iter()
                    .map(|command| crate::input::BackedUpToken {
                        spelling: command.spelling(),
                        source_range: command.source_range(),
                    })
                    .collect::<Vec<_>>(),
            )),
            crate::input::TokenBehavior::BackedUp(crate::input::BackupTreatment::Ordinary),
            crate::input::RetirementBehavior::Pop,
            crate::input::ReplayTrace::BackedUp,
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests;
