//! Executor-facing canonical scalar scanners.

use tex_state::glue::{GlueSpec, Order};
use tex_state::meaning::Meaning;
use tex_state::scaled::{PhysicalUnit, Scaled, round_decimal_fraction, scaled_from_decimal_parts};
use tex_state::token::OriginId;

use crate::{CommandError, CurrentCommand, processor::CommandProcessor};

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
                    self.replay_scalar_commands(consumed);
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
                self.replay_scalar_commands(consumed);
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
        let value = match self.internal_value_from_command(&first)? {
            Some(InternalValue::Integer(value)) => value,
            Some(_) => {
                self.back_input(first)?;
                return Ok(ScannedScalar {
                    value: 0,
                    recovery: ScalarRecovery::InsertedZero,
                    provenance: ScalarProvenance {
                        primary: provenance,
                    },
                });
            }
            None => match first.meaning() {
                Meaning::CharToken { ch, .. } if ch.is_ascii_digit() => {
                    self.scan_decimal_tail(ch)?
                }
                _ => {
                    self.back_input(first)?;
                    return Ok(ScannedScalar {
                        value: 0,
                        recovery: ScalarRecovery::InsertedZero,
                        provenance: ScalarProvenance {
                            primary: provenance,
                        },
                    });
                }
            },
        };
        Ok(ScannedScalar {
            value: if negative {
                value.saturating_neg()
            } else {
                value
            },
            recovery: ScalarRecovery::None,
            provenance: ScalarProvenance {
                primary: provenance,
            },
        })
    }

    /// Scans a dimension or an internal dimension quantity.
    pub fn scan_dimension(&mut self) -> Result<ScannedScalar<Scaled>, CommandError> {
        let sign = self.scan_optional_sign()?;
        let provenance = sign.provenance;
        let Some(first) = self.get_x_token()? else {
            return Ok(ScannedScalar {
                value: Scaled::from_raw(0),
                recovery: ScalarRecovery::InsertedZero,
                provenance,
            });
        };
        let value = match self.internal_value_from_command(&first)? {
            Some(InternalValue::Dimension(value)) => value,
            Some(_) => {
                self.back_input(first)?;
                return Ok(ScannedScalar {
                    value: Scaled::from_raw(0),
                    recovery: ScalarRecovery::InsertedZero,
                    provenance,
                });
            }
            None => match first.meaning() {
                Meaning::CharToken { ch, .. } if ch.is_ascii_digit() || ch == '.' => {
                    self.scan_decimal_dimension(ch)?
                }
                _ => {
                    self.back_input(first)?;
                    return Ok(ScannedScalar {
                        value: Scaled::from_raw(0),
                        recovery: ScalarRecovery::InsertedZero,
                        provenance,
                    });
                }
            },
        };
        Ok(ScannedScalar {
            value: if sign.value {
                Scaled::from_raw(-value.raw())
            } else {
                value
            },
            recovery: ScalarRecovery::None,
            provenance,
        })
    }

    /// Scans a normal or mu glue specification.
    pub fn scan_glue(&mut self, mu: bool) -> Result<ScannedScalar<GlueSpec>, CommandError> {
        let width = self.scan_dimension()?;
        let mut value = GlueSpec {
            width: width.value,
            ..GlueSpec::ZERO
        };
        let mut recovery = width.recovery;
        if self.scan_keyword("plus")?.value {
            let stretch = self.scan_dimension()?;
            value.stretch = stretch.value;
            recovery = stretch.recovery;
            value.stretch_order = self.scan_infinite_order()?;
        }
        if self.scan_keyword("minus")?.value {
            let shrink = self.scan_dimension()?;
            value.shrink = shrink.value;
            recovery = shrink.recovery;
            value.shrink_order = self.scan_infinite_order()?;
        }
        let _ = mu;
        Ok(ScannedScalar {
            value,
            recovery,
            provenance: width.provenance,
        })
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

    fn scan_decimal_tail(&mut self, first: char) -> Result<i32, CommandError> {
        let mut value = i32::from(first as u8 - b'0');
        loop {
            let Some(command) = self.get_x_token()? else {
                break;
            };
            match command.meaning() {
                Meaning::CharToken { ch, .. } if ch.is_ascii_digit() => {
                    value = value
                        .saturating_mul(10)
                        .saturating_add(i32::from(ch as u8 - b'0'))
                }
                _ => {
                    self.back_input(command)?;
                    break;
                }
            }
        }
        Ok(value)
    }

    fn scan_decimal_dimension(&mut self, first: char) -> Result<Scaled, CommandError> {
        let mut integer = String::new();
        let mut fraction = String::new();
        let mut decimal = first == '.';
        if !decimal {
            integer.push(first);
        }
        loop {
            let Some(command) = self.get_x_token()? else {
                break;
            };
            match command.meaning() {
                Meaning::CharToken { ch, .. } if ch.is_ascii_digit() => {
                    if decimal {
                        fraction.push(ch)
                    } else {
                        integer.push(ch)
                    }
                }
                Meaning::CharToken { ch: '.', .. } if !decimal => decimal = true,
                _ => {
                    self.back_input(command)?;
                    break;
                }
            }
        }
        let unit = self.scan_dimension_unit()?;
        let integer = integer.parse::<i32>().unwrap_or(0);
        let digits = fraction.bytes().map(|byte| byte - b'0').collect::<Vec<_>>();
        scaled_from_decimal_parts(integer, round_decimal_fraction(&digits), unit)
            .map_err(|_| CommandError::InputInvariant)
    }

    fn scan_dimension_unit(&mut self) -> Result<PhysicalUnit, CommandError> {
        let mut name = String::with_capacity(2);
        while name.len() < 2 {
            let command = self.get_x_token()?.ok_or(CommandError::InputInvariant)?;
            match command.meaning() {
                Meaning::CharToken { ch, .. } if ch.is_ascii_alphabetic() => {
                    name.push(ch.to_ascii_lowercase())
                }
                Meaning::CharToken { ch: ' ', .. } if name.is_empty() => {}
                _ => return Err(CommandError::InputInvariant),
            }
        }
        match name.as_str() {
            "sp" => Ok(PhysicalUnit::Sp),
            "pt" => Ok(PhysicalUnit::Pt),
            "in" => Ok(PhysicalUnit::In),
            "pc" => Ok(PhysicalUnit::Pc),
            "cm" => Ok(PhysicalUnit::Cm),
            "mm" => Ok(PhysicalUnit::Mm),
            "bp" => Ok(PhysicalUnit::Bp),
            "dd" => Ok(PhysicalUnit::Dd),
            "cc" => Ok(PhysicalUnit::Cc),
            _ => Err(CommandError::InputInvariant),
        }
    }

    fn scan_infinite_order(&mut self) -> Result<Order, CommandError> {
        for (name, order) in [
            ("filll", Order::Filll),
            ("fill", Order::Fill),
            ("fil", Order::Fil),
        ] {
            if self.scan_keyword(name)?.value {
                return Ok(order);
            }
        }
        Ok(Order::Normal)
    }

    fn internal_value_from_command(
        &mut self,
        command: &CurrentCommand,
    ) -> Result<Option<InternalValue>, CommandError> {
        let value = match command.meaning() {
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
                InternalValue::Glue(self.state.glue(self.state.muskip(index)))
            }
            Meaning::GlueParam(index) => {
                InternalValue::Glue(self.state.glue(self.state.glue_param(index)))
            }
            Meaning::MuGlueParam(index) => {
                InternalValue::Glue(self.state.glue(self.state.glue_param(index)))
            }
            _ => return Ok(None),
        };
        Ok(Some(value))
    }

    fn replay_scalar_commands(&mut self, commands: Vec<CurrentCommand>) {
        if commands.is_empty() {
            return;
        }
        for command in &commands {
            self.undo_alignment_delivery(command);
        }
        self.command.push_token_level(
            crate::input::TokenPayload::Transient(crate::input::SharedTokenBuffer::new(
                commands
                    .into_iter()
                    .map(|command| command.spelling())
                    .collect::<Vec<_>>(),
            )),
            crate::input::TokenBehavior::BackedUp(crate::input::BackupTreatment::Ordinary),
            crate::input::RetirementBehavior::Pop,
            crate::input::ReplayTrace::BackedUp,
        );
    }
}

#[cfg(test)]
mod tests;
