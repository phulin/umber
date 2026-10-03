//! Forward-only TFM ligature/kern processing (TeX82 §§1034–1040).
use super::{FinalHNodeSink, PendingHRunChar, add_scaled, font_code, scaled_font_code};
use tex_fonts::{LigKernChar, LigKernCommand};
use tex_state::CommandContext;
use tex_state::diagnostic::DiagnosticEffects;
use tex_state::ids::FontId;
use tex_state::node::KernKind;
use tex_state::scaled::Scaled;
use tex_state::token::OriginId;

mod scratch;
use scratch::{LigatureBoundaryCell, ProvenanceSpan, TfmCell, TfmRunLoan};
pub(crate) use scratch::{LigatureGlyphCell, LigatureSourceEntry, TfmScratch};

#[derive(Clone, Copy)]
pub(crate) enum LigatureRightBoundary {
    Suppressed,
    Font,
    Character(u8),
}

#[allow(clippy::too_many_arguments)] // Actual source, boundary, diagnostic, fuel and output owners.
pub(crate) fn run_tfm_ligature_machine_with_work<G>(
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    source: &[crate::mode::PendingHChar],
    no_left_boundary: bool,
    right_boundary: LigatureRightBoundary,
    insert_hyphen_discs: bool,
    fuel: &mut tex_command::CommandFuel,
    scratch: &mut TfmScratch,
    sink: &mut impl FinalHNodeSink,
) -> Result<(), tex_command::CommandError> {
    let loan = TfmRunLoan::new(scratch);
    run(
        stores,
        diagnostic_effects,
        source,
        no_left_boundary,
        right_boundary,
        insert_hyphen_discs,
        fuel,
        &mut *loan.scratch,
        sink,
    )
}

#[allow(clippy::too_many_arguments)] // Nested replay lends the parent's scratch tail.
pub(crate) fn run_tfm_ligature_machine_nested_with_work<G>(
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    source: &[crate::mode::PendingHChar],
    no_left_boundary: bool,
    right_boundary: LigatureRightBoundary,
    insert_hyphen_discs: bool,
    fuel: &mut tex_command::CommandFuel,
    scratch: &mut TfmScratch,
    sink: &mut impl FinalHNodeSink,
) -> Result<(), tex_command::CommandError> {
    let loan = TfmRunLoan::nested(scratch);
    run(
        stores,
        diagnostic_effects,
        source,
        no_left_boundary,
        right_boundary,
        insert_hyphen_discs,
        fuel,
        &mut *loan.scratch,
        sink,
    )
}

struct Characters<'a> {
    source: &'a [crate::mode::PendingHChar],
    position: usize,
    right_boundary: Option<LigatureBoundaryCell>,
    stack_start: usize,
}

impl Characters<'_> {
    fn next<G>(
        &mut self,
        stores: &CommandContext<'_, G>,
        scratch: &mut TfmScratch,
    ) -> Option<TfmCell> {
        if scratch.stack.len() > self.stack_start {
            return scratch.stack.pop();
        }
        if let Some(entry) = self.source.get(self.position) {
            self.position += 1;
            let provenance = scratch.append_source(entry.ch, entry.origin);
            return Some(TfmCell::Glyph(LigatureGlyphCell::new(
                stores, entry.font, entry.ch, provenance, false, false, false,
            )));
        }
        self.right_boundary.take().map(TfmCell::Boundary)
    }
}

struct Output {
    literal_hyphen: bool,
    insert_hyphen_discs: bool,
}

impl Output {
    #[allow(clippy::too_many_arguments)] // Writes into the actual sink or its required physical suffix.
    fn emit<G, Sink: FinalHNodeSink>(
        &mut self,
        stores: &mut CommandContext<'_, G>,
        effects: &mut DiagnosticEffects,
        fuel: &mut tex_command::CommandFuel,
        scratch: &mut TfmScratch,
        sink: &mut Sink,
        cell: TfmCell,
    ) -> Result<(), tex_command::CommandError> {
        if let TfmCell::Glyph(glyph) = &cell
            && glyph.discard_if_missing
            && !stores.font_character_exists(glyph.font, glyph.ch)
        {
            crate::diagnostics::report_missing_character_warning(
                stores, effects, glyph.font, glyph.ch, false,
            );
            return Ok(());
        }
        if Sink::REQUIRES_PHYSICAL_SUFFIX {
            if !matches!(cell, TfmCell::Boundary(_)) {
                scratch.output.push(Some(cell));
            }
            return Ok(());
        }
        self.write(stores, effects, fuel, scratch, sink, 0, cell)
    }

    #[allow(clippy::too_many_arguments)] // Callback retains actual provenance, diagnostics and fuel.
    fn write<G>(
        &mut self,
        stores: &mut CommandContext<'_, G>,
        effects: &mut DiagnosticEffects,
        fuel: &mut tex_command::CommandFuel,
        scratch: &mut TfmScratch,
        sink: &mut impl FinalHNodeSink,
        current: usize,
        cell: TfmCell,
    ) -> Result<(), tex_command::CommandError> {
        if !matches!(
            cell,
            TfmCell::Kern {
                kind: KernKind::Auto,
                ..
            }
        ) && self.literal_hyphen
        {
            sink.explicit_hyphen_disc(stores);
            self.literal_hyphen = false;
        }
        match cell {
            TfmCell::Boundary(_) => {}
            TfmCell::Glyph(glyph) => {
                let last = scratch
                    .source(glyph.provenance)
                    .last()
                    .map(|entry| entry.ch);
                self.literal_hyphen = self.insert_hyphen_discs
                    && stores.font_hyphen_char(glyph.font) == last.unwrap_or(glyph.ch) as i32;
                sink.glyph_cell_with_work(stores, current, glyph, scratch, effects, fuel)?;
            }
            TfmCell::Kern { amount, kind } => {
                sink.kern_with_work(stores, current, amount, kind, scratch, effects, fuel)?
            }
        }
        Ok(())
    }
}

#[allow(clippy::too_many_arguments)] // One native TFM loop, independent of the final sink's demand.
fn run<G, Sink: FinalHNodeSink>(
    stores: &mut CommandContext<'_, G>,
    effects: &mut DiagnosticEffects,
    source: &[crate::mode::PendingHChar],
    no_left_boundary: bool,
    right_boundary: LigatureRightBoundary,
    insert_hyphen_discs: bool,
    fuel: &mut tex_command::CommandFuel,
    scratch: &mut TfmScratch,
    sink: &mut Sink,
) -> Result<(), tex_command::CommandError> {
    let Some(first) = source.first() else {
        return Ok(());
    };
    let font = first.font;
    let false_bchar = stores.font_false_boundary_char(font);
    let boundary = LigatureBoundaryCell {
        code: None,
        lig_kern_start: stores.font_lig_kern_start(font, LigKernChar::Boundary),
        leading_auto_kern: Scaled::from_raw(0),
    };
    let mut characters = Characters {
        source,
        position: 0,
        stack_start: scratch.stack.len(),
        right_boundary: match right_boundary {
            LigatureRightBoundary::Suppressed => None,
            LigatureRightBoundary::Font => Some(boundary),
            LigatureRightBoundary::Character(code) => Some(LigatureBoundaryCell {
                code: Some(code),
                leading_auto_kern: auto_kern_values(stores, font, Some(code)).0,
                ..boundary
            }),
        },
    };
    let output_start = scratch.output.len();
    let mut output = Output {
        literal_hyphen: false,
        insert_hyphen_discs,
    };
    let mut left = if no_left_boundary {
        characters
            .next(stores, scratch)
            .expect("nonempty source owns first glyph")
    } else {
        TfmCell::Boundary(boundary)
    };
    while let Some(mut right) = characters.next(stores, scratch) {
        fuel.charge()?;
        let pair = match (&left, &right) {
            (TfmCell::Boundary(_), TfmCell::Glyph(right)) => right
                .code
                .map(|right| (LigKernChar::Boundary, LigKernChar::Char(right))),
            (TfmCell::Glyph(left), TfmCell::Boundary(right)) => left.code.map(|left| {
                (
                    LigKernChar::Char(left),
                    right.code.map_or(LigKernChar::Boundary, LigKernChar::Char),
                )
            }),
            (TfmCell::Glyph(left), TfmCell::Glyph(right)) if left.font == right.font => left
                .code
                .zip(right.code)
                .map(|(left, right)| (LigKernChar::Char(left), LigKernChar::Char(right))),
            _ => None,
        };
        let false_boundary_match = matches!(&right, TfmCell::Glyph(right) if right.font == font && right.code.is_some_and(|code| Some(code) == false_bchar));
        if false_boundary_match && let TfmCell::Glyph(right) = &mut right {
            right.discard_if_missing = true;
        }
        let Some((_, right_code)) = pair else {
            output.emit(stores, effects, fuel, scratch, sink, left)?;
            left = right;
            continue;
        };
        if false_boundary_match {
            if let TfmCell::Glyph(right) = &right
                && !stores.font_character_exists(right.font, right.ch)
            {
                crate::diagnostics::report_missing_character_warning(
                    stores, effects, right.font, right.ch, false,
                );
                break;
            }
            output.emit(stores, effects, fuel, scratch, sink, left)?;
            left = right;
            continue;
        }
        if let Some(amount) = auto_kern(&left, &right) {
            output.emit(stores, effects, fuel, scratch, sink, left)?;
            output.emit(
                stores,
                effects,
                fuel,
                scratch,
                sink,
                TfmCell::Kern {
                    amount,
                    kind: KernKind::Auto,
                },
            )?;
            left = right;
            continue;
        }
        let start = match &left {
            TfmCell::Boundary(boundary) => boundary.lig_kern_start,
            TfmCell::Glyph(glyph) => glyph.lig_kern_start,
            TfmCell::Kern { .. } => None,
        };
        let command = start
            .and_then(|start| stores.font_lig_kern_command_from_start(font, start, right_code));
        match command {
            None => {
                output.emit(stores, effects, fuel, scratch, sink, left)?;
                left = right;
            }
            Some(LigKernCommand::Kern(amount)) => {
                output.emit(stores, effects, fuel, scratch, sink, left)?;
                output.emit(
                    stores,
                    effects,
                    fuel,
                    scratch,
                    sink,
                    TfmCell::Kern {
                        amount,
                        kind: KernKind::Font,
                    },
                )?;
                left = right;
            }
            Some(LigKernCommand::Ligature(lig)) => {
                let (left_span, left_hit, left_boundary, left_discard) = match &left {
                    TfmCell::Glyph(glyph) => (
                        glyph.provenance,
                        glyph.left_hit,
                        false,
                        glyph.discard_if_missing,
                    ),
                    TfmCell::Boundary(_) => (ProvenanceSpan::EMPTY, false, true, false),
                    TfmCell::Kern { .. } => unreachable!(),
                };
                let (right_span, right_hit, right_boundary, right_discard) = match &right {
                    TfmCell::Glyph(glyph) => (
                        glyph.provenance,
                        glyph.right_hit,
                        false,
                        glyph.discard_if_missing,
                    ),
                    TfmCell::Boundary(_) => (ProvenanceSpan::EMPTY, false, true, false),
                    TfmCell::Kern { .. } => unreachable!(),
                };
                let provenance = scratch.merge_source_spans(
                    if lig.delete_current {
                        left_span
                    } else {
                        ProvenanceSpan::EMPTY
                    },
                    if lig.delete_next {
                        right_span
                    } else {
                        ProvenanceSpan::EMPTY
                    },
                );
                let mut replacement = LigatureGlyphCell::new(
                    stores,
                    font,
                    char::from(lig.replacement),
                    provenance,
                    true,
                    left_boundary || left_hit,
                    right_boundary || right_hit,
                );
                replacement.discard_if_missing =
                    (lig.delete_current && left_discard) || (lig.delete_next && right_discard);
                if !lig.delete_next {
                    scratch.stack.push(right);
                }
                if lig.delete_current {
                    left = TfmCell::Glyph(replacement);
                } else {
                    scratch.stack.push(TfmCell::Glyph(replacement));
                }
                let op_byte = lig.pass_over * 4
                    + u8::from(!lig.delete_current) * 2
                    + u8::from(!lig.delete_next);
                for _ in 0..match op_byte {
                    5..=7 => 1,
                    11 => 2,
                    _ => 0,
                } {
                    output.emit(stores, effects, fuel, scratch, sink, left)?;
                    let Some(next) = characters.next(stores, scratch) else {
                        return finish(
                            stores,
                            effects,
                            fuel,
                            scratch,
                            sink,
                            output_start,
                            &mut output,
                        );
                    };
                    left = next;
                }
            }
        }
    }
    output.emit(stores, effects, fuel, scratch, sink, left)?;
    // A missing false boundary ends pair processing. The untouched suffix still
    // belongs to the pending run and must be emitted without extra fuel charges.
    while let Some(cell) = characters.next(stores, scratch) {
        output.emit(stores, effects, fuel, scratch, sink, cell)?;
    }
    finish(
        stores,
        effects,
        fuel,
        scratch,
        sink,
        output_start,
        &mut output,
    )
}

fn finish<G, Sink: FinalHNodeSink>(
    stores: &mut CommandContext<'_, G>,
    effects: &mut DiagnosticEffects,
    fuel: &mut tex_command::CommandFuel,
    scratch: &mut TfmScratch,
    sink: &mut Sink,
    start: usize,
    output: &mut Output,
) -> Result<(), tex_command::CommandError> {
    if Sink::REQUIRES_PHYSICAL_SUFFIX {
        let end = scratch.output.len();
        for current in start..end {
            let cell = scratch.output[current]
                .take()
                .expect("completed physical suffix owns its cell");
            output.write(stores, effects, fuel, scratch, sink, current, cell)?;
        }
    }
    if output.literal_hyphen {
        sink.explicit_hyphen_disc(stores);
    }
    Ok(())
}

fn auto_kern(left: &TfmCell, right: &TfmCell) -> Option<Scaled> {
    let amount = match (left, right) {
        (TfmCell::Boundary(_), TfmCell::Glyph(glyph)) => glyph.leading_auto_kern,
        (TfmCell::Glyph(glyph), TfmCell::Boundary(boundary)) => {
            add_scaled(glyph.trailing_auto_kern, boundary.leading_auto_kern)
        }
        (TfmCell::Glyph(left), TfmCell::Glyph(right)) if left.font == right.font => {
            add_scaled(left.trailing_auto_kern, right.leading_auto_kern)
        }
        (TfmCell::Glyph(glyph), TfmCell::Glyph(_)) => glyph.trailing_auto_kern,
        _ => Scaled::from_raw(0),
    };
    (amount.raw() != 0).then_some(amount)
}

fn auto_kern_values<G>(
    stores: &CommandContext<'_, G>,
    font: FontId,
    code: Option<u8>,
) -> (Scaled, Scaled) {
    let configuration = stores.pdf_font_configuration();
    let leading = if configuration.prepends_kerns() {
        code.map_or(Scaled::from_raw(0), |code| {
            scaled_font_code(
                stores,
                font,
                stores.pdf_font_code(tex_state::PdfFontCode::Knbc, font, code),
            )
        })
    } else {
        Scaled::from_raw(0)
    };
    let trailing = if configuration.appends_kerns() {
        code.map_or(Scaled::from_raw(0), |code| {
            scaled_font_code(
                stores,
                font,
                stores.pdf_font_code(tex_state::PdfFontCode::Knac, font, code),
            )
        })
    } else {
        Scaled::from_raw(0)
    };
    (leading, trailing)
}

#[cfg(test)]
mod tests;
