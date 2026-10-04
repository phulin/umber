//! Executor-facing structured scanners owned by the command input machine.
//!
//! These wrappers intentionally expose frozen values, provenance, and the
//! canonical filename scanning only. Input levels, raw tokens, and macro
//! argument frames remain private to `tex-command`.

use core::marker::PhantomData;

use tex_state::glue::GlueSpec;
use tex_state::ids::FontId;
use tex_state::interner::Symbol;
use tex_state::meaning::{Meaning, ResolvedMeaning, UnexpandablePrimitive};
use tex_state::scaled::{FontSizeSpec, Scaled};
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};
use tex_state::{
    SourceId,
    env::banks::{GlueParam, IntParam},
};

use crate::attempt::{AttemptTokenBufferId, AttemptTokenListId};

use crate::input::{
    BackupTreatment, InputLevelId, ReplayTrace, RetirementBehavior, StoredReplayReason,
    TokenBehavior,
};
use crate::processor::alignment::{PREAMBLE_ALIGN_STATE, is_character_command};
use crate::processor::status::{
    AlignmentScanContext, ScannerEpisode, ScannerStatus, ScannerStatusVisibility, TokenBuilderId,
};
use crate::scan_toks::{ScanToksMode, ScannedToks};
use crate::scanners::RestrictedIntegerClass;
use crate::{
    AlignmentCellTemplates, AlignmentIdentity, AlignmentPreamble, CommandError, CommandProcessor,
    CurrentCommand, InternalValue,
    processor::{
        DeliveryStatus, print_cs_text, render_the_value, selector_meaning_text, string_text,
    },
};

use internal_state::{
    AlignmentPreamblePhase, AlignmentPreambleScalar, AlignmentPreambleScalarPhase,
    AlignmentPreambleState, EXTRA_PARAMETER_DIAGNOSTIC, MISSING_DELIMITER_DIAGNOSTIC,
    MISSING_DELIMITER_HELP, MISSING_PARAMETER_DIAGNOSTIC, MathFieldRestrictedKind,
    PackingScalarPhase, PdfActionScalarPhase, PdfActionScalarProgress, PdfFormScalarPhase,
    PdfFormScalarProgress, PdfGraphicsScalarPhase, PdfImageScalarPhase, PdfImageScalarProgress,
    PdfNavigationScalarPhase, PdfNavigationScalarProgress, PdfObjectScalarPhase,
    PdfObjectScalarProgress, PendingPdfActionOwner, PendingPdfColorStackAction,
    PendingPdfImagePage, RuleScalarPhase, static_meaning,
};
mod box_values;
mod definition_values;
mod input_values;
mod internal_state;
mod math_values;
mod pdf_values;
mod token_requests;
pub use token_requests::TokenRequest;

pub use box_values::{
    ScannedAccent, ScannedAccentBase, ScannedBoxConstruction, ScannedBoxKind, ScannedBoxRegister,
    ScannedBoxShift, ScannedBoxShiftPayload, ScannedDiscretionaryOpening, ScannedDisplayDiagnostic,
    ScannedInsertConstruction, ScannedLeaderPayload, ScannedPackingSpec, ScannedRuleSpec,
    ScannedSetBoxAssignment, ScannedSetBoxPath, ScannedVSplit,
};
pub use definition_values::{
    ExpandedWriteText, FontLoadRequest, FontSizeRecovery, GeneratedFontKind, PdfActionDestination,
    PdfActionIdentifier, PdfActionSpec, PdfActionTarget, ScannedBalancedText,
    ScannedCharacterDefinition, ScannedGeneratedFontDefinition, ScannedMacroDefinition,
    ScannedRegisterDefinition, StructuredProvenance,
};
use input_values::FILE_NAME_POOL_CAPACITY;
pub use input_values::{
    AlignmentCellOpening, FileNameComponents, ImmediateExtension, InputStreamRequest,
    RegisteredInput, ScannedFileName, WriteStreamSelector,
};
pub use math_values::{
    MathDelimiterBoundary, MathDelimiterBoundaryKind, MathFamilySize, MathFieldBody,
    MathFieldEpisode, MathFractionKind, MathScriptKind, ScannedMathCharacter, ScannedMathDelimiter,
    ScannedMathFraction,
};
pub use pdf_values::{
    PdfAnnotationRequest, PdfColorStackActionRequest, PdfDestinationRequest,
    PdfDocumentFragmentRequest, PdfFormRequest, PdfGraphicsRequest, PdfImagePageBox,
    PdfImagePageSelection, PdfImageRequest, PdfNavigationRequest, PdfObjectRequest,
    PdfOutlineRequest, PdfReferenceObjectRequest, PdfStartLinkRequest, PdfThreadRequest,
    ScannedPdfFontAction,
};

impl<G, Facts: crate::CommandHostFacts<G>> CommandProcessor<'_, '_, G, Facts> {
    // Structured scans run synchronously through the sole command processor.
    // Their typed results cross to the executor only after scanning completes;
    // no scanner continuation survives a resource boundary.
    /// Expands a frozen whatsit payload at output traversal time.
    ///
    /// The caller decides how the resulting token spellings are rendered;
    /// this operation owns only canonical replay/expansion state.
    pub fn expand_output_replay(
        &mut self,
        tokens: tex_state::TokenListId<G>,
    ) -> Result<crate::attempt::AttemptTokenListId, CommandError> {
        let episode = self.command.push_output_replay_episode(self.state, tokens);
        let expanded = self
            .command
            .attempt
            .allocate_token_buffer()
            .map_err(|_| CommandError::input_invariant())?;
        let mut destination = None;
        loop {
            match self.get_x_or_protected_with_replay_completion_into(&mut destination)? {
                DeliveryStatus::Command => {
                    let command = destination
                        .take()
                        .ok_or_else(|| CommandError::input_invariant())?;
                    self.command
                        .attempt
                        .push_buffer_token(&expanded, command.spelling())
                        .map_err(|_| CommandError::input_invariant())?;
                }
                DeliveryStatus::ReplayCompleted(completed) if completed == episode => break,
                DeliveryStatus::ReplayCompleted(_) => continue,
                DeliveryStatus::End => return Err(CommandError::input_invariant()),
                _ => return Err(CommandError::input_invariant()),
            }
        }
        self.command
            .attempt
            .finish_token_buffer(expanded)
            .map_err(|_| CommandError::input_invariant())
    }

    /// The raw spelling selects the actual definition cell.
    fn project_definition_target(
        &mut self,
        spelling: tex_state::token::TokenWord,
    ) -> Option<tex_state::interner::Symbol> {
        match spelling.semantic_token() {
            Token::Cs(symbol) => Some(symbol),
            Token::Char {
                ch,
                cat: Catcode::Active,
            } => Some(
                self.state
                    .active_character_symbol(ch)
                    .unwrap_or_else(|| self.state.intern_active_character(ch)),
            ),
            _ => None,
        }
    }

    /// TeX82 §1215's `get_r_token`, including its restart after inserting
    /// the inaccessible target. The rejected delivery is backed up, so the
    /// caller's following operand scan still owns it.
    fn scan_definition_target(&mut self) -> Result<tex_state::interner::Symbol, CommandError> {
        let mut consume =
            |processor: &mut Self,
             spelling: TracedTokenWord,
             _: Option<&tex_state::meaning::MeaningWord<G>>,
             exceptional: Option<&crate::CurrentCommand<G>>| {
                if matches!(
                    spelling.semantic_token(),
                    Token::Char {
                        cat: Catcode::Space,
                        ..
                    }
                ) {
                    return Ok(std::ops::ControlFlow::Continue(()));
                }
                if let Some(target) = processor.project_definition_target(spelling.token_word()) {
                    return Ok(std::ops::ControlFlow::Break(target));
                }
                // §1215 backs up an ordinary non-control token (`cur_cs=0`),
                // while an already-frozen control token is consumed before the
                // inaccessible sentinel is inserted.
                if !matches!(
                    spelling.semantic_token(),
                    tex_state::token::Token::Frozen(_)
                ) {
                    processor.back_consumed_word(spelling, exceptional)?;
                }
                let inaccessible = Token::Cs(
                    processor
                        .state
                        .intern_internal_control_sequence("inaccessible"),
                );
                // §1215's `ins_error` is §327: the synthesized token is a live
                // `inserted` level during §82's report, and `goto restart` then
                // consumes that same level as the definition target.
                processor.push_inserted_error_token(inaccessible);
                let context = processor.command.output_open_context(processor.state);
                let mut report = processor
                    .state
                    .print_err("Missing control sequence inserted");
                report
                    .help(&[
                        "Please don't say `\\def cs{...}', say `\\def\\cs{...}'.",
                        "I've inserted an inaccessible control sequence so that your",
                        "definition will be completed without mixing me up too badly.",
                        "You can recover graciously from this error, if you're",
                        "careful; see exercise 27.2 in The TeXbook.",
                    ])
                    .context(context);
                let outcome = report.error();
                processor.finish_error_outcome(outcome)?;
                Ok(std::ops::ControlFlow::Continue(()))
            };
        if let Some(target) = self.consume_raw_words::<false, _>(&mut consume)? {
            return Ok(target);
        }
        self.consume_raw_words::<false, _>(&mut consume)?
            .ok_or_else(CommandError::input_invariant)
    }

    /// Scans TeX82 §1224's complete `\\chardef` or `\\mathchardef` operand.
    ///
    /// The target remains a raw control-sequence delivery as required by
    /// `get_r_token`; the optional equals sign and numeric value use the
    /// canonical command-owned scalar scanners. §1224 spells the value scan
    /// as `char_def_code: scan_char_num` and `math_char_def_code:
    /// scan_fifteen_bit_int`, so the class-specific bound and its
    /// recover-to-zero belong to this scan and not to the assignment that
    /// consumes it.
    pub fn scan_character_definition(
        &mut self,
        class: RestrictedIntegerClass,
        provisional_global: bool,
        trace: impl FnOnce(
            &mut tex_state::CommandContext<'_, G>,
            &mut tex_state::diagnostic::DiagnosticEffects,
            Token,
            ResolvedMeaning<G>,
            Meaning,
            bool,
        ),
    ) -> Result<ScannedCharacterDefinition, CommandError> {
        let target = self.scan_definition_target()?;
        trace(
            self.state,
            self.diagnostic_effects,
            Token::Cs(target),
            self.state.meaning(target),
            Meaning::Relax,
            provisional_global,
        );
        self.state
            .set_provisional_meaning(target, Meaning::Relax, provisional_global);
        observe!(
            self,
            crate::CommandObservation::Mutation(crate::MutationRecord {
                target: crate::MutationTarget::Meaning,
                key: crate::ObservationValue::Name(self.state.resolve(target).to_owned()),
                value: crate::ObservationValue::Name("relax".into()),
                global: provisional_global,
            }),
        );
        self.scan_optional_equals()?;
        let scanned = self.scan_restricted_integer(class)?;
        Ok(ScannedCharacterDefinition {
            target,
            class,
            value: scanned.value,
        })
    }

    /// Scans TeX82 §1224's complete register-definition operand.
    ///
    /// As in §1224, TeX temporarily gives the target `\relax` before the
    /// index scan. This makes a repeated target terminate its own integer
    /// scan rather than expand its previous meaning or report undefined.
    pub fn scan_register_definition(
        &mut self,
        provisional_global: bool,
        trace: impl FnOnce(
            &mut tex_state::CommandContext<'_, G>,
            &mut tex_state::diagnostic::DiagnosticEffects,
            Token,
            ResolvedMeaning<G>,
            Meaning,
            bool,
        ),
    ) -> Result<ScannedRegisterDefinition, CommandError> {
        let target = self.scan_definition_target()?;
        trace(
            self.state,
            self.diagnostic_effects,
            Token::Cs(target),
            self.state.meaning(target),
            Meaning::Relax,
            provisional_global,
        );
        self.state
            .set_provisional_meaning(target, Meaning::Relax, provisional_global);
        observe!(
            self,
            crate::CommandObservation::Mutation(crate::MutationRecord {
                target: crate::MutationTarget::Meaning,
                key: crate::ObservationValue::Name(self.state.resolve(target).to_owned()),
                value: crate::ObservationValue::Name("relax".into()),
                global: provisional_global,
            }),
        );
        self.scan_optional_equals()?;
        // TeX82 §1224 uses `scan_eight_bit_int`, while e-TeX 2.6
        // etex.ch [49.1224] replaces that scan with `scan_register_num` so
        // sparse register shorthands may address 0..=32767. pdfTeX inherits
        // the same e-TeX register extension.
        let index = if self.command.profile().capabilities().supports_etex() {
            let result = self.scan_extended_register_index();
            result?
        } else {
            let result = self.scan_eight_bit_register_index();
            result?
        };
        Ok(ScannedRegisterDefinition { target, index })
    }
}

mod alignment;
mod boxes;
mod character;
mod definitions;
mod filename;
mod math;
mod pdf_action;
mod pdf_graphics;
mod pdf_image;
mod pdf_navigation;
mod pdf_resources;
mod streams_fonts;
mod write;

fn provenance(scanned: &ScannedToks) -> StructuredProvenance {
    StructuredProvenance {
        primary: scanned.primary,
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
#[path = "filename/tests.rs"]
mod filename_tests;
