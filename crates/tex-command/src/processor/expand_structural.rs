//! Structural TeX expansion primitives.

use tex_state::meaning::{ExpandablePrimitive, Meaning};
use tex_state::token::{OriginId, Token, TracedTokenWord};

use crate::CommandError;
use crate::command::CurrentCommand;
use crate::execution_scratch::{ArgumentRunAdmission, CsnameNameSink};
use crate::input::BackupTreatment;
use crate::processor::status::{ScannerStatus, ScannerStatusVisibility};

use super::expand_render::print_esc_text;
use super::{CommandProcessor, DeliveryStatus};

const END_CS_NAME_WORD: u64 = Meaning::ExpandablePrimitive(ExpandablePrimitive::EndCsName).encode();

/// Stable pending-diagnostic identity for TeX.web's `Missing \\endcsname
/// inserted` recovery. Rendering belongs to the diagnostic milestone.
pub(crate) const MISSING_ENDCSNAME_DIAGNOSTIC: u64 = 0x6373_6e61_6d65_0001;

impl<G, Facts: crate::CommandHostFacts<G>> CommandProcessor<'_, '_, G, Facts> {
    /// TeX.web's `\noexpand`: read normally, then replay exactly one target
    /// from a backed-up level carrying the non-sticky suppression treatment.
    pub(super) fn expand_noexpand(&mut self) -> Result<(), CommandError> {
        let episode = (!matches!(self.command.scanner, ScannerStatus::Normal)).then(|| {
            self.begin_scanner_episode(ScannerStatus::Normal, ScannerStatusVisibility::Observed)
        });
        let mut adjustment = super::AlignmentDeliveryAdjustment::None;
        let mut observed = None;
        let result = self.read_backup_operand(&mut adjustment, &mut observed);
        if let Some(episode) = episode {
            self.finish_scanner_episode(episode);
        }
        let spelling = result?;
        self.back_retained_word(
            spelling,
            adjustment,
            observed,
            BackupTreatment::SuppressExpandableControlSequence,
        )
    }

    /// Reads one token with TeX82's temporary `scanner_status := normal`
    /// scope, restoring the complete prior scanner state before returning.
    ///
    /// Both `\noexpand` (§25) and `conv_toks`'s `\string`/`\meaning` cases
    /// (§27) need this scope: their operand is delivered normally even while
    /// an enclosing `\edef` is collecting replacement text.
    pub(super) fn get_token_with_normal_scanner_status_into(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        if matches!(self.command.scanner, ScannerStatus::Normal) {
            return self.get_token_into(destination);
        }

        let episode =
            self.begin_scanner_episode(ScannerStatus::Normal, ScannerStatusVisibility::Observed);
        let delivery = self.get_token_into(destination);
        self.finish_scanner_episode(episode);
        delivery
    }

    /// TeX.web's `\expandafter`: preserve the first token, expand (or back
    /// up) the second token, then put the first token above the resulting
    /// input. The first delivery is intentionally replayed through an
    /// explicit backed-up level because it is no longer the latest delivery.
    pub(super) fn expand_expandafter(&mut self) -> Result<(), CommandError> {
        let mut adjustment = super::AlignmentDeliveryAdjustment::None;
        let mut observed_spelling = None;
        let first = self.read_backup_operand(&mut adjustment, &mut observed_spelling)?;
        self.expand_next_token()?;
        self.back_retained_word(
            first,
            adjustment,
            observed_spelling,
            BackupTreatment::Ordinary,
        )
    }

    /// Collects TeX82 §372's expanded character list through `\\endcsname`.
    ///
    /// e-TeX 2.6 etex.ch [17.4765--4779] deliberately reuses this exact
    /// name-building scan for `\\ifcsname`; only the subsequent hash-table
    /// operation differs.
    pub(crate) fn scan_csname_characters(
        &mut self,
        mut name: String,
    ) -> Result<String, CommandError> {
        // pdfTeX section 57 saves and restores the prior flag so nested name
        // scans remain true to ifincsname and unwind to their caller.
        let previous = std::mem::replace(&mut self.is_in_csname, true);
        let result = (|| {
            let mut completed = false;
            while !self.is_observed() && self.consume_csname_run(&mut name)? {}
            self.consume_expanded_words(&mut crate::terminal_consumer::<Self, G>(
                &mut |processor, spelling, meaning, _code, control_sequence, exceptional| {
                    if *meaning == tex_state::meaning::MeaningWord::scalar(END_CS_NAME_WORD) {
                        completed = true;
                        return Ok(std::ops::ControlFlow::Break(()));
                    }
                    if control_sequence.is_none()
                        && let Some((ch, _)) = meaning.character()
                    {
                        name.push(ch);
                        while !processor.is_observed() && processor.consume_csname_run(&mut name)? {
                        }
                        return Ok(std::ops::ControlFlow::Continue(()));
                    }
                    let site = processor.capture_consumed_site(
                        spelling,
                        meaning,
                        control_sequence,
                        exceptional,
                    );
                    let rendered = print_esc_text(processor.state, "endcsname");
                    processor.back_consumed_word(spelling, exceptional)?;
                    processor.report_backed_error(
                        site,
                        MISSING_ENDCSNAME_DIAGNOSTIC,
                        format!("Missing {rendered} inserted"),
                        &[
                            "The control sequence marked <to be read again> should",
                            "not appear between \\csname and \\endcsname.",
                        ],
                    )?;
                    completed = true;
                    Ok(std::ops::ControlFlow::Break(()))
                },
            ))?;
            if completed {
                Ok(name)
            } else {
                Err(CommandError::input_invariant())
            }
        })();
        if let Ok(name) = &result {
            self.command
                .record_csname_buffer_usage(name.chars().count());
        }
        self.is_in_csname = previous;
        result
    }

    /// Appends the literal-character prefix of the current macro body or
    /// argument to a `\csname` name in one resident run. Braces, alignment
    /// tabs, active characters, and control sequences stay with the scalar
    /// scan, which owns their `align_state` and expansion effects. Returns
    /// whether a settled transition allows another run before scalar delivery.
    fn consume_csname_run(&mut self, name: &mut String) -> Result<bool, CommandError> {
        let run = self.command.consume_plain_resident_run(
            &mut CsnameNameSink { name },
            self.fuel,
            ArgumentRunAdmission::characters(),
        )?;
        // A run that stopped before a refused word leaves it to the scalar
        // scan; one that drained its frame may resume on the next.
        if run.at_transition {
            return self.settle_resident_run_boundary();
        }
        Ok(false)
    }

    /// TeX82 section 372's complete `\csname` expansion.  The character
    /// collector owns its local name and the ordinary expanded-token scanner
    /// owns every nested expansion; no caller or operand phase is retained in
    /// command scratch.
    pub(super) fn expand_csname(&mut self, opener: OriginId) -> Result<(), CommandError> {
        let buffer = self.command.take_csname_scratch();
        let name = self.scan_csname_characters(buffer)?;
        let symbol = self.state.intern_relaxed_control_sequence(&name);
        self.command.recycle_csname_scratch(name);
        self.back_input_token(TracedTokenWord::pack(Token::Cs(symbol), opener))
    }
}
