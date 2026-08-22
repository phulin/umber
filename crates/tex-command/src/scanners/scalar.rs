//! Executor-facing canonical scalar scanners.

use tex_state::env::banks::DimenParam;
use tex_state::glue::{GlueSpec, Order};
use tex_state::ids::FontId;
use tex_state::interner::Symbol;
use tex_state::meaning::{InternalInteger, Meaning, ResolvedMeaning, UnexpandablePrimitive};
use tex_state::scaled::{
    PhysicalUnit, Scaled, nx_plus_y, round_decimal_fraction, scale_true_dimension_parts,
    scaled_from_decimal_parts, xn_over_d,
};
use tex_state::token::{Catcode, OriginId, Token};

use tex_state::{BoxDimension, PenaltyArrayKind, PrepareMagDiagnostic};

use crate::observation::canonical_names::glue_order_name;
use crate::scanners::RestrictedIntegerClass;
use crate::{
    CommandError, CurrentCommand,
    processor::{CommandProcessor, meaning_text},
};
use crate::{CommandObservation, ObservationValue, ScannerRecord};

const IMPROPER_AUXILIARY_DIAGNOSTIC: u64 = 0x6175_785f_0000_0418;
const IMPROPER_AUXILIARY_HELP: &[&str] = &[
    "You can refer to \\spacefactor only in horizontal mode;",
    "you can refer to \\prevdepth only in vertical mode; and",
    "neither of these is meaningful inside \\write. So",
    "I'm forgetting what you said and using zero instead.",
];

/// Inline capacity for TeX/e-TeX/pdfTeX's current keyword vocabulary.
///
/// The longest production keyword is 13 characters. `scan_keyword` remains
/// open to callers with longer strings and moves to `spill` when necessary.
const KEYWORD_PREFIX_INLINE_CAPACITY: usize = 13;

/// TeX82 §§440--445 scalar state whose next expanded token crossed an
/// immutable host boundary.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum PendingIntegerScan {
    Leading {
        negative: bool,
        provenance: OriginId,
    },
    Radix {
        negative: bool,
        provenance: OriginId,
        radix: u8,
        value: i32,
        vacuous: bool,
        overflowed: bool,
    },
    CharacterOptionalSpace {
        negative: bool,
        provenance: OriginId,
        value: i32,
    },
}

struct MatchedKeywordPrefix<G> {
    inline: [Option<CurrentCommand<G>>; KEYWORD_PREFIX_INLINE_CAPACITY],
    len: usize,
    spill: Option<Vec<CurrentCommand<G>>>,
}

impl<G> MatchedKeywordPrefix<G> {
    fn new() -> Self {
        Self {
            inline: std::array::from_fn(|_| None),
            len: 0,
            spill: None,
        }
    }

    fn is_empty(&self) -> bool {
        self.len == 0
    }

    fn push(&mut self, command: CurrentCommand<G>) {
        if let Some(spill) = &mut self.spill {
            spill.push(command);
        } else if self.len < KEYWORD_PREFIX_INLINE_CAPACITY {
            self.inline[self.len] = Some(command);
        } else {
            let mut spill = Vec::with_capacity(KEYWORD_PREFIX_INLINE_CAPACITY * 2);
            spill.extend(self.inline.iter_mut().filter_map(Option::take));
            spill.push(command);
            self.spill = Some(spill);
        }
        self.len += 1;
    }

    fn into_vec(mut self) -> Vec<CurrentCommand<G>> {
        self.spill.take().unwrap_or_else(|| {
            self.inline
                .iter_mut()
                .take(self.len)
                .filter_map(Option::take)
                .collect()
        })
    }
}

fn observed_glue_value(value: &GlueSpec) -> ObservationValue {
    ObservationValue::Glue {
        width: i64::from(value.width.raw()),
        stretch: i64::from(value.stretch.raw()),
        stretch_order: glue_order_name(value.stretch_order),
        shrink: i64::from(value.shrink.raw()),
        shrink_order: glue_order_name(value.shrink_order),
    }
}

fn pdf_font_code_table(primitive: UnexpandablePrimitive) -> tex_state::PdfFontCode {
    match primitive {
        UnexpandablePrimitive::PdfLpCode => tex_state::PdfFontCode::Lp,
        UnexpandablePrimitive::PdfRpCode => tex_state::PdfFontCode::Rp,
        UnexpandablePrimitive::PdfEfCode => tex_state::PdfFontCode::Ef,
        UnexpandablePrimitive::PdfTagCode => tex_state::PdfFontCode::Tag,
        UnexpandablePrimitive::PdfKnbsCode => tex_state::PdfFontCode::Knbs,
        UnexpandablePrimitive::PdfStbsCode => tex_state::PdfFontCode::Stbs,
        UnexpandablePrimitive::PdfShbsCode => tex_state::PdfFontCode::Shbs,
        UnexpandablePrimitive::PdfKnbcCode => tex_state::PdfFontCode::Knbc,
        UnexpandablePrimitive::PdfKnacCode => tex_state::PdfFontCode::Knac,
        _ => unreachable!("caller restricts pdfTeX font-code primitives"),
    }
}

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
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InternalValue {
    Integer(i32),
    Dimension(Scaled),
    Glue(GlueSpec),
    MuGlue(GlueSpec),
    /// TeX82's `ident_val`: a font's stable control-sequence identity.
    Font(Symbol),
    /// TeX82's `tok_val`. §466 copies the register's or parameter's list, so
    /// which slot it came from is not part of the value: `\\the` installs the
    /// copy through §467's `ins_list` either way.
    Tokens {
        tokens: crate::AttemptTokenListId,
    },
}

impl InternalValue {
    /// TeX82 §410's `cur_val_level` for this value.
    ///
    /// The six levels are totally ordered (`int_val` < `dimen_val` <
    /// `glue_val` < `mu_val` < `ident_val` < `tok_val`), which is what makes
    /// §413's `while cur_val_level>level` coercion loop well defined.
    const fn level(&self) -> InternalLevel {
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

/// Returns the static command code used by scalar scanners.
///
/// Macros are outside the scalar-internal command ranges, so a scalar scan
/// treats a generation-branded macro exactly like any other non-operand.
const fn scalar_meaning<G>(meaning: ResolvedMeaning<G>) -> Meaning {
    match meaning {
        ResolvedMeaning::Static(meaning) => meaning,
        ResolvedMeaning::Macro { .. } => Meaning::Undefined,
    }
}

/// The outcome of one TeX82 §413 `scan_something_internal` call.
///
/// §413 distinguishes an internal result from a token that never entered its
/// case table. §416 recovery is itself an ordinary committed internal result.
#[derive(Clone, Debug, Eq, PartialEq)]
enum InternalScan {
    /// The command is not an internal quantity: `cur_cmd` is outside §413's
    /// `min_internal..max_internal` range, so TeX never enters §413 at all and
    /// the caller owns the ordinary-syntax path for the token it still holds.
    NotInternal,
    /// §413's single exit: the value at the requested level, after §429's
    /// lowering cascade and §430's negation.
    Value(InternalValue),
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

/// TeX82 §430's `if negative then ... negate(cur_val)`.
///
/// §430 states its own precondition: "If `negative` is true, `cur_val_level`
/// is known to be `<=mu_val`." Only `scan_glue` passes `negative`, and §461
/// gets there only from a leading sign in front of a numeric quantity.
fn negated_internal_value(value: InternalValue) -> InternalValue {
    match value {
        InternalValue::Integer(value) => InternalValue::Integer(value.saturating_neg()),
        InternalValue::Dimension(value) => InternalValue::Dimension(Scaled::from_raw(-value.raw())),
        InternalValue::Glue(glue) => InternalValue::Glue(negated_glue(glue)),
        InternalValue::MuGlue(glue) => InternalValue::MuGlue(negated_glue(glue)),
        InternalValue::Font(_) | InternalValue::Tokens { .. } => {
            unreachable!("TeX82 §430 negates only levels at or below mu_val")
        }
    }
}

/// TeX82 §413's `toks_register, assign_toks, def_family, set_font, def_font`
/// case: the five command codes routed to §415.
///
/// §415 is guarded on `level=tok_val`, and it is the only case in §413's table
/// that is, so the guard has to be answered from the command alone -- before
/// the fetch scans a register index, a family index, or a font identifier.
const fn is_token_list_or_font_identifier(meaning: Meaning) -> bool {
    matches!(
        meaning,
        // `assign_toks`: `\output`, `\everypar`, ... and `\toksdef` names.
        Meaning::TokParam(_) | Meaning::ToksRegister(_)
            // `set_font`: `\font`-defined identifiers and `\nullfont`.
            | Meaning::Font(_)
            | Meaning::UnexpandablePrimitive(
                // `toks_register`: `\toks`.
                UnexpandablePrimitive::Toks
                // `def_font`: `\font`.
                | UnexpandablePrimitive::Font
                // `def_family`: the three math size banks.
                | UnexpandablePrimitive::TextFont
                | UnexpandablePrimitive::ScriptFont
                | UnexpandablePrimitive::ScriptScriptFont,
            )
    )
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

/// TeX82 §440's initial `radix:=0`: no numeric constant was scanned, so no
/// decimal fraction may follow.
const NO_RADIX: u8 = 0;

/// TeX82 §444's `radix:=10`, the only base a decimal fraction may follow.
const DECIMAL_RADIX: u8 = 10;

/// Recognizes TeX82 §448's `point_token` and `continental_point_token`.
///
/// §438 defines both as `other_token` plus a character, and §289 defines
/// `other_token` as `2^8*other_char`, so this is a test on the whole token,
/// not on its character alone: only a category-12 `.` or `,` introduces a
/// decimal fraction. (§445 defines `zero_token`, `A_token`, and
/// `other_A_token`, not these two.) `scan_dimen` aliases
/// `continental_point_token` to `point_token` twice, so a comma behaves
/// exactly like a period: `3,5pt` is `3.5pt`, and a leading `,5pt` is
/// `0.5pt`.
fn is_point_token<G>(command: &CurrentCommand<G>) -> bool {
    matches!(
        scalar_meaning(command.meaning()),
        Meaning::CharToken {
            ch: '.' | ',',
            cat: Catcode::Other,
        }
    )
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

/// A completed TeX82 §453 "Scan units and set `cur_val`" step.
struct ScannedUnits {
    value: Scaled,
    order: Order,
    /// TeX82 §460's `arith_error`, retained until `attach_sign` after
    /// §448 has scanned its trailing optional space.
    arith_error: bool,
    /// TeX82 §455's `found:` exit ends in `goto attach_sign`, which bypasses
    /// `scan_dimen`'s trailing `<Scan an optional space>`. Both internal-unit
    /// paths take it; every other unit falls through `attach_fraction`/`done`
    /// into that optional space.
    attach_sign: bool,
}

impl<G> CommandProcessor<'_, '_, G> {
    /// Consumes TeX82 §405's other-category optional equals sign, after spaces.
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
            match scalar_meaning(command.meaning()) {
                Meaning::CharToken {
                    ch: ' ',
                    cat: Catcode::Space,
                } => continue,
                // §405 compares `cur_tok` with `other_token + '='`, not the
                // character code alone. Every other-category variant is an
                // operand and follows the canonical backup path below.
                Meaning::CharToken {
                    ch: '=',
                    cat: Catcode::Other,
                } => {
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

    /// TeX82 §407's `scan_keyword`, which tex.web writes as
    ///
    /// ```text
    /// p:=backup_head; link(p):=null; k:=str_start[s];
    /// while k<str_start[s+1] do
    ///   begin get_x_token;
    ///   if (cur_cs=0)and((cur_chr=so(str_pool[k]))or
    ///                    (cur_chr=so(str_pool[k])-"a"+"A")) then
    ///     begin store_new_token(cur_tok); incr(k);
    ///     end
    ///   else if (cur_cmd<>spacer)or(p<>backup_head) then
    ///     begin back_input;
    ///     if p<>backup_head then back_list(link(backup_head));
    ///     scan_keyword:=false; return;
    ///     end;
    ///   end;
    /// flush_list(link(backup_head)); scan_keyword:=true;
    /// ```
    ///
    /// Three properties of that loop are load-bearing and none of them is a
    /// special case.
    ///
    /// - The failed match restores the input as **two** levels, not one:
    ///   §325's `back_input` for the offending token, then §323's `back_list`
    ///   for the matched prefix pushed on top of it. Both pushes are
    ///   observable, and only the first carries a recovery record.
    /// - A spacer read while nothing has matched yet (`p=backup_head`) takes
    ///   neither branch: it is consumed, discarded for good, and `k` does not
    ///   advance. `scan_keyword` therefore skips leading spaces rather than
    ///   restoring them, and a spacer *after* a partial match is an ordinary
    ///   mismatch.
    /// - `cur_cs=0` restricts a match to a character token. A control
    ///   sequence `\let` to a character (or an active character) has the same
    ///   `cur_cmd`/`cur_chr` and still cannot spell a keyword letter.
    ///
    /// The comparison itself is on `cur_chr` alone, so a keyword letter
    /// matches under any category code, and `cur_chr-"a"+"A"` accepts the
    /// uppercase form of tex.web's all-lowercase keywords.
    pub fn scan_keyword(&mut self, keyword: &str) -> Result<ScannedScalar<bool>, CommandError> {
        // `link(backup_head)`: the tokens matched so far, in delivery order.
        let mut matched = MatchedKeywordPrefix::new();
        let mut provenance = OriginId::UNKNOWN;
        let mut letters = keyword.chars().peekable();
        while let Some(&letter) = letters.peek() {
            let Some(command) = self.get_x_token()? else {
                // tex.web cannot reach this: `get_x_token` always yields, and
                // exhausted input is `\\end`'s business. Restore the prefix
                // the same way a mismatch would and report no keyword.
                if !matched.is_empty() {
                    self.back_matched_keyword_prefix(matched.into_vec());
                }
                return Ok(Self::keyword_result(false, provenance));
            };
            if provenance == OriginId::UNKNOWN {
                provenance = command.origin();
            }
            if command.control_sequence().is_none()
                && matches!(
                    scalar_meaning(command.meaning()),
                    Meaning::CharToken { ch, .. } if ch.eq_ignore_ascii_case(&letter)
                )
            {
                matched.push(command);
                letters.next();
            } else if matched.is_empty()
                && matches!(
                    scalar_meaning(command.meaning()),
                    Meaning::CharToken {
                        cat: Catcode::Space,
                        ..
                    }
                )
            {
                // `(cur_cmd<>spacer)or(p<>backup_head)` is false: §407 drops
                // the space and rereads without advancing `k`.
            } else {
                self.back_input(command)?;
                if !matched.is_empty() {
                    self.back_matched_keyword_prefix(matched.into_vec());
                }
                return Ok(Self::keyword_result(false, provenance));
            }
        }
        // `flush_list(link(backup_head))`: a complete match consumes its
        // tokens outright.
        Ok(Self::keyword_result(true, provenance))
    }

    /// §407's `back_list(link(backup_head))`, over commands rather than raw
    /// tokens so the replayed prefix keeps each delivery's exact spelling and
    /// source provenance.
    fn back_matched_keyword_prefix(&mut self, matched: Vec<CurrentCommand<G>>) {
        self.back_list(
            matched
                .into_iter()
                .map(|command| crate::input::BackedUpToken {
                    spelling: command.spelling(),
                    source_provenance: command.source_provenance(),
                })
                .collect(),
        );
    }

    const fn keyword_result(value: bool, primary: OriginId) -> ScannedScalar<bool> {
        ScannedScalar {
            value,
            recovery: ScalarRecovery::None,
            provenance: ScalarProvenance { primary },
        }
    }

    /// Scans an integer or an internal integer quantity.
    pub fn scan_integer(&mut self) -> Result<ScannedScalar<i32>, CommandError> {
        self.scan_integer_with_resource_continuation(false)
    }

    /// Scans the operand of an expandable integer conversion whose opener is
    /// retained by the expanded-delivery machine across immutable host input.
    pub(crate) fn scan_expanded_integer(&mut self) -> Result<ScannedScalar<i32>, CommandError> {
        self.scan_integer_with_resource_continuation(true)
    }

    fn scan_integer_with_resource_continuation(
        &mut self,
        retain_continuation: bool,
    ) -> Result<ScannedScalar<i32>, CommandError> {
        let pending = retain_continuation
            .then(|| self.command.pending_integer_scans.pop())
            .flatten();
        if let Some(PendingIntegerScan::Radix {
            negative,
            provenance,
            radix,
            value,
            vacuous,
            overflowed,
        }) = pending
        {
            let (value, vacuous) = self.scan_radix_tail_from(
                value,
                vacuous,
                overflowed,
                radix,
                Some((negative, provenance)),
            )?;
            if vacuous {
                self.missing_number_error()?;
                return Ok(self.inserted_zero_integer(provenance));
            }
            return Ok(self.finish_integer(value, negative, provenance, ScalarRecovery::None));
        }
        if let Some(PendingIntegerScan::CharacterOptionalSpace {
            negative,
            provenance,
            value,
        }) = pending
        {
            if let Err(error) = self.scan_optional_space() {
                if error.is_resource_suspension() {
                    self.command.pending_integer_scans.push(
                        PendingIntegerScan::CharacterOptionalSpace {
                            negative,
                            provenance,
                            value,
                        },
                    );
                }
                return Err(error);
            }
            return Ok(self.finish_integer(value, negative, provenance, ScalarRecovery::None));
        }
        let (mut negative, mut provenance) = match pending {
            Some(PendingIntegerScan::Leading {
                negative,
                provenance,
            }) => (negative, provenance),
            None => (false, OriginId::UNKNOWN),
            Some(
                PendingIntegerScan::Radix { .. }
                | PendingIntegerScan::CharacterOptionalSpace { .. },
            ) => unreachable!(),
        };
        let first = loop {
            let command = match self.get_x_token() {
                Ok(command) => command,
                Err(error) => {
                    if retain_continuation && error.is_resource_suspension() {
                        self.command
                            .pending_integer_scans
                            .push(PendingIntegerScan::Leading {
                                negative,
                                provenance,
                            });
                    }
                    return Err(error);
                }
            };
            let Some(command) = command else {
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
            match scalar_meaning(command.meaning()) {
                Meaning::CharToken { ch: ' ', .. } | Meaning::CharToken { ch: '+', .. } => {}
                Meaning::CharToken { ch: '-', .. } => negative = !negative,
                _ => break command,
            }
        };
        Ok(self
            .complete_integer(first, negative, provenance, retain_continuation)?
            .0)
    }

    /// TeX82 §440's `scan_int` body, from the token its
    /// `<Get the next non-blank non-sign token>` left in hand.
    ///
    /// The second result is §440's `radix` global, which `scan_int`
    /// initializes to zero and only §444's "Scan a numeric constant" sets:
    /// 10 for a decimal constant (including §444's `vacuous` recovery, which
    /// runs after `radix:=10`), 8 after `'`, and 16 after `"`. It stays zero
    /// for an internal quantity and for §442's alphabetic character code.
    /// §448 needs it because a decimal fraction may follow only a decimal
    /// constant.
    fn complete_integer(
        &mut self,
        first: CurrentCommand<G>,
        negative: bool,
        provenance: OriginId,
        retain_continuation: bool,
    ) -> Result<(ScannedScalar<i32>, u8), CommandError> {
        // TeX82 §440's `scan_int` calls `scan_something_internal(int_val,
        // false)`, so §413's §429 loop lowers every numeric level to an
        // integer: a dimension keeps its scaled representation, and glue or mu
        // glue first becomes its width. Plain's `\ht\z@` relies on the
        // dimension step because `\z@` is a dimension register, not a numeric
        // literal; `\ifnum\parskip>0` and `\count0=\skip3` rely on the glue
        // step.
        let (value, radix, recovery) =
            match self.scan_something_internal(&first, InternalLevel::Integer, false)? {
                InternalScan::Value(InternalValue::Integer(value)) => {
                    (value, NO_RADIX, ScalarRecovery::None)
                }
                InternalScan::Value(_) => {
                    unreachable!("TeX82 §429 lowers an int_val request to an integer")
                }
                InternalScan::NotInternal => match scalar_meaning(first.meaning()) {
                    Meaning::CharToken {
                        ch,
                        cat: Catcode::Other,
                    } if ch.is_ascii_digit() => (
                        self.scan_radix_tail(
                            Some(ch as u8 - b'0'),
                            DECIMAL_RADIX,
                            retain_continuation.then_some((negative, provenance)),
                        )?
                        .0,
                        DECIMAL_RADIX,
                        ScalarRecovery::None,
                    ),
                    // TeX.web `scan_int` treats an apostrophe or double quote as
                    // an octal or hexadecimal introducer. The following digits
                    // still travel through `get_x_token`, so their deliveries are
                    // observable before the completed scanner result.
                    Meaning::CharToken {
                        ch: '\'',
                        cat: Catcode::Other,
                    } => {
                        let (value, vacuous) = self.scan_radix_tail(
                            None,
                            8,
                            retain_continuation.then_some((negative, provenance)),
                        )?;
                        if vacuous {
                            self.missing_number_error()?;
                            return Ok((self.inserted_zero_integer(provenance), 8));
                        }
                        (value, 8, ScalarRecovery::None)
                    }
                    Meaning::CharToken {
                        ch: '"',
                        cat: Catcode::Other,
                    } => {
                        let (value, vacuous) = self.scan_radix_tail(
                            None,
                            16,
                            retain_continuation.then_some((negative, provenance)),
                        )?;
                        if vacuous {
                            self.missing_number_error()?;
                            return Ok((self.inserted_zero_integer(provenance), 16));
                        }
                        (value, 16, ScalarRecovery::None)
                    }
                    // TeX's `\` character-code form consumes its following token
                    // through raw delivery: that token supplies a character code,
                    // rather than participating in ordinary expansion.  The
                    // optional following space remains an expanded scanner token.
                    // §442 never enters §444, so `radix` stays zero.
                    Meaning::CharToken {
                        ch: '`',
                        cat: Catcode::Other,
                    } => {
                        let (value, recovery, optional_space) = self.scan_character_code()?;
                        if optional_space && let Err(error) = self.scan_optional_space() {
                            if retain_continuation && error.is_resource_suspension() {
                                self.command.pending_integer_scans.push(
                                    PendingIntegerScan::CharacterOptionalSpace {
                                        negative,
                                        provenance,
                                        value,
                                    },
                                );
                            }
                            return Err(error);
                        }
                        (value, NO_RADIX, recovery)
                    }
                    _ => {
                        // §444's `vacuous` case. `radix:=10` is assigned before
                        // the accumulation loop, so it survives §446's recovery.
                        self.back_input(first)?;
                        self.missing_number_error()?;
                        return Ok((self.inserted_zero_integer(provenance), DECIMAL_RADIX));
                    }
                },
            };
        let scanned = self.finish_integer(value, negative, provenance, recovery);
        Ok((scanned, radix))
    }

    fn finish_integer(
        &mut self,
        value: i32,
        negative: bool,
        provenance: OriginId,
        recovery: ScalarRecovery,
    ) -> ScannedScalar<i32> {
        let scanned = ScannedScalar {
            value: if negative {
                value.saturating_neg()
            } else {
                value
            },
            recovery,
            provenance: ScalarProvenance {
                primary: provenance,
            },
        };
        observe!(
            self,
            CommandObservation::Scanner(ScannerRecord {
                kind: "integer",
                value: ObservationValue::Integer(i64::from(scanned.value)),
            }),
        );
        scanned
    }

    /// TeX82 §416's and §446's shared outcome: the offending token has
    /// already been backed up, "Missing number, treated as zero" is reported,
    /// and the scan publishes zero.
    fn inserted_zero_integer(&mut self, provenance: OriginId) -> ScannedScalar<i32> {
        let scanned = ScannedScalar {
            value: 0,
            recovery: ScalarRecovery::InsertedZero,
            provenance: ScalarProvenance {
                primary: provenance,
            },
        };
        observe!(
            self,
            CommandObservation::Scanner(ScannerRecord {
                kind: "integer",
                value: ObservationValue::Integer(i64::from(scanned.value)),
            }),
        );
        scanned
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
        // TeX82 §448's `<Get the next non-blank non-sign token>` leaves that
        // token in hand and branches on it: an internal quantity (a command
        // code in §208/§209's `min_internal..=max_internal`) goes straight to
        // `scan_something_internal`, and only the other branch runs
        // `back_input` before `scan_int` owns the constant. Backing the token
        // up unconditionally and re-reading it would install a backup level
        // and redeliver the command that TeX never re-delivers.
        //
        // `internal_value_from_command` is that command-code test: it is
        // exhaustive over the internal families and consumes an internal
        // quantity's own operand (a register index, a font selector, a
        // character code), so it must see the token before any `back_input`
        // could split the quantity from its operand.
        let mut negative = false;
        let mut provenance = OriginId::UNKNOWN;
        let first = loop {
            let Some(command) = self.get_x_token()? else {
                break None;
            };
            if provenance == OriginId::UNKNOWN {
                provenance = command.origin();
            }
            match scalar_meaning(command.meaning()) {
                Meaning::CharToken { ch: ' ', .. } | Meaning::CharToken { ch: '+', .. } => {}
                Meaning::CharToken { ch: '-', .. } => negative = !negative,
                _ => break Some(command),
            }
        };
        let provenance = ScalarProvenance {
            primary: provenance,
        };
        // TeX82 §449's "Fetch an internal dimension and goto attach_sign,
        // or fetch an internal integer": `scan_something_internal(mu_val,
        // false)` when `mu`, and `scan_something_internal(dimen_val,false)`
        // otherwise. A quantity that ends up at the requested dimension level
        // is the whole answer and skips both the unit scan and its optional
        // space; one that ends up an integer becomes the numeric prefix of an
        // ordinary units scan (`\dimen0=\count5 pt`).
        let level = if mu {
            InternalLevel::MuGlue
        } else {
            InternalLevel::Dimension
        };
        let (value, order, attach_sign, arith_error, recovery) = match first {
            Some(first) => match self.scan_something_internal(&first, level, false)? {
                InternalScan::Value(value) => match self.fetch_internal_dimension(value, mu)? {
                    InternalDimension::Complete(value) => {
                        (value, Order::Normal, true, false, ScalarRecovery::None)
                    }
                    InternalDimension::Prefix(integer) => {
                        self.last_integer_terminator = None;
                        let (units, flip) =
                            self.scan_units_after_integer(integer, false, allow_infinite, mu)?;
                        negative ^= flip;
                        (
                            units.value,
                            units.order,
                            units.attach_sign,
                            units.arith_error,
                            ScalarRecovery::None,
                        )
                    }
                },
                // TeX82 §448's other branch: `back_input`, then either
                // `scan_int` or §448's own `radix:=10; cur_val:=0` owns the
                // numeric part, and the unit scan always follows.
                InternalScan::NotInternal => {
                    let (units, flip, recovery) =
                        self.scan_dimension_constant(first, allow_infinite, mu, provenance)?;
                    negative ^= flip;
                    (
                        units.value,
                        units.order,
                        units.attach_sign,
                        units.arith_error,
                        recovery,
                    )
                }
            },
            None => (
                Scaled::from_raw(0),
                Order::Normal,
                true,
                false,
                ScalarRecovery::InsertedZero,
            ),
        };
        // TeX82 §448's trailing `<Scan an optional space>` sits between the
        // unit scan and `attach_sign:`, so every path that reached
        // `attach_sign` by a `goto` -- §449's whole internal dimension and
        // §455's two internal-unit exits -- skips it.
        if !attach_sign {
            self.scan_optional_space()?;
        }
        // TeX82 §448-§460: every path converges at `attach_sign`, including
        // §449's internal-dimension shortcut.  The range check therefore
        // applies even when the stored internal value could only have arisen
        // through arithmetic (for example, an overflowing glue width).
        let value = if arith_error || value.raw().unsigned_abs() >= 1 << 30 {
            self.dimension_too_large_error()?;
            Scaled::MAX_DIMEN
        } else {
            value
        };
        let scanned = ScannedScalar {
            value: if negative {
                Scaled::from_raw(-value.raw())
            } else {
                value
            },
            recovery,
            provenance,
        };
        observe!(
            self,
            CommandObservation::Scanner(ScannerRecord {
                kind: "dimension",
                value: ObservationValue::Scaled(i64::from(scanned.value.raw())),
            }),
        );
        Ok((scanned, order))
    }

    /// TeX82 §448's non-internal branch of `scan_dimen`, which tex.web writes
    /// as
    ///
    /// ```text
    /// back_input;
    /// if cur_tok=continental_point_token then cur_tok:=point_token;
    /// if cur_tok<>point_token then scan_int
    /// else begin radix:=10; cur_val:=0; end;
    /// if cur_tok=continental_point_token then cur_tok:=point_token;
    /// if (radix=10)and(cur_tok=point_token) then <Scan decimal fraction>;
    /// ```
    ///
    /// `back_input` does not disturb `cur_tok`, so the point test reads the
    /// token that was just backed up **without fetching it again**. A leading
    /// decimal point therefore never reaches `scan_int` at all: §448 assigns
    /// `radix:=10; cur_val:=0` directly, and §452's `get_token` is the single
    /// delivery that re-scans the point. Re-reading it through `get_x_token`
    /// first would add an expanded delivery, a vacuous §444 integer scan with
    /// §446's second backup, and a `scan_int` result TeX never computes --
    /// six semantic events per leading-point dimension, which is what
    /// `\vskip .5cm` produced.
    ///
    /// For every other token the integer part is `scan_int`'s (§440), whose
    /// own `<Get the next non-blank non-sign token>` performs the replay. The
    /// backup and that redelivery are both observable, so the hand-off stays
    /// a real backup/replay cycle rather than passing the already-delivered
    /// command straight through.
    ///
    /// §444's `vacuous` case is reported by `scan_int` itself: no number was
    /// there, so §446's "Express astonishment that no
    /// number was here" has already backed the offending token up a second
    /// time (`back_error`) and committed zero. It still proceeds through
    /// §448's unit scan and optional-space scan; only the recovery marker is
    /// carried into the completed dimension. Asking `scan_int` instead of
    /// pre-testing the token for an ASCII digit is what keeps §444's octal
    /// (`'`) and hexadecimal (`"`) introducers and §442's alphabetic constant
    /// out of the vacuous bucket -- all three are legal dimension prefixes,
    /// as §448's own `-'77 pt` example says.
    ///
    /// The `radix=10` conjunct is why `'77.5pt` has no fractional part:
    /// §440 initializes `radix:=0` and only §444's decimal branch sets it to
    /// 10, so an octal, hexadecimal, or alphabetic constant leaves a
    /// following point to the unit scan.
    fn scan_dimension_constant(
        &mut self,
        first: CurrentCommand<G>,
        allow_infinite: bool,
        mu: bool,
        provenance: ScalarProvenance,
    ) -> Result<(ScannedUnits, bool, ScalarRecovery), CommandError> {
        let leading_point = is_point_token(&first);
        self.back_input(first)?;
        // TeX82 `scan_dimen` delegates the integral prefix to `scan_int`.
        // Besides sharing radix and recovery rules, that ownership is
        // observable: `scan_int` backs up the decimal point before
        // completing, then `scan_dimen` consumes it raw before scanning
        // fractional digits. Keep that hand-off inside the command scanner
        // rather than collapsing it into a private decimal parser.
        self.last_integer_terminator = None;
        let (integer, decimal, recovery) = if leading_point {
            (0, true, ScalarRecovery::None)
        } else {
            let replayed = self.get_x_token()?.ok_or(CommandError::input_invariant())?;
            let (scanned, radix) =
                self.complete_integer(replayed, false, provenance.primary, false)?;
            let decimal = radix == DECIMAL_RADIX
                && self
                    .last_integer_terminator
                    .as_ref()
                    .is_some_and(is_point_token);
            (scanned.value, decimal, scanned.recovery)
        };
        if decimal {
            // §452: "|point_token| is being re-scanned". It is `get_token`,
            // not `get_x_token`, so the point is delivered raw exactly once
            // and never expanded. A unit stays in the backed-up input for the
            // keyword scanner instead. §13's rule that every raw-delivery
            // caller matches the section it implements makes this `get_token`
            // rather than `get_next`, even though the token being re-scanned
            // can only ever be the point this branch just backed up.
            let _ = self.get_token()?;
        }
        let (units, flip) = self.scan_units_after_integer(integer, decimal, allow_infinite, mu)?;
        Ok((units, flip, recovery))
    }

    /// Classifies §413's committed value for TeX82 §449's two exits,
    /// including §451's "Coerce glue to a dimension".
    ///
    /// §413 has already run §429's cascade for the level §449 asked for, so
    /// this only sorts the surviving levels into "the whole dimension" and
    /// "the numeric prefix of a units scan".
    fn fetch_internal_dimension(
        &mut self,
        value: InternalValue,
        mu: bool,
    ) -> Result<InternalDimension, CommandError> {
        if !mu {
            // `scan_something_internal(dimen_val,false)`: §429 has lowered glue
            // and mu glue to a width, so only `dimen_val` and `int_val`
            // survive, and §449 takes a dimension as the complete answer.
            return Ok(match value {
                InternalValue::Dimension(value) => InternalDimension::Complete(value),
                InternalValue::Integer(value) => InternalDimension::Prefix(value),
                _ => {
                    unreachable!("TeX82 §429 lowers a dimen_val request to a dimension or integer")
                }
            });
        }
        // `scan_something_internal(mu_val,false)`: `mu_val` is the highest
        // numeric level, so §429's loop never runs and the level reached is
        // exactly the quantity's own.
        Ok(match value {
            // §451 replaces the specification by its width while leaving
            // `cur_val_level` at `mu_val`, so `goto attach_sign` takes it as
            // the whole mu dimension. This is what `\mkern\thinmuskip` reads.
            InternalValue::MuGlue(glue) => InternalDimension::Complete(glue.width),
            // §451 coerces this width too, but `cur_val_level` stays
            // `glue_val`, which is neither `mu_val` nor `int_val`: TeX reports
            // `mu_error` and continues with the width as the units prefix.
            InternalValue::Glue(glue) => {
                self.mu_error()?;
                InternalDimension::Prefix(glue.width.raw())
            }
            // `dimen_val` is below `glue_val`, so §451 does not apply; the
            // level test still reports `mu_error` and keeps the value.
            InternalValue::Dimension(value) => {
                self.mu_error()?;
                InternalDimension::Prefix(value.raw())
            }
            InternalValue::Integer(value) => InternalDimension::Prefix(value),
            InternalValue::Font(_) | InternalValue::Tokens { .. } => {
                unreachable!("TeX82 §416 reports a mu_val request for a font or token list")
            }
        })
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
    ) -> Result<(ScannedUnits, bool), CommandError> {
        let flip = integer < 0;
        let integer = if flip {
            integer.saturating_neg()
        } else {
            integer
        };
        let units = self.scan_decimal_dimension(integer, decimal, allow_infinite, mu)?;
        Ok((units, flip))
    }

    /// TeX82 §448's `scan_dimen(mu,false,true)` shortcut: the integer part is
    /// already in hand, so only the units remain to be scanned.
    fn scan_dimension_shortcut(&mut self, integer: i32, mu: bool) -> Result<Scaled, CommandError> {
        self.last_integer_terminator = None;
        let (units, flip) = self.scan_units_after_integer(integer, false, false, mu)?;
        if !units.attach_sign {
            self.scan_optional_space()?;
        }
        let mut value = if flip {
            Scaled::from_raw(-units.value.raw())
        } else {
            units.value
        };
        if units.arith_error || value.raw().unsigned_abs() >= 1 << 30 {
            self.dimension_too_large_error()?;
            value = if flip {
                Scaled::from_raw(-Scaled::MAX_DIMEN.raw())
            } else {
                Scaled::MAX_DIMEN
            };
        }
        observe!(
            self,
            CommandObservation::Scanner(ScannerRecord {
                kind: "dimension",
                value: ObservationValue::Scaled(i64::from(value.raw())),
            }),
        );
        Ok(value)
    }

    /// Scans a normal or mu glue specification.
    pub fn scan_glue(&mut self, mu: bool) -> Result<ScannedScalar<GlueSpec>, CommandError> {
        self.scanned_glue_identity = None;
        self.scanned_glue_register = None;
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
            match scalar_meaning(command.meaning()) {
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
            // §461 is the one caller that passes §413's `negative` flag, so
            // §430 negates the committed value -- all three components of a
            // glue specification together -- before §413 returns.
            Some(command) => match self.scan_something_internal(&command, level, negative)? {
                // §461: `if cur_val_level>=glue_val then begin if
                // cur_val_level<>level then mu_error; return end`. The
                // specification is the complete answer, so no `plus`/`minus`
                // components follow it. §429 has already lowered a `mu_val`
                // quantity to `glue_val` (reporting `mu_error` on the way), so
                // the level test that remains here is §461's `glue_val`
                // quantity where `mu_val` was requested.
                InternalScan::Value(
                    internal @ (InternalValue::Glue(_) | InternalValue::MuGlue(_)),
                ) => {
                    if internal.level() != level {
                        self.mu_error()?;
                    }
                    let (InternalValue::Glue(glue) | InternalValue::MuGlue(glue)) = internal else {
                        unreachable!("outer pattern restricts the value to a glue specification")
                    };
                    (glue, ScalarRecovery::None, provenance, true)
                }
                // TeX82's `scan_glue` accepts an internal dimension as the
                // width of an ordinary glue specification.  In particular,
                // `\ht<box>` has already consumed its bounded box index;
                // backing up the primitive here would both replay an
                // incomplete internal value and use a stale delivery proof.
                // See TeX.web §461 (`scan_glue`). A mu-level request reports
                // `mu_error` first and keeps the dimension.
                InternalScan::Value(InternalValue::Dimension(width)) => {
                    if mu {
                        self.mu_error()?;
                    }
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
                // §461: `if cur_val_level=int_val then scan_dimen(mu,false,
                // true)`. §430 has already negated the integer, which is the
                // numeric prefix of a units-only dimension scan.
                InternalScan::Value(InternalValue::Integer(integer)) => {
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
                InternalScan::Value(InternalValue::Font(_) | InternalValue::Tokens { .. }) => {
                    unreachable!("TeX82 §416 converts identifiers to dimen_val zero before §461")
                }
                // §461's non-internal branch: `back_input; scan_dimen(mu,
                // false,false); if negative then negate(cur_val)`.
                InternalScan::NotInternal => {
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
        if negative {
            self.scanned_glue_identity = None;
            self.scanned_glue_register = None;
        }
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
        observe!(
            self,
            CommandObservation::Scanner(ScannerRecord {
                kind: "glue",
                value: observed_glue_value(&scanned.value),
            }),
        );
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
        match self.scan_the_internal_value(&command)? {
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

    /// Runs TeX82 §465's `the_toks` recovery for a non-internal operand.
    ///
    /// Unlike the probe-shaped [`Self::scan_internal_value`], this operation
    /// owns the operand: §465 consumes an invalid target, reports `you_cant`,
    /// and publishes integer zero for both `\the` and `\showthe`.
    pub(crate) fn scan_internal_value_or_zero(
        &mut self,
    ) -> Result<ScannedScalar<InternalValue>, CommandError> {
        let command = self.get_x_token()?.ok_or(CommandError::input_invariant())?;
        let provenance = ScalarProvenance {
            primary: command.origin(),
        };
        let value = match self.scan_the_internal_value(&command)? {
            Some(value) => value,
            None => {
                let rendered = meaning_text(&mut self.state, &command);
                let context = self.command.output_open_context(&self.state);
                let mut report = self
                    .state
                    .print_err(&format!("You can't use `{rendered}' after \\the"));
                report
                    .help(&["I'm forgetting what you said and using zero instead."])
                    .context(context);
                report.error().jump_out()?;
                InternalValue::Integer(0)
            }
        };
        Ok(ScannedScalar {
            value,
            recovery: ScalarRecovery::None,
            provenance,
        })
    }

    /// Runs TeX82 §465's `scan_something_internal(tok_val,false)` on a target
    /// the caller already holds.
    ///
    /// `tok_val` is the top level, so §429's cascade never runs and a font
    /// identifier or token list is a value in its own right rather than §416's
    /// missing number. `None` is §413's "not an internal quantity" test.
    pub(crate) fn scan_the_internal_value(
        &mut self,
        target: &CurrentCommand<G>,
    ) -> Result<Option<InternalValue>, CommandError> {
        match self.scan_something_internal(target, InternalLevel::Tokens, false)? {
            InternalScan::Value(value) => Ok(Some(value)),
            InternalScan::NotInternal => Ok(None),
        }
    }

    /// §445's `<Accumulate the constant until cur_tok is not a suitable
    /// digit>`, plus §444's `vacuous` flag.
    ///
    /// `first` is the decimal introducer's own digit, which §444 has already
    /// accumulated; `'` and `"` pass `None` because §444 consumed them with
    /// `get_x_token` and no digit has been seen yet. The flag is the second
    /// result because the two exits differ: §444 ends a constant that scanned
    /// at least one digit with `else if cur_cmd<>spacer then back_input`,
    /// absorbing one terminating space, while §446's `back_error` backs the
    /// terminator up unconditionally before reporting the missing number.
    fn scan_radix_tail(
        &mut self,
        first: Option<u8>,
        radix: u8,
        integer_continuation: Option<(bool, OriginId)>,
    ) -> Result<(i32, bool), CommandError> {
        self.scan_radix_tail_from(
            i32::from(first.unwrap_or(0)),
            first.is_none(),
            false,
            radix,
            integer_continuation,
        )
    }

    fn scan_radix_tail_from(
        &mut self,
        mut value: i32,
        mut vacuous: bool,
        mut overflowed: bool,
        radix: u8,
        integer_continuation: Option<(bool, OriginId)>,
    ) -> Result<(i32, bool), CommandError> {
        loop {
            let command = match self.get_x_token() {
                Ok(command) => command,
                Err(error) => {
                    if error.is_resource_suspension()
                        && let Some((negative, provenance)) = integer_continuation
                    {
                        self.command
                            .pending_integer_scans
                            .push(PendingIntegerScan::Radix {
                                negative,
                                provenance,
                                radix,
                                value,
                                vacuous,
                                overflowed,
                            });
                    }
                    return Err(error);
                }
            };
            let Some(command) = command else {
                break;
            };
            match Self::radix_digit(&command) {
                Some(digit) if digit < radix => {
                    vacuous = false;
                    let next = value
                        .checked_mul(i32::from(radix))
                        .and_then(|value| value.checked_add(i32::from(digit)));
                    match next {
                        Some(next) => value = next,
                        None => {
                            value = i32::MAX;
                            // TeX82 §445 calls `error` as soon as the first
                            // overflowing digit has been fetched, before the
                            // following digit or optional space advances
                            // `loc`. Later digits remain consumed, but do not
                            // repeat the report (`OK_so_far:=false`).
                            if !overflowed {
                                self.number_too_big_error()?;
                                overflowed = true;
                            }
                        }
                    }
                }
                _ => {
                    let terminator = command.copy_for_backup();
                    if vacuous {
                        // §446's `back_error`, which is `back_input; error`
                        // and so keeps even a spacer for the caller's report
                        // to pseudoprint.
                        self.back_input(command)?;
                    } else if self.back_input_unless_spacer(command)? {
                        // A numeric constant absorbs one terminating space,
                        // so replay must not manufacture a backup transition
                        // before publishing it.
                        self.last_integer_terminator = Some(terminator);
                    }
                    break;
                }
            }
        }
        Ok((value, vacuous))
    }

    fn radix_digit(command: &CurrentCommand<G>) -> Option<u8> {
        match scalar_meaning(command.meaning()) {
            Meaning::CharToken {
                ch: ch @ '0'..='9',
                cat: Catcode::Other,
            } => Some(ch as u8 - b'0'),
            Meaning::CharToken {
                ch: ch @ 'A'..='F',
                cat: Catcode::Letter | Catcode::Other,
            } => Some(ch as u8 - b'A' + 10),
            _ => None,
        }
    }

    fn scan_character_code(&mut self) -> Result<(i32, ScalarRecovery, bool), CommandError> {
        let Some(command) = self.get_next_character_code()? else {
            return Ok((0, ScalarRecovery::None, false));
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
                        self.improper_alphabetic_constant_error()?;
                        return Ok((0, ScalarRecovery::InsertedZero, false));
                    }
                }
            }
            _ => {
                self.back_input(command)?;
                self.improper_alphabetic_constant_error()?;
                return Ok((0, ScalarRecovery::InsertedZero, false));
            }
        };
        if value > i32::from(u8::MAX) && !self.command.profile().capabilities().supports_unicode() {
            self.back_input(command)?;
            self.improper_alphabetic_constant_error()?;
            return Ok((0, ScalarRecovery::InsertedZero, false));
        }
        Ok((value, ScalarRecovery::None, true))
    }

    fn scan_decimal_dimension(
        &mut self,
        integer: i32,
        decimal: bool,
        allow_infinite: bool,
        mu: bool,
    ) -> Result<ScannedUnits, CommandError> {
        let mut fraction = [0_u8; 17];
        let mut fraction_len = 0;
        if decimal {
            loop {
                let Some(command) = self.get_x_token()? else {
                    break;
                };
                match scalar_meaning(command.meaning()) {
                    Meaning::CharToken {
                        ch,
                        cat: Catcode::Other,
                    } if ch.is_ascii_digit() => {
                        if fraction_len < fraction.len() {
                            fraction[fraction_len] = ch as u8 - b'0';
                            fraction_len += 1;
                        }
                    }
                    // §452's closing `if cur_cmd<>spacer then back_input`: a
                    // fraction absorbs the space that ends it exactly as
                    // §444's integer constant does, so `.5 in` reaches the
                    // unit scan with `i` still unread rather than with a
                    // backed-up space in front of it.
                    _ => {
                        self.back_input_unless_spacer(command)?;
                        break;
                    }
                }
            }
        }
        let (unit, magnification) = if allow_infinite
            && !decimal
            && self
                .last_integer_terminator
                .as_ref()
                .is_some_and(|command| {
                    matches!(
                        scalar_meaning(command.meaning()),
                        Meaning::CharToken { ch: 'f', .. }
                    )
                }) {
            // `scan_int` has already observed and backed up the leading `f`.
            // Replay it once, then finish the `fil` suffix without routing the
            // same candidate through unrelated physical-unit keywords.
            let Some(first) = self.get_x_token()? else {
                return Err(CommandError::input_invariant());
            };
            if !matches!(
                scalar_meaning(first.meaning()),
                Meaning::CharToken { ch: 'f', .. }
            ) {
                return Err(CommandError::input_invariant());
            }
            (self.scan_infinite_unit_from_fil()?, None)
        } else {
            self.scan_dimension_unit(allow_infinite, mu)?
        };
        let fraction = round_decimal_fraction(&fraction[..fraction_len]);
        // TeX82 §457's "Adjust for the magnification ratio", applied between
        // recognizing `true` and converting the physical unit: the unit scan
        // above has already run `prepare_mag`, which validates and freezes
        // `\mag` and reports at tex.web's own point, and a magnification other
        // than 1000 divides the scanned quantity by `mag/1000` so that one
        // `true` unit still measures one physical unit on the magnified page.
        // Every arithmetic failure below is tex.web's `arith_error`, which
        // §460's `attach_sign` turns into "Dimension too large" -- a reported,
        // recoverable error that clamps to `max_dimen` and keeps going, not an
        // abandoned job.
        let mut arith_error = false;
        let (integer, fraction) = if let Some(mag) = magnification {
            scale_true_dimension_parts(integer, fraction, mag).unwrap_or_else(|_| {
                arith_error = true;
                (integer, fraction)
            })
        } else {
            (integer, fraction)
        };
        let (value, order) = match unit {
            // §455's `nx_plus_y(save_cur_val,v,xn_over_d(v,f,@'200000))`.
            DimensionUnit::Internal(unit) => {
                let fraction = match xn_over_d(unit, fraction, Scaled::UNITY) {
                    Ok(result) => result.quotient,
                    Err(_) => {
                        arith_error = true;
                        Scaled::MAX_DIMEN
                    }
                };
                let value = nx_plus_y(integer, unit, fraction);
                if value.is_err() {
                    arith_error = true;
                }
                (value.unwrap_or(Scaled::MAX_DIMEN), Order::Normal)
            }
            DimensionUnit::Physical(unit) => {
                let value = scaled_from_decimal_parts(integer, fraction, unit);
                if value.is_err() {
                    arith_error = true;
                }
                (value.unwrap_or(Scaled::MAX_DIMEN), Order::Normal)
            }
            // TeX stores an infinite glue component's finite coefficient as a
            // scaled value, while its order is carried separately.
            DimensionUnit::Infinite(order) => {
                let value = scaled_from_decimal_parts(integer, fraction, PhysicalUnit::Pt);
                if value.is_err() {
                    arith_error = true;
                }
                (value.unwrap_or(Scaled::MAX_DIMEN), order)
            }
            // A mu unit has the same scaled representation as a point; its
            // distinctness belongs to the surrounding glue family.
            DimensionUnit::Mu => {
                let value = scaled_from_decimal_parts(integer, fraction, PhysicalUnit::Pt);
                if value.is_err() {
                    arith_error = true;
                }
                (value.unwrap_or(Scaled::MAX_DIMEN), Order::Normal)
            }
        };
        Ok(ScannedUnits {
            value: if arith_error {
                Scaled::MAX_DIMEN
            } else {
                value
            },
            order,
            arith_error,
            attach_sign: matches!(unit, DimensionUnit::Internal(_)),
        })
    }

    /// TeX82 §453's "Scan units and set `cur_val`".
    ///
    /// The returned magnification is `Some` exactly when §457's `true` prefix
    /// was recognized, and is the frozen `\mag` that
    /// `<Adjust for the magnification ratio>` has already validated. §457 runs
    /// that adjustment the instant `true` is scanned -- *before* it looks for
    /// `pt` -- so §288's `prepare_mag` report is emitted here rather than by
    /// the caller: reporting it after the unit had been consumed put the unit
    /// inside §82's context display, which showed `l.4 \dimen1=1truept`
    /// where tex.web shows `l.4 \dimen1=1true`.
    fn scan_dimension_unit(
        &mut self,
        allow_infinite: bool,
        mu: bool,
    ) -> Result<(DimensionUnit, Option<i32>), CommandError> {
        // TeX82 §455 first looks for an internal dimension, then probes `em`
        // and `ex`, before accepting `true`, `pt`, or a physical unit.  Each
        // unsuccessful probe owns one `back_input` hand-off.  In particular,
        // an `in` following a fraction must be replayed through precisely the
        // internal/`em`/`ex`/`true`/`pt` probes before `scan_keyword("in")`
        // consumes its `i` and its following `n` directly.
        if allow_infinite && self.scan_keyword("fil")?.value {
            return Ok((self.scan_infinite_unit(Order::Fil)?, None));
        }
        if let Some(unit) = self.probe_dimension_unit(mu)? {
            return Ok((DimensionUnit::Internal(unit), None));
        }
        // §455 recognizes `em` and `ex` before the physical units, and skips
        // both entirely when a mu dimension is required (`if mu then goto
        // not_found`).  They are current-font parameters 6 (quad) and 5
        // (x-height), respectively; keep the successful keyword result so the
        // shared internal-unit fixed-point path scales both whole and
        // fractional dimensions.
        //
        // §455 places its `<Scan an optional space>` here, on the `em`/`ex`
        // path alone: the internal-dimension probe above jumps straight to
        // `found:`.  Both then reach `attach_sign`, so `scan_dimen`'s own
        // trailing optional space never runs for either.  `3em x` therefore
        // consumes exactly one space and `3\dimen0 x` consumes none.
        if !mu {
            for (keyword, parameter) in [("em", 6), ("ex", 5)] {
                if self.scan_keyword(keyword)?.value {
                    let unit = self.state.current_font_parameter(parameter);
                    self.scan_optional_space()?;
                    return Ok((DimensionUnit::Internal(unit), None));
                }
            }
            // pdfTeX 1.40.29 §455 extends TeX82's internal-dimension-unit
            // branch after `em` and `ex` with `px`, whose live scale is the
            // assignable `\pdfpxdimen` parameter initialized in §32a. This
            // belongs to the installed pdfTeX command profile, like the
            // additional physical units in §458 below.
            if self.command.profile().capabilities().supports_pdftex()
                && self.scan_keyword("px")?.value
            {
                let unit = self.state.dimen_param(DimenParam::PDF_PX_DIMEN);
                self.scan_optional_space()?;
                return Ok((DimensionUnit::Internal(unit), None));
            }
        }
        // §456: a mu dimension admits only the `mu` unit, and `true` is never
        // recognized in a mu context. A missing `mu` is "Illegal unit of
        // measure (mu inserted)": TeX reports it, keeps the scanned quantity
        // as mu, and leaves the offending text for the caller to re-read.
        if mu {
            if !self.scan_keyword("mu")?.value {
                self.illegal_unit_mu_error()?;
            }
            return Ok((DimensionUnit::Mu, None));
        }
        // §457's `if scan_keyword("true") then <Adjust for the magnification
        // ratio>`, whose first statement is `prepare_mag`.
        let magnification = if self.scan_keyword("true")?.value {
            let (mag, diagnostic) = self.state.prepare_mag();
            if let Some(diagnostic) = diagnostic {
                self.prepare_mag_error(diagnostic)?;
            }
            Some(mag)
        } else {
            None
        };
        if self.scan_keyword("pt")?.value {
            return Ok((DimensionUnit::Physical(PhysicalUnit::Pt), magnification));
        }
        for (keyword, unit) in [
            ("in", PhysicalUnit::In),
            ("pc", PhysicalUnit::Pc),
            ("cm", PhysicalUnit::Cm),
            ("mm", PhysicalUnit::Mm),
            ("bp", PhysicalUnit::Bp),
            ("dd", PhysicalUnit::Dd),
            ("cc", PhysicalUnit::Cc),
        ] {
            if self.scan_keyword(keyword)?.value {
                return Ok((DimensionUnit::Physical(unit), magnification));
            }
        }
        // TeX82 §458 proceeds directly from `cc` to `sp`. pdfTeX 1.40.29's
        // corresponding change inserts `nd` and `nc` between them, so these
        // probes (including their observable failed-keyword backups) belong
        // only to the pdfTeX command profile.
        if self.command.profile().capabilities().supports_pdftex() {
            for (keyword, unit) in [("nd", PhysicalUnit::Nd), ("nc", PhysicalUnit::Nc)] {
                if self.scan_keyword(keyword)?.value {
                    return Ok((DimensionUnit::Physical(unit), magnification));
                }
            }
        }
        if self.scan_keyword("sp")?.value {
            return Ok((DimensionUnit::Physical(PhysicalUnit::Sp), magnification));
        }
        // §459's "Complain about unknown unit": TeX assumes `pt` and
        // finishes the job that a hard scanner failure here would abandon.
        self.illegal_unit_pt_error()?;
        Ok((DimensionUnit::Physical(PhysicalUnit::Pt), magnification))
    }

    /// Performs TeX82 §455's internal-dimension unit lookahead.
    ///
    /// This scanner does not yet materialize internal units, but the failed
    /// lookahead is still a real command-owned operation: when it consumes a
    /// token replayed by the fractional scanner, TeX's `back_input` first
    /// retires that exhausted backup before installing the new one.
    fn probe_dimension_unit(&mut self, mu: bool) -> Result<Option<Scaled>, CommandError> {
        // §455 opens with §406's "Get the next non-blank non-call token"
        // (`repeat get_x_token until cur_cmd<>spacer`), and only the surviving
        // non-internal token is backed up -- the spaces it skipped are gone.
        // §445 and §452 absorb one trailing space each, so this loop is what
        // covers the coefficient forms that ran neither: §449's `int_val`
        // internal quantity, and §448's `shortcut`. Reading a single token
        // instead ended the probe on that space, so `\dimen0=\pretolerance
        // \hsize` found no unit and §459 recovered it as `pt`
        // (umber2-johp.115).
        let command = loop {
            let Some(command) = self.get_x_token()? else {
                return Ok(None);
            };
            if !matches!(
                scalar_meaning(command.meaning()),
                Meaning::CharToken {
                    cat: Catcode::Space,
                    ..
                }
            ) {
                break command;
            }
        };
        // §455 asks at `mu_val` when `mu` and at `dimen_val` otherwise, so
        // §429 lowers an internal glue to its width for an ordinary unit.
        let level = if mu {
            InternalLevel::MuGlue
        } else {
            InternalLevel::Dimension
        };
        // §455's internal branch is selected by the command range, not by the
        // level the resulting quantity happens to have. §413 has already
        // performed every downward coercion requested above; an integer is a
        // scaled unit unchanged, while either glue kind supplies its width.
        // In a mu scan §455 reports `mu_error` for every level below `mu_val`,
        // but still accepts that value as the unit.
        match self.scan_something_internal(&command, level, false)? {
            InternalScan::Value(value) => {
                if mu && !matches!(value, InternalValue::MuGlue(_)) {
                    self.mu_error()?;
                }
                let unit = match value {
                    InternalValue::Integer(value) => Scaled::from_raw(value),
                    InternalValue::Dimension(value) => value,
                    InternalValue::Glue(value) | InternalValue::MuGlue(value) => value.width,
                    InternalValue::Font(_) | InternalValue::Tokens { .. } => {
                        unreachable!("TeX82 §416 recovers nonnumeric internal values before §455")
                    }
                };
                Ok(Some(unit))
            }
            // Only this range-miss branch owns §455's `back_input`.
            InternalScan::NotInternal => {
                self.back_input(command)?;
                Ok(None)
            }
        }
    }

    /// TeX82 §443's `⟨Scan an optional space⟩`:
    /// `get_x_token; if cur_cmd<>spacer then back_input`.
    fn scan_optional_space(&mut self) -> Result<(), CommandError> {
        let Some(command) = self.get_x_token()? else {
            return Ok(());
        };
        self.back_input_unless_spacer(command)?;
        Ok(())
    }

    /// TeX82's `if cur_cmd<>spacer then back_input`, applied to a terminator
    /// the caller already holds. Reports whether the command was backed up.
    ///
    /// This is one mechanism, not a coincidence repeated three times: §443's
    /// `⟨Scan an optional space⟩`, §444's `⟨Scan a numeric constant⟩`, and
    /// §452's `⟨Scan decimal fraction⟩` all end a numeric scan by absorbing a
    /// terminating `spacer` and backing anything else up. A scan that backs
    /// the space up instead publishes a spurious backup input level and then
    /// re-reads the space, which the next scanner sees as an extra token.
    ///
    /// The test is on the command, so it is the token's category code and not
    /// its character: §207 makes `spacer` the command a category-10 character
    /// carries, and §349's "Enter `skip_blanks` state, emit a space" is what
    /// normalizes such a character's `cur_chr` to a space inside §341's
    /// `get_next`.
    fn back_input_unless_spacer(
        &mut self,
        command: CurrentCommand<G>,
    ) -> Result<bool, CommandError> {
        if matches!(
            scalar_meaning(command.meaning()),
            Meaning::CharToken {
                cat: Catcode::Space,
                ..
            }
        ) {
            return Ok(false);
        }
        self.back_input(command)?;
        Ok(true)
    }

    fn scan_infinite_unit(&mut self, mut order: Order) -> Result<DimensionUnit, CommandError> {
        // TeX82 §454 deliberately calls §407's full keyword scanner for
        // every suffix letter. Besides accepting either case, that consumes
        // leading spaces before each candidate (`fill L L L`) and restores
        // the first non-`l` token after the loop.
        while self.scan_keyword("l")?.value {
            if order == Order::Filll {
                self.excess_infinite_order_error()?;
            } else {
                order = raise_infinite_order(order);
            }
        }
        Ok(DimensionUnit::Infinite(order))
    }

    fn scan_infinite_unit_from_fil(&mut self) -> Result<DimensionUnit, CommandError> {
        for expected in ['i', 'l'] {
            let command = self.get_x_token()?.ok_or(CommandError::input_invariant())?;
            if !matches!(
                scalar_meaning(command.meaning()),
                Meaning::CharToken { ch, .. } if ch.eq_ignore_ascii_case(&expected)
            ) {
                return Err(CommandError::input_invariant());
            }
        }
        self.scan_infinite_unit(Order::Fil)
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
    /// `ident_val` and `tok_val` never enter the loop in tex.web: §416 has
    /// already replaced a font identifier or token list requested below
    /// `tok_val` with a backed-up zero, and
    /// [`Self::scan_something_internal`]'s own §416 guard answers that case
    /// before the fetch runs. `None` reports the same §415 branch from here,
    /// so a value that reaches the loop above `tok_val` by some other route
    /// is recovered by §416 rather than coercing a font into a number.
    fn coerce_internal_value(
        &mut self,
        mut value: InternalValue,
        level: InternalLevel,
    ) -> Result<Option<InternalValue>, CommandError> {
        while value.level() > level {
            value = match value {
                InternalValue::MuGlue(glue) => {
                    self.mu_error()?;
                    InternalValue::Glue(glue)
                }
                InternalValue::Glue(glue) => InternalValue::Dimension(glue.width),
                InternalValue::Dimension(dimension) => InternalValue::Integer(dimension.raw()),
                InternalValue::Font(_) | InternalValue::Tokens { .. } => return Ok(None),
                InternalValue::Integer(_) => {
                    unreachable!("int_val is the lowest level, so it never exceeds a target level")
                }
            };
        }
        Ok(Some(value))
    }

    /// TeX82 §408's `mu_error`: "Incompatible glue units" -- mu and non-mu
    /// quantities were mixed, and TeX assumes `1mu=1pt` and continues.
    ///
    fn mu_error(&mut self) -> Result<(), CommandError> {
        let context = self.command.output_open_context(&self.state);
        let mut report = self.state.print_err("Incompatible glue units");
        report
            .help(&["I'm going to assume that 1mu=1pt when they're mixed."])
            .context(context);
        report.error().jump_out()?;
        Ok(())
    }

    /// TeX82 §415's `back_error` before the scanner publishes zero.
    fn missing_number_error(&mut self) -> Result<(), CommandError> {
        // TeX82 §82 completes every error with `show_context`, and §415
        // reaches it only after §325's `back_error` has installed the
        // offending token as a `backed_up` level. CommandState, rather than
        // Universe, owns that level, so capture its display while it is live
        // for both immediate and deferred reporting.
        let context = self.command.output_open_context(&self.state);
        // §380 performs an undefined-control-sequence expansion before §444
        // reaches its vacuous constant. Queue behind any already-detected
        // command report. §1370's nested deferred-write processor must also
        // queue even when this is its first diagnostic: it runs inside an
        // artifact transaction, where direct transcript effects are staging
        // scratch and would be consumed at commit.
        if !self.command.semantic_diagnostics.is_empty() || self.command.expanding_deferred_write()
        {
            self.command
                .semantic_diagnostics
                .push(crate::CommandSemanticDiagnostic::MissingNumber { context });
            return Ok(());
        }
        let mut report = self.state.print_err("Missing number, treated as zero");
        report
            .help(&[
                "A number should have been here; I inserted `0'.",
                "(If you can't figure out why I needed to see a number,",
                "look up `weird error' in the index to The TeXbook.)",
            ])
            .context(context);
        report.error().jump_out()?;
        Ok(())
    }

    /// TeX82 §445's capped integer recovery.
    fn number_too_big_error(&mut self) -> Result<(), CommandError> {
        let context = self.command.output_open_context(&self.state);
        let mut report = self.state.print_err("Number too big");
        report
            .help(&[
                "I can only go up to 2147483647='17777777777=\"7FFFFFFF,",
                "so I'm using that number instead of yours.",
            ])
            .context(context);
        report.error().jump_out()?;
        Ok(())
    }

    /// TeX82 §456's unit recovery for math glue.
    /// TeX82 §442's out-of-range alphabetic-constant recovery.
    fn improper_alphabetic_constant_error(&mut self) -> Result<(), CommandError> {
        // §442 reaches `back_error`, so the caller has already restored the
        // offending token and §314 names it `<to be read again>`.
        let context = self.command.output_open_context(&self.state);
        let mut report = self.state.print_err("Improper alphabetic constant");
        report
            .help(&[
                "A one-character control sequence belongs after a ` mark.",
                "So I'm essentially inserting \\0 here.",
            ])
            .context(context);
        report.error().jump_out()?;
        Ok(())
    }

    /// TeX82 §418's wrong-mode half of `set_aux` while fetching an
    /// internal value. The value is still published as zero after the report.
    fn improper_auxiliary_error(&mut self, name: &str) -> Result<(), CommandError> {
        let context = self.command.output_open_context(&self.state);
        // TeX82 §1370 keeps the write_text level live while expanded
        // scan_toks calls §418. Shipout expansion is transactional in Umber,
        // so carry the report (including §82's already-rendered context) over
        // that boundary in detection order with command traces and earlier
        // scanner reports.
        if !self.command.semantic_diagnostics.is_empty() || self.command.expanding_deferred_write()
        {
            self.command
                .semantic_diagnostics
                .push(crate::CommandSemanticDiagnostic::Recoverable {
                    identity: IMPROPER_AUXILIARY_DIAGNOSTIC,
                    runaway: None,
                    message: format!("Improper \\{name}"),
                    help: IMPROPER_AUXILIARY_HELP,
                    context,
                    integer_error: None,
                });
            return Ok(());
        }
        let mut report = self.state.print_err(&format!("Improper \\{name}"));
        report.help(IMPROPER_AUXILIARY_HELP).context(context);
        report.error().jump_out()?;
        Ok(())
    }

    fn illegal_unit_mu_error(&mut self) -> Result<(), CommandError> {
        let context = self.command.output_open_context(&self.state);
        let mut report = self
            .state
            .print_err("Illegal unit of measure (mu inserted)");
        report
            .help(&[
                "The unit of measurement in math glue must be mu.",
                "To recover gracefully from this error, it's best to",
                "delete the erroneous units; e.g., type `2' to delete",
                "two letters. (See Chapter 27 of The TeXbook.)",
            ])
            .context(context);
        report.error().jump_out()?;
        Ok(())
    }

    /// TeX82 §454's recovery for each `l` beyond `filll`.
    fn excess_infinite_order_error(&mut self) -> Result<(), CommandError> {
        // §407 has consumed the successful one-letter keyword before §454
        // calls §82's `error`, so capture the source cursor at that exact
        // point rather than reusing context from an earlier scanner report.
        let context = self.command.output_open_context(&self.state);
        let mut report = self
            .state
            .print_err("Illegal unit of measure (replaced by filll)");
        report
            .help(&["I dddon't go any higher than filll."])
            .context(context);
        report.error().jump_out()?;
        Ok(())
    }

    /// TeX82 §459's unit recovery for ordinary dimensions, extended by
    /// pdfTeX 1.40.29 §459 with `nd`/`nc` even for a loaded TeX82 format.
    fn illegal_unit_pt_error(&mut self) -> Result<(), CommandError> {
        let context = self.command.output_open_context(&self.state);
        let mut report = self
            .state
            .print_err("Illegal unit of measure (pt inserted)");
        let unit_help = if self.command.engine_semantics().supports_pdftex() {
            "cm, mm, dd, cc, nd, nc, bp, or sp; but yours is a new one!"
        } else {
            "cm, mm, dd, cc, bp, or sp; but yours is a new one!"
        };
        report
            .help(&[
                "Dimensions can be in units of em, ex, in, pt, pc,",
                unit_help,
                "I'll assume that you meant to say pt, for printer's points.",
                "To recover gracefully from this error, it's best to",
                "delete the erroneous units; e.g., type `2' to delete",
                "two letters. (See Chapter 27 of The TeXbook.)",
            ])
            .context(context);
        report.error().jump_out()?;
        Ok(())
    }

    /// TeX82 §460's clamped dimension recovery.
    fn dimension_too_large_error(&mut self) -> Result<(), CommandError> {
        let context = self.command.output_open_context(&self.state);
        let mut report = self.state.print_err("Dimension too large");
        report
            .help(&[
                "I can't work with sizes bigger than about 19 feet.",
                "Continue and I'll use the largest value I can.",
            ])
            .context(context);
        report.error().jump_out()?;
        Ok(())
    }

    /// Renders the recoverable `prepare_mag` outcome used by §457.
    fn prepare_mag_error(&mut self, diagnostic: PrepareMagDiagnostic) -> Result<(), CommandError> {
        // TeX82 §82 calls `show_context` from every `error`, including the
        // §288 `prepare_mag` diagnostics. Capture it before opening the
        // report so a §325 one-token backup remains visible as
        // `<to be read again>`.
        let context = self.command.output_open_context(&self.state);
        match diagnostic {
            PrepareMagDiagnostic::IllegalMagnification { attempted } => {
                let mut report = self
                    .state
                    .print_err("Illegal magnification has been changed to 1000");
                report
                    .help(&["The magnification ratio must be between 1 and 32768."])
                    .context(context);
                report.int_error(attempted).jump_out()?;
            }
            PrepareMagDiagnostic::IncompatibleMagnification {
                attempted,
                retained,
            } => {
                // §288 breaks the message itself: `print_err("Incompatible
                // magnification ("); print_int(mag); print(");");
                // print_nl(" the previous value will be retained")`, with the
                // retained value supplied by `int_error` rather than by the
                // message text.
                let mut report = self
                    .state
                    .print_err(&format!("Incompatible magnification ({attempted});"));
                report.print_nl(" the previous value will be retained");
                report
                    .help(&[
                        "I can handle only one magnification ratio per job. So I've",
                        "reverted to the magnification you used earlier on this run.",
                    ])
                    .context(context);
                report.int_error(retained).jump_out()?;
            }
        }
        Ok(())
    }

    /// Runs TeX82 §413's `scan_something_internal` end to end.
    ///
    /// §413 takes the requested `level` and, at its single exit, has already
    /// run §429's lowering cascade and §430's negation. The level a caller --
    /// and therefore an observer -- sees is the level that was *asked for*,
    /// never the level the quantity happens to be stored at. `\fam\z@` asks at
    /// `int_val` and `\z@` is a `dimen_val` register, so §429 lowers it and
    /// TeX commits an `int_val`; fetching and coercing on opposite sides of
    /// the observation boundary reported `\z@`'s own `dimen_val` instead
    /// (umber2-johp.163).
    ///
    /// Every caller therefore names its level here rather than coercing
    /// afterwards, which is also what keeps §429's cascade from being
    /// re-derived once per scanner.
    fn scan_something_internal(
        &mut self,
        command: &CurrentCommand<G>,
        level: InternalLevel,
        negative: bool,
    ) -> Result<InternalScan, CommandError> {
        // §416 is titled "Fetch a token list or font identifier, provided that
        // |level=tok_val|", and its `level<>tok_val` test runs BEFORE any
        // operand is scanned: the whole branch is `back_error;
        // scanned_result(0)(dimen_val)`. Testing it after the fetch instead
        // would run §415's own `scan_eight_bit_int`, §577's
        // `scan_four_bit_int`, and §415's `back_input; scan_font_ident` on a
        // path tex.web never reaches, consuming tokens TeX leaves in place.
        if level != InternalLevel::Tokens
            && is_token_list_or_font_identifier(scalar_meaning(command.meaning()))
        {
            return self.missing_number_internal_result(command, level);
        }
        let Some(value) = self.fetch_internal_value(command)? else {
            return Ok(InternalScan::NotInternal);
        };
        let Some(value) = self.coerce_internal_value(value, level)? else {
            return self.missing_number_internal_result(command, level);
        };
        let value = if negative {
            negated_internal_value(value)
        } else {
            value
        };
        // e-TeX 2.6 [53a.4965--4998] handles expression primitives and glue
        // conversions in `Process an expression and return`: unlike the
        // ordinary TeX82 cases, that branch returns before
        // `scan_something_internal`'s generic result boundary. The expression
        // or conversion scanner has already published its typed result, and
        // the requesting `scan_int`/`scan_dimen`/`scan_glue` publishes the
        // outer typed result.
        if !matches!(
            scalar_meaning(command.meaning()),
            Meaning::UnexpandablePrimitive(
                UnexpandablePrimitive::NumExpr
                    | UnexpandablePrimitive::DimExpr
                    | UnexpandablePrimitive::GlueExpr
                    | UnexpandablePrimitive::MuExpr
                    | UnexpandablePrimitive::GlueToMu
                    | UnexpandablePrimitive::MuToGlue
            )
        ) {
            self.observe_internal_value(value.clone());
        }
        Ok(InternalScan::Value(value))
    }

    /// TeX82 §416's `back_error; scanned_result(0)(dimen_val)`: the backup,
    /// diagnostic, and zero commit belong to the internal scan, then §429
    /// lowers the result for its caller.
    fn missing_number_internal_result(
        &mut self,
        command: &CurrentCommand<G>,
        level: InternalLevel,
    ) -> Result<InternalScan, CommandError> {
        self.back_input(command.copy_for_backup())?;
        self.missing_number_error()?;
        let value = self
            .coerce_internal_value(InternalValue::Dimension(Scaled::from_raw(0)), level)?
            .expect("TeX82 §429 always lowers §416's dimen_val zero");
        self.observe_internal_value(value.clone());
        Ok(InternalScan::Value(value))
    }

    /// Runs TeX82 §413's case table alone: the fetch, with no coercion,
    /// negation, or observation.
    ///
    /// `None` is §413's `cur_cmd<min_internal or cur_cmd>max_internal` test
    /// failing. Only [`Self::scan_something_internal`] may call this; §413 has
    /// exactly one exit and committing a fetched value from anywhere else
    /// would publish a level TeX never commits.
    fn fetch_internal_value(
        &mut self,
        command: &CurrentCommand<G>,
    ) -> Result<Option<InternalValue>, CommandError> {
        let value = match scalar_meaning(command.meaning()) {
            // TeX82 `scan_something_internal` owns a register primitive's
            // restricted (`scan_eight_bit_int`) index scan. e-TeX 2.6
            // [26.415], [26.420], and [26.427] replace it with
            // `scan_register_num` for token, box, and word/glue registers.
            // Keeping every register family here means its index deliveries,
            // nested integer result, and internal-value observer precede the
            // outer scalar scanner's result.
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Count) => {
                let index = self.scan_profile_register_index()?;
                InternalValue::Integer(self.state.count(index).unwrap_or(0))
            }
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Dimen) => {
                let index = self.scan_profile_register_index()?;
                InternalValue::Dimension(self.state.dimen(index))
            }
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Skip) => {
                let index = self.scan_profile_register_index()?;
                let identity = self.state.glue_register(index).ok().flatten();
                self.scanned_glue_identity = identity;
                self.scanned_glue_register = Some((false, index));
                InternalValue::Glue(
                    identity.map_or_else(|| GlueSpec::ZERO, |id| self.state.glue(id)),
                )
            }
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Muskip) => {
                let index = self.scan_profile_register_index()?;
                let identity = self.state.muskip(index);
                self.scanned_glue_identity = identity;
                self.scanned_glue_register = Some((true, index));
                InternalValue::MuGlue(
                    identity.map_or_else(|| GlueSpec::ZERO, |id| self.state.glue(id)),
                )
            }
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Toks) => {
                let index = self.scan_profile_register_index()?;
                let tokens = self
                    .state
                    .token_register(index)
                    .expect("scanner produced an admitted token-register index");
                InternalValue::Tokens {
                    tokens: self.copy_durable_token_list_into_attempt(tokens)?,
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
                let index = self.scan_profile_register_index()?;
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
                // TeX82 §§578--579 calls `find_font_dimen(false)` for
                // enquiries: unlike assignment, even the newest font cannot
                // grow. The rejected location is the zero-valued scratch
                // cell, but the error context belongs at this scanner cursor.
                if !self.font_dimen_readable(font, number) {
                    let context = self.error_context();
                    self.command.semantic_diagnostics.push(
                        crate::CommandSemanticDiagnostic::FontDimenUnavailable { font, context },
                    );
                }
                let number = u32::try_from(number).unwrap_or(0);
                InternalValue::Dimension(self.state.font_dimen(font, number))
            }
            // e-TeX 2.6 etex.ch [17.3413--3453]'s four font-character
            // enquiries are read-only dimensions.  They scan the font
            // identifier before the bounded character number, then select
            // one component of the TFM character info; an absent character
            // (including every character of nullfont) supplies zero.
            Meaning::UnexpandablePrimitive(
                primitive @ (UnexpandablePrimitive::FontCharWd
                | UnexpandablePrimitive::FontCharHt
                | UnexpandablePrimitive::FontCharDp
                | UnexpandablePrimitive::FontCharIc),
            ) => {
                let font = self.scan_font_selector()?;
                let character = self.scan_character_number()?;
                let metrics = u8::try_from(u32::from(character))
                    .ok()
                    .and_then(|code| self.state.font_char_metrics(font, code));
                let dimension = metrics.map_or_else(
                    || Scaled::from_raw(0),
                    |metrics| match primitive {
                        UnexpandablePrimitive::FontCharWd => metrics.width,
                        UnexpandablePrimitive::FontCharHt => metrics.height,
                        UnexpandablePrimitive::FontCharDp => metrics.depth,
                        UnexpandablePrimitive::FontCharIc => metrics.italic_correction,
                        _ => unreachable!("outer match restricts primitive"),
                    },
                );
                InternalValue::Dimension(dimension)
            }
            // TeX82 §426's "Fetch a font integer": `assign_font_int` runs
            // `scan_font_ident` and then reads `hyphen_char[cur_val]` (`m=0`,
            // `\hyphenchar`) or `skew_char[cur_val]` (`\skewchar`) at
            // `int_val`. Without this arm both primitives fell through to
            // `scan_int`'s missing-number recovery and silently scanned as
            // zero, so `\ifnum\hyphenchar\font=-1` -- the standard probe for a
            // font whose hyphenation was disabled -- compared against the
            // wrong value with no diagnostic.
            Meaning::UnexpandablePrimitive(
                primitive @ (UnexpandablePrimitive::HyphenChar | UnexpandablePrimitive::SkewChar),
            ) => {
                let font = self.scan_font_selector()?;
                let value = match primitive {
                    UnexpandablePrimitive::HyphenChar => self.state.font_hyphen_char(font),
                    UnexpandablePrimitive::SkewChar => self.state.font_skew_char(font),
                    _ => unreachable!("outer match restricts primitive to the font integers"),
                };
                InternalValue::Integer(value)
            }
            // pdftex.web §§468--470 extends `scan_something_internal` with
            // nine per-font byte-code tables. The font identifier precedes
            // the bounded character selector, and the live checkpointed
            // table supplies the integer result (including TFM defaults).
            Meaning::UnexpandablePrimitive(
                primitive @ (UnexpandablePrimitive::PdfLpCode
                | UnexpandablePrimitive::PdfRpCode
                | UnexpandablePrimitive::PdfEfCode
                | UnexpandablePrimitive::PdfTagCode
                | UnexpandablePrimitive::PdfKnbsCode
                | UnexpandablePrimitive::PdfStbsCode
                | UnexpandablePrimitive::PdfShbsCode
                | UnexpandablePrimitive::PdfKnbcCode
                | UnexpandablePrimitive::PdfKnacCode),
            ) => {
                let font = self.scan_font_selector()?;
                let character = self.scan_character_number()?;
                let code = u8::try_from(u32::from(character))
                    .expect("pdfTeX character scanner is byte bounded");
                InternalValue::Integer(self.state.pdf_font_code(
                    pdf_font_code_table(primitive),
                    font,
                    code,
                ))
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
                InternalValue::Integer(value)
            }
            // TeX82 §423's "Fetch the par_shape size": `\parshape` reads the
            // number of lines in the current shape, or zero when none is set.
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::ParShape) => {
                InternalValue::Integer(i32::try_from(self.state.paragraph_shape_len()).unwrap_or(0))
            }
            // e-TeX 2.6 etex.ch [3455--3488]'s three parshape enquiries
            // scan one integer and return a dimension. Length/indent select
            // the corresponding component of that line; dimen interleaves
            // indent and length. Nonpositive indexes and an empty shape
            // return zero, while positive indexes beyond the shape repeat
            // its final line.
            Meaning::UnexpandablePrimitive(
                primitive @ (UnexpandablePrimitive::ParShapeLength
                | UnexpandablePrimitive::ParShapeIndent
                | UnexpandablePrimitive::ParShapeDimen),
            ) => {
                let number = self.scan_integer()?.value;
                let (line, width) = match primitive {
                    UnexpandablePrimitive::ParShapeLength => (number, true),
                    UnexpandablePrimitive::ParShapeIndent => (number, false),
                    UnexpandablePrimitive::ParShapeDimen if number > 0 => {
                        ((number + 1) / 2, number % 2 == 0)
                    }
                    UnexpandablePrimitive::ParShapeDimen => (0, false),
                    _ => unreachable!("outer match restricts primitive"),
                };
                InternalValue::Dimension(self.state.paragraph_shape_dimension(line, width))
            }
            // e-TeX 2.6 penalties module's "Fetch a penalties array
            // element" extends TeX82 §423's `set_shape` internal-value
            // branch. The selector is a full `scan_int`: zero returns the
            // array length, negative values return zero, and positive values
            // beyond the array repeat its final element.
            Meaning::UnexpandablePrimitive(
                primitive @ (UnexpandablePrimitive::InterLinePenalties
                | UnexpandablePrimitive::ClubPenalties
                | UnexpandablePrimitive::WidowPenalties
                | UnexpandablePrimitive::DisplayWidowPenalties),
            ) => {
                let kind = match primitive {
                    UnexpandablePrimitive::InterLinePenalties => PenaltyArrayKind::InterLine,
                    UnexpandablePrimitive::ClubPenalties => PenaltyArrayKind::Club,
                    UnexpandablePrimitive::WidowPenalties => PenaltyArrayKind::Widow,
                    UnexpandablePrimitive::DisplayWidowPenalties => PenaltyArrayKind::DisplayWidow,
                    _ => unreachable!("outer match restricts penalty-array primitives"),
                };
                let index = self.scan_integer()?.value;
                InternalValue::Integer(self.state.penalty_array_value(kind, index))
            }
            // e-TeX 2.6 etex.ch [17.5363--5404] extracts one component from
            // `scan_normal_glue`.  The nested ordinary-glue scanner owns all
            // literal/register/parameter expansion, mu-glue coercion and its
            // TeX82 §408 diagnostic, and following-token replay.
            Meaning::UnexpandablePrimitive(
                primitive @ (UnexpandablePrimitive::GlueStretch
                | UnexpandablePrimitive::GlueShrink
                | UnexpandablePrimitive::GlueStretchOrder
                | UnexpandablePrimitive::GlueShrinkOrder),
            ) => {
                let glue = self.scan_glue(false)?.value;
                let (kind, value) = match primitive {
                    UnexpandablePrimitive::GlueStretch => {
                        ("glue_stretch", InternalValue::Dimension(glue.stretch))
                    }
                    UnexpandablePrimitive::GlueShrink => {
                        ("glue_shrink", InternalValue::Dimension(glue.shrink))
                    }
                    UnexpandablePrimitive::GlueStretchOrder => (
                        "glue_stretch_order",
                        InternalValue::Integer(glue.stretch_order as i32),
                    ),
                    UnexpandablePrimitive::GlueShrinkOrder => (
                        "glue_shrink_order",
                        InternalValue::Integer(glue.shrink_order as i32),
                    ),
                    _ => unreachable!("outer match restricts primitive"),
                };
                // e-TeX 2.6 [53a.5362--5402] owns a typed enquiry result
                // after extracting the component/order and before §413's
                // common internal-value boundary.
                let rendered = match value {
                    InternalValue::Integer(value) => ObservationValue::Integer(i64::from(value)),
                    InternalValue::Dimension(value) => {
                        ObservationValue::Scaled(i64::from(value.raw()))
                    }
                    _ => unreachable!("glue enquiries return only integer or dimension values"),
                };
                self.observe(CommandObservation::Scanner(ScannerRecord {
                    kind,
                    value: rendered,
                }));
                value
            }
            // e-TeX 2.6 etex.ch [53a.4945--5360]: all four expression
            // primitives are `last_item` internal quantities. `scan_expr`
            // owns their complete typed factor/operator grammar and publishes
            // its result before §413's common internal-value exit below.
            Meaning::UnexpandablePrimitive(
                primitive @ (UnexpandablePrimitive::NumExpr
                | UnexpandablePrimitive::DimExpr
                | UnexpandablePrimitive::GlueExpr
                | UnexpandablePrimitive::MuExpr),
            ) => self.scan_expression_primitive(primitive)?,
            // e-TeX 2.6 etex.ch [53a.5404--5425]: conversions run the source
            // glue scanner at its own level and return the identical
            // components and orders at the destination level.
            Meaning::UnexpandablePrimitive(
                primitive @ (UnexpandablePrimitive::GlueToMu | UnexpandablePrimitive::MuToGlue),
            ) => self.scan_glue_conversion_primitive(primitive)?,
            // e-TeX 2.6 etex.ch §3736 extends TeX82's `set_page_int`
            // internal-value fetch: chr_code 2 returns the live global
            // `interaction` scalar before the same primitive's assignment
            // form scans a replacement.
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::InteractionMode) => {
                let value = self.state.interaction_mode_value();
                // The e-TeX observer change records the chr_code=2 fetch at
                // its own scanner boundary before §413 records the generic
                // internal result. This is an observation of the same value,
                // not a second scan or a second owner of interaction state.
                self.observe(CommandObservation::Scanner(ScannerRecord {
                    kind: "interaction_mode",
                    value: ObservationValue::Integer(i64::from(value)),
                }));
                InternalValue::Integer(value)
            }
            Meaning::CountRegister(index) => {
                InternalValue::Integer(self.state.count(index).unwrap_or(0))
            }
            Meaning::IntParam(index) => InternalValue::Integer(
                self.state
                    .int_param(tex_state::env::banks::IntParam::new(index)),
            ),
            Meaning::PageInteger(integer) => {
                InternalValue::Integer(self.state.page_integer(integer))
            }
            Meaning::DimenRegister(index) => InternalValue::Dimension(self.state.dimen(index)),
            Meaning::DimenParam(index) => {
                InternalValue::Dimension(self.state.dimen_param(DimenParam::new(index)))
            }
            Meaning::PageDimension(dimension) => {
                InternalValue::Dimension(self.state.page_dimension(dimension))
            }
            Meaning::SkipRegister(index) => {
                let identity = self.state.glue_register(index).ok().flatten();
                self.scanned_glue_identity = identity;
                self.scanned_glue_register = Some((false, index));
                InternalValue::Glue(
                    identity.map_or_else(|| GlueSpec::ZERO, |id| self.state.glue(id)),
                )
            }
            Meaning::MuskipRegister(index) => {
                let identity = self.state.muskip(index);
                self.scanned_glue_identity = identity;
                self.scanned_glue_register = Some((true, index));
                InternalValue::MuGlue(
                    identity.map_or_else(|| GlueSpec::ZERO, |id| self.state.glue(id)),
                )
            }
            Meaning::GlueParam(index) => {
                let identity = self
                    .state
                    .glue_param(tex_state::env::banks::GlueParam::new(index));
                self.scanned_glue_identity = identity;
                InternalValue::Glue(
                    identity.map_or_else(|| GlueSpec::ZERO, |id| self.state.glue(id)),
                )
            }
            Meaning::MuGlueParam(index) => {
                let identity = self
                    .state
                    .glue_param(tex_state::env::banks::GlueParam::new(index));
                self.scanned_glue_identity = identity;
                InternalValue::MuGlue(
                    identity.map_or_else(|| GlueSpec::ZERO, |id| self.state.glue(id)),
                )
            }
            Meaning::ToksRegister(index) => {
                let tokens = self
                    .state
                    .token_register(index)
                    .expect("meaning contains an admitted token-register index");
                InternalValue::Tokens {
                    tokens: self.copy_durable_token_list_into_attempt(tokens)?,
                }
            }
            Meaning::TokParam(index) => {
                let tokens = self
                    .state
                    .token_parameter(tex_state::env::banks::TokParam::new(index))
                    .expect("meaning contains an admitted token-parameter index");
                InternalValue::Tokens {
                    tokens: self.copy_durable_token_list_into_attempt(tokens)?,
                }
            }
            Meaning::InternalInteger(integer) => {
                let value = self.fetch_internal_integer(integer);
                // e-TeX 2.6 etex.ch [17.4750--4790] observes each current
                // group and current-condition enquiry immediately after it
                // commits its value, before §413's shared internal result.
                let enquiry_kind = match integer {
                    InternalInteger::CurrentGroupLevel => Some("current_group_level"),
                    InternalInteger::CurrentGroupType => Some("current_group_type"),
                    InternalInteger::CurrentIfLevel => Some("current_condition_level"),
                    InternalInteger::CurrentIfType => Some("current_condition_type"),
                    InternalInteger::CurrentIfBranch => Some("current_condition_branch"),
                    _ => None,
                };
                if let Some(kind) = enquiry_kind {
                    self.observe(CommandObservation::Scanner(ScannerRecord {
                        kind,
                        value: ObservationValue::Integer(i64::from(value)),
                    }));
                }
                InternalValue::Integer(value)
            }
            // `space_factor` is owned by the executor's active horizontal
            // list, rather than durable command state.  The bounded host
            // capability is refreshed before each command operation, so an
            // expanded definition can still scan `\the\spacefactor` through
            // the ordinary internal-value path.
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::SpaceFactor) => {
                let value = self.host.space_factor();
                if value.is_none() {
                    self.improper_auxiliary_error("spacefactor")?;
                }
                InternalValue::Integer(value.unwrap_or(0))
            }
            // TeX82 §418's "Fetch the `space_factor` or the `prev_depth`":
            // `\prevdepth` is the vertical-mode half of `set_aux` and reads at
            // `dimen_val`. Like `space_factor` above it is executor-owned mode
            // nest state projected through the bounded host capability; `None`
            // records tex.web's `abs(mode)<>m` case, which reports "Improper
            // \prevdepth" and reads zero. Reading it silently as a missing
            // number would change every `\ifdim\prevdepth>-1000pt` vertical
            // spacing decision.
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PrevDepth) => {
                let value = self.host.prev_depth();
                if value.is_none() {
                    self.improper_auxiliary_error("prevdepth")?;
                }
                InternalValue::Dimension(value.unwrap_or(Scaled::from_raw(0)))
            }
            // TeX82 §422's "Fetch the `prev_graf`": the paragraph count of the
            // nearest enclosing vertical level, at `int_val`. `None` records
            // tex.web's `mode=0` case (inside `\write`), which reads zero.
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PrevGraf) => {
                InternalValue::Integer(self.host.prev_graf().unwrap_or(0))
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
            // TeX82 §413's `scan_something_internal` groups `char_given` and
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
            Meaning::MathCharGiven(code) => InternalValue::Integer(i32::from(code)),
            // TeX82 §415's font-identifier branch, verbatim: "back_input;
            // scan_font_ident; scanned_result(font_id_base+cur_val)(ident_val)".
            //
            // §415 does NOT read the font off the command it already holds. It
            // pushes that command back and re-reads it through §577's
            // `scan_font_ident`, which is the only routine in TeX that turns a
            // token into a font -- so `def_family`'s §435 family index, §406's
            // space skipping, and the "Missing font identifier" recovery all
            // live in exactly one place. Reading the font here instead skipped
            // §415's backup level, its recovery record, and the re-delivery of
            // the command, which is five semantic events per `\the\font` and
            // per `\the\textfont<n>` (umber2-johp.259).
            Meaning::Font(_)
            | Meaning::UnexpandablePrimitive(
                UnexpandablePrimitive::Font
                | UnexpandablePrimitive::TextFont
                | UnexpandablePrimitive::ScriptFont
                | UnexpandablePrimitive::ScriptScriptFont,
            ) => {
                self.back_input(command.copy_for_backup())?;
                let font = self.scan_font_selector()?;
                self.font_identity(font)
            }
            // This deliberately names every non-internal Meaning variant.
            // Adding one to tex-state now fails here until its §413 range
            // classification is decided, instead of silently becoming zero.
            Meaning::Undefined
            | Meaning::Relax
            | Meaning::CharToken { .. }
            | Meaning::ExpandablePrimitive(_)
            | Meaning::EndV
            | Meaning::UnexpandablePrimitive(_)
            | Meaning::Unknown(_) => return Ok(None),
        };
        Ok(Some(value))
    }

    fn fetch_internal_integer(&self, integer: InternalInteger) -> i32 {
        match integer {
            InternalInteger::InputLineNumber => self.command.input.current_file_line_number(),
            // e-TeX 2.6 `etex.ch` [26.424] reads the effective tail of the
            // current list, not merely the page builder's remembered tail.
            InternalInteger::LastNodeType => self.host.last_node_type(),
            InternalInteger::CurrentGroupLevel => self.state.current_group_values().0,
            InternalInteger::CurrentGroupType => self.state.current_group_values().1,
            InternalInteger::CurrentIfLevel => self.command.conditions.current_etex_values().0,
            InternalInteger::CurrentIfType => self.command.conditions.current_etex_values().1,
            InternalInteger::CurrentIfBranch => self.command.conditions.current_etex_values().2,
            _ => self.state.internal_integer(integer).unwrap_or(0),
        }
    }

    /// Commits TeX82 §413's single internal scanner result.
    ///
    /// `scan_something_internal` has exactly one exit, and every case above
    /// reaches it, so exactly one internal result is committed per successful
    /// internal scan -- never one per case. Emitting it per case instead left
    /// most of §413's cases (every classical register, every named parameter,
    /// the page quantities, the read-only integers, and the font identifiers)
    /// committing no internal result at all, which does not merely mislabel
    /// an event: it makes Umber emit one event FEWER than the oracle and
    /// desynchronizes the whole remaining stream.
    ///
    /// The value committed here is the one §413 returns -- after §429's
    /// lowering cascade and §430's negation -- so this must stay reachable
    /// only from [`Self::scan_something_internal`]'s exits.
    fn observe_internal_value(&mut self, value: InternalValue) {
        let value = match value {
            InternalValue::Integer(value) => ObservationValue::Integer(i64::from(value)),
            InternalValue::Dimension(value) => ObservationValue::Scaled(i64::from(value.raw())),
            InternalValue::Glue(glue) | InternalValue::MuGlue(glue) => observed_glue_value(&glue),
            // TeX82 §413's `ident_val`: the font's own control-sequence
            // spelling, never a font number or file name.
            InternalValue::Font(symbol) => {
                ObservationValue::Name(self.state.resolve(symbol).to_owned())
            }
            InternalValue::Tokens { tokens, .. } => {
                let words = self
                    .command
                    .attempt
                    .arena()
                    .token_words(tokens)
                    .map_err(crate::scan_toks::attempt_command_error)
                    .expect("internal token value belongs to the installed attempt")
                    .to_vec();
                ObservationValue::Tokens(
                    words
                        .into_iter()
                        .map(|word| self.observed_token(word))
                        .collect(),
                )
            }
        };
        self.observe(CommandObservation::Scanner(ScannerRecord {
            kind: "internal",
            value,
        }));
    }

    fn font_identity(&mut self, font: FontId) -> InternalValue {
        InternalValue::Font(
            self.state
                .font_identifier_symbol(font)
                .expect("TeX font identifiers have a control-sequence identity"),
        )
    }

    /// Scans TeX82 §434's `scan_char_num` character selector.
    ///
    /// The bound and its recovery live in [`RestrictedIntegerClass`]; this
    /// wrapper only converts the recovered code into Umber's character type.
    pub(crate) fn scan_character_number(&mut self) -> Result<char, CommandError> {
        let scanned = self.scan_restricted_integer(RestrictedIntegerClass::CharacterCode)?;
        Ok(u32::try_from(scanned.value)
            .ok()
            .and_then(char::from_u32)
            .expect("a recovered character code is a character"))
    }

    /// Scans TeX82 §433's `scan_eight_bit_int` register index.
    ///
    /// TeX82 `scan_something_internal` uses this bounded scan for register
    /// primitives, §505's box predicates use it for
    /// `\ifvoid`/`\ifhbox`/`\ifvbox`, and §1079/§1110/§1241 use it for the
    /// box-valued commands; an out-of-range value recovers as register zero
    /// rather than truncating or addressing an extended bank. e-TeX replaces
    /// those register scans with `scan_register_num` at [26.415], [26.420],
    /// and [26.427].
    pub fn scan_eight_bit_register_index(&mut self) -> Result<u16, CommandError> {
        let scanned = self.scan_restricted_integer(RestrictedIntegerClass::EightBit)?;
        Ok(scanned.value as u16)
    }

    /// Scans the profile's ordinary register-command selector.
    ///
    /// TeX82 §1237 uses `scan_eight_bit_int`. e-TeX 2.6 change [49.1237]
    /// replaces it with `scan_register_num`, accepting sparse registers up to
    /// 32767 in extended mode; pdfTeX inherits that behavior.
    pub fn scan_profile_register_index(&mut self) -> Result<u16, CommandError> {
        if self.profile().capabilities().supports_etex() {
            self.scan_extended_register_index()
        } else {
            self.scan_eight_bit_register_index()
        }
    }

    /// Scans e-TeX 2.6 `etex.ch`'s `scan_register_num`.
    ///
    /// Extension primitives using this scanner are absent from TeX82.
    /// e-TeX and pdfTeX accept `0..=32767`, with invalid selectors diagnosed
    /// and recovered to zero.
    pub fn scan_extended_register_index(&mut self) -> Result<u16, CommandError> {
        let scanned = self.scan_restricted_integer(RestrictedIntegerClass::Register)?;
        Ok(scanned.value as u16)
    }
}

#[cfg(test)]
mod tests;
