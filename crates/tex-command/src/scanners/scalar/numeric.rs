//! Native numeric entry and substantive operand consumption.

use super::*;

use std::ops::ControlFlow;

/// The actual integer grammar and value, shared by full and resident entry.
#[derive(Default)]
struct IntegerOperand {
    negative: bool,
    provenance: OriginId,
    radix: u8,
    value: i32,
    have_digits: bool,
    overflowed: bool,
    scanned: Option<ScannedScalar<i32>>,
    ready: bool,
}

impl IntegerOperand {
    #[inline(always)]
    fn accept(
        &mut self,
        character: Option<(char, Catcode)>,
        origin: OriginId,
    ) -> ControlFlow<(), bool> {
        if self.radix == 0 {
            match character {
                Some((' ' | '+', _)) => {}
                Some(('-', _)) => self.negative = !self.negative,
                Some((ch @ '0'..='9', Catcode::Other)) => {
                    self.radix = DECIMAL_RADIX;
                    self.value = i32::from(ch as u8 - b'0');
                    self.have_digits = true;
                }
                Some(('\'', Catcode::Other)) => self.radix = 8,
                Some(('"', Catcode::Other)) => self.radix = 16,
                _ => return ControlFlow::Continue(false),
            }
            if self.provenance == OriginId::UNKNOWN {
                self.provenance = origin;
            }
            return ControlFlow::Continue(true);
        }
        if let Some(digit) = radix_digit(character)
            && digit < self.radix
        {
            if !self.overflowed {
                let Some(next) = self
                    .value
                    .checked_mul(i32::from(self.radix))
                    .and_then(|value| value.checked_add(i32::from(digit)))
                else {
                    return ControlFlow::Continue(false);
                };
                self.value = next;
            }
            self.have_digits = true;
            return ControlFlow::Continue(true);
        }
        if self.have_digits && matches!(character, Some((_, Catcode::Space))) {
            return ControlFlow::Break(());
        }
        ControlFlow::Continue(false)
    }
    // Recovery temporaries live only in this actual scanner call. The inline
    // native terminal seam discards its unused selector before this boundary.
    #[inline(never)]
    fn consume_full<G, Facts: crate::CommandHostFacts<G>>(
        &mut self,
        processor: &mut CommandProcessor<'_, '_, G, Facts>,
        spelling: TracedTokenWord,
        meaning: &tex_state::meaning::MeaningWord<G>,
        control_sequence: Option<Symbol>,
        exceptional: Option<&CurrentCommand<G>>,
    ) -> Result<ControlFlow<()>, CommandError> {
        if self.radix == 0 && self.provenance == OriginId::UNKNOWN {
            self.provenance = spelling.origin();
        }
        let character = native_character(meaning);
        match self.accept(character, spelling.origin()) {
            ControlFlow::Break(()) => return Ok(ControlFlow::Break(())),
            ControlFlow::Continue(true) => {
                self.ready = true;
                return Ok(ControlFlow::Continue(()));
            }
            ControlFlow::Continue(false) => {}
        }
        if self.radix == 0 {
            self.scanned = Some(processor.scan_integer_internal_or_alphabetic(
                spelling,
                meaning,
                control_sequence,
                exceptional,
                self.negative,
                self.provenance,
            )?);
            return Ok(ControlFlow::Break(()));
        }
        if radix_digit(character).is_some_and(|digit| digit < self.radix) {
            // Ordinary admission declines the first overflowing digit. §445's
            // report needs full processor access at this actual boundary.
            self.value = i32::MAX;
            self.have_digits = true;
            let site =
                processor.capture_consumed_site(spelling, meaning, control_sequence, exceptional);
            processor.number_too_big_error(Some(site))?;
            self.overflowed = true;
            self.ready = true;
            return Ok(ControlFlow::Continue(()));
        }
        if !self.have_digits {
            // §446 restores even a spacer before reporting a missing number.
            let site =
                processor.capture_consumed_site(spelling, meaning, control_sequence, exceptional);
            processor.back_consumed_word(spelling, exceptional)?;
            processor.missing_number_error_at(Some(site))?;
            self.scanned = Some(processor.inserted_zero_integer(self.provenance));
        } else if !matches!(character, Some((_, Catcode::Space))) {
            processor.back_consumed_word(spelling, exceptional)?;
            processor.integer_terminator_character = character;
        }
        Ok(ControlFlow::Break(()))
    }
}

impl<'state, 'host, G, Facts: crate::CommandHostFacts<G>>
    crate::NativeWordConsumer<CommandProcessor<'state, 'host, G, Facts>, G, Facts>
    for IntegerOperand
{
    const RESIDENT_TERMINALS: bool = true;
    const RESIDENT_LITERALS: bool = true;

    #[inline(always)]
    fn resident_ready(&self) -> bool {
        self.ready
    }

    #[inline(always)]
    fn input_boundary(&mut self) {
        self.ready = false;
    }

    #[inline(always)]
    fn resident_terminal(
        &mut self,
        _context: &mut tex_state::ResidentGroups<'_, G>,
        _facts: &mut Facts,
        _entered_line: u32,
        spelling: TracedTokenWord,
        meaning: &tex_state::meaning::MeaningWord<G>,
    ) -> ControlFlow<(), bool> {
        let result = self.accept(native_character(meaning), spelling.origin());
        if matches!(result, ControlFlow::Continue(false)) {
            self.ready = false;
        }
        result
    }

    #[inline(always)]
    fn consume(
        &mut self,
        processor: &mut CommandProcessor<'state, 'host, G, Facts>,
        spelling: TracedTokenWord,
        meaning: &tex_state::meaning::MeaningWord<G>,
        _code: u64,
        control_sequence: Option<Symbol>,
        exceptional: Option<&CurrentCommand<G>>,
    ) -> Result<ControlFlow<()>, CommandError> {
        self.consume_full(processor, spelling, meaning, control_sequence, exceptional)
    }
}

impl<G, Facts: crate::CommandHostFacts<G>> CommandProcessor<'_, '_, G, Facts> {
    pub(super) fn scan_integer_operand(&mut self) -> Result<ScannedScalar<i32>, CommandError> {
        // Ordinary initial syntax demands no full processor entry. Expansion
        // invalidates readiness before its first resulting terminal word.
        let mut integer = IntegerOperand {
            ready: true,
            ..IntegerOperand::default()
        };
        self.consume_expanded_words(&mut integer)?;
        if let Some(scanned) = integer.scanned {
            return Ok(scanned);
        }
        if integer.radix == 0 {
            return Ok(ScannedScalar {
                value: 0,
                provenance: ScalarProvenance {
                    primary: integer.provenance,
                },
            });
        }
        if !integer.have_digits {
            self.missing_number_error_at(None)?;
            return Ok(self.inserted_zero_integer(integer.provenance));
        }
        Ok(self.finish_integer(integer.value, integer.negative, integer.provenance))
    }

    /// TeX82 §440's `scan_int` body, from the token its
    /// `<Get the next non-blank non-sign token>` left in hand.
    #[inline(never)]
    #[allow(clippy::too_many_arguments)]
    fn scan_integer_internal_or_alphabetic(
        &mut self,
        spelling: TracedTokenWord,
        meaning: &tex_state::meaning::MeaningWord<G>,
        control_sequence: Option<tex_state::interner::Symbol>,
        exceptional: Option<&CurrentCommand<G>>,
        negative: bool,
        provenance: OriginId,
    ) -> Result<ScannedScalar<i32>, CommandError> {
        // TeX82 §440's `scan_int` calls `scan_something_internal(int_val,
        // false)`, so §413's §429 loop lowers every numeric level to an
        // integer: a dimension keeps its scaled representation, and glue or mu
        // glue first becomes its width. Plain's `\ht\z@` relies on the
        // dimension step because `\z@` is a dimension register, not a numeric
        // literal; `\ifnum\parskip>0` and `\count0=\skip3` rely on the glue
        // step.
        let internal = self.scan_something_internal(
            spelling,
            meaning,
            control_sequence,
            exceptional,
            InternalLevel::Integer,
            false,
        )?;
        let value = match internal {
            InternalScan::Value(InternalValue::Integer(value)) => value,
            InternalScan::Value(_) => {
                unreachable!("TeX82 §429 lowers an int_val request to an integer")
            }
            InternalScan::NotInternal => match scalar_meaning(meaning.resolve()) {
                // TeX's `\` character-code form consumes its following token
                // through raw delivery: that token supplies a character code,
                // rather than participating in ordinary expansion.  The
                // optional following space remains an expanded scanner token.
                Meaning::CharToken {
                    ch: '`',
                    cat: Catcode::Other,
                } => {
                    let (value, optional_space) = self.scan_character_code()?;
                    if optional_space && let Err(error) = self.scan_optional_space() {
                        return Err(error);
                    }
                    value
                }
                _ => {
                    // §444's vacuous constant backs up its offender before reporting.
                    let site = self.capture_consumed_site(
                        spelling,
                        meaning,
                        control_sequence,
                        exceptional,
                    );
                    self.back_consumed_word(spelling, exceptional)?;
                    self.missing_number_error_at(Some(site))?;
                    return Ok(self.inserted_zero_integer(provenance));
                }
            },
        };
        let scanned = self.finish_integer(value, negative, provenance);
        Ok(scanned)
    }

    pub(super) fn scan_dimension_from_leading(
        &mut self,
        mut negative: bool,
        allow_infinite: bool,
        mu: bool,
    ) -> Result<(Scaled, Order), CommandError> {
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
        let mut scanned = None;
        self.consume_expanded_words(&mut crate::terminal_consumer::<Self, G, _>(
            |processor, spelling, meaning, _code, control_sequence, exceptional| {
                match native_character(meaning) {
                    Some((' ' | '+', _)) => {}
                    Some(('-', _)) => negative = !negative,
                    _ => {
                        scanned = Some(processor.scan_dimension_after_first(
                            spelling,
                            meaning,
                            control_sequence,
                            exceptional,
                            negative,
                            allow_infinite,
                            mu,
                        )?);
                        return Ok(std::ops::ControlFlow::Break(()));
                    }
                }
                Ok(std::ops::ControlFlow::Continue(()))
            },
        ))?;
        match scanned {
            Some(scanned) => Ok(scanned),
            None => self.finish_dimension(Scaled::from_raw(0), Order::Normal, negative, false),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn scan_dimension_after_first(
        &mut self,
        spelling: TracedTokenWord,
        meaning: &tex_state::meaning::MeaningWord<G>,
        control_sequence: Option<Symbol>,
        exceptional: Option<&CurrentCommand<G>>,
        negative: bool,
        allow_infinite: bool,
        mu: bool,
    ) -> Result<(Scaled, Order), CommandError> {
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
        let internal = self.scan_something_internal(
            spelling,
            meaning,
            control_sequence,
            exceptional,
            level,
            false,
        )?;
        match internal {
            InternalScan::Value(value) => match self.fetch_internal_dimension(value, mu)? {
                InternalDimension::Complete(value) => {
                    self.finish_dimension(value, Order::Normal, negative, false)
                }
                InternalDimension::Prefix(integer) => {
                    self.integer_terminator_character = None;
                    self.scan_dimension_units(integer, false, negative, allow_infinite, mu)
                }
            },
            // TeX82 §448's other branch: `back_input`, then either
            // `scan_int` or §448's own `radix:=10; cur_val:=0` owns the
            // numeric part, and the unit scan always follows.
            InternalScan::NotInternal => {
                let leading_point = is_point_character(native_character(meaning));
                self.back_consumed_word(spelling, exceptional)?;
                if leading_point {
                    self.consume_raw_words::<false, _>(&mut |_, _, _, _| {
                        Ok(std::ops::ControlFlow::Break(()))
                    })?;
                    self.scan_dimension_units(0, true, negative, allow_infinite, mu)
                } else {
                    self.scan_dimension_after_integer(negative, allow_infinite, mu)
                }
            }
        }
    }

    pub(super) fn scan_glue_from_leading(
        &mut self,
        mu: bool,
        mut negative: bool,
    ) -> Result<GlueSpec, CommandError> {
        // TeX82 §461's `<Get the next non-blank non-sign token>`: `scan_glue`
        // owns its own leading signs so that §430 can negate an internal
        // glue's three components as a unit (`\skip0=-\skip1`). Routing a
        // signed internal glue through the width-only dimension scanner
        // instead would drop its stretch and shrink.
        let mut scanned = None;
        self.consume_expanded_words(&mut crate::terminal_consumer::<Self, G, _>(
            |processor, spelling, meaning, _code, control_sequence, exceptional| {
                match native_character(meaning) {
                    Some((' ' | '+', _)) => {}
                    Some(('-', _)) => negative = !negative,
                    _ => {
                        scanned = Some(processor.scan_glue_after_first(
                            spelling,
                            meaning,
                            control_sequence,
                            exceptional,
                            mu,
                            negative,
                        )?);
                        return Ok(std::ops::ControlFlow::Break(()));
                    }
                }
                Ok(std::ops::ControlFlow::Continue(()))
            },
        ))?;
        match scanned {
            Some(scanned) => Ok(scanned),
            None => self.scan_glue_width(mu, negative),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn scan_glue_after_first(
        &mut self,
        spelling: TracedTokenWord,
        meaning: &tex_state::meaning::MeaningWord<G>,
        control_sequence: Option<Symbol>,
        exceptional: Option<&CurrentCommand<G>>,
        mu: bool,
        negative: bool,
    ) -> Result<GlueSpec, CommandError> {
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
        match self.scan_something_internal(
            spelling,
            meaning,
            control_sequence,
            exceptional,
            level,
            negative,
        )? {
            // §461 is the one caller that passes §413's `negative` flag, so
            // §430 negates the committed value -- all three components of a
            // glue specification together -- before §413 returns.
            // §461: `if cur_val_level>=glue_val then begin if
            // cur_val_level<>level then mu_error; return end`. The
            // specification is the complete answer, so no `plus`/`minus`
            // components follow it. §429 has already lowered a `mu_val`
            // quantity to `glue_val` (reporting `mu_error` on the way), so
            // the level test that remains here is §461's `glue_val`
            // quantity where `mu_val` was requested.
            InternalScan::Value(internal @ (InternalValue::Glue(_) | InternalValue::MuGlue(_))) => {
                if internal.level() != level {
                    self.mu_error()?;
                }
                if negative {
                    // §430 changed the specification, so the physical pointer and
                    // source-register identity captured while fetching the internal
                    // value no longer describe the scanned result. Retaining them
                    // would let e-TeX §277 mistake `\skip0=-\skip0` for an
                    // identical-pointer reassignment and discard the negation.
                    self.scanned_glue_identity = None;
                    self.scanned_glue_register = None;
                    self.scanned_glue_shared_zero = false;
                }
                let (InternalValue::Glue(glue) | InternalValue::MuGlue(glue)) = internal else {
                    unreachable!("outer pattern restricts the value to a glue specification")
                };
                self.finish_glue(glue)
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
                if negative {
                    self.scanned_glue_identity = None;
                    self.scanned_glue_register = None;
                }
                let result = self.scan_glue_plus_keyword(
                    mu,
                    GlueSpec {
                        width,
                        ..GlueSpec::ZERO
                    },
                );
                self.clear_fresh_glue_scan_origin();
                result
            }
            // §461: `if cur_val_level=int_val then scan_dimen(mu,false,
            // true)`. §430 has already negated the integer, which is the
            // numeric prefix of a units-only dimension scan.
            InternalScan::Value(InternalValue::Integer(integer)) => {
                let width = self.scan_dimension_shortcut(integer, mu)?;
                if negative {
                    self.scanned_glue_identity = None;
                    self.scanned_glue_register = None;
                }
                let result = self.scan_glue_plus_keyword(
                    mu,
                    GlueSpec {
                        width,
                        ..GlueSpec::ZERO
                    },
                );
                self.clear_fresh_glue_scan_origin();
                result
            }
            InternalScan::Value(InternalValue::Font(_) | InternalValue::Tokens { .. }) => {
                unreachable!("TeX82 §416 converts identifiers to dimen_val zero before §461")
            }
            // §461's non-internal branch: `back_input; scan_dimen(mu,
            // false,false); if negative then negate(cur_val)`.
            InternalScan::NotInternal => {
                self.back_consumed_word(spelling, exceptional)?;
                self.scan_glue_width(mu, negative)
            }
        }
    }
}
