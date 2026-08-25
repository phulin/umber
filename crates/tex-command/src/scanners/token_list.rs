//! Executor-facing token-list assignment scans.
//!
//! The command processor owns all operand consumption for these assignments:
//! register numbers, the optional equals sign, and `scan_toks` collection.
//! Replay receives only the frozen completed request and can therefore apply
//! the aggregate mutation without acquiring a second input path.

use tex_state::env::banks::TokParam;
use tex_state::interner::Symbol;
use tex_state::meaning::{Meaning, ResolvedMeaning, UnexpandablePrimitive};
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};

use super::structured::{
    PendingStructuredScalarPhase, PendingStructuredScanner, PendingStructuredScannerPhase,
    PendingTokenListOwner, StructuredScannerChildDestination,
};
use crate::scan_toks::ScanToksMode;
use crate::{AttemptTokenListId, CommandError, CommandProcessor};

/// A completed TeX token-register assignment operand.
///
/// The register number follows the active profile: TeX82's eight-bit bound or
/// e-TeX's 15-bit sparse-register bound. The token list is already frozen by
/// the command-owned collector or copied from an internal token-list value.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScannedTokenRegisterAssignment<G> {
    pub index: u16,
    pub tokens: Option<AttemptTokenListId>,
    pub source: Option<tex_state::TokenListId<G>>,
}

/// A completed TeX token-parameter assignment operand.
///
/// `None` is tex.web's null token-list pointer. `Some` deliberately does not
/// imply a nonempty list: §1226 copies a present source pointer even when its
/// list is empty, while a newly scanned empty braced list becomes null.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScannedTokenParameterAssignment<G> {
    pub tokens: Option<AttemptTokenListId>,
    pub source: Option<tex_state::TokenListId<G>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ScannedTokenListRightHandSide<G> {
    tokens: Option<AttemptTokenListId>,
    source: Option<tex_state::TokenListId<G>>,
    pointer_present: bool,
}

impl<G> CommandProcessor<'_, '_, G> {
    /// Scans the operand sequence of TeX82's `\toks` assignment.
    ///
    /// This follows TeX82 §§403/470's register scan, optional equals, and
    /// internal-token-list branch before unexpanded `scan_toks`; e-TeX 2.6
    /// [49.1226] widens the target scan to `scan_register_num`. A non-internal
    /// RHS is backed up before the collector begins, preserving
    /// `scan_left_brace` recovery.
    pub fn scan_token_register_assignment(
        &mut self,
        owner: Symbol,
    ) -> Result<ScannedTokenRegisterAssignment<G>, CommandError> {
        let pending = self.take_pending_structured_scanner()?;
        let (index, equals_done, mut child) = match pending {
            Some(PendingStructuredScanner {
                phase:
                    PendingStructuredScannerPhase::Scalar(
                        PendingStructuredScalarPhase::TokenRegisterIndex {
                            owner: retained_owner,
                        },
                    ),
                child,
            }) if retained_owner == owner => {
                let mut child = child;
                self.restore_structured_scanner_child(
                    &mut child,
                    StructuredScannerChildDestination::Scalar,
                )?;
                let result = self.scan_profile_register_index_retained();
                let index = self.retain_structured_scalar(
                    result,
                    PendingStructuredScalarPhase::TokenRegisterIndex { owner },
                )?;
                (index, false, None)
            }
            Some(PendingStructuredScanner {
                phase:
                    PendingStructuredScannerPhase::Scalar(
                        PendingStructuredScalarPhase::TokenListEquals(
                            PendingTokenListOwner::Register {
                                owner: retained_owner,
                                index,
                            },
                        ),
                    ),
                child,
            }) if retained_owner == owner => (index, false, child),
            Some(PendingStructuredScanner {
                phase:
                    PendingStructuredScannerPhase::TokenListRightHandSide(
                        PendingTokenListOwner::Register {
                            owner: retained_owner,
                            index,
                        },
                    ),
                child,
            }) if retained_owner == owner => (index, true, child),
            Some(PendingStructuredScanner {
                phase:
                    PendingStructuredScannerPhase::Scalar(
                        PendingStructuredScalarPhase::TokenListRhsRegister(
                            PendingTokenListOwner::Register {
                                owner: retained_owner,
                                index,
                            },
                        ),
                    ),
                child,
            }) if retained_owner == owner => (index, true, child),
            Some(mut pending) => {
                if let Some(child) = pending.take_child() {
                    self.abort_continuation(child)?;
                }
                return Err(CommandError::input_invariant());
            }
            None => {
                let result = self.scan_profile_register_index_retained();
                let index = self.retain_structured_scalar(
                    result,
                    PendingStructuredScalarPhase::TokenRegisterIndex { owner },
                )?;
                (index, false, None)
            }
        };
        let pending_owner = PendingTokenListOwner::Register { owner, index };
        if !equals_done {
            self.restore_structured_scanner_child(
                &mut child,
                StructuredScannerChildDestination::Scalar,
            )?;
            let result = self.scan_optional_equals_retained();
            self.retain_structured_scalar(
                result,
                PendingStructuredScalarPhase::TokenListEquals(pending_owner),
            )?;
        }
        let scalar_child = child
            .as_ref()
            .is_some_and(|child| child.destination() == StructuredScannerChildDestination::Scalar);
        self.restore_structured_scanner_child(
            &mut child,
            if scalar_child {
                StructuredScannerChildDestination::Scalar
            } else {
                StructuredScannerChildDestination::TokenListRightHandSide
            },
        )?;
        let value =
            match self.scan_token_list_right_hand_side(owner, false, pending_owner, scalar_child) {
                Ok(value) => value,
                Err(error) => {
                    if error.is_resource_suspension()
                        && !self
                            .scanner_resume
                            .as_ref()
                            .is_some_and(crate::ScannerFrameKey::is_structured_scanner)
                    {
                        self.retain_structured_scanner(
                            PendingStructuredScannerPhase::TokenListRightHandSide(pending_owner),
                            StructuredScannerChildDestination::TokenListRightHandSide,
                        )?;
                    }
                    return Err(error);
                }
            };
        Ok(ScannedTokenRegisterAssignment {
            index,
            tokens: value.tokens,
            source: value.source,
        })
    }

    /// Scans the right-hand side of an already selected token register.
    ///
    /// Register shorthands installed by `\toksdef` share TeX82's exact
    /// optional-equals and internal-token-list behavior with `\toks<n>`.
    pub fn scan_token_register_value(
        &mut self,
        owner: Symbol,
    ) -> Result<
        (
            Option<AttemptTokenListId>,
            Option<tex_state::TokenListId<G>>,
        ),
        CommandError,
    > {
        let pending_owner = PendingTokenListOwner::Value { owner };
        let pending = self.take_pending_structured_scanner()?;
        let (equals_done, mut child) = match pending {
            Some(PendingStructuredScanner {
                phase:
                    PendingStructuredScannerPhase::Scalar(
                        PendingStructuredScalarPhase::TokenListEquals(retained),
                    ),
                child,
            }) if retained == pending_owner => (false, child),
            Some(PendingStructuredScanner {
                phase: PendingStructuredScannerPhase::TokenListRightHandSide(retained),
                child,
            }) if retained == pending_owner => (true, child),
            Some(PendingStructuredScanner {
                phase:
                    PendingStructuredScannerPhase::Scalar(
                        PendingStructuredScalarPhase::TokenListRhsRegister(retained),
                    ),
                child,
            }) if retained == pending_owner => (true, child),
            Some(mut pending) => {
                if let Some(child) = pending.take_child() {
                    self.abort_continuation(child)?;
                }
                return Err(CommandError::input_invariant());
            }
            None => (false, None),
        };
        if !equals_done {
            self.restore_structured_scanner_child(
                &mut child,
                StructuredScannerChildDestination::Scalar,
            )?;
            let result = self.scan_optional_equals_retained();
            self.retain_structured_scalar(
                result,
                PendingStructuredScalarPhase::TokenListEquals(pending_owner),
            )?;
        }
        let scalar_child = child
            .as_ref()
            .is_some_and(|child| child.destination() == StructuredScannerChildDestination::Scalar);
        self.restore_structured_scanner_child(
            &mut child,
            if scalar_child {
                StructuredScannerChildDestination::Scalar
            } else {
                StructuredScannerChildDestination::TokenListRightHandSide
            },
        )?;
        let value =
            match self.scan_token_list_right_hand_side(owner, false, pending_owner, scalar_child) {
                Ok(value) => value,
                Err(error) => {
                    if error.is_resource_suspension()
                        && !self
                            .scanner_resume
                            .as_ref()
                            .is_some_and(crate::ScannerFrameKey::is_structured_scanner)
                    {
                        self.retain_structured_scanner(
                            PendingStructuredScannerPhase::TokenListRightHandSide(pending_owner),
                            StructuredScannerChildDestination::TokenListRightHandSide,
                        )?;
                    }
                    return Err(error);
                }
            };
        Ok((value.tokens, value.source))
    }

    /// Scans a token-parameter assignment such as `\everypar={...}`.
    ///
    /// The delimiting braces are `scan_toks`'s own (tex.web §473: "this left
    /// brace will not be part of the token list, nor will the matching right
    /// brace that comes at the end"), so the stored value is the balanced text
    /// between them. tex.web §1226 then adds *one* enclosing brace pair back,
    /// and only for `output_routine_loc`: "For safety's sake, we place an
    /// enclosing pair of braces around an `\output` list." Every other
    /// token-list parameter -- `\everypar`, `\everymath`, `\everycr`,
    /// `\errhelp`, ... -- keeps the bare balanced text, exactly like a `\toks`
    /// register. §1226 also tests emptiness *before* enclosing, so `\output={}`
    /// reverts to the empty default rather than storing a brace pair.
    ///
    /// The command processor owns both that representation choice and
    /// optional-equals consumption, so replay receives one frozen value.
    pub fn scan_token_parameter_assignment(
        &mut self,
        parameter: TokParam,
        owner: Symbol,
    ) -> Result<ScannedTokenParameterAssignment<G>, CommandError> {
        let pending_owner = PendingTokenListOwner::Parameter { parameter, owner };
        let pending = self.take_pending_structured_scanner()?;
        let (equals_done, mut child) = match pending {
            Some(PendingStructuredScanner {
                phase:
                    PendingStructuredScannerPhase::Scalar(
                        PendingStructuredScalarPhase::TokenListEquals(retained),
                    ),
                child,
            }) if retained == pending_owner => (false, child),
            Some(PendingStructuredScanner {
                phase: PendingStructuredScannerPhase::TokenListRightHandSide(retained),
                child,
            }) if retained == pending_owner => (true, child),
            Some(PendingStructuredScanner {
                phase:
                    PendingStructuredScannerPhase::Scalar(
                        PendingStructuredScalarPhase::TokenListRhsRegister(retained),
                    ),
                child,
            }) if retained == pending_owner => (true, child),
            Some(mut pending) => {
                if let Some(child) = pending.take_child() {
                    self.abort_continuation(child)?;
                }
                return Err(CommandError::input_invariant());
            }
            None => (false, None),
        };
        if !equals_done {
            self.restore_structured_scanner_child(
                &mut child,
                StructuredScannerChildDestination::Scalar,
            )?;
            let result = self.scan_optional_equals_retained();
            self.retain_structured_scalar(
                result,
                PendingStructuredScalarPhase::TokenListEquals(pending_owner),
            )?;
        }
        let scalar_child = child
            .as_ref()
            .is_some_and(|child| child.destination() == StructuredScannerChildDestination::Scalar);
        self.restore_structured_scanner_child(
            &mut child,
            if scalar_child {
                StructuredScannerChildDestination::Scalar
            } else {
                StructuredScannerChildDestination::TokenListRightHandSide
            },
        )?;
        let right_hand_side = match self.scan_token_list_right_hand_side(
            owner,
            parameter == TokParam::OUTPUT,
            pending_owner,
            scalar_child,
        ) {
            Ok(value) => value,
            Err(error) => {
                if error.is_resource_suspension()
                    && !self
                        .scanner_resume
                        .as_ref()
                        .is_some_and(crate::ScannerFrameKey::is_structured_scanner)
                {
                    self.retain_structured_scanner(
                        PendingStructuredScannerPhase::TokenListRightHandSide(pending_owner),
                        StructuredScannerChildDestination::TokenListRightHandSide,
                    )?;
                }
                return Err(error);
            }
        };
        Ok(ScannedTokenParameterAssignment {
            tokens: right_hand_side
                .pointer_present
                .then_some(right_hand_side.tokens)
                .flatten(),
            source: right_hand_side.source,
        })
    }

    fn scan_token_list_right_hand_side(
        &mut self,
        owner: Symbol,
        enclose_collected: bool,
        pending_owner: PendingTokenListOwner,
        resume_register: bool,
    ) -> Result<ScannedTokenListRightHandSide<G>, CommandError> {
        if resume_register {
            let result = self.scan_profile_register_index_retained();
            let index = self.retain_structured_scalar(
                result,
                PendingStructuredScalarPhase::TokenListRhsRegister(pending_owner),
            )?;
            return Ok(self.token_register_rhs(index));
        }
        let command = self
            .next_non_blank_non_relax_x_token()?
            .ok_or_else(CommandError::input_invariant)?;
        let collected = match static_meaning(command.meaning()) {
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Toks) => {
                // e-TeX 2.6 [49.1227] widens this RHS enquiry alongside
                // [49.1226]'s assignment target; both select the same sparse
                // token-register namespace.
                let result = self.scan_profile_register_index_retained();
                let index = self.retain_structured_scalar(
                    result,
                    PendingStructuredScalarPhase::TokenListRhsRegister(pending_owner),
                )?;
                return Ok(self.token_register_rhs(index));
            }
            Meaning::ToksRegister(index) => {
                let tokens = self
                    .state
                    .token_register(index)
                    .expect("meaning contains an admitted token-register index");
                return Ok(ScannedTokenListRightHandSide {
                    tokens: None,
                    source: tokens.clone(),
                    pointer_present: tokens
                        .is_some_and(|tokens| !self.state.token_list(tokens).is_empty()),
                });
            }
            Meaning::TokParam(index) => {
                return Ok(
                    match self
                        .state
                        .token_parameter(tex_state::env::banks::TokParam::new(index))
                        .expect("meaning contains an admitted token-parameter index")
                    {
                        Some(tokens) => ScannedTokenListRightHandSide {
                            tokens: None,
                            source: Some(tokens),
                            pointer_present: true,
                        },
                        None => ScannedTokenListRightHandSide {
                            tokens: None,
                            source: None,
                            pointer_present: false,
                        },
                    },
                );
            }
            Meaning::CharToken {
                cat: Catcode::BeginGroup,
                ..
            } => {
                // TeX82 has already delivered the required opening brace to
                // choose the non-internal branch. Back it up once, then let
                // `scan_toks` install absorbing status before it redelivers
                // that exact token. Re-scanning it through `scan_left_brace`
                // would add a second raw delivery before that transition.
                let primary = command.origin();
                self.back_input(command)?;
                self.scan_toks(ScanToksMode::GeneralAfterOpening {
                    expanded: false,
                    primary,
                    owner: Some(owner),
                })?
                .replacement_text
            }
            _ => {
                self.back_input(command)?;
                self.scan_toks(ScanToksMode::GeneralFor {
                    expanded: false,
                    owner,
                })?
                .replacement_text
            }
        };
        if self
            .command
            .attempt
            .arena()
            .token_words(collected)
            .map_err(crate::scan_toks::attempt_command_error)?
            .is_empty()
        {
            return Ok(ScannedTokenListRightHandSide {
                tokens: None,
                source: None,
                pointer_present: false,
            });
        }
        if !enclose_collected {
            return Ok(ScannedTokenListRightHandSide {
                tokens: Some(collected),
                source: None,
                pointer_present: true,
            });
        }
        let mut tokens = Vec::new();
        tokens.push(TracedTokenWord::pack(
            Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            },
            OriginId::UNKNOWN,
        ));
        tokens.extend_from_slice(
            self.command
                .attempt
                .arena()
                .token_words(collected)
                .map_err(crate::scan_toks::attempt_command_error)?,
        );
        tokens.push(TracedTokenWord::pack(
            Token::Char {
                ch: '}',
                cat: Catcode::EndGroup,
            },
            OriginId::UNKNOWN,
        ));
        Ok(ScannedTokenListRightHandSide {
            tokens: Some(
                self.command
                    .attempt
                    .arena_mut()
                    .allocate_token_list(tokens)
                    .map_err(crate::scan_toks::attempt_command_error)?,
            ),
            source: None,
            pointer_present: true,
        })
    }

    fn token_register_rhs(&self, index: u16) -> ScannedTokenListRightHandSide<G> {
        let tokens = self
            .state
            .token_register(index)
            .expect("scanner produced an admitted token-register index");
        ScannedTokenListRightHandSide {
            tokens: None,
            source: tokens.clone(),
            pointer_present: tokens.is_some_and(|tokens| !self.state.token_list(tokens).is_empty()),
        }
    }
}

fn static_meaning<G>(meaning: ResolvedMeaning<G>) -> Meaning {
    match meaning {
        ResolvedMeaning::Static(meaning) => meaning,
        ResolvedMeaning::Macro { .. } => Meaning::Undefined,
    }
}

#[cfg(test)]
mod tests;
