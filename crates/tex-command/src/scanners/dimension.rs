//! Typed TeX dimension scanning for command predicates.

use tex_state::meaning::Meaning;
use tex_state::scaled::{PhysicalUnit, Scaled, round_decimal_fraction, scaled_from_decimal_parts};

use crate::{CommandError, processor::CommandProcessor};

#[cfg(any(test, feature = "instrumentation"))]
use crate::observation::{CommandObservation, ScannerRecord};

impl CommandProcessor<'_> {
    pub(crate) fn scan_dimension(&mut self) -> Result<Scaled, CommandError> {
        let mut negative = false;
        let first = loop {
            let token = self.get_x_token()?.ok_or(CommandError::InputInvariant)?;
            match token.meaning() {
                Meaning::CharToken { ch: ' ', .. } | Meaning::CharToken { ch: '+', .. } => {}
                Meaning::CharToken { ch: '-', .. } => negative = !negative,
                _ => break token,
            }
        };
        let value = match first.meaning() {
            Meaning::DimenRegister(index) => self.state.dimen(index),
            Meaning::DimenParam(index) => self.state.dimen_param(index),
            Meaning::PageDimension(dimension) => self.state.page_dimension(dimension),
            Meaning::CharToken { ch, .. } if ch.is_ascii_digit() || ch == '.' => {
                let mut integer = String::new();
                let mut fraction = String::new();
                let mut decimal = ch == '.';
                if !decimal {
                    integer.push(ch);
                }
                loop {
                    let token = self.get_x_token()?.ok_or(CommandError::InputInvariant)?;
                    match token.meaning() {
                        Meaning::CharToken { ch, .. } if ch.is_ascii_digit() => {
                            if decimal {
                                fraction.push(ch);
                            } else {
                                integer.push(ch);
                            }
                        }
                        Meaning::CharToken { ch: '.', .. } if !decimal => decimal = true,
                        _ => {
                            self.back_input(token)?;
                            break;
                        }
                    }
                }
                let unit = self.scan_dimension_unit()?;
                let integer = integer
                    .parse::<i32>()
                    .map_err(|_| CommandError::InputInvariant)?;
                let digits = fraction.bytes().map(|byte| byte - b'0').collect::<Vec<_>>();
                scaled_from_decimal_parts(integer, round_decimal_fraction(&digits), unit)
                    .map_err(|_| CommandError::InputInvariant)?
            }
            _ => return Err(CommandError::InputInvariant),
        };
        let result = if negative {
            Scaled::from_raw(-value.raw())
        } else {
            value
        };
        #[cfg(any(test, feature = "instrumentation"))]
        self.observe(CommandObservation::Scanner(ScannerRecord {
            kind: "dimension",
            value: result.raw().to_string(),
        }));
        Ok(result)
    }

    fn scan_dimension_unit(&mut self) -> Result<PhysicalUnit, CommandError> {
        let mut name = String::with_capacity(2);
        while name.len() < 2 {
            let token = self.get_x_token()?.ok_or(CommandError::InputInvariant)?;
            match token.meaning() {
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
}
