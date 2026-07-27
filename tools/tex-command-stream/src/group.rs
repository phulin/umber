//! Presentation-level collapse of exact recurrences of one root site.
//!
//! The comparator (`crate::compare`) already reports root sites rather than
//! index cascade, but one defect still reaches the worklist once per *source
//! position* it recurs at. A preload loop that assigns the same wrong meaning
//! forty-eight times is forty-eight entries that are byte-identical apart from
//! their [`SourceLocation`]s, and a coordinator triaging the worklist by hand
//! has to read all forty-eight to learn there is one thing to fix.
//!
//! This module groups those recurrences. It changes nothing about what the
//! comparator counts as a divergence: every divergence still exists, is still
//! ordered, and appears in exactly one group. Grouping only decides how the
//! worklist *prints*.
//!
//! Two divergences are the same root site when they are equal after erasing
//! every source position -- and nothing else. The projection
//! ([`positionless_event`]) is an exhaustive match over the oracle event
//! schema, so a new schema variant has to be handled here explicitly rather
//! than falling into a catch-all that might silently erase something
//! semantic. Nothing else is normalized away: differing operands, differing
//! token payloads, differing repair shapes, and differing macro token-list
//! addresses all keep two entries apart. Under-merging leaves a longer
//! worklist; over-merging hides a second defect behind the first, which is far
//! more expensive, so every judgement call here is made in the conservative
//! direction.

use std::collections::HashMap;

use tex_oracle::{
    AlignmentEvent, CanonicalCommand, CanonicalValue, CommandEvent, ConditionEvent,
    DiagnosticEvent, EffectEvent, Event, InputEvent, MacroEvent, MutationEvent, OracleToken,
    RecoveryEvent, ScannerEvent, ScannerStatusEvent, TokenListEvent,
};

use crate::compare::{MismatchSides, Repair, ResyncAnchor};
use crate::{Divergence, ReplayFailure};

#[cfg(test)]
mod tests;

/// One worklist entry after grouping: a root site and every ordered divergence
/// that is an exact recurrence of it.
///
/// `occurrences` holds every member in stream order and its first element is
/// the representative, so no member is reachable only through the count.
#[derive(Clone, Debug)]
pub struct RootSite<'a> {
    identity: RootSiteIdentity,
    occurrences: Vec<&'a Divergence>,
}

impl<'a> RootSite<'a> {
    /// The first divergence of this root site, in stream order.
    pub fn representative(&self) -> &'a Divergence {
        self.occurrences[0]
    }

    /// Every divergence collapsed into this entry, in stream order.
    pub fn occurrences(&self) -> &[&'a Divergence] {
        &self.occurrences
    }

    /// How many divergences this entry stands for. Never zero.
    pub fn count(&self) -> usize {
        self.occurrences.len()
    }

    /// The fixture identity every member of this group shares.
    pub fn fixture(&self) -> &str {
        match &self.identity {
            RootSiteIdentity::Mismatch { fixture, .. }
            | RootSiteIdentity::Failure { fixture, .. } => fixture,
        }
    }

    /// Summed cascade the members of this group stand in for.
    pub fn suppressed_cascade(&self) -> usize {
        self.occurrences
            .iter()
            .map(|divergence| divergence.suppressed_cascade())
            .sum()
    }
}

/// Collapses exact recurrences, preserving stream order.
///
/// Groups are returned in the order their representatives appear in the
/// ungrouped worklist, and the members of a group likewise, so the report stays
/// an ordered worklist. Every input divergence lands in exactly one group.
pub fn group(divergences: &[Divergence]) -> Vec<RootSite<'_>> {
    let mut sites: Vec<RootSite<'_>> = Vec::new();
    // Bucketing by the two cheapest identity fields keeps the exact comparison
    // below off unrelated groups; it is an index, never a merging rule.
    let mut buckets: HashMap<(&str, &'static str), Vec<usize>> = HashMap::new();
    for divergence in divergences {
        let identity = RootSiteIdentity::of(divergence);
        let bucket = buckets
            .entry((divergence.fixture(), divergence.kind()))
            .or_default();
        match bucket
            .iter()
            .find(|position| sites[**position].identity == identity)
        {
            Some(position) => sites[*position].occurrences.push(divergence),
            None => {
                bucket.push(sites.len());
                sites.push(RootSite {
                    identity,
                    occurrences: vec![divergence],
                });
            }
        }
    }
    sites
}

/// Everything that makes two divergences the same defect rather than the same
/// defect seen twice.
#[derive(Clone, Debug, Eq, PartialEq)]
enum RootSiteIdentity {
    Mismatch {
        fixture: String,
        kind: &'static str,
        repair: Repair,
        sides: PositionlessSides,
    },
    Failure {
        fixture: String,
        failure: ReplayFailure,
    },
}

impl RootSiteIdentity {
    fn of(divergence: &Divergence) -> Self {
        match divergence {
            Divergence::Mismatch(mismatch) => Self::Mismatch {
                fixture: mismatch.fixture.clone(),
                kind: mismatch.kind,
                repair: positionless_repair(&mismatch.repair),
                sides: PositionlessSides::of(&mismatch.sides),
            },
            Divergence::Failure {
                fixture, failure, ..
            } => Self::Failure {
                fixture: fixture.clone(),
                failure: failure.clone(),
            },
        }
    }
}

/// The two events of a divergence with every source position erased. The
/// observed event's diagnostic context is deliberately absent: it is
/// provenance text about *where* the event happened, which is exactly what a
/// recurrence differs in.
#[derive(Clone, Debug, Eq, PartialEq)]
enum PositionlessSides {
    Both { expected: Event, actual: Event },
    ExpectedOnly(Event),
    ActualOnly(Event),
}

impl PositionlessSides {
    fn of(sides: &MismatchSides) -> Self {
        match sides {
            MismatchSides::Both { expected, actual } => Self::Both {
                expected: positionless_event(expected),
                actual: positionless_event(&actual.event),
            },
            MismatchSides::ExpectedOnly(expected) => {
                Self::ExpectedOnly(positionless_event(expected))
            }
            MismatchSides::ActualOnly(actual) => Self::ActualOnly(positionless_event(&actual.event)),
        }
    }
}

/// A repair with the *positions* in it erased and everything structural kept.
///
/// Skip counts stay: "three oracle events dropped" and "twenty-one oracle
/// events dropped" are different defects. Only the anchor's source line is a
/// position, and only when the anchor is a line rather than an input-stack
/// boundary.
fn positionless_repair(repair: &Repair) -> Repair {
    match repair {
        Repair::Changed => Repair::Changed,
        Repair::Dropped { count } => Repair::Dropped { count: *count },
        Repair::Extra { count } => Repair::Extra { count: *count },
        Repair::Replaced { expected, actual } => Repair::Replaced {
            expected: *expected,
            actual: *actual,
        },
        Repair::AnchorResync {
            expected_skipped,
            actual_skipped,
            anchor,
        } => Repair::AnchorResync {
            expected_skipped: *expected_skipped,
            actual_skipped: *actual_skipped,
            anchor: match anchor {
                ResyncAnchor::Input {
                    transition,
                    reason,
                    name,
                } => ResyncAnchor::Input {
                    transition: *transition,
                    reason: *reason,
                    name: name.clone(),
                },
                ResyncAnchor::Line { source, .. } => ResyncAnchor::Line {
                    source: source.clone(),
                    line: 0,
                },
            },
        },
        Repair::Abandoned => Repair::Abandoned,
        Repair::Truncated { remaining } => Repair::Truncated {
            remaining: *remaining,
        },
    }
}

/// One oracle event with every reachable [`tex_oracle::SourceLocation`] erased.
///
/// Exhaustive over the event schema on purpose. A catch-all arm would make a
/// newly added variant carry its positions into the grouping key -- splitting
/// genuine recurrences, which is merely noisy -- but a catch-all that *cloned
/// through* an added position-bearing field would be worse, and neither
/// failure mode announces itself. Adding a schema variant must therefore fail
/// to compile here.
fn positionless_event(event: &Event) -> Event {
    match event {
        Event::Command(CommandEvent { delivery, command }) => Event::Command(CommandEvent {
            delivery: *delivery,
            command: CanonicalCommand {
                command: command.command.clone(),
                operand: positionless_value(&command.operand),
                control_sequence: command.control_sequence.clone(),
                location: None,
            },
        }),
        Event::Input(InputEvent {
            transition,
            reason,
            name,
        }) => Event::Input(InputEvent {
            transition: *transition,
            reason: *reason,
            name: name.clone(),
        }),
        Event::Recovery(RecoveryEvent { kind, tokens }) => Event::Recovery(RecoveryEvent {
            kind: *kind,
            tokens: positionless_tokens(tokens),
        }),
        Event::ScannerStatus(ScannerStatusEvent { from, to }) => {
            Event::ScannerStatus(ScannerStatusEvent {
                from: *from,
                to: *to,
            })
        }
        Event::Macro(MacroEvent::Argument { parameter, tokens }) => {
            Event::Macro(MacroEvent::Argument {
                parameter: *parameter,
                tokens: positionless_tokens(tokens),
            })
        }
        Event::Macro(MacroEvent::Activation {
            control_sequence,
            argument_count,
        }) => Event::Macro(MacroEvent::Activation {
            control_sequence: control_sequence.clone(),
            argument_count: *argument_count,
        }),
        Event::Condition(ConditionEvent {
            transition,
            condition,
            limit,
            branch,
        }) => Event::Condition(ConditionEvent {
            transition: *transition,
            condition: condition.clone(),
            limit: limit.clone(),
            branch: branch.clone(),
        }),
        Event::Scanner(ScannerEvent { scanner, result }) => Event::Scanner(ScannerEvent {
            scanner: scanner.clone(),
            result: positionless_value(result),
        }),
        Event::TokenList(TokenListEvent {
            transition,
            purpose,
            tokens,
        }) => Event::TokenList(TokenListEvent {
            transition: *transition,
            purpose: purpose.clone(),
            tokens: positionless_tokens(tokens),
        }),
        Event::Alignment(AlignmentEvent {
            transition,
            align_state,
            template,
            nesting,
            previous_align_state,
            delimiter,
            recovery,
        }) => Event::Alignment(AlignmentEvent {
            transition: *transition,
            align_state: *align_state,
            template: template.clone(),
            nesting: *nesting,
            previous_align_state: *previous_align_state,
            delimiter: delimiter.clone(),
            recovery: recovery.clone(),
        }),
        Event::Mutation(MutationEvent {
            target,
            key,
            value,
            scope,
        }) => Event::Mutation(MutationEvent {
            target: target.clone(),
            key: positionless_value(key),
            value: positionless_value(value),
            scope: scope.clone(),
        }),
        Event::Diagnostic(DiagnosticEvent {
            severity,
            diagnostic,
            arguments,
        }) => Event::Diagnostic(DiagnosticEvent {
            severity: *severity,
            diagnostic: diagnostic.clone(),
            arguments: arguments.iter().map(positionless_value).collect(),
        }),
        Event::Effect(EffectEvent {
            kind,
            channel,
            value,
        }) => Event::Effect(EffectEvent {
            kind: *kind,
            channel: channel.clone(),
            value: positionless_value(value),
        }),
    }
}

/// One canonical value with the positions of any tokens it carries erased.
/// Exhaustive for the same reason as [`positionless_event`].
fn positionless_value(value: &CanonicalValue) -> CanonicalValue {
    match value {
        CanonicalValue::None => CanonicalValue::None,
        CanonicalValue::Bool(value) => CanonicalValue::Bool(*value),
        CanonicalValue::Integer(value) => CanonicalValue::Integer(*value),
        CanonicalValue::Character(value) => CanonicalValue::Character(*value),
        CanonicalValue::Scaled(value) => CanonicalValue::Scaled(*value),
        CanonicalValue::Glue {
            width,
            stretch,
            stretch_order,
            shrink,
            shrink_order,
        } => CanonicalValue::Glue {
            width: *width,
            stretch: *stretch,
            stretch_order: stretch_order.clone(),
            shrink: *shrink,
            shrink_order: shrink_order.clone(),
        },
        CanonicalValue::Token(token) => CanonicalValue::Token(positionless_token(token)),
        CanonicalValue::Tokens(tokens) => CanonicalValue::Tokens(positionless_tokens(tokens)),
        CanonicalValue::Name(name) => CanonicalValue::Name(name.clone()),
        CanonicalValue::Bytes(bytes) => CanonicalValue::Bytes(bytes.clone()),
    }
}

fn positionless_tokens(tokens: &[OracleToken]) -> Vec<OracleToken> {
    tokens.iter().map(positionless_token).collect()
}

fn positionless_token(token: &OracleToken) -> OracleToken {
    OracleToken {
        character: token.character,
        catcode: token.catcode.clone(),
        control_sequence: token.control_sequence.clone(),
        location: None,
    }
}
