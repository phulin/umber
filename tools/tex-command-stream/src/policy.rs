//! Named host-side policies for comparing canonical observation streams.

use std::collections::BTreeSet;
use std::fmt;

use tex_oracle::{
    CanonicalCommand, CanonicalValue, CommandDelivery, CommandEvent, Event, MutationEvent,
    NormalizedEvent, ObservationStream, SchemaVersion, StateTarget,
};

use crate::compare::{AlignmentTuning, find_divergences};
use crate::{Divergence, ObservedEvent, group};

/// The bounded, realigning policy used by ordinary differential comparisons.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OrdinaryComparisonPolicy {
    pub max_divergences: usize,
    pub alignment: AlignmentTuning,
}

/// Accounting returned in the same pass as an ordinary comparison.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct OrdinaryComparisonAccounting {
    pub ordered_divergences: usize,
    pub root_sites: usize,
    pub budget_reached: bool,
}

/// Divergences and their complete bounded-run accounting.
#[derive(Clone, Debug)]
pub struct OrdinaryComparison {
    pub divergences: Vec<Divergence>,
    pub accounting: OrdinaryComparisonAccounting,
}

impl OrdinaryComparisonPolicy {
    #[must_use]
    pub fn compare(
        self,
        fixture: &str,
        expected: &[NormalizedEvent],
        actual: &[ObservedEvent],
    ) -> OrdinaryComparison {
        let comparison = find_divergences(
            fixture,
            expected,
            actual,
            self.max_divergences,
            self.alignment,
        );
        let budget_reached = comparison.budget_reached;
        let divergences = comparison
            .entries
            .into_iter()
            .map(Box::new)
            .map(Divergence::Mismatch)
            .collect::<Vec<_>>();
        let accounting = OrdinaryComparisonAccounting {
            ordered_divergences: divergences.len(),
            root_sites: group(&divergences).len(),
            budget_reached,
        };
        OrdinaryComparison {
            divergences,
            accounting,
        }
    }
}

/// Strict TRIP stream channel and its schema contract.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StrictTripChannel {
    Command,
    Geometry,
}

impl StrictTripChannel {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Command => "command_events",
            Self::Geometry => "geometry_events",
        }
    }
}

/// The first strict positional divergence.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StrictTripDivergence {
    Presence,
    Header {
        expected: String,
        actual: String,
    },
    Event {
        index: usize,
        expected: Option<Box<NormalizedEvent>>,
        actual: Option<Box<NormalizedEvent>>,
    },
}

/// Counts over both raw streams and the strict TRIP projection.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct StrictTripAccounting {
    pub expected_events: Option<usize>,
    pub actual_events: Option<usize>,
    pub projected_equivalent: Option<usize>,
    pub projected_divergences: Option<usize>,
}

/// Strict divergence and accounting produced by one parse and projection walk.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StrictTripComparison {
    pub divergence: Option<StrictTripDivergence>,
    pub accounting: StrictTripAccounting,
}

/// Inputs to the strict positional policy used by two-phase TRIP comparison.
#[derive(Clone, Copy, Debug)]
pub struct StrictTripComparisonPolicy<'a> {
    pub channel: StrictTripChannel,
    pub expected_initialization: Option<&'a [u8]>,
    pub actual_initialization: Option<&'a [u8]>,
}

impl StrictTripComparisonPolicy<'_> {
    pub fn compare(
        self,
        expected: Option<&[u8]>,
        actual: Option<&[u8]>,
    ) -> Result<StrictTripComparison, StrictTripError> {
        let accounting = StrictTripAccounting {
            expected_events: expected.map(event_count),
            actual_events: actual.map(event_count),
            projected_equivalent: None,
            projected_divergences: None,
        };
        let (Some(expected), Some(actual)) = (expected, actual) else {
            return Ok(StrictTripComparison {
                divergence: (expected.is_some() != actual.is_some())
                    .then_some(StrictTripDivergence::Presence),
                accounting,
            });
        };
        let expected = parse_stream(expected, "expected", self.channel)?;
        let actual = parse_stream(actual, "actual", self.channel)?;
        if expected.header.schema != actual.header.schema {
            return Err(StrictTripError(format!(
                "{} schema mismatch: expected v{}, actual v{}",
                self.channel.name(),
                expected.header.schema,
                actual.header.schema
            )));
        }
        let header_divergence =
            (expected.header != actual.header).then(|| StrictTripDivergence::Header {
                expected: expected.header.manifest.clone(),
                actual: actual.header.manifest.clone(),
            });

        let mut projection = TripProjection::from_initialization(
            self.expected_initialization,
            self.actual_initialization,
        )?;
        let mut expected_events = projected(self.channel, &expected.events);
        let mut actual_events = projected(self.channel, &actual.events);
        let mut first = header_divergence;
        let mut projected_equivalent = expected.events.len()
            - projected_len(self.channel, &expected.events)
            + actual.events.len()
            - projected_len(self.channel, &actual.events);
        let mut projected_divergences = 0;
        let mut index = 0;
        loop {
            match (expected_events.next(), actual_events.next()) {
                (Some(expected), Some(actual)) => {
                    let matches = projection.events_match(expected.1, actual.1);
                    projected_equivalent +=
                        usize::from(expected.1.semantic != actual.1.semantic && matches);
                    projected_divergences += usize::from(!matches);
                    if !matches && first.is_none() {
                        first = Some(StrictTripDivergence::Event {
                            index,
                            expected: Some(Box::new(projected_event(index, expected.1))),
                            actual: Some(Box::new(projected_event(index, actual.1))),
                        });
                    }
                }
                (Some(expected), None) => {
                    projected_divergences += 1;
                    if first.is_none() {
                        first = Some(StrictTripDivergence::Event {
                            index,
                            expected: Some(Box::new(projected_event(index, expected.1))),
                            actual: None,
                        });
                    }
                    projected_divergences += expected_events.count();
                    break;
                }
                (None, Some(actual)) => {
                    projected_divergences += 1;
                    if first.is_none() {
                        first = Some(StrictTripDivergence::Event {
                            index,
                            expected: None,
                            actual: Some(Box::new(projected_event(index, actual.1))),
                        });
                    }
                    projected_divergences += actual_events.count();
                    break;
                }
                (None, None) => break,
            }
            index += 1;
        }
        Ok(StrictTripComparison {
            divergence: first,
            accounting: StrictTripAccounting {
                expected_events: Some(expected.events.len()),
                actual_events: Some(actual.events.len()),
                projected_equivalent: Some(projected_equivalent),
                projected_divergences: Some(projected_divergences),
            },
        })
    }
}

fn parse_stream(
    bytes: &[u8],
    side: &'static str,
    channel: StrictTripChannel,
) -> Result<ObservationStream, StrictTripError> {
    let stream = ObservationStream::from_canonical_json_lines(bytes).map_err(|error| {
        StrictTripError(format!(
            "{side} TRIP command-event stream is not canonical schema-v1 JSONL: {error}"
        ))
    })?;
    let schema = SchemaVersion::try_from(stream.header.schema).map_err(StrictTripError)?;
    match channel {
        StrictTripChannel::Command if schema != SchemaVersion::V1 => {
            return Err(StrictTripError(
                "command_events must use canonical schema-v1 on both sides".into(),
            ));
        }
        StrictTripChannel::Geometry if schema < SchemaVersion::V2 => {
            return Err(StrictTripError(
                "geometry_events require a canonical geometry schema".into(),
            ));
        }
        _ => {}
    }
    Ok(stream)
}

fn projected(
    channel: StrictTripChannel,
    events: &[NormalizedEvent],
) -> impl Iterator<Item = (usize, &NormalizedEvent)> {
    events.iter().enumerate().filter(move |(_, event)| {
        channel != StrictTripChannel::Command
            || !matches!(
                &event.semantic,
                Event::TokenList(token_list)
                    if token_list.purpose == "protected_delivery_suppression"
            )
    })
}

fn projected_len(channel: StrictTripChannel, events: &[NormalizedEvent]) -> usize {
    projected(channel, events).count()
}

fn projected_event(sequence: usize, event: &NormalizedEvent) -> NormalizedEvent {
    NormalizedEvent {
        sequence: sequence as u64,
        semantic: event.semantic.clone(),
    }
}

fn event_count(bytes: &[u8]) -> usize {
    bytes
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .count()
        .saturating_sub(1)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StrictTripError(String);

impl fmt::Display for StrictTripError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for StrictTripError {}

#[derive(Default)]
struct TripProjection {
    matched_macros: BTreeSet<String>,
    explicit_group_macro_scopes: Vec<BTreeSet<String>>,
}

impl TripProjection {
    fn from_initialization(
        expected: Option<&[u8]>,
        actual: Option<&[u8]>,
    ) -> Result<Self, StrictTripError> {
        let mut projection = Self::default();
        let (Some(expected), Some(actual)) = (expected, actual) else {
            return Ok(projection);
        };
        let expected = ObservationStream::from_canonical_json_lines(expected).map_err(|error| {
            StrictTripError(format!(
                "expected initialization history is not canonical schema-v1 JSONL: {error}"
            ))
        })?;
        let actual = ObservationStream::from_canonical_json_lines(actual).map_err(|error| {
            StrictTripError(format!(
                "actual initialization history is not canonical schema-v1 JSONL: {error}"
            ))
        })?;
        let mut expected_events = projected(StrictTripChannel::Command, &expected.events);
        let mut actual_events = projected(StrictTripChannel::Command, &actual.events);
        loop {
            match (expected_events.next(), actual_events.next()) {
                (Some(expected), Some(actual)) if projection.events_match(expected.1, actual.1) => {
                }
                (Some(_), Some(_)) => return Ok(Self::default()),
                (Some(event), None) => {
                    invalidate_meaning(&mut projection, &event.1.semantic);
                    for event in expected_events {
                        invalidate_meaning(&mut projection, &event.1.semantic);
                    }
                    break;
                }
                (None, Some(event)) => {
                    invalidate_meaning(&mut projection, &event.1.semantic);
                    for event in actual_events {
                        invalidate_meaning(&mut projection, &event.1.semantic);
                    }
                    break;
                }
                (None, None) => break,
            }
        }
        Ok(projection)
    }

    fn events_match(&mut self, expected: &NormalizedEvent, actual: &NormalizedEvent) -> bool {
        let matches = expected.semantic == actual.semantic
            || macro_call_operand_is_reference(
                &expected.semantic,
                &actual.semantic,
                &self.matched_macros,
            )
            || frozen_endwrite_operand_is_reference(&expected.semantic, &actual.semantic)
            || sparse_register_operand_is_reference(&expected.semantic, &actual.semantic);
        self.observe_meaning_mutations(&expected.semantic, &actual.semantic);
        self.observe_explicit_group_boundary(&expected.semantic, &actual.semantic);
        matches
    }

    fn observe_meaning_mutations(&mut self, expected: &Event, actual: &Event) {
        let expected = meaning_mutation(expected);
        let actual = meaning_mutation(actual);
        for name in expected
            .map(|(name, _)| name)
            .into_iter()
            .chain(actual.map(|(name, _)| name))
        {
            self.matched_macros.remove(name);
        }
        let (Some((expected_name, expected_mutation)), Some((actual_name, actual_mutation))) =
            (expected, actual)
        else {
            return;
        };
        if expected_name == actual_name
            && expected_mutation == actual_mutation
            && matches!(expected_mutation.value, CanonicalValue::Tokens(_))
        {
            self.matched_macros.insert(expected_name.to_owned());
        }
        if expected_name == actual_name
            && expected_mutation == actual_mutation
            && expected_mutation.scope == "global"
        {
            for scope in &mut self.explicit_group_macro_scopes {
                scope.remove(expected_name);
                if matches!(expected_mutation.value, CanonicalValue::Tokens(_)) {
                    scope.insert(expected_name.to_owned());
                }
            }
        }
    }

    fn observe_explicit_group_boundary(&mut self, expected: &Event, actual: &Event) {
        if expected != actual {
            return;
        }
        let Event::Command(CommandEvent {
            delivery: CommandDelivery::Expanded,
            command,
        }) = expected
        else {
            return;
        };
        match command.command.as_str() {
            "begin_group" => self
                .explicit_group_macro_scopes
                .push(self.matched_macros.clone()),
            "end_group" => {
                if let Some(restored) = self.explicit_group_macro_scopes.pop() {
                    self.matched_macros = restored;
                }
            }
            _ => {}
        }
    }
}

fn invalidate_meaning(projection: &mut TripProjection, event: &Event) {
    if let Some((name, _)) = meaning_mutation(event) {
        projection.matched_macros.remove(name);
    }
}

fn meaning_mutation(event: &Event) -> Option<(&str, &MutationEvent)> {
    let Event::Mutation(mutation) = event else {
        return None;
    };
    if mutation.target != StateTarget::Meaning {
        return None;
    }
    let CanonicalValue::Name(name) = &mutation.key else {
        return None;
    };
    Some((name, mutation))
}

fn sparse_register_operand_is_reference(expected: &Event, actual: &Event) -> bool {
    let (
        Event::Command(CommandEvent {
            delivery: ed,
            command:
                CanonicalCommand {
                    command: ec,
                    operand: CanonicalValue::Integer(_),
                    control_sequence: ecs,
                    location: el,
                },
        }),
        Event::Command(CommandEvent {
            delivery: ad,
            command:
                CanonicalCommand {
                    command: ac,
                    operand: CanonicalValue::Name(_),
                    control_sequence: acs,
                    location: al,
                },
        }),
    ) = (expected, actual)
    else {
        return false;
    };
    matches!(ec.as_str(), "register" | "toks_register")
        && ed == ad
        && ec == ac
        && ecs == acs
        && el == al
}

fn macro_call_operand_is_reference(
    expected: &Event,
    actual: &Event,
    matched: &BTreeSet<String>,
) -> bool {
    let (
        Event::Command(CommandEvent {
            delivery: ed,
            command:
                CanonicalCommand {
                    command: ec,
                    operand: CanonicalValue::Integer(_),
                    control_sequence: Some(ecs),
                    location: el,
                },
        }),
        Event::Command(CommandEvent {
            delivery: ad,
            command:
                CanonicalCommand {
                    command: ac,
                    operand: CanonicalValue::Integer(_) | CanonicalValue::None,
                    control_sequence: acs,
                    location: al,
                },
        }),
    ) = (expected, actual)
    else {
        return false;
    };
    matched.contains(ecs)
        && matches!(
            ec.as_str(),
            "call" | "long_call" | "outer_call" | "long_outer_call"
        )
        && ed == ad
        && ec == ac
        && Some(ecs) == acs.as_ref()
        && el == al
}

fn frozen_endwrite_operand_is_reference(expected: &Event, actual: &Event) -> bool {
    let (
        Event::Command(CommandEvent {
            delivery: ed,
            command:
                CanonicalCommand {
                    command: ec,
                    operand: CanonicalValue::Integer(_),
                    control_sequence: ecs,
                    location: None,
                },
        }),
        Event::Command(CommandEvent {
            delivery: ad,
            command:
                CanonicalCommand {
                    command: ac,
                    operand: CanonicalValue::Integer(_) | CanonicalValue::None,
                    control_sequence: acs,
                    location: None,
                },
        }),
    ) = (expected, actual)
    else {
        return false;
    };
    ed == ad && ec == "outer_call" && ac == ec && ecs.as_deref() == Some("endwrite") && acs == ecs
}

#[cfg(test)]
mod tests;
