//! Two-tier keyed comparison of the pinned oracle stream against the observed
//! canonical stream, with bounded local realignment after a structural
//! divergence.
//!
//! Index-aligned comparison is only correct while the two streams agree on how
//! *many* events each delivery produces. One dropped or extra event turns every
//! later index into a mismatch, so a single root defect can fill the whole
//! worklist with cascade noise that says nothing new. This module compares the
//! two streams as sequences instead of as index-parallel arrays:
//!
//! 1. Every event is split into an [`AlignmentKey`] -- event kind, canonical
//!    command identity, structural transition, and source position -- and a
//!    payload (operands, values, tokens). Equal key with differing payload is
//!    reported once as [`Repair::Changed`] and the streams stay aligned, so a
//!    content-only defect never cascades.
//! 2. A key mismatch is a candidate desynchronization. A wavefront search
//!    confined to [`AlignmentTuning::window`] events on each side looks for the
//!    earliest local repair, and accepts it only when
//!    [`AlignmentTuning::confirmation`] consecutive key-equal pairs follow it.
//!    The repair's shape -- oracle events dropped, Umber events added, or a
//!    short replaced run -- is itself the diagnosis.
//! 3. If nothing confirms inside the window the divergence is structural. One
//!    anchor resync is attempted, scanning both streams forward over every
//!    high-salience boundary (an input-stack push/retire, or a new source
//!    line) within [`AlignmentTuning::anchor_scan`] events and rejoining at
//!    the most identifying shared one that carries the same confirmation --
//!    a shared source line first, an anonymous nesting boundary only when no
//!    line is shared in reach, and least total skip within either class.
//! 4. If that also fails, comparison of the fixture stops and says so. An
//!    over-eager realignment hides a real defect silently; cascade noise at
//!    least stays visible. The bias is deliberately toward declaring a
//!    structural divergence.
//!
//! This is *not* a global minimum-edit-distance diff. A global alignment would
//! happily pair oracle event 700 with observed event 40 000 when that minimizes
//! total edits; the result is mathematically minimal and diagnostically
//! useless. Every search here is local and bounded, and the cost per reported
//! divergence is constant.

use std::fmt;

use tex_oracle::{
    AlignmentTransition, CanonicalCommand, CommandDelivery, CommandEvent, ConditionTransition,
    DiagnosticSeverity, EffectKind, Event, GeometryEvent, InputReason, InputTransition, MacroEvent,
    NormalizedEvent, RecoveryKind, StateTarget, TokenListTransition,
};

use crate::{ObservedEvent, bounded_debug, hidden_difference_excerpt};

#[cfg(test)]
mod tests;

/// Half-width of the bounded realignment search, in events per stream.
///
/// A wavefront over a `window`-wide band costs `O(window^2)` key comparisons in
/// the worst case and is paid at most once per reported divergence, so the run
/// cost stays linear in the streams. 64 comfortably spans every repair shape
/// this epic has produced -- a missing backup push, a duplicated raw/expanded
/// delivery pair, a macro activation expanded one level too far -- while
/// staying far below the distance at which a confirmed match would be
/// coincidence rather than the same point in the document.
pub const DEFAULT_REALIGN_WINDOW: usize = 64;

/// Consecutive key-equal pairs required before a realignment is accepted.
///
/// This is the standard diff resync heuristic and it is what keeps a
/// speculative repair from swallowing an independent later divergence: eight
/// consecutive agreeing events is far more than the two or three that repeat by
/// chance inside a run of like-catcode characters, and small enough that a
/// genuine repair immediately followed by a *second* independent defect is
/// still confirmed (the second defect is then reported on its own).
pub const DEFAULT_REALIGN_CONFIRMATION: usize = 8;

/// How far the structural anchor fallback scans forward on each side.
///
/// Only reached when the window search already failed, so it may be generous;
/// 4096 events is roughly a page of document activity, enough to cross a macro
/// body or an alignment that was replayed completely differently, and still a
/// hard bound. It is the *only* bound on the fallback's reach: every anchor
/// inside the scan is a candidate, so widening the flag really does widen the
/// search.
pub const DEFAULT_ANCHOR_SCAN: usize = 4096;

/// Bounds on how hard the comparator works to repair an alignment before it
/// declares a structural divergence and says so.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AlignmentTuning {
    /// Half-width of the bounded realignment search (`--realign-window`).
    pub window: usize,
    /// Consecutive key-equal pairs required to accept a realignment
    /// (`--realign-confirm`).
    pub confirmation: usize,
    /// Events scanned forward on each side by the anchor fallback
    /// (`--anchor-scan`).
    pub anchor_scan: usize,
}

impl Default for AlignmentTuning {
    fn default() -> Self {
        Self {
            window: DEFAULT_REALIGN_WINDOW,
            confirmation: DEFAULT_REALIGN_CONFIRMATION,
            anchor_scan: DEFAULT_ANCHOR_SCAN,
        }
    }
}

/// What the comparator did with the two streams at one divergence.
///
/// The variant is the structural diagnosis. Event-count defects have been the
/// dominant class of canonical divergence in this epic, and the difference
/// between "Umber never emitted this" and "Umber emitted this twice" used to be
/// derived by hand from a wall of cascade entries.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Repair {
    /// The two events denote the same point in the stream and only their
    /// payload differs. Nothing was skipped and the streams remain aligned.
    Changed,
    /// The oracle emitted `count` events Umber never produced.
    Dropped { count: usize },
    /// Umber emitted `count` events the oracle never produced.
    Extra { count: usize },
    /// A short edit run: `expected` oracle events stand where Umber produced
    /// `actual` different ones.
    Replaced { expected: usize, actual: usize },
    /// No repair confirmed inside the window; the streams were rejoined at a
    /// shared structural anchor after skipping the recorded events.
    AnchorResync {
        expected_skipped: usize,
        actual_skipped: usize,
        anchor: ResyncAnchor,
    },
    /// No repair confirmed and no shared anchor found. Comparison of this
    /// fixture stopped here rather than guessing.
    Abandoned,
    /// One stream ended while the other still had events.
    Truncated { remaining: usize },
}

/// The structural boundary a [`Repair::AnchorResync`] rejoined the two streams
/// at, kept as data rather than as pre-rendered text.
///
/// Structure matters because a source line is a *position*: two recurrences of
/// one root site rejoin at the same kind of anchor at different lines, and
/// the report's recurrence grouping can only recognize that if the line is a
/// separate field rather than a substring of a message.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ResyncAnchor {
    /// An input-stack push, retire, or stop: a real nesting boundary.
    Input {
        transition: InputTransition,
        reason: InputReason,
        name: String,
    },
    /// The first delivery attributed to a new source line.
    Line { source: String, line: u32 },
}

impl fmt::Display for ResyncAnchor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Input {
                transition,
                reason,
                name,
            } => write!(formatter, "input {transition:?}/{reason:?} {name}"),
            Self::Line { source, line } => write!(formatter, "{source} line {line}"),
        }
    }
}

impl From<&AnchorKey<'_>> for ResyncAnchor {
    fn from(key: &AnchorKey<'_>) -> Self {
        match key {
            AnchorKey::Input {
                transition,
                reason,
                name,
            } => Self::Input {
                transition: *transition,
                reason: *reason,
                name: (*name).into(),
            },
            AnchorKey::Line { source, line } => Self::Line {
                source: (*source).into(),
                line: *line,
            },
        }
    }
}

impl Repair {
    fn describe(&self) -> String {
        match self {
            Self::Changed => "payload differs, streams stay aligned".into(),
            Self::Dropped { count } => {
                format!("{count} oracle event(s) dropped by Umber")
            }
            Self::Extra { count } => format!("{count} extra Umber event(s)"),
            Self::Replaced { expected, actual } => {
                format!("{expected} oracle event(s) replaced by {actual} Umber event(s)")
            }
            Self::AnchorResync {
                expected_skipped,
                actual_skipped,
                anchor,
            } => format!(
                "structural: no confirmed realignment in window; rejoined at {anchor} \
                 after skipping {expected_skipped} oracle and {actual_skipped} Umber event(s)"
            ),
            Self::Abandoned => "structural: no confirmed realignment and no shared anchor; \
                 comparison of this fixture stopped here"
                .into(),
            Self::Truncated { remaining } => {
                format!("one stream ended with {remaining} event(s) remaining in the other")
            }
        }
    }
}

/// The events a divergence has to show. Exhaustive by construction: a
/// divergence always has at least one side, so there is no "neither stream had
/// an event" case to fall through silently.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MismatchSides {
    Both {
        expected: Box<Event>,
        actual: Box<ObservedEvent>,
    },
    ExpectedOnly(Box<Event>),
    ActualOnly(Box<ObservedEvent>),
}

/// One ordered divergence between the pinned oracle stream and the observed
/// canonical stream, together with how the comparator resynchronized after it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StreamMismatch {
    pub(crate) fixture: String,
    /// Index into the pinned oracle stream.
    pub(crate) index: usize,
    /// Index into the observed stream. Equal to `index` until an earlier
    /// repair shifted the two streams apart.
    pub(crate) actual_index: usize,
    /// Cheap structural label distinguishing the kind of disagreement (e.g.
    /// a wrong command identity vs. a wrong operand vs. a different event
    /// kind entirely) so a coordinator can group same-kind divergences
    /// without re-running the engine or re-deriving what differs by hand.
    pub(crate) kind: &'static str,
    pub(crate) repair: Repair,
    pub(crate) sides: MismatchSides,
    /// How many additional mismatches plain index-aligned comparison would
    /// have reported for the stream region this entry covers -- the cascade
    /// this entry stands in for. See [`attribute_suppressed_cascade`].
    pub(crate) suppressed_cascade: usize,
}

impl StreamMismatch {
    /// Index into the pinned oracle stream.
    pub fn index(&self) -> usize {
        self.index
    }

    /// How the comparator resynchronized after this divergence.
    pub fn repair(&self) -> &Repair {
        &self.repair
    }

    /// Cascade mismatches this entry stands in for.
    pub fn suppressed_cascade(&self) -> usize {
        self.suppressed_cascade
    }
}

impl fmt::Display for StreamMismatch {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "fixture {} diverged at event {}",
            self.fixture, self.index
        )?;
        if self.actual_index != self.index {
            write!(formatter, " (observed event {})", self.actual_index)?;
        }
        write!(
            formatter,
            " [{}] (resync: {}; suppressed {} cascade event(s))",
            self.kind,
            self.repair.describe(),
            self.suppressed_cascade
        )?;
        match &self.sides {
            MismatchSides::Both { expected, actual } => {
                write!(formatter, "\n  expected: {}", bounded_debug(expected))?;
                write!(formatter, "\n  actual: {}", bounded_debug(&actual.event))?;
                if let Some(excerpt) = hidden_difference_excerpt(expected, &actual.event) {
                    formatter.write_str(&excerpt)?;
                }
                if !actual.context.is_empty() {
                    write!(formatter, "\n  context: {}", actual.context)?;
                }
            }
            MismatchSides::ExpectedOnly(expected) => {
                write!(formatter, "\n  expected: {}", bounded_debug(expected))?;
                formatter.write_str("\n  actual: <end of observer stream>")?;
            }
            MismatchSides::ActualOnly(actual) => {
                formatter.write_str("\n  expected: <end of committed stream>")?;
                write!(formatter, "\n  actual: {}", bounded_debug(&actual.event))?;
                if !actual.context.is_empty() {
                    write!(formatter, "\n  context: {}", actual.context)?;
                }
            }
        }
        Ok(())
    }
}

/// One fixture's complete comparison: the ordered divergences it produced, and
/// whether `max_results` cut the comparison short.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Comparison {
    /// The ordered divergences, at most `max_results` of them.
    pub entries: Vec<StreamMismatch>,
    /// `true` when the `max_results` budget was reached while both streams
    /// still had events left to compare: the rest of this fixture was never
    /// examined and more divergences may exist beyond the last entry.
    ///
    /// A bound that is not reported is a bound that silently becomes the
    /// answer, so the runner prints this rather than leaving a reader to infer
    /// it from `entries.len() == max_results`.
    pub budget_reached: bool,
}

/// Compares complete ordered streams without omitting unsupported event kinds,
/// reporting only the earliest divergence. Kept for callers that only need
/// the first mismatch; [`find_divergences`] reports up to `max_results`.
pub fn compare_streams(
    fixture: &str,
    expected: &[NormalizedEvent],
    actual: &[ObservedEvent],
) -> Result<(), Box<StreamMismatch>> {
    match find_divergences(fixture, expected, actual, 1, AlignmentTuning::default())
        .entries
        .into_iter()
        .next()
    {
        Some(mismatch) => Err(Box::new(mismatch)),
        None => Ok(()),
    }
}

/// Compares complete ordered streams as sequences, collecting up to
/// `max_results` ordered divergences and resynchronizing after each one.
///
/// See the module documentation for the alignment contract. Every returned
/// entry carries the repair the comparator applied and the number of
/// index-aligned mismatches it stands in for.
pub fn find_divergences(
    fixture: &str,
    expected: &[NormalizedEvent],
    actual: &[ObservedEvent],
    max_results: usize,
    tuning: AlignmentTuning,
) -> Comparison {
    let aligner = Aligner::new(fixture, expected, actual, tuning);
    let (mut entries, budget_reached) = aligner.align(max_results);
    attribute_suppressed_cascade(&mut entries, expected, actual);
    Comparison {
        entries,
        budget_reached,
    }
}

/// Whether the two streams agree at one already-aligned position.
pub(crate) fn events_match(expected: Option<&Event>, actual: Option<&Event>) -> bool {
    match (expected, actual) {
        (Some(expected), Some(actual)) if macro_call_operand_is_reference(expected, actual) => true,
        _ => expected == actual,
    }
}

fn macro_call_operand_is_reference(expected: &Event, actual: &Event) -> bool {
    let (
        Event::Command(CommandEvent {
            delivery: expected_delivery,
            command:
                CanonicalCommand {
                    command: expected_command,
                    operand: tex_oracle::CanonicalValue::Integer(_),
                    control_sequence: expected_control_sequence,
                    location: expected_location,
                },
        }),
        Event::Command(CommandEvent {
            delivery: actual_delivery,
            command:
                CanonicalCommand {
                    command: actual_command,
                    operand: tex_oracle::CanonicalValue::None,
                    control_sequence: actual_control_sequence,
                    location: actual_location,
                },
        }),
    ) = (expected, actual)
    else {
        return false;
    };

    matches!(
        expected_command.as_str(),
        "call" | "long_call" | "outer_call" | "long_outer_call"
    ) && expected_delivery == actual_delivery
        && expected_command == actual_command
        && expected_control_sequence == actual_control_sequence
        && expected_location == actual_location
}

/// The identity half of an event: what makes two events *the same point* in the
/// stream, independent of the values they carry.
///
/// Structural transitions (input push/retire, condition push/branch/pop,
/// alignment transitions, token-list splice/complete) belong here because they
/// change stream shape; operands, scanner results, mutation keys and values,
/// and token payloads do not, and stay in the payload so that a content defect
/// is reported once instead of desynchronizing the comparison.
///
/// A command's source position is part of its identity. Long runs of
/// like-catcode characters are otherwise indistinguishable by identity alone,
/// and a shifted stream could confirm a realignment against the wrong
/// occurrence; the byte position makes those runs unambiguous.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AlignmentKey<'a> {
    Command {
        delivery: CommandDelivery,
        command: &'a str,
        control_sequence: Option<&'a str>,
        location: Option<(&'a str, u32, u32)>,
    },
    Input {
        transition: InputTransition,
        reason: InputReason,
        name: &'a str,
    },
    Recovery {
        kind: RecoveryKind,
    },
    ScannerStatus,
    MacroArgument {
        parameter: u16,
    },
    MacroActivation {
        control_sequence: &'a str,
        argument_count: u16,
    },
    Condition {
        transition: ConditionTransition,
        condition: &'a str,
    },
    Scanner {
        scanner: &'a str,
    },
    TokenList {
        transition: TokenListTransition,
        purpose: &'a str,
    },
    Alignment {
        transition: AlignmentTransition,
    },
    Mutation {
        target: &'a StateTarget,
    },
    Diagnostic {
        severity: DiagnosticSeverity,
        diagnostic: &'a str,
    },
    Effect {
        kind: EffectKind,
        channel: &'a str,
    },
    Geometry {
        transition: &'static str,
    },
}

fn alignment_key(event: &Event) -> AlignmentKey<'_> {
    match event {
        Event::Command(command) => AlignmentKey::Command {
            delivery: command.delivery,
            command: &command.command.command,
            control_sequence: command.command.control_sequence.as_deref(),
            location: command
                .command
                .location
                .as_ref()
                .map(|location| (location.source.as_str(), location.line, location.byte)),
        },
        Event::Input(input) => AlignmentKey::Input {
            transition: input.transition,
            reason: input.reason,
            name: &input.name,
        },
        Event::Recovery(recovery) => AlignmentKey::Recovery {
            kind: recovery.kind,
        },
        Event::ScannerStatus(_) => AlignmentKey::ScannerStatus,
        Event::Macro(MacroEvent::Argument { parameter, .. }) => AlignmentKey::MacroArgument {
            parameter: *parameter,
        },
        Event::Macro(MacroEvent::Activation {
            control_sequence,
            argument_count,
        }) => AlignmentKey::MacroActivation {
            control_sequence,
            argument_count: *argument_count,
        },
        Event::Condition(condition) => AlignmentKey::Condition {
            transition: condition.transition,
            condition: &condition.condition,
        },
        Event::Scanner(scanner) => AlignmentKey::Scanner {
            scanner: &scanner.scanner,
        },
        Event::TokenList(list) => AlignmentKey::TokenList {
            transition: list.transition,
            purpose: &list.purpose,
        },
        Event::Alignment(alignment) => AlignmentKey::Alignment {
            transition: alignment.transition,
        },
        Event::Mutation(mutation) => AlignmentKey::Mutation {
            target: &mutation.target,
        },
        Event::Diagnostic(diagnostic) => AlignmentKey::Diagnostic {
            severity: diagnostic.severity,
            diagnostic: &diagnostic.diagnostic,
        },
        Event::Effect(effect) => AlignmentKey::Effect {
            kind: effect.kind,
            channel: &effect.channel,
        },
        Event::Geometry(GeometryEvent::Hpack { .. }) => AlignmentKey::Geometry {
            transition: "hpack",
        },
        Event::Geometry(GeometryEvent::Vpack { .. }) => AlignmentKey::Geometry {
            transition: "vpack",
        },
        Event::Geometry(GeometryEvent::Shipout { .. }) => AlignmentKey::Geometry {
            transition: "shipout",
        },
    }
}

/// A high-salience structural boundary both streams can be rejoined at when the
/// bounded window search has already failed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AnchorKey<'a> {
    /// An input-stack push, retire, or stop: a real nesting boundary.
    Input {
        transition: InputTransition,
        reason: InputReason,
        name: &'a str,
    },
    /// The first delivery attributed to a new source line.
    Line { source: &'a str, line: u32 },
}

/// How much of the document an anchor kind actually identifies.
///
/// The two anchor kinds are not equally identifying, and treating them as if
/// they were is what lets a rejoin land the streams on different parts of the
/// document. A [`AnchorKey::Line`] names a physical position in a named source
/// file, so a shared one is positive evidence that both streams are at the
/// *same point in the document*. An [`AnchorKey::Input`] names only the shape
/// of a nesting boundary: every macro push in the run carries the identical
/// key `Push/Macro macro`, and so does every backup push, so a shared one is
/// evidence of nothing beyond "both sides pushed something".
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum AnchorClass {
    /// A shared source position: the same line of the same file on both sides.
    Position,
    /// A shared nesting boundary of the same shape, at an unknown position.
    Shape,
}

impl AnchorKey<'_> {
    fn class(&self) -> AnchorClass {
        match self {
            Self::Line { .. } => AnchorClass::Position,
            Self::Input { .. } => AnchorClass::Shape,
        }
    }
}

struct Aligner<'a> {
    fixture: &'a str,
    expected: Vec<&'a Event>,
    actual_events: Vec<&'a Event>,
    actual: &'a [ObservedEvent],
    expected_keys: Vec<AlignmentKey<'a>>,
    actual_keys: Vec<AlignmentKey<'a>>,
    tuning: AlignmentTuning,
}

impl<'a> Aligner<'a> {
    fn new(
        fixture: &'a str,
        expected: &'a [NormalizedEvent],
        actual: &'a [ObservedEvent],
        tuning: AlignmentTuning,
    ) -> Self {
        let expected: Vec<&Event> = expected.iter().map(|event| &event.semantic).collect();
        let actual_events: Vec<&Event> = actual.iter().map(|event| &event.event).collect();
        let expected_keys = expected
            .iter()
            .map(|event| alignment_key(event))
            .collect::<Vec<_>>();
        let actual_keys = actual_events
            .iter()
            .map(|event| alignment_key(event))
            .collect::<Vec<_>>();
        Self {
            fixture,
            expected,
            actual_events,
            actual,
            expected_keys,
            actual_keys,
            tuning,
        }
    }

    /// Aligns the two streams, returning the ordered divergences and whether
    /// `max_results` stopped the walk before either stream was exhausted.
    fn align(&self, max_results: usize) -> (Vec<StreamMismatch>, bool) {
        let mut entries = Vec::new();
        let (mut expected_index, mut actual_index) = (0usize, 0usize);
        loop {
            if entries.len() >= max_results {
                // Every other exit from this loop is terminal and leaves
                // through `break`, so reaching the budget here is the one case
                // where unexamined events remain.
                let remaining =
                    expected_index < self.expected.len() || actual_index < self.actual.len();
                return (entries, remaining);
            }
            match (
                expected_index < self.expected.len(),
                actual_index < self.actual.len(),
            ) {
                (false, false) => break,
                (true, false) => {
                    entries.push(self.entry(
                        expected_index,
                        actual_index,
                        Repair::Truncated {
                            remaining: self.expected.len() - expected_index,
                        },
                        MismatchSides::ExpectedOnly(Box::new(
                            self.expected[expected_index].clone(),
                        )),
                    ));
                    break;
                }
                (false, true) => {
                    entries.push(self.entry(
                        expected_index,
                        actual_index,
                        Repair::Truncated {
                            remaining: self.actual.len() - actual_index,
                        },
                        MismatchSides::ActualOnly(Box::new(self.actual[actual_index].clone())),
                    ));
                    break;
                }
                (true, true) => {}
            }

            if self.expected_keys[expected_index] == self.actual_keys[actual_index] {
                if !events_match(
                    Some(self.expected[expected_index]),
                    Some(&self.actual[actual_index].event),
                ) {
                    entries.push(self.both(expected_index, actual_index, Repair::Changed));
                }
                expected_index += 1;
                actual_index += 1;
                continue;
            }

            if let Some((expected_skip, actual_skip)) = self.realign(expected_index, actual_index) {
                let repair = match (expected_skip, actual_skip) {
                    (0, actual) => Repair::Extra { count: actual },
                    (expected, 0) => Repair::Dropped { count: expected },
                    (expected, actual) => Repair::Replaced { expected, actual },
                };
                entries.push(self.both(expected_index, actual_index, repair));
                expected_index += expected_skip;
                actual_index += actual_skip;
                continue;
            }

            match self.anchor_resync(expected_index, actual_index) {
                Some((expected_skip, actual_skip, anchor)) => {
                    entries.push(self.both(
                        expected_index,
                        actual_index,
                        Repair::AnchorResync {
                            expected_skipped: expected_skip,
                            actual_skipped: actual_skip,
                            anchor,
                        },
                    ));
                    expected_index += expected_skip;
                    actual_index += actual_skip;
                }
                None => {
                    entries.push(self.both(expected_index, actual_index, Repair::Abandoned));
                    break;
                }
            }
        }
        (entries, false)
    }

    fn both(&self, expected_index: usize, actual_index: usize, repair: Repair) -> StreamMismatch {
        self.entry(
            expected_index,
            actual_index,
            repair,
            MismatchSides::Both {
                expected: Box::new(self.expected[expected_index].clone()),
                actual: Box::new(self.actual[actual_index].clone()),
            },
        )
    }

    fn entry(
        &self,
        expected_index: usize,
        actual_index: usize,
        repair: Repair,
        sides: MismatchSides,
    ) -> StreamMismatch {
        StreamMismatch {
            fixture: self.fixture.into(),
            index: expected_index,
            actual_index,
            kind: classify_mismatch_kind(&sides),
            repair,
            sides,
            suppressed_cascade: 0,
        }
    }

    /// Bounded wavefront search for the earliest local repair.
    ///
    /// Candidates are visited in ascending total edit distance, and within one
    /// distance in ascending oracle skip, so the smallest repair always wins
    /// and the tie-break is deterministic. A candidate is only accepted once
    /// [`AlignmentTuning::confirmation`] consecutive key-equal pairs follow it.
    fn realign(&self, expected_index: usize, actual_index: usize) -> Option<(usize, usize)> {
        let window = self.tuning.window;
        for distance in 1..=2 * window {
            let lowest = distance.saturating_sub(window);
            for expected_skip in lowest..=distance.min(window) {
                let actual_skip = distance - expected_skip;
                if expected_index + expected_skip > self.expected.len()
                    || actual_index + actual_skip > self.actual.len()
                {
                    continue;
                }
                if self.confirms(expected_index + expected_skip, actual_index + actual_skip) {
                    return Some((expected_skip, actual_skip));
                }
            }
        }
        None
    }

    /// Whether the two streams agree on their next
    /// [`AlignmentTuning::confirmation`] alignment keys.
    ///
    /// Near the end of a stream fewer than `confirmation` pairs remain; those
    /// are accepted when *every* remaining pair agrees, so that a defect in the
    /// last few events is still repaired rather than abandoned. Mutual
    /// exhaustion with no agreeing pair at all is not a confirmation: skipping
    /// both streams to their ends is a total fork, not a repair.
    fn confirms(&self, expected_index: usize, actual_index: usize) -> bool {
        let mut agreed = 0;
        for offset in 0..self.tuning.confirmation {
            match (
                self.expected_keys.get(expected_index + offset),
                self.actual_keys.get(actual_index + offset),
            ) {
                (Some(expected), Some(actual)) if expected == actual => agreed += 1,
                (None, None) => return agreed > 0,
                _ => return false,
            }
        }
        true
    }

    /// The one structural fallback: rejoin both streams at the most
    /// identifying shared boundary in reach, still requiring the usual
    /// confirmation.
    ///
    /// The two anchor classes are searched in [`AnchorClass`] order, and only
    /// within one class does least total skip decide. Ranking every anchor by
    /// skip alone was the defect this replaced: a rejoin's job is to put both
    /// streams back at the *same point in the document*, and a nearby
    /// `Push/Backup backup` says nothing about position, so it routinely
    /// outbid the shared source line that did. The streams then continued from
    /// two different places, every later structural divergence inherited that
    /// offset, and once it exceeded [`AlignmentTuning::anchor_scan`] the
    /// fixture had no shared anchor left in reach and was abandoned -- a fork
    /// reported hundreds of thousands of events after the cheap rejoin that
    /// caused it.
    ///
    /// Within a class the search is exhaustive over the anchors inside
    /// [`AlignmentTuning::anchor_scan`] on each side, because a rejoin costing
    /// more than that class's minimum has the same failure mode in miniature.
    /// Both anchor lists are in ascending offset order, so once a confirmed
    /// candidate exists every later candidate whose skip already equals or
    /// exceeds it can be cut without a key comparison.
    fn anchor_resync(
        &self,
        expected_index: usize,
        actual_index: usize,
    ) -> Option<(usize, usize, ResyncAnchor)> {
        let expected_anchors =
            collect_anchors(&self.expected, expected_index, self.tuning.anchor_scan);
        let actual_anchors =
            collect_anchors(&self.actual_events, actual_index, self.tuning.anchor_scan);

        [AnchorClass::Position, AnchorClass::Shape]
            .into_iter()
            .find_map(|class| {
                self.cheapest_shared_anchor(
                    expected_index,
                    actual_index,
                    &expected_anchors,
                    &actual_anchors,
                    class,
                )
            })
    }

    /// The least-total-skip confirmed rejoin over one anchor class.
    fn cheapest_shared_anchor(
        &self,
        expected_index: usize,
        actual_index: usize,
        expected_anchors: &[(usize, AnchorKey<'a>)],
        actual_anchors: &[(usize, AnchorKey<'a>)],
        class: AnchorClass,
    ) -> Option<(usize, usize, ResyncAnchor)> {
        let mut best: Option<(usize, usize, usize, ResyncAnchor)> = None;
        for (expected_skip, expected_anchor) in expected_anchors {
            if best
                .as_ref()
                .is_some_and(|(best, _, _, _)| *best <= *expected_skip)
            {
                break;
            }
            if expected_anchor.class() != class {
                continue;
            }
            for (actual_skip, actual_anchor) in actual_anchors {
                let cost = expected_skip + actual_skip;
                if best.as_ref().is_some_and(|(best, _, _, _)| *best <= cost) {
                    break;
                }
                if expected_anchor != actual_anchor {
                    continue;
                }
                if !self.confirms(expected_index + expected_skip, actual_index + actual_skip) {
                    continue;
                }
                best = Some((
                    cost,
                    *expected_skip,
                    *actual_skip,
                    ResyncAnchor::from(expected_anchor),
                ));
            }
        }
        best.map(|(_, expected_skip, actual_skip, anchor)| (expected_skip, actual_skip, anchor))
    }
}

/// Collects every structural anchor within `scan` events of `start`, as
/// offsets from `start`, in ascending offset order.
fn collect_anchors<'a>(
    events: &[&'a Event],
    start: usize,
    scan: usize,
) -> Vec<(usize, AnchorKey<'a>)> {
    let mut anchors = Vec::new();
    let mut previous_line: Option<(&str, u32)> = None;
    let limit = events.len().min(start.saturating_add(scan));
    for (offset, event) in events[start..limit].iter().enumerate() {
        match event {
            Event::Input(input) => anchors.push((
                offset,
                AnchorKey::Input {
                    transition: input.transition,
                    reason: input.reason,
                    name: &input.name,
                },
            )),
            Event::Command(command) => {
                let Some(location) = command.command.location.as_ref() else {
                    continue;
                };
                let position = (location.source.as_str(), location.line);
                if previous_line == Some(position) {
                    continue;
                }
                previous_line = Some(position);
                anchors.push((
                    offset,
                    AnchorKey::Line {
                        source: location.source.as_str(),
                        line: location.line,
                    },
                ));
            }
            _ => {}
        }
    }
    anchors
}

/// Records, on each entry, how many mismatches plain index-aligned comparison
/// would have reported for the stream region that entry covers.
///
/// The region runs from the entry's oracle index up to the next reported
/// entry's oracle index -- to the end of the streams for the last entry -- and
/// the entry's own position is not counted. This is the cascade the entry
/// stands in for: under index-aligned comparison a single desynchronizing
/// defect reports one mismatch per following index, and that count is exactly
/// what a coordinator needs to tell "one root site" from "many".
fn attribute_suppressed_cascade(
    entries: &mut [StreamMismatch],
    expected: &[NormalizedEvent],
    actual: &[ObservedEvent],
) {
    if entries.is_empty() {
        return;
    }
    let indexed: Vec<usize> = (0..expected.len().max(actual.len()))
        .filter(|index| {
            !events_match(
                expected.get(*index).map(|event| &event.semantic),
                actual.get(*index).map(|event| &event.event),
            )
        })
        .collect();

    let mut cursor = 0;
    for position in 0..entries.len() {
        let start = entries[position].index;
        let end = entries
            .get(position + 1)
            .map_or(usize::MAX, |entry| entry.index);
        while cursor < indexed.len() && indexed[cursor] < start {
            cursor += 1;
        }
        let mut suppressed = 0;
        while cursor < indexed.len() && indexed[cursor] < end {
            if indexed[cursor] != start {
                suppressed += 1;
            }
            cursor += 1;
        }
        entries[position].suppressed_cascade = suppressed;
    }
}

/// Cheap structural label for one divergence, computed from the already
/// available events without any additional engine work.
///
/// Exhaustive over every oracle event kind: a new schema variant must be
/// labeled here rather than silently collapsing into a generic bucket.
pub(crate) fn classify_mismatch_kind(sides: &MismatchSides) -> &'static str {
    let (expected, actual) = match sides {
        MismatchSides::Both { expected, actual } => (expected.as_ref(), &actual.event),
        MismatchSides::ExpectedOnly(_) => return "stream_truncated_early",
        MismatchSides::ActualOnly(_) => return "stream_has_unexpected_trailing_events",
    };
    match (expected, actual) {
        (Event::Command(expected), Event::Command(actual)) => {
            classify_command_mismatch_kind(expected, actual)
        }
        (Event::Input(_), Event::Input(_)) => "input_transition_mismatch",
        (Event::Recovery(_), Event::Recovery(_)) => "recovery_mismatch",
        (Event::ScannerStatus(_), Event::ScannerStatus(_)) => "scanner_status_mismatch",
        (Event::Macro(_), Event::Macro(_)) => "macro_transition_mismatch",
        (Event::Condition(_), Event::Condition(_)) => "condition_mismatch",
        (Event::Scanner(_), Event::Scanner(_)) => "scanner_result_mismatch",
        (Event::TokenList(_), Event::TokenList(_)) => "token_list_mismatch",
        (Event::Alignment(_), Event::Alignment(_)) => "alignment_mismatch",
        (Event::Mutation(_), Event::Mutation(_)) => "mutation_mismatch",
        (Event::Diagnostic(_), Event::Diagnostic(_)) => "diagnostic_mismatch",
        (Event::Effect(_), Event::Effect(_)) => "effect_mismatch",
        (Event::Geometry(_), Event::Geometry(_)) => "geometry_mismatch",
        (
            Event::Command(_)
            | Event::Input(_)
            | Event::Recovery(_)
            | Event::ScannerStatus(_)
            | Event::Macro(_)
            | Event::Condition(_)
            | Event::Scanner(_)
            | Event::TokenList(_)
            | Event::Alignment(_)
            | Event::Mutation(_)
            | Event::Diagnostic(_)
            | Event::Effect(_)
            | Event::Geometry(_),
            _,
        ) => "event_kind_mismatch",
    }
}

fn classify_command_mismatch_kind(expected: &CommandEvent, actual: &CommandEvent) -> &'static str {
    if expected.command.command != actual.command.command {
        "command_identity_mismatch"
    } else if expected.command.operand != actual.command.operand {
        "command_operand_mismatch"
    } else if expected.command.control_sequence != actual.command.control_sequence {
        "command_spelling_mismatch"
    } else if expected.delivery != actual.delivery {
        "command_delivery_mismatch"
    } else {
        "command_location_mismatch"
    }
}
