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

use crate::attempt::{AttemptDefinitionId, AttemptTokenBufferId, AttemptTokenListId};

use crate::input::{
    BackupTreatment, InputLevelId, PackedTokenSpanHandle, ReplayTrace, RetirementBehavior,
    StoredReplayReason, TokenBehavior,
};
use crate::processor::alignment::{PREAMBLE_ALIGN_STATE, is_character_command};
use crate::processor::status::{
    AlignmentId, AlignmentScanContext, ScannerEpisode, ScannerStatus, ScannerStatusVisibility,
    ScannerWarning, TokenBuilderId,
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

pub use box_values::{
    ScannedAccent, ScannedAccentBase, ScannedBoxConstruction, ScannedBoxKind, ScannedBoxRegister,
    ScannedBoxShift, ScannedBoxShiftPayload, ScannedDiscretionaryOpening, ScannedDisplayDiagnostic,
    ScannedGlueParameterAssignment, ScannedInsertConstruction, ScannedLeaderPayload,
    ScannedPackingSpec, ScannedRuleSpec, ScannedSetBoxAssignment, ScannedSetBoxPath, ScannedVSplit,
};
pub use definition_values::{
    ExpandedWriteText, FontLoadRequest, FontSizeRecovery, GeneratedFontKind, PdfActionDestination,
    PdfActionIdentifier, PdfActionSpec, PdfActionTarget, ScannedBalancedText,
    ScannedCharacterDefinition, ScannedGeneratedFontDefinition, ScannedMacroDefinition,
    ScannedRegisterDefinition, StructuredProvenance,
};
pub use input_values::{
    AlignmentCellOpening, FileNameComponents, ImmediateExtension, InputStreamRequest,
    RegisteredInput, ScannedFileName, WriteStreamSelector,
};
use input_values::{DefinitionTargetProjection, FILE_NAME_POOL_CAPACITY};
pub use math_values::{
    EquationNumberSide, MathDelimiterBoundary, MathDelimiterBoundaryKind, MathFamilySize,
    MathFieldBody, MathFieldEpisode, MathFractionKind, MathLimitKind, MathRequest, MathScriptKind,
    MathStyleKind, MathTextFieldKind, ScannedEquationNumber, ScannedMathCharacter,
    ScannedMathDelimiter, ScannedMathFamily, ScannedMathFraction, ScannedMathMuMaterial,
    ScannedMathScript,
};
pub use pdf_values::{
    PdfAnnotationRequest, PdfColorStackActionRequest, PdfDestinationRequest,
    PdfDocumentFragmentRequest, PdfFormRequest, PdfGraphicsRequest, PdfImagePageBox,
    PdfImagePageSelection, PdfImageRequest, PdfNavigationRequest, PdfObjectRequest,
    PdfOutlineRequest, PdfReferenceObjectRequest, PdfStartLinkRequest, PdfThreadRequest,
    ScannedPdfFontAction,
};

impl<G> CommandProcessor<'_, '_, G> {
    // Structured scans run as ordinary synchronous Rust calls.  These tiny
    // adapters are kept only while the legacy per-command phase tables are
    // being collapsed; they never store or restore anything across a resource
    // boundary.
    /// Expands a frozen whatsit payload at output traversal time.
    ///
    /// The caller decides how the resulting token spellings are rendered;
    /// this operation owns only canonical replay/expansion state.
    pub fn expand_output_replay(
        &mut self,
        tokens: tex_state::TokenListId<G>,
    ) -> Result<crate::attempt::AttemptTokenListId, CommandError> {
        self.invalidate_delivery_freshness();
        let episode = self.command.push_output_replay_episode(self.state, tokens);
        let expanded = self
            .command
            .attempt
            .arena_mut()
            .allocate_token_buffer()
            .map_err(|_| CommandError::input_invariant())?;
        let mut destination = None;
        loop {
            match self.get_x_or_protected_with_replay_completion_into(&mut destination)? {
                DeliveryStatus::Command => {
                    let command = destination.take().ok_or(CommandError::input_invariant())?;
                    self.command
                        .attempt
                        .arena_mut()
                        .push_buffer_token(expanded, command.spelling())
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
            .arena_mut()
            .finish_token_buffer(expanded)
            .map_err(|_| CommandError::input_invariant())
    }

    /// Projects the packed token spelling into a definition target without
    /// decoding its effective meaning. `Token::Cs` already carries the
    /// ordinary control-sequence identity; active characters use their
    /// separate active-character map and are interned only when §1215 first
    /// needs to make that cell addressable. No fuel is charged here.
    fn project_definition_target(
        &mut self,
        command: &crate::CurrentCommand<G>,
    ) -> DefinitionTargetProjection {
        match command.spelling().semantic_token() {
            Token::Cs(symbol) => {
                if let Some(cached) = command.control_sequence() {
                    debug_assert_eq!(
                        cached, symbol,
                        "cached control-sequence metadata disagrees with token spelling"
                    );
                }
                DefinitionTargetProjection::Target(symbol)
            }
            Token::Char {
                ch,
                cat: Catcode::Active,
            } => {
                let symbol = self
                    .state
                    .active_character_symbol(ch)
                    .unwrap_or_else(|| self.state.intern_active_character(ch));
                if let Some(cached) = command.control_sequence() {
                    debug_assert_eq!(
                        cached, symbol,
                        "cached active-character metadata disagrees with active map"
                    );
                }
                DefinitionTargetProjection::Target(symbol)
            }
            _ => {
                debug_assert!(
                    command.control_sequence().is_none(),
                    "malformed definition target carries control-sequence metadata"
                );
                DefinitionTargetProjection::Malformed
            }
        }
    }

    /// TeX82 §1215's `get_r_token`, including its restart after inserting
    /// the inaccessible target. The rejected delivery is backed up, so the
    /// caller's following operand scan still owns it.
    fn scan_definition_target(&mut self) -> Result<tex_state::interner::Symbol, CommandError> {
        let mut destination = None;
        loop {
            let command = match self.next_non_space_raw_into(&mut destination)? {
                DeliveryStatus::Command => {
                    destination.take().ok_or(CommandError::input_invariant())?
                }
                DeliveryStatus::End => {
                    if self.next_non_space_raw_into(&mut destination)? != DeliveryStatus::Command {
                        return Err(CommandError::input_invariant());
                    }
                    destination.take().ok_or(CommandError::input_invariant())?
                }
                _ => return Err(CommandError::input_invariant()),
            };
            if let DefinitionTargetProjection::Target(target) =
                self.project_definition_target(&command)
            {
                return Ok(target);
            }

            // §1215 backs up an ordinary non-control token (`cur_cs=0`),
            // while an already-frozen control token is consumed before the
            // inaccessible sentinel is inserted.
            if !matches!(
                command.spelling().semantic_token(),
                tex_state::token::Token::Frozen(_)
            ) {
                self.back_input(command)?;
            }
            let inaccessible =
                Token::Cs(self.state.intern_internal_control_sequence("inaccessible"));
            // §1215's `ins_error` is §327: the synthesized token is a live
            // `inserted` level during §82's report, and `goto restart` then
            // consumes that same level as the definition target.
            self.push_inserted_error_token(inaccessible);
            let context = self.command.output_open_context(self.state);
            let mut report = self.state.print_err("Missing control sequence inserted");
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
            self.finish_error_outcome(outcome)?;
        }
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
    ) -> Result<ScannedCharacterDefinition<G>, CommandError> {
        let target = self.scan_definition_target()?;
        let provisional_old = self.state.meaning(target);
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
        self.scan_optional_equals_retained().into_result()?;
        let scanned = self.scan_restricted_integer_retained(class).into_result()?;
        Ok(ScannedCharacterDefinition {
            target,
            provisional_old,
            class,
            value: scanned.value,
            scanned: scanned.scanned,
            recovered: scanned.recovered,
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
    ) -> Result<ScannedRegisterDefinition<G>, CommandError> {
        let target = self.scan_definition_target()?;
        let provisional_old = self.state.meaning(target);
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
        self.scan_optional_equals_retained().into_result()?;
        // TeX82 §1224 uses `scan_eight_bit_int`, while e-TeX 2.6
        // etex.ch [49.1224] replaces that scan with `scan_register_num` so
        // sparse register shorthands may address 0..=32767. pdfTeX inherits
        // the same e-TeX register extension.
        let index = if self.command.profile().capabilities().supports_etex() {
            let result = self.scan_extended_register_index_retained();
            result.into_result()?
        } else {
            let result = self.scan_eight_bit_register_index_retained();
            result.into_result()?
        };
        Ok(ScannedRegisterDefinition {
            target,
            provisional_old,
            index,
        })
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
