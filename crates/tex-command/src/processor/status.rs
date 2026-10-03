//! Scanner-status ownership and EOF classification.
//!
//! TeX.web's `scanner_status` is live command-machine state, rather than a
//! collection of flags on individual scanners.  This module deliberately
//! chooses the status and the canonical incomplete-input classification, but
//! does not inject recovery input: that is the outer-validity operation.

use tex_state::interner::Symbol;

use crate::{CommandProcessor, CommandState};

use crate::observation::{CommandObservation, ScannerStatusRecord};

#[allow(dead_code)] // invoked by the ordered scanner, macro-call, and conditional milestones
impl<G> CommandState<G> {
    pub(crate) fn begin_scanner_status(&mut self, status: ScannerStatus) -> ScannerStatus {
        let prior = std::mem::replace(&mut self.scanner, status);
        self.delivery_mode
            .set_scanner_active(!matches!(status, ScannerStatus::Normal));
        prior
    }

    pub(crate) fn restore_scanner_status(&mut self, prior: ScannerStatus) {
        self.scanner = prior;
        self.delivery_mode
            .set_scanner_active(!matches!(self.scanner, ScannerStatus::Normal));
    }

    pub(crate) fn clear_scanner_for_recovery(&mut self) {
        self.scanner = ScannerStatus::Normal;
        self.delivery_mode.set_scanner_active(false);
    }

    /// Runs one scanner operation with typed status and warning ownership.
    ///
    /// The complete former scanner state is restored after the operation's
    /// normal or recoverable return, including after nested scanner episodes.
    pub(crate) fn with_scanner_status<T>(
        &mut self,
        status: ScannerStatus,
        operation: impl FnOnce(&mut Self) -> T,
    ) -> T {
        let prior = self.begin_scanner_status(status);
        let result = operation(self);
        self.restore_scanner_status(prior);
        result
    }
}

impl<G, Facts: crate::CommandHostFacts<G>> CommandProcessor<'_, '_, G, Facts> {
    /// Enters one processor-owned scanner episode.
    ///
    /// The returned value retains both the installed semantic status and the
    /// complete prior scanner state.  Callers hand it back to
    /// [`Self::finish_scanner_episode`] on every normal or recoverable exit;
    /// outer-validity recovery may clear the live state in between without
    /// losing the canonical exit transition.
    #[inline(always)]
    pub(crate) fn begin_scanner_episode(
        &mut self,
        status: ScannerStatus,
        visibility: ScannerStatusVisibility,
    ) -> ScannerEpisode {
        let prior = self.command.begin_scanner_status(status);
        let observed = visibility.is_observed() && self.is_observed();
        if observed {
            self.observe_scanner_status_transition(prior, self.command.scanner);
        }
        ScannerEpisode {
            installed: status,
            prior,
            observed,
        }
    }

    /// Reasserts an episode cleared by nested outer-validity recovery.
    ///
    /// TeX82 §400 restores an enclosing collector after §394 aborts a
    /// macro argument.  Keeping this on the episode prevents the collector
    /// from reconstructing status or observation policy independently.
    pub(crate) fn resume_scanner_episode_after_recovery(&mut self, episode: &ScannerEpisode) {
        if !matches!(self.command.scanner, ScannerStatus::Normal) {
            return;
        }
        let displaced = self.command.begin_scanner_status(episode.installed);
        if episode.observed {
            self.observe_scanner_status_transition(displaced, self.command.scanner);
        }
    }

    /// Publishes an episode's recovery-aware exit and restores its complete
    /// prior scanner state.
    pub(crate) fn finish_scanner_episode(&mut self, episode: ScannerEpisode) {
        if episode.observed {
            self.restore_scanner_status_with_observation(episode.installed, episode.prior);
        } else {
            self.command.restore_scanner_status(episode.prior);
        }
    }

    #[allow(unused_variables)]
    pub(crate) fn observe_scanner_status_transition(
        &mut self,
        from: ScannerStatus,
        to: ScannerStatus,
    ) {
        let from = crate::observation::canonical_names::scanner_status_name(&from);
        let to = crate::observation::canonical_names::scanner_status_name(&to);
        if from == to {
            return;
        }
        observe!(
            self,
            CommandObservation::ScannerStatus(ScannerStatusRecord { from, to }),
        );
    }

    /// Publishes a scoped scanner-status exit before restoring its complete
    /// former state. Outer-validity recovery clears a live episode so that its
    /// inserted token is delivered normally; that lexical cleanup must still
    /// retain the episode's canonical exit transition.
    pub(crate) fn restore_scanner_status_with_observation(
        &mut self,
        installed: ScannerStatus,
        prior: ScannerStatus,
    ) {
        let current = self.command.scanner;
        let from = if matches!(current, ScannerStatus::Normal)
            && !matches!(installed, ScannerStatus::Normal)
        {
            installed
        } else {
            current
        };
        self.observe_scanner_status_transition(from, prior);
        self.command.restore_scanner_status(prior);
    }
}

/// Whether a semantic scanner episode appears in the detached TeX observer.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum ScannerStatusVisibility {
    Observed,
    Hidden,
}

impl ScannerStatusVisibility {
    const fn is_observed(self) -> bool {
        matches!(self, Self::Observed)
    }
}

/// Processor-owned lifetime of one live scanner-status installation.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct ScannerEpisode {
    installed: ScannerStatus,
    prior: ScannerStatus,
    /// Observation eligibility is settled once at admission. An ordinary
    /// unobserved scan never enters name translation or observer dispatch.
    observed: bool,
}

/// Typed shell for TeX's live `scanner_status`.
#[allow(dead_code)] // status variants are installed by later scanner callers
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub(crate) enum ScannerStatus {
    #[default]
    Normal,
    Skipping(SkippingContext),
    Defining(DefinitionContext),
    Matching(MatchingContext),
    Aligning(AlignmentScanContext),
    Absorbing(AbsorbingContext),
}

#[allow(dead_code)] // invoked by ScannerStatus installation
impl ScannerStatus {
    pub(crate) const fn is_quiescent(&self) -> bool {
        matches!(self, Self::Normal)
    }

    /// The canonical runaway family selected at terminal input exhaustion.
    pub(crate) const fn eof_legality(&self) -> EofLegality {
        match self {
            Self::Normal => EofLegality::Legal,
            Self::Skipping(_) => EofLegality::Runaway(RunawayKind::Conditional),
            Self::Defining(_) => EofLegality::Runaway(RunawayKind::Definition),
            Self::Matching(_) => EofLegality::Runaway(RunawayKind::MacroArgument),
            Self::Aligning(_) => EofLegality::Runaway(RunawayKind::AlignmentPreamble),
            Self::Absorbing(_) => EofLegality::Runaway(RunawayKind::AbsorbedTokens),
        }
    }
}

/// Stable identity of the condition whose text is being skipped.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct ConditionId(pub(crate) u64);

/// Identity of a live token builder in transient command state.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct TokenBuilderId(pub(crate) u64);

/// Context for skipped conditional text.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct SkippingContext {
    /// TeX82 §494's `skip_line:=line`, which §336 prints as "all text was
    /// ignored after line N". It is the line skipping *began* on, not the
    /// line the `\if` opened on, so it cannot be read back off the frame.
    pub(crate) skip_line: u32,
    /// TeX82 §336's `cur_if`: the conditional whose text was being skipped.
    pub(crate) conditional: crate::conditionals::ConditionalKind,
}

/// Context for a macro definition's parameter/replacement scan.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct DefinitionContext {
    pub(crate) target: Option<Symbol>,
}

/// Context for macro argument matching.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct MatchingContext {
    pub(crate) macro_name: Symbol,
}

/// Context for an alignment preamble scan.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct AlignmentScanContext {
    pub(crate) builder: TokenBuilderId,
    /// TeX82 §774's saved `cs_ptr`, installed as §776's
    /// `warning_index` for the duration of the preamble scan.
    pub(crate) owner: Option<Symbol>,
}

/// Context for a balanced token-list scan other than a definition/preamble.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct AbsorbingContext {
    pub(crate) owner: Option<Symbol>,
}

/// Whether terminal EOF is permitted by the current scanner episode.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum EofLegality {
    Legal,
    Runaway(RunawayKind),
}

/// Canonical incomplete-input family, independent from diagnostic wording.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum RunawayKind {
    Conditional,
    Definition,
    MacroArgument,
    AlignmentPreamble,
    AbsorbedTokens,
}
