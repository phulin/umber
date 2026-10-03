//! Named command checkpoints captured by value.
//!
//! A named boundary is quiescent: no scanner, attempt, alignment template, or
//! expansion is live. What remains is small: source and stored-token input
//! rows, open conditionals, group payloads, and a few scalars. Publication
//! copies exactly that remainder, so execution between boundaries writes no
//! undo record at all. Source bytes and replay segments are `Arc`-shared, so a
//! copy duplicates only cursors and row headers.
//!
//! One candidate fork may be open at a time. It moves the live roots aside as
//! the accepted head and detaches every capture newer than its base;
//! rejection reinstalls both by move and acceptance drops them.

use core::fmt;
use std::sync::atomic::{AtomicU64, Ordering};

use tex_state::{GenerationOwner, Universe};

use crate::conditionals::ConditionFrame;
use crate::input::InputCapture;
use crate::processor::AlignmentDeliveryState;
use crate::profile::{CommandProfileBoundary, CommandProfileFingerprint, CommandProfileMismatch};
use crate::state::{CommandPayload, CommandState, CommandStateRoots};

mod boundary;

static NEXT_COMMAND_CHECKPOINT_OWNER: AtomicU64 = AtomicU64::new(1);

fn bounded_command_identity<G>(roots: &CommandStateRoots<G>) -> u64 {
    use std::hash::{Hash as _, Hasher as _};

    let mut hash = 0xcbf2_9ce4_8422_2325_u64 ^ 0x636f_6d6d_616e_6431;
    let mut feed = |value: u64| {
        for byte in value.to_le_bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    };
    let mut semantics = std::collections::hash_map::DefaultHasher::new();
    roots.engine_semantics.hash(&mut semantics);
    feed(semantics.finish());
    feed(roots.input.levels.len() as u64);
    feed(roots.input.retained_file_line_number as u64);
    feed(u64::from(roots.input.terminal_context_line.is_some()));
    feed(roots.input.pending_sources.len() as u64);
    feed(u64::from(roots.input.force_eof));
    feed(roots.conditions.tracked_stack_projection());
    feed(roots.alignment.align_state as u64);
    feed(roots.alignment.align_stack.len() as u64);
    feed(roots.alignment.suspended.len() as u64);
    feed(u64::from(roots.alignment.active_alignment.is_some()));
    feed(u64::from(roots.alignment.active_cell.is_some()));
    feed(u64::from(roots.alignment.completed_preamble.is_some()));
    feed(roots.replay_completions.len() as u64);
    feed(roots.semantic_diagnostics.len() as u64);
    // The last §342 report site is captured for restore, but is source
    // provenance. Editor rebinds give equal future states different SourceIds
    // (and edits may move their line/column), so it cannot gate convergence.
    feed(u64::from(roots.afterassignment.is_some()));
    feed(u64::from(roots.name_in_progress));
    feed(u64::from(roots.pending_input_open.is_some()));
    feed(roots.named_token_list_pushes.len() as u64);
    hash
}

fn vec_bytes<T>(values: &[T], capacity: usize) -> usize {
    debug_assert!(values.len() <= capacity);
    capacity.saturating_mul(std::mem::size_of::<T>())
}

/// The command-only state one named boundary copies.
///
/// Every field omitted here is either job configuration (profile, engine
/// semantics), session accounting that must survive restore (stack maxima,
/// identity allocators, pending source registrations), or provably empty at
/// a quiescent boundary and reset by install.
pub(crate) struct CommandCapture<G> {
    input: InputCapture<G>,
    conditions: Vec<ConditionFrame>,
    afterassignment: Option<CommandPayload<G>>,
    alignment: AlignmentDeliveryState<G>,
    name_in_progress: bool,
    last_diagnostic_location: Option<crate::DiagnosticLocation>,
}

impl<G> CommandCapture<G> {
    fn retained_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            .saturating_add(self.input.retained_bytes())
            .saturating_add(vec_bytes(&self.conditions, self.conditions.capacity()))
    }
}

type SerialCapture<G> = (u64, Box<CommandCapture<G>>);

/// Every named command capture retained by one live command owner, in
/// publication order, plus the sole open candidate fork.
pub(crate) struct CommandCheckpoints<G> {
    owner: u64,
    next_serial: u64,
    captures: Vec<SerialCapture<G>>,
    fork: Option<CheckpointFork<G>>,
}

/// One open candidate: the accepted head moved out of the live owner and the
/// accepted captures newer than the candidate's base.
struct CheckpointFork<G> {
    base: u64,
    detached: Vec<SerialCapture<G>>,
    accepted: Box<CommandStateRoots<G>>,
}

impl<G> Default for CommandCheckpoints<G> {
    fn default() -> Self {
        Self {
            owner: NEXT_COMMAND_CHECKPOINT_OWNER.fetch_add(1, Ordering::Relaxed),
            next_serial: 0,
            captures: Vec::new(),
            fork: None,
        }
    }
}

impl<G> fmt::Debug for CommandCheckpoints<G> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CommandCheckpoints")
            .field("owner", &self.owner)
            .field("captures", &self.captures.len())
            .field("fork", &self.fork.as_ref().map(|fork| fork.base))
            .finish()
    }
}

impl<G> CommandCheckpoints<G> {
    pub(crate) const fn owner_id(&self) -> u64 {
        self.owner
    }

    pub(crate) const fn candidate_active(&self) -> bool {
        self.fork.is_some()
    }

    fn position(&self, serial: u64) -> Option<usize> {
        self.captures
            .binary_search_by_key(&serial, |(serial, _)| *serial)
            .ok()
    }

    /// Whether a restore may install `serial`. Within a candidate only the
    /// base and the candidate's own captures are on the current lineage.
    fn restorable(&self, serial: u64) -> bool {
        self.position(serial).is_some() && self.fork.as_ref().is_none_or(|fork| serial >= fork.base)
    }

    fn push(&mut self, capture: CommandCapture<G>) -> Result<u64, CommandSummaryError> {
        let serial = self.next_serial;
        self.next_serial = serial
            .checked_add(1)
            .ok_or(CommandSummaryError::TimelineCapacity)?;
        self.captures.push((serial, Box::new(capture)));
        Ok(serial)
    }

    /// Drops every capture newer than `serial` and returns the target.
    fn truncate_after(&mut self, serial: u64) -> &CommandCapture<G> {
        let index = self
            .position(serial)
            .expect("prepared command restore names a retained capture");
        self.captures.truncate(index + 1);
        &self.captures[index].1
    }

    fn release(&mut self, serial: u64) -> bool {
        if let Some(index) = self.position(serial) {
            self.captures.remove(index);
            return true;
        }
        let Some(fork) = self.fork.as_mut() else {
            return false;
        };
        match fork
            .detached
            .binary_search_by_key(&serial, |(serial, _)| *serial)
        {
            Ok(index) => {
                fork.detached.remove(index);
                true
            }
            Err(_) => false,
        }
    }
}

/// Restartable command state retained at a named in-session boundary.
///
/// The summary is a handle: the capture it names lives in the command owner
/// that published it until checkpoint history releases it.
pub struct CommandSummary<G> {
    generation: GenerationOwner<G>,
    owner: u64,
    serial: u64,
    profile_fingerprint: u64,
    root_source_anchor: Option<u64>,
    reachable_state_identity_root: Option<u64>,
    retained_owner_bytes: usize,
}

impl<G> Clone for CommandSummary<G> {
    fn clone(&self) -> Self {
        Self {
            generation: self.generation.clone(),
            owner: self.owner,
            serial: self.serial,
            profile_fingerprint: self.profile_fingerprint,
            root_source_anchor: self.root_source_anchor,
            reachable_state_identity_root: self.reachable_state_identity_root,
            retained_owner_bytes: self.retained_owner_bytes,
        }
    }
}

impl<G> fmt::Debug for CommandSummary<G> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CommandSummary")
            .field("owner", &self.owner)
            .field("serial", &self.serial)
            .field("profile_fingerprint", &self.profile_fingerprint)
            .field("root_source_anchor", &self.root_source_anchor)
            .finish()
    }
}

impl<G> CommandSummary<G> {
    #[must_use]
    pub const fn profile_fingerprint(&self) -> u64 {
        self.profile_fingerprint
    }

    #[must_use]
    pub const fn root_source_anchor(&self) -> Option<u64> {
        self.root_source_anchor
    }

    /// Returns the command owner's maintained future-state root, if the
    /// command ownership lineage supports the complete identity contract.
    #[must_use]
    pub const fn reachable_state_identity_root(&self) -> Option<u64> {
        self.reachable_state_identity_root
    }

    /// Returns the bytes owned by this summary's capture.
    #[must_use]
    pub const fn retained_owner_bytes(&self) -> usize {
        self.retained_owner_bytes
    }

    /// Returns the process-unique identity of this summary's capture, used
    /// only to deduplicate retention accounting.
    #[must_use]
    pub const fn retention_owner(&self) -> (u64, u64) {
        (self.owner, self.serial)
    }

    fn addresses(&self, generation: &GenerationOwner<G>, owner: u64) -> bool {
        self.owner == owner && self.generation.same_generation(generation)
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
    PendingSemanticDiagnostic,
    ResourceSuspension,
    AttemptSuspended,
    TimelineCapacity,
    GenerationUnavailable,
    CheckpointIneligible,
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
            Self::PendingSemanticDiagnostic => {
                "a command semantic diagnostic is awaiting executor delivery"
            }
            Self::ResourceSuspension => "a command resource request is pending",
            Self::AttemptSuspended => "the command attempt is owned by a suspension",
            Self::TimelineCapacity => "the command checkpoint serial space is exhausted",
            Self::GenerationUnavailable => "the command generation is unavailable",
            Self::CheckpointIneligible => {
                "the state checkpoint boundary is inside a TeX group or transaction"
            }
        })
    }
}

impl std::error::Error for CommandSummaryError {}

/// Validation failure for one in-session command-root restore.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandRestoreError {
    Profile(CommandProfileMismatch),
    ForeignGeneration,
    InvalidCursor,
}

impl fmt::Display for CommandRestoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Profile(error) => error.fmt(formatter),
            Self::ForeignGeneration => {
                formatter.write_str("the command checkpoint belongs to another generation")
            }
            Self::InvalidCursor => formatter.write_str("the command checkpoint cursor is invalid"),
        }
    }
}

impl std::error::Error for CommandRestoreError {}

/// Fully validated command-root switch. Applying it cannot fail.
pub struct PreparedCommandRestore<G> {
    owner: u64,
    serial: u64,
    brand: core::marker::PhantomData<fn(&G) -> &G>,
}

#[cfg(test)]
#[path = "snapshot/tests.rs"]
mod tests;
