//! Presentation-level collapse of exact recurrences of one root site.
//!
//! The comparator (`crate::compare`) already reports root sites rather than
//! index cascade, but one defect still reaches the worklist once per *source
//! position* it recurs at. A preload loop that assigns the same wrong meaning
//! forty-eight times is forty-eight entries that are byte-identical apart from
//! their [`tex_oracle::SourceLocation`]s, and a coordinator triaging by hand
//! has to read all forty-eight to learn there is one thing to fix.
//!
//! This module groups those recurrences. It changes nothing about what the
//! comparator counts as a divergence: every divergence still exists, is still
//! ordered, and appears in exactly one group. Grouping only decides how the
//! worklist *prints*.
//!
//! Two divergences are the same root site when they are equal after erasing
//! every source position -- and nothing else. The projection delegates to
//! `tex-oracle`'s exhaustive schema-owned location erasure, so new variants
//! cannot silently change what merges. Nothing else is normalized away:
//! differing operands, differing
//! token payloads, differing repair shapes, and differing macro token-list
//! addresses all keep two entries apart. Under-merging leaves a longer
//! worklist; over-merging hides a second defect behind the first, which is far
//! more expensive, so every judgement call here is made in the conservative
//! direction.

use std::collections::HashMap;

use tex_oracle::Event;
#[cfg(test)]
use tex_oracle::{
    CanonicalCommand, CanonicalValue, CommandEvent, DiagnosticEvent, EffectEvent, MacroEvent,
    MutationEvent, OracleToken, RecoveryEvent, TokenListEvent,
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
    Both {
        expected: Box<Event>,
        actual: Box<Event>,
    },
    ExpectedOnly(Box<Event>),
    ActualOnly(Box<Event>),
}

impl PositionlessSides {
    fn of(sides: &MismatchSides) -> Self {
        match sides {
            MismatchSides::Both { expected, actual } => Self::Both {
                expected: Box::new(positionless_event(expected)),
                actual: Box::new(positionless_event(&actual.event)),
            },
            MismatchSides::ExpectedOnly(expected) => {
                Self::ExpectedOnly(Box::new(positionless_event(expected)))
            }
            MismatchSides::ActualOnly(actual) => {
                Self::ActualOnly(Box::new(positionless_event(&actual.event)))
            }
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
/// The schema-owned projection is exhaustive: newly added position-bearing
/// fields must be handled there before this grouping key can compile.
fn positionless_event(event: &Event) -> Event {
    event.without_locations()
}
