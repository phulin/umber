//! Executor-facing canonical scalar scanners.

use tex_state::BoxDimension;
use tex_state::glue::{GlueSpec, Order};
use tex_state::ids::{FontId, TokenListId};
use tex_state::interner::Symbol;
use tex_state::meaning::{Meaning, UnexpandablePrimitive};
use tex_state::scaled::{
    PhysicalUnit, Scaled, nx_plus_y, round_decimal_fraction, scale_true_dimension_parts,
    scaled_from_decimal_parts, xn_over_d,
};
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
    /// TeX82's `ident_val`: a font's stable control-sequence identity.
    Font(Symbol),
    Tokens {
        tokens: TokenListId,
        index: u16,
        parameter: bool,
    },
}

impl InternalValue {
    /// TeX82 §410's `cur_val_level` for this value.
    ///
    /// The six levels are totally ordered (`int_val` < `dimen_val` <
    /// `glue_val` < `mu_val` < `ident_val` < `tok_val`), which is what makes
    /// §413's `while cur_val_level>level` coercion loop well defined.
    const fn level(self) -> InternalLevel {
        match self {
            Self::Integer(_) => InternalLevel::Integer,
            Self::Dimension(_) => InternalLevel::Dimension,
            Self::Glue(_) => InternalLevel::Glue,
            Self::MuGlue(_) => InternalLevel::MuGlue,
            Self::Font(_) => InternalLevel::Font,
            Self::Tokens { .. } => InternalLevel::Tokens,
        }
    }
}

/// TeX82 §410's six internal-quantity levels, in their defining order.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum InternalLevel {
    /// `int_val`.
    Integer,
    /// `dimen_val`.
    Dimension,
    /// `glue_val`.
    Glue,
    /// `mu_val`.
    MuGlue,
    /// `ident_val`.
    Font,
    /// `tok_val`.
    Tokens,
}

/// TeX82 §430's "Negate all three glue components of `cur_val`".
///
/// A leading sign in front of an internal glue or mu-glue quantity negates
/// the whole specification, not just its width: `\skip0=-\skip1` keeps the
/// stretch and shrink and negates them too. The orders of infinity are
/// magnitudes and are unaffected.
fn negated_glue(glue: GlueSpec) -> GlueSpec {
    GlueSpec {
        width: Scaled::from_raw(-glue.width.raw()),
        stretch: Scaled::from_raw(-glue.stretch.raw()),
        shrink: Scaled::from_raw(-glue.shrink.raw()),
        ..glue
    }
}

/// TeX82 §454's `while scan_keyword("l") do ... incr(cur_order)`.
///
/// Every `l` after `fil` is consumed, and one past `filll` is an error that
/// tex.web reports and then discards ("A specification like `filllll` ...
/// will lead to two error messages"). Leaving the excess `l` in the input
/// instead would leak it into later parsing as literal text.
const fn raise_infinite_order(order: Order) -> Order {
    match order {
        Order::Normal => Order::Fil,
        Order::Fil => Order::Fill,
        Order::Fill | Order::Filll => Order::Filll,
    }
}

/// Recognizes TeX82 §448's decimal point.
///
/// `scan_dimen` aliases `continental_point_token` to `point_token` twice, so
/// a comma introduces a decimal fraction exactly like a period: `3,5pt` is
/// `3.5pt`, and a leading `,5pt` is `0.5pt`.
const fn is_decimal_point(ch: char) -> bool {
    matches!(ch, '.' | ',')
}

/// The outcome of TeX82 §449's internal-quantity fetch inside `scan_dimen`.
enum InternalDimension {
    /// The quantity reached the requested dimension level: `goto attach_sign`
    /// with no unit scan and no trailing optional space.
    Complete(Scaled),
    /// The quantity settled at `int_val`: it is the numeric prefix of an
    /// ordinary units scan.
    Prefix(i32),
}

enum DimensionUnit {
    Physical(PhysicalUnit),
    Internal(Scaled),
    Infinite(Order),
    Mu,
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
            // TeX82 §440's `scan_int` fetches at `int_val`, so §413's §429
            // loop lowers every numeric level to an integer: a dimension keeps
            // its scaled representation, and glue or mu glue first becomes its
            // width. Plain's `\ht\z@` relies on the dimension step because
            // `\z@` is a dimension register, not a numeric literal; `\ifnum
            // \parskip>0` and `\count0=\skip3` rely on the glue step.
            Some(value) => match self.coerce_internal_value(value, InternalLevel::Integer) {
                Some(InternalValue::Integer(value)) => value,
                Some(_) => unreachable!("coercion to int_val always yields an integer"),
                // TeX82 §415: a font identifier or token list requested below
                // `tok_val` is not coerced at all. TeX prints "Missing
                // number, treated as zero", backs the operand up, and yields
                // zero.
                None => {
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
        Ok(self.scan_dimension_with_order(false, false)?.0)
    }

    /// Scans a dimension requiring TeX's `mu` unit. This is public only at
    /// the command-scanner boundary; replay receives the completed scaled
    /// value and never performs unit recognition itself.
    pub fn scan_mu_dimension(&mut self) -> Result<ScannedScalar<Scaled>, CommandError> {
        Ok(self.scan_dimension_with_order(false, true)?.0)
    }

    fn scan_dimension_with_order(
        &mut self,
        allow_infinite: bool,
        mu: bool,
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
            None => self.get_x_token()?.ok_or(CommandError::input_invariant())?,
        };
        let (value, order, internal_dimension) = match self.internal_value_from_command(&first)? {
            // TeX82 §449's "Fetch an internal dimension and goto attach_sign,
            // or fetch an internal integer". A quantity that ends up at the
            // requested dimension level is the whole answer and skips both the
            // unit scan and its optional space; one that ends up an integer
            // becomes the numeric prefix of an ordinary units scan (`\dimen0=
            // \count5 pt`).
            Some(value) => match self.fetch_internal_dimension(value, mu) {
                Some(InternalDimension::Complete(value)) => (value, Order::Normal, true),
                Some(InternalDimension::Prefix(integer)) => {
                    self.last_integer_terminator = None;
                    let (value, order, flip) =
                        self.scan_units_after_integer(integer, false, allow_infinite, mu)?;
                    negative ^= flip;
                    (value, order, false)
                }
                None => {
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
            None => match first.meaning() {
                Meaning::CharToken { ch, .. } if ch.is_ascii_digit() || is_decimal_point(ch) => {
                    // TeX82 `scan_dimen` delegates the integral prefix to
                    // `scan_int`.  Besides sharing radix and recovery rules,
                    // that ownership is observable: `scan_int` backs up the
                    // decimal point before completing, then `scan_dimen`
                    // consumes it raw before scanning fractional digits.
                    // Keep that hand-off inside the command scanner rather
                    // than collapsing it into a private decimal parser.
                    self.last_integer_terminator = None;
                    let leading_decimal = matches!(
                        first.meaning(),
                        Meaning::CharToken { ch, .. } if is_decimal_point(ch)
                    );
                    let integer = self
                        .complete_integer(first, false, provenance.primary)?
                        .value;
                    let decimal = leading_decimal
                        || self
                            .last_integer_terminator
                            .as_ref()
                            .is_some_and(|command| {
                                matches!(
                                    command.meaning(),
                                    Meaning::CharToken { ch, .. } if is_decimal_point(ch)
                                )
                            });
                    if decimal {
                        // The decimal point is replayed raw after `scan_int`
                        // backed it up.  A unit stays in the backed-up input
                        // for the keyword scanner instead.
                        let _ = self.get_next()?;
                    }
                    let (value, order, flip) =
                        self.scan_units_after_integer(integer, decimal, allow_infinite, mu)?;
                    negative ^= flip;
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

    /// TeX82 §449's "Fetch an internal dimension ..., or fetch an internal
    /// integer", including §450's "Coerce glue to a dimension".
    ///
    /// `None` is §415's missing-number case: a font identifier or token list
    /// where a dimension was requested. The caller owns backing that operand
    /// up and reporting the inserted zero.
    fn fetch_internal_dimension(
        &mut self,
        value: InternalValue,
        mu: bool,
    ) -> Option<InternalDimension> {
        if !mu {
            // `scan_something_internal(dimen_val,false)`: §429 lowers glue and
            // mu glue to a width, so anything that is still a dimension after
            // the cascade is the complete answer.
            return match self.coerce_internal_value(value, InternalLevel::Dimension)? {
                InternalValue::Dimension(value) => Some(InternalDimension::Complete(value)),
                InternalValue::Integer(value) => Some(InternalDimension::Prefix(value)),
                _ => unreachable!("coercion to dimen_val yields a dimension or an integer"),
            };
        }
        // `scan_something_internal(mu_val,false)`: `mu_val` is the highest
        // numeric level, so §429's loop never runs and the level reached is
        // exactly the quantity's own.
        match value {
            // §450 replaces the specification by its width while leaving
            // `cur_val_level` at `mu_val`, so `goto attach_sign` takes it as
            // the whole mu dimension. This is what `\mkern\thinmuskip` reads.
            InternalValue::MuGlue(glue) => Some(InternalDimension::Complete(glue.width)),
            // §450 coerces this width too, but `cur_val_level` stays
            // `glue_val`, which is neither `mu_val` nor `int_val`: TeX reports
            // `mu_error` and continues with the width as the units prefix.
            InternalValue::Glue(glue) => {
                self.mu_error();
                Some(InternalDimension::Prefix(glue.width.raw()))
            }
            // `dimen_val` is below `glue_val`, so §450 does not apply; the
            // level test still reports `mu_error` and keeps the value.
            InternalValue::Dimension(value) => {
                self.mu_error();
                Some(InternalDimension::Prefix(value.raw()))
            }
            InternalValue::Integer(value) => Some(InternalDimension::Prefix(value)),
            InternalValue::Font(_) | InternalValue::Tokens { .. } => None,
        }
    }

    /// TeX82 §448's shared tail: normalize the integer part's sign, then scan
    /// units for `cur_val + f/2^16`.
    ///
    /// The returned flag is §448's `if cur_val<0 then negative:=not negative`.
    /// Only an internal integer prefix (`\dimen0=\count5 pt`, or `\mkern`'s
    /// mu-level mismatch recovery) can deliver a negative value here, and the
    /// fixed-point unit conversion is defined only for a nonnegative operand.
    fn scan_units_after_integer(
        &mut self,
        integer: i32,
        decimal: bool,
        allow_infinite: bool,
        mu: bool,
    ) -> Result<(Scaled, Order, bool), CommandError> {
        let flip = integer < 0;
        let integer = if flip {
            integer.saturating_neg()
        } else {
            integer
        };
        let (value, order) = self.scan_decimal_dimension(integer, decimal, allow_infinite, mu)?;
        Ok((value, order, flip))
    }

    /// TeX82 §448's `scan_dimen(mu,false,true)` shortcut: the integer part is
    /// already in hand, so only the units remain to be scanned.
    fn scan_dimension_shortcut(&mut self, integer: i32, mu: bool) -> Result<Scaled, CommandError> {
        self.last_integer_terminator = None;
        let (value, _order, flip) = self.scan_units_after_integer(integer, false, false, mu)?;
        self.scan_optional_space()?;
        Ok(if flip {
            Scaled::from_raw(-value.raw())
        } else {
            value
        })
    }

    /// Scans a normal or mu glue specification.
    pub fn scan_glue(&mut self, mu: bool) -> Result<ScannedScalar<GlueSpec>, CommandError> {
        // TeX82 §461's `<Get the next non-blank non-sign token>`: `scan_glue`
        // owns its own leading signs so that §430 can negate an internal
        // glue's three components as a unit (`\skip0=-\skip1`). Routing a
        // signed internal glue through the width-only dimension scanner
        // instead would drop its stretch and shrink.
        let mut negative = false;
        let mut provenance = OriginId::UNKNOWN;
        let first = loop {
            let Some(command) = self.get_x_token()? else {
                break None;
            };
            if provenance == OriginId::UNKNOWN {
                provenance = command.origin();
            }
            match command.meaning() {
                Meaning::CharToken { ch: ' ', .. } | Meaning::CharToken { ch: '+', .. } => {}
                Meaning::CharToken { ch: '-', .. } => negative = !negative,
                _ => break Some(command),
            }
        };
        let provenance = ScalarProvenance {
            primary: provenance,
        };
        let level = if mu {
            InternalLevel::MuGlue
        } else {
            InternalLevel::Glue
        };
        // TeX82 probes an internal glue quantity before treating the input as
        // a width dimension.  An ordinary number therefore travels through
        // one canonical backup/replay cycle before `scan_dimen` owns its
        // integer prefix; collapsing the probe loses that input lifecycle and
        // also prevents a direct `\skip` RHS from being accepted as glue.
        let (mut value, mut recovery, provenance, internal_glue) = match first {
            Some(command) => match self.internal_value_from_command(&command)? {
                // §461: `if cur_val_level>=glue_val then begin if
                // cur_val_level<>level then mu_error; return end`. The
                // specification is the complete answer, so no `plus`/`minus`
                // components follow it, and §430 negates all three of its
                // components together when a leading sign asked for it.
                Some(internal @ (InternalValue::Glue(_) | InternalValue::MuGlue(_))) => {
                    if internal.level() != level {
                        self.mu_error();
                    }
                    let (InternalValue::Glue(glue) | InternalValue::MuGlue(glue)) = internal else {
                        unreachable!("outer pattern restricts the value to a glue specification")
                    };
                    (
                        if negative { negated_glue(glue) } else { glue },
                        ScalarRecovery::None,
                        provenance,
                        true,
                    )
                }
                // TeX82's `scan_glue` accepts an internal dimension as the
                // width of an ordinary glue specification, negated by §430
                // because `dimen_val` is below `glue_val`.  In particular,
                // `\ht<box>` has already consumed its bounded box index;
                // backing up the primitive here would both replay an
                // incomplete internal value and use a stale delivery proof.
                // See TeX.web §461 (`scan_glue`). A mu-level request reports
                // `mu_error` first and keeps the dimension.
                Some(InternalValue::Dimension(width)) => {
                    if mu {
                        self.mu_error();
                    }
                    (
                        GlueSpec {
                            width: if negative {
                                Scaled::from_raw(-width.raw())
                            } else {
                                width
                            },
                            ..GlueSpec::ZERO
                        },
                        ScalarRecovery::None,
                        provenance,
                        false,
                    )
                }
                // §461: `if cur_val_level=int_val then scan_dimen(mu,false,
                // true)`. §430 has already negated the integer, which is the
                // numeric prefix of a units-only dimension scan.
                Some(InternalValue::Integer(integer)) => {
                    let integer = if negative {
                        integer.saturating_neg()
                    } else {
                        integer
                    };
                    let width = self.scan_dimension_shortcut(integer, mu)?;
                    (
                        GlueSpec {
                            width,
                            ..GlueSpec::ZERO
                        },
                        ScalarRecovery::None,
                        provenance,
                        false,
                    )
                }
                // §461's non-internal branch: `back_input; scan_dimen(mu,
                // false,false); if negative then negate(cur_val)`. §415's
                // missing-number recovery for a font identifier or token list
                // reaches the same zero through one more probe cycle.
                Some(InternalValue::Font(_) | InternalValue::Tokens { .. }) | None => {
                    self.back_input(command)?;
                    let width = self.scan_dimension_with_order(false, mu)?.0;
                    (
                        GlueSpec {
                            width: if negative {
                                Scaled::from_raw(-width.value.raw())
                            } else {
                                width.value
                            },
                            ..GlueSpec::ZERO
                        },
                        width.recovery,
                        width.provenance,
                        false,
                    )
                }
            },
            None => {
                let width = self.scan_dimension_with_order(false, mu)?.0;
                (
                    GlueSpec {
                        width: if negative {
                            Scaled::from_raw(-width.value.raw())
                        } else {
                            width.value
                        },
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
                let (stretch, order) = self.scan_dimension_with_order(true, mu)?;
                value.stretch = stretch.value;
                recovery = stretch.recovery;
                value.stretch_order = order;
            }
            if self.scan_keyword("minus")?.value {
                let (shrink, order) = self.scan_dimension_with_order(true, mu)?;
                value.shrink = shrink.value;
                recovery = shrink.recovery;
                value.shrink_order = order;
            }
        }
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
        mu: bool,
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
        let (unit, true_dimension) = if allow_infinite
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
                return Err(CommandError::input_invariant());
            };
            if !matches!(first.meaning(), Meaning::CharToken { ch: 'f', .. }) {
                return Err(CommandError::input_invariant());
            }
            (self.scan_infinite_unit_from_fil()?, false)
        } else {
            self.scan_dimension_unit(allow_infinite, mu)?
        };
        let digits = fraction.bytes().map(|byte| byte - b'0').collect::<Vec<_>>();
        let fraction = round_decimal_fraction(&digits);
        // TeX82 §457's "Adjust for the magnification ratio", applied between
        // recognizing `true` and converting the physical unit: `prepare_mag`
        // validates and freezes `\mag`, and a magnification other than 1000
        // divides the scanned quantity by `mag/1000` so that one `true` unit
        // still measures one physical unit on the magnified page. Its
        // diagnostic (an illegal or job-incompatible `\mag`) needs a canonical
        // scanner diagnostic channel that does not exist yet.
        // Every arithmetic failure below is tex.web's `arith_error`, which
        // §460's `attach_sign` turns into "Dimension too large" -- a reported,
        // recoverable error that clamps to `max_dimen` and keeps going, not an
        // abandoned job.
        let mut arith_error = false;
        let (integer, fraction) = if true_dimension {
            let (mag, _diagnostic) = self.state.prepare_mag();
            scale_true_dimension_parts(integer, fraction, mag).unwrap_or_else(|_| {
                arith_error = true;
                (integer, fraction)
            })
        } else {
            (integer, fraction)
        };
        let (value, order) = match unit {
            // §455's `nx_plus_y(save_cur_val,v,xn_over_d(v,f,@'200000))`.
            DimensionUnit::Internal(unit) => (
                nx_plus_y(
                    integer,
                    unit,
                    xn_over_d(unit, fraction, Scaled::UNITY)
                        .map_or(Scaled::MAX_DIMEN, |result| result.quotient),
                )
                .unwrap_or(Scaled::MAX_DIMEN),
                Order::Normal,
            ),
            DimensionUnit::Physical(unit) => (
                scaled_from_decimal_parts(integer, fraction, unit).unwrap_or(Scaled::MAX_DIMEN),
                Order::Normal,
            ),
            // TeX stores an infinite glue component's finite coefficient as a
            // scaled value, while its order is carried separately.
            DimensionUnit::Infinite(order) => (
                scaled_from_decimal_parts(integer, fraction, PhysicalUnit::Pt)
                    .unwrap_or(Scaled::MAX_DIMEN),
                order,
            ),
            // A mu unit has the same scaled representation as a point; its
            // distinctness belongs to the surrounding glue family.
            DimensionUnit::Mu => (
                scaled_from_decimal_parts(integer, fraction, PhysicalUnit::Pt)
                    .unwrap_or(Scaled::MAX_DIMEN),
                Order::Normal,
            ),
        };
        Ok((
            if arith_error {
                Scaled::MAX_DIMEN
            } else {
                value
            },
            order,
        ))
    }

    /// TeX82 §453's "Scan units and set `cur_val`".
    ///
    /// The returned flag reports that §457's `true` prefix was recognized, so
    /// the caller applies "Adjust for the magnification ratio" to the integer
    /// and fractional parts before converting the physical unit.
    fn scan_dimension_unit(
        &mut self,
        allow_infinite: bool,
        mu: bool,
    ) -> Result<(DimensionUnit, bool), CommandError> {
        // TeX82 §455 first looks for an internal dimension, then probes `em`
        // and `ex`, before accepting `true`, `pt`, or a physical unit.  Each
        // unsuccessful probe owns one `back_input` hand-off.  In particular,
        // an `in` following a fraction must be replayed through precisely the
        // internal/`em`/`ex`/`true`/`pt` probes before `scan_keyword("in")`
        // consumes its `i` and its following `n` directly.
        if allow_infinite && self.scan_keyword("fil")?.value {
            return Ok((self.scan_infinite_unit(Order::Fil)?, false));
        }
        if let Some(unit) = self.probe_dimension_unit(mu)? {
            return Ok((DimensionUnit::Internal(unit), false));
        }
        // §455 recognizes `em` and `ex` before the physical units, and skips
        // both entirely when a mu dimension is required (`if mu then goto
        // not_found`).  They are current-font parameters 6 (quad) and 5
        // (x-height), respectively; keep the successful keyword result so the
        // shared internal-unit fixed-point path scales both whole and
        // fractional dimensions.
        if !mu {
            for (keyword, parameter) in [("em", 6), ("ex", 5)] {
                if self.scan_keyword(keyword)?.value {
                    return Ok((
                        DimensionUnit::Internal(self.state.current_font_parameter(parameter)),
                        false,
                    ));
                }
            }
        }
        // §456: a mu dimension admits only the `mu` unit, and `true` is never
        // recognized in a mu context. A missing `mu` is "Illegal unit of
        // measure (mu inserted)": TeX reports it, keeps the scanned quantity
        // as mu, and leaves the offending text for the caller to re-read.
        if mu {
            let _ = self.scan_keyword("mu")?;
            return Ok((DimensionUnit::Mu, false));
        }
        let true_dimension = self.scan_keyword("true")?.value;
        if self.scan_keyword("pt")?.value {
            return Ok((DimensionUnit::Physical(PhysicalUnit::Pt), true_dimension));
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
                return Ok((DimensionUnit::Physical(unit), true_dimension));
            }
        }
        // §459's "Complain about unknown unit": TeX reports "Illegal unit of
        // measure (pt inserted)", assumes `pt`, and finishes the job that a
        // hard scanner failure here would abandon.
        Ok((DimensionUnit::Physical(PhysicalUnit::Pt), true_dimension))
    }

    /// Performs TeX82 §455's internal-dimension unit lookahead.
    ///
    /// This scanner does not yet materialize internal units, but the failed
    /// lookahead is still a real command-owned operation: when it consumes a
    /// token replayed by the fractional scanner, TeX's `back_input` first
    /// retires that exhausted backup before installing the new one.
    fn probe_dimension_unit(&mut self, mu: bool) -> Result<Option<Scaled>, CommandError> {
        let Some(command) = self.get_x_token()? else {
            return Ok(None);
        };
        let unit = match self.internal_value_from_command(&command)? {
            Some(InternalValue::Dimension(value)) if !mu => Some(value),
            Some(InternalValue::MuGlue(value)) if mu => Some(value.width),
            _ => None,
        };
        if unit.is_none() {
            self.retire_exhausted_backup_before_scalar_replay(command.delivery_stamp())?;
            self.back_input(command)?;
        }
        Ok(unit)
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
                Meaning::CharToken { ch: 'l', .. } => order = raise_infinite_order(order),
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
            let command = self.get_x_token()?.ok_or(CommandError::input_invariant())?;
            if !matches!(command.meaning(), Meaning::CharToken { ch, .. } if ch == expected) {
                return Err(CommandError::input_invariant());
            }
        }
        let mut order = Order::Fil;
        loop {
            let Some(command) = self.get_x_token()? else {
                break;
            };
            match command.meaning() {
                Meaning::CharToken { ch: 'l', .. } => order = raise_infinite_order(order),
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

    /// Runs TeX82 §413's `while cur_val_level>level do <Convert cur_val to a
    /// lower level>` cascade over one fetched internal quantity.
    ///
    /// §429 defines a single downward step: a `glue_val` becomes its `width`,
    /// a `mu_val` reports [`Self::mu_error`] and keeps the same value while
    /// dropping to `glue_val`, and a `dimen_val` becomes an `int_val` holding
    /// the identical scaled representation. Repeating that step is what makes
    /// `\count0=\skip3`, `\dimen0=\parskip`, and `\hsize=\baselineskip` read
    /// the register's width instead of silently scanning as zero.
    ///
    /// `ident_val` and `tok_val` never enter the loop in tex.web: §415 has
    /// already replaced a font identifier or token list requested below
    /// `tok_val` with a backed-up zero. `None` reports that same case to the
    /// caller, which owns the backup and the missing-number recovery.
    fn coerce_internal_value(
        &mut self,
        mut value: InternalValue,
        level: InternalLevel,
    ) -> Option<InternalValue> {
        while value.level() > level {
            value = match value {
                InternalValue::MuGlue(glue) => {
                    self.mu_error();
                    InternalValue::Glue(glue)
                }
                InternalValue::Glue(glue) => InternalValue::Dimension(glue.width),
                InternalValue::Dimension(dimension) => InternalValue::Integer(dimension.raw()),
                InternalValue::Font(_) | InternalValue::Tokens { .. } => return None,
                InternalValue::Integer(_) => {
                    unreachable!("int_val is the lowest level, so it never exceeds a target level")
                }
            };
        }
        Some(value)
    }

    /// TeX82 §408's `mu_error`: "Incompatible glue units" -- mu and non-mu
    /// quantities were mixed, and TeX assumes `1mu=1pt` and continues.
    ///
    /// The recovery is the observable behavior and is implemented by every
    /// caller. The accompanying terminal/log text is not: `tex-command` has
    /// no diagnostic channel yet (no canonical scanner prints anything), so
    /// the message is tracked separately rather than half-routed here.
    fn mu_error(&mut self) {
        let _ = self;
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
            // TeX82 §424's `scan_something_internal` treats `\wd`, `\ht`,
            // and `\dp` as dimensions. Their register selectors use the
            // same bounded scan as the classical register primitives, and a
            // void box supplies the zero dimension.
            Meaning::UnexpandablePrimitive(
                primitive @ (UnexpandablePrimitive::Wd
                | UnexpandablePrimitive::Ht
                | UnexpandablePrimitive::Dp),
            ) => {
                let index = self.scan_eight_bit_register_index()?;
                let dimension = match primitive {
                    UnexpandablePrimitive::Wd => BoxDimension::Width,
                    UnexpandablePrimitive::Ht => BoxDimension::Height,
                    UnexpandablePrimitive::Dp => BoxDimension::Depth,
                    _ => unreachable!("outer match restricts primitive"),
                };
                InternalValue::Dimension(
                    self.state
                        .box_dimension(index, dimension)
                        .unwrap_or_else(|| Scaled::from_raw(0)),
                )
            }
            // TeX82 §424's `scan_something_internal` reaches `assign_font_dimen`
            // through §8548's "Fetch a font dimension": `find_font_dimen(false)`
            // scans the parameter number, then the font selector, and reads the
            // named font's dimension (not just the current font, unlike
            // `current_font_parameter`).
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::FontDimen) => {
                let number = self.scan_integer()?.value;
                let font = self.scan_font_selector()?;
                let number = u32::try_from(number).unwrap_or(0);
                let value = self.state.font_dimen(font, number);
                #[cfg(any(test, feature = "instrumentation"))]
                self.observe(CommandObservation::Scanner(ScannerRecord {
                    kind: "internal",
                    value: format!("scaled:{}", value.raw()),
                    tokens: None,
                }));
                InternalValue::Dimension(value)
            }
            // TeX82 §414's "Fetch a character code from some table":
            // `scan_char_num` selects the entry and every code table reads at
            // `int_val`. These six primitives were wired for assignment only,
            // so `\the\catcode`\A` failed outright while `\ifnum\catcode`\A
            // =13` -- the standard active-character and verbatim catcode
            // probes -- silently compared against zero.
            Meaning::UnexpandablePrimitive(
                primitive @ (UnexpandablePrimitive::CatCode
                | UnexpandablePrimitive::LcCode
                | UnexpandablePrimitive::UcCode
                | UnexpandablePrimitive::SfCode
                | UnexpandablePrimitive::MathCode
                | UnexpandablePrimitive::DelCode),
            ) => {
                let character = self.scan_character_number()?;
                let value = match primitive {
                    UnexpandablePrimitive::CatCode => {
                        i32::from(self.state.catcode(character) as u8)
                    }
                    UnexpandablePrimitive::LcCode => {
                        i32::try_from(self.state.lccode(character)).unwrap_or(0)
                    }
                    UnexpandablePrimitive::UcCode => {
                        i32::try_from(self.state.uccode(character)).unwrap_or(0)
                    }
                    UnexpandablePrimitive::SfCode => i32::from(self.state.sfcode(character)),
                    UnexpandablePrimitive::MathCode => {
                        i32::try_from(self.state.mathcode(character)).unwrap_or(0)
                    }
                    UnexpandablePrimitive::DelCode => self.state.delcode(character),
                    _ => unreachable!("outer match restricts primitive to the code tables"),
                };
                #[cfg(any(test, feature = "instrumentation"))]
                self.observe(CommandObservation::Scanner(ScannerRecord {
                    kind: "internal",
                    value: value.to_string(),
                    tokens: None,
                }));
                InternalValue::Integer(value)
            }
            // TeX82 §423's "Fetch the par_shape size": `\parshape` reads the
            // number of lines in the current shape, or zero when none is set.
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::ParShape) => {
                InternalValue::Integer(i32::try_from(self.state.paragraph_shape_len()).unwrap_or(0))
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
            // `space_factor` is owned by the executor's active horizontal
            // list, rather than durable command state.  The bounded host
            // capability is refreshed before each command operation, so an
            // expanded definition can still scan `\the\spacefactor` through
            // the ordinary internal-value path.
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::SpaceFactor) => {
                let Some(value) = self.host.space_factor() else {
                    return Ok(None);
                };
                InternalValue::Integer(value)
            }
            // TeX82 §424's "Fetch an item in the current node, if
            // appropriate" (`last_item`): `\lastpenalty`, `\lastkern`, and
            // `\lastskip` read the current list's tail node, or -- in the
            // outer vertical list, whose tail moves to the page immediately
            // -- the page builder's own `last_penalty`/`last_kern`/
            // `last_glue` memo (§996). Like `space_factor` above, this is
            // executor/page-owned state projected through the bounded host
            // capability rather than durable command state. When the tail
            // matches none of the three tracked shapes (empty list,
            // character, or any other node type), tex.web leaves `cur_val`
            // at the zero it initialized for the requested level; `None`
            // here plays that same role for every one of the three
            // primitives.
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::LastPenalty) => {
                InternalValue::Integer(match self.host.last_node() {
                    Some(crate::LastNodeItem::Penalty(value)) => value,
                    _ => 0,
                })
            }
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::LastKern) => {
                InternalValue::Dimension(match self.host.last_node() {
                    Some(crate::LastNodeItem::Kern(value)) => value,
                    _ => Scaled::from_raw(0),
                })
            }
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::LastSkip) => {
                match self.host.last_node() {
                    Some(crate::LastNodeItem::Glue(value)) => InternalValue::Glue(value),
                    Some(crate::LastNodeItem::MuGlue(value)) => InternalValue::MuGlue(value),
                    _ => InternalValue::Glue(GlueSpec::ZERO),
                }
            }
            // TeX82 §424's `scan_something_internal` groups `char_given` and
            // `math_given` under one case: `scanned_result(cur_chr)(int_val)`.
            // A `\chardef` or `\mathchardef` constant scans as its stored
            // code, exactly like an internal integer. Plain TeX relies on the
            // former for `\catcode` assignments such as `\catcode`\^^L=\active`,
            // and on the latter for `\@M` (`\mathchardef\@M=10000`), which
            // `\break`/`\eject` scan as `\penalty-\@M`; treating either as a
            // missing number silently produces zero instead of the stored
            // code.
            Meaning::CharGiven(character) => InternalValue::Integer(
                i32::try_from(u32::from(character)).expect("characters fit in i32"),
            ),
            Meaning::MathCharGiven(value) => InternalValue::Integer(i32::from(value)),
            // TeX82 §424 represents `set_font`, `def_font`, and `def_family`
            // as ident_val at the token-list level. Preserve the control
            // sequence identity instead of rendering a font number or name.
            Meaning::Font(font) => self.font_identity(font),
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Font) => {
                self.font_identity(self.state.current_font())
            }
            Meaning::UnexpandablePrimitive(
                primitive @ (UnexpandablePrimitive::TextFont
                | UnexpandablePrimitive::ScriptFont
                | UnexpandablePrimitive::ScriptScriptFont),
            ) => {
                let size = match primitive {
                    UnexpandablePrimitive::TextFont => crate::MathFamilySize::Text,
                    UnexpandablePrimitive::ScriptFont => crate::MathFamilySize::Script,
                    UnexpandablePrimitive::ScriptScriptFont => crate::MathFamilySize::ScriptScript,
                    _ => unreachable!("font-family primitive is exhaustive"),
                };
                let family = self.scan_math_family(size)?;
                self.font_identity(self.state.math_family_font(
                    match family.size {
                        crate::MathFamilySize::Text => tex_state::math::MathFontSize::Text,
                        crate::MathFamilySize::Script => tex_state::math::MathFontSize::Script,
                        crate::MathFamilySize::ScriptScript => {
                            tex_state::math::MathFontSize::ScriptScript
                        }
                    },
                    family.family,
                ))
            }
            _ => return Ok(None),
        };
        Ok(Some(value))
    }

    fn font_identity(&self, font: FontId) -> InternalValue {
        InternalValue::Font(
            self.state
                .font_identifier_symbol(font)
                .expect("TeX font identifiers have a control-sequence identity"),
        )
    }

    /// Scans TeX82's `scan_eight_bit_int` register index.
    ///
    /// `scan_something_internal` uses this bounded scan for `\count`,
    /// `\dimen`, `\skip`, and `\muskip`; an out-of-range value recovers as
    /// register zero rather than truncating or addressing an extended bank.
    /// Scans one bounded classical register selector for an assignment.
    ///
    /// The recovery is part of TeX82's `scan_eight_bit_int`: values outside
    /// the classical range select register zero after the integer scanner has
    /// completed its normal command-owned delivery and backup lifecycle.
    /// Scans TeX82's `scan_char_num` character selector.
    ///
    /// tex.web bounds the scanned integer to a character code and recovers
    /// from anything outside that range by selecting character zero, exactly
    /// as `scan_eight_bit_int` recovers to register zero. Umber's character
    /// domain is the Unicode scalar range, so a non-scalar value takes the
    /// same recovery.
    fn scan_character_number(&mut self) -> Result<char, CommandError> {
        let value = self.scan_integer()?.value;
        Ok(u32::try_from(value)
            .ok()
            .and_then(char::from_u32)
            .unwrap_or('\0'))
    }

    pub fn scan_eight_bit_register_index(&mut self) -> Result<u16, CommandError> {
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
                        source_provenance: command.source_provenance(),
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
