//! Bounded in-session command snapshots and named command summaries.
//!
//! A retained value owns one complete generation at coarse granularity and a
//! fixed tuple of scalar cursors. It never owns, clones, or borrows an input,
//! token, definition, provenance, or attempt row. The subsystem which owns the
//! live command timeline is responsible for validating these cursors before it
//! restores anything.

#![allow(dead_code)] // The .6.4 integration installs capture/restore consumers.

use core::fmt;
use core::marker::PhantomData;

use tex_state::GenerationOwner;

/// Watermarks for command-owned append-only storage.
///
/// Each coordinate is an exclusive row count. The corresponding arena
/// validates the coordinate against its own generation before truncation.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct CommandArenaCursors {
    input_rows: u32,
    input_words: u32,
    parameter_words: u32,
    builder_words: u32,
    attempt_rows: u32,
}

impl CommandArenaCursors {
    #[must_use]
    pub const fn new(
        input_rows: u32,
        input_words: u32,
        parameter_words: u32,
        builder_words: u32,
        attempt_rows: u32,
    ) -> Self {
        Self {
            input_rows,
            input_words,
            parameter_words,
            builder_words,
            attempt_rows,
        }
    }

    #[must_use]
    pub const fn input_rows(self) -> u32 {
        self.input_rows
    }

    #[must_use]
    pub const fn input_words(self) -> u32 {
        self.input_words
    }

    #[must_use]
    pub const fn parameter_words(self) -> u32 {
        self.parameter_words
    }

    #[must_use]
    pub const fn builder_words(self) -> u32 {
        self.builder_words
    }

    #[must_use]
    pub const fn attempt_rows(self) -> u32 {
        self.attempt_rows
    }
}

/// Length cursors for command-owned stacks and ordered ledgers.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct CommandStackCursors {
    input_depth: u32,
    parameter_depth: u32,
    condition_depth: u32,
    alignment_depth: u32,
    replay_depth: u32,
    diagnostic_count: u32,
    framing_event_count: u32,
}

impl CommandStackCursors {
    #[must_use]
    pub const fn new(
        input_depth: u32,
        parameter_depth: u32,
        condition_depth: u32,
        alignment_depth: u32,
        replay_depth: u32,
        diagnostic_count: u32,
        framing_event_count: u32,
    ) -> Self {
        Self {
            input_depth,
            parameter_depth,
            condition_depth,
            alignment_depth,
            replay_depth,
            diagnostic_count,
            framing_event_count,
        }
    }

    #[must_use]
    pub const fn input_depth(self) -> u32 {
        self.input_depth
    }

    #[must_use]
    pub const fn parameter_depth(self) -> u32 {
        self.parameter_depth
    }

    #[must_use]
    pub const fn condition_depth(self) -> u32 {
        self.condition_depth
    }

    #[must_use]
    pub const fn alignment_depth(self) -> u32 {
        self.alignment_depth
    }

    #[must_use]
    pub const fn replay_depth(self) -> u32 {
        self.replay_depth
    }

    #[must_use]
    pub const fn diagnostic_count(self) -> u32 {
        self.diagnostic_count
    }

    #[must_use]
    pub const fn framing_event_count(self) -> u32 {
        self.framing_event_count
    }
}

/// Complete fixed-size command coordinate captured at a restorable boundary.
///
/// `command_journal` addresses scalar and replacement mutations. Arena and
/// stack cursors address append-only suffixes. Restoration must acquire the
/// retained generation before replaying the journal or exposing any restored
/// coordinate, and may truncate suffixes only after roots have transferred.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct CommandSnapshotCursor {
    command_journal: u32,
    arenas: CommandArenaCursors,
    stacks: CommandStackCursors,
}

impl CommandSnapshotCursor {
    #[must_use]
    pub const fn new(
        command_journal: u32,
        arenas: CommandArenaCursors,
        stacks: CommandStackCursors,
    ) -> Self {
        Self {
            command_journal,
            arenas,
            stacks,
        }
    }

    #[must_use]
    pub const fn command_journal(self) -> u32 {
        self.command_journal
    }

    #[must_use]
    pub const fn arenas(self) -> CommandArenaCursors {
        self.arenas
    }

    #[must_use]
    pub const fn stacks(self) -> CommandStackCursors {
        self.stacks
    }
}

/// Exact in-session command snapshot for one admitted generation.
///
/// The default owner is [`GenerationOwner<G>`]. The owner parameter exists so
/// the fixed-cursor contract can be tested without constructing a live TeX
/// session; production construction remains crate-private.
pub struct CommandStateSnapshot<G, Owner = GenerationOwner<G>> {
    generation: Owner,
    cursor: CommandSnapshotCursor,
    brand: PhantomData<fn(&G) -> &G>,
}

impl<G, Owner: Clone> Clone for CommandStateSnapshot<G, Owner> {
    fn clone(&self) -> Self {
        Self {
            generation: self.generation.clone(),
            cursor: self.cursor,
            brand: PhantomData,
        }
    }
}

impl<G, Owner: fmt::Debug> fmt::Debug for CommandStateSnapshot<G, Owner> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CommandStateSnapshot")
            .field("generation", &self.generation)
            .field("cursor", &self.cursor)
            .finish()
    }
}

impl<G, Owner> CommandStateSnapshot<G, Owner> {
    #[must_use]
    pub(crate) const fn new(generation: Owner, cursor: CommandSnapshotCursor) -> Self {
        Self {
            generation,
            cursor,
            brand: PhantomData,
        }
    }

    #[must_use]
    pub const fn cursor(&self) -> CommandSnapshotCursor {
        self.cursor
    }

    #[must_use]
    pub(crate) const fn generation(&self) -> &Owner {
        &self.generation
    }

    #[must_use]
    pub(crate) fn into_parts(self) -> (Owner, CommandSnapshotCursor) {
        (self.generation, self.cursor)
    }
}

impl<G> CommandStateSnapshot<G> {
    /// Whether this snapshot addresses the admitted generation retained by
    /// `generation`.
    #[must_use]
    pub(crate) fn addresses(&self, generation: &GenerationOwner<G>) -> bool {
        self.generation.same_generation(generation)
    }
}

/// Restartable command state retained at a named in-session boundary.
///
/// A summary differs from an operation snapshot only in its publication
/// proof: construction requires quiescent command state and records the
/// portable profile fingerprint. The live form still contains no copied
/// command graph; cold detachment turns its selected roots into recipes.
pub struct CommandSummary<G, Owner = GenerationOwner<G>> {
    generation: Owner,
    cursor: CommandSnapshotCursor,
    profile_fingerprint: u64,
    root_source_anchor: Option<u64>,
    brand: PhantomData<fn(&G) -> &G>,
}

impl<G, Owner: Clone> Clone for CommandSummary<G, Owner> {
    fn clone(&self) -> Self {
        Self {
            generation: self.generation.clone(),
            cursor: self.cursor,
            profile_fingerprint: self.profile_fingerprint,
            root_source_anchor: self.root_source_anchor,
            brand: PhantomData,
        }
    }
}

impl<G, Owner: fmt::Debug> fmt::Debug for CommandSummary<G, Owner> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CommandSummary")
            .field("generation", &self.generation)
            .field("cursor", &self.cursor)
            .field("profile_fingerprint", &self.profile_fingerprint)
            .field("root_source_anchor", &self.root_source_anchor)
            .finish()
    }
}

impl<G, Owner> CommandSummary<G, Owner> {
    #[must_use]
    pub(crate) const fn new(
        generation: Owner,
        cursor: CommandSnapshotCursor,
        profile_fingerprint: u64,
        root_source_anchor: Option<u64>,
    ) -> Self {
        Self {
            generation,
            cursor,
            profile_fingerprint,
            root_source_anchor,
            brand: PhantomData,
        }
    }

    #[must_use]
    pub const fn cursor(&self) -> CommandSnapshotCursor {
        self.cursor
    }

    #[must_use]
    pub const fn profile_fingerprint(&self) -> u64 {
        self.profile_fingerprint
    }

    #[must_use]
    pub const fn root_source_anchor(&self) -> Option<u64> {
        self.root_source_anchor
    }

    #[must_use]
    pub(crate) const fn generation(&self) -> &Owner {
        &self.generation
    }

    #[must_use]
    pub(crate) fn into_parts(self) -> (Owner, CommandSnapshotCursor, u64, Option<u64>) {
        (
            self.generation,
            self.cursor,
            self.profile_fingerprint,
            self.root_source_anchor,
        )
    }
}

/// The first nonquiescent command-state class preventing summary publication.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum CommandSummaryError {
    ConditionalSkip,
    MacroMatch,
    DefinitionScan,
    AlignmentScan,
    AbsorbingScan,
    ExpansionActive,
    AlignmentTemplateActive,
    SuspendedAlignment,
    LiveTokenBuilder,
    LiveRollbackRoot,
    ScannerWarningContext,
    PendingSemanticDiagnostic,
    ResourceSuspension,
    AttemptSuspended,
}

impl fmt::Display for CommandSummaryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ConditionalSkip => "conditional skipping is active",
            Self::MacroMatch => "macro argument matching is active",
            Self::DefinitionScan => "definition scanning is active",
            Self::AlignmentScan => "alignment scanning is active",
            Self::AbsorbingScan => "balanced token absorption is active",
            Self::ExpansionActive => "command expansion is active",
            Self::AlignmentTemplateActive => "alignment template delivery is active",
            Self::SuspendedAlignment => "an alignment delivery context is suspended",
            Self::LiveTokenBuilder => "a semantic token builder is live",
            Self::LiveRollbackRoot => "a temporary rollback root is live",
            Self::ScannerWarningContext => "scanner warning context remains installed",
            Self::PendingSemanticDiagnostic => {
                "a command semantic diagnostic is awaiting executor delivery"
            }
            Self::ResourceSuspension => "a command resource request is pending",
            Self::AttemptSuspended => "the command attempt is owned by a suspension",
        })
    }
}

impl std::error::Error for CommandSummaryError {}

#[cfg(test)]
#[path = "snapshot/tests.rs"]
mod tests;
