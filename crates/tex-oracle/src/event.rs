//! Exhaustive borrowed views and schema-owned event projections.
//!
//! Every operation in this module enters through [`EventView`] or
//! [`EventViewMut`]. Adding an [`Event`] variant therefore makes the central
//! view constructors fail to compile instead of leaving downstream walkers
//! to drift independently.

use std::fmt;

use crate::{
    AlignmentEvent, AlignmentTransition, CanonicalCommand, CanonicalValue, CommandDelivery,
    CommandEvent, ConditionEvent, ConditionTransition, DiagnosticEvent, DiagnosticLifecycleEvent,
    DiagnosticSeverity, EffectEvent, EffectKind, Event, GeometryEvent, GeometryLocation,
    InputEvent, InputReason, InputTransition, MacroEvent, MutationEvent, OracleToken,
    RecoveryEvent, RecoveryKind, ScannerEvent, ScannerStatusEvent, SourceLocation, StateTarget,
    TokenListEvent, TokenListTransition,
};

/// The concrete payload borrowed from any oracle event.
#[derive(Clone, Copy, Debug)]
pub enum EventView<'a> {
    Command(&'a CommandEvent),
    Input(&'a InputEvent),
    Recovery(&'a RecoveryEvent),
    ScannerStatus(&'a ScannerStatusEvent),
    Macro(&'a MacroEvent),
    Condition(&'a ConditionEvent),
    Scanner(&'a ScannerEvent),
    TokenList(&'a TokenListEvent),
    Alignment(&'a AlignmentEvent),
    Mutation(&'a MutationEvent),
    Diagnostic(&'a DiagnosticEvent),
    DiagnosticLifecycle(&'a DiagnosticLifecycleEvent),
    Effect(&'a EffectEvent),
    Geometry(&'a GeometryEvent),
}

/// The concrete payload mutably borrowed from any oracle event.
#[derive(Debug)]
pub enum EventViewMut<'a> {
    Command(&'a mut CommandEvent),
    Input(&'a mut InputEvent),
    Recovery(&'a mut RecoveryEvent),
    ScannerStatus(&'a mut ScannerStatusEvent),
    Macro(&'a mut MacroEvent),
    Condition(&'a mut ConditionEvent),
    Scanner(&'a mut ScannerEvent),
    TokenList(&'a mut TokenListEvent),
    Alignment(&'a mut AlignmentEvent),
    Mutation(&'a mut MutationEvent),
    Diagnostic(&'a mut DiagnosticEvent),
    DiagnosticLifecycle(&'a mut DiagnosticLifecycleEvent),
    Effect(&'a mut EffectEvent),
    Geometry(&'a mut GeometryEvent),
}

/// Stable schema family of an event.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum EventClass {
    Command,
    Input,
    Recovery,
    ScannerStatus,
    Macro,
    Condition,
    Scanner,
    TokenList,
    Alignment,
    Mutation,
    Diagnostic,
    DiagnosticLifecycle,
    Effect,
    Geometry,
}

impl EventClass {
    /// Existing diagnostic label for a same-class payload mismatch.
    #[must_use]
    pub const fn mismatch_kind(self) -> &'static str {
        match self {
            Self::Command => "command_mismatch",
            Self::Input => "input_transition_mismatch",
            Self::Recovery => "recovery_mismatch",
            Self::ScannerStatus => "scanner_status_mismatch",
            Self::Macro => "macro_transition_mismatch",
            Self::Condition => "condition_mismatch",
            Self::Scanner => "scanner_result_mismatch",
            Self::TokenList => "token_list_mismatch",
            Self::Alignment => "alignment_mismatch",
            Self::Mutation => "mutation_mismatch",
            Self::Diagnostic => "diagnostic_mismatch",
            Self::DiagnosticLifecycle => "diagnostic_lifecycle_mismatch",
            Self::Effect => "effect_mismatch",
            Self::Geometry => "geometry_mismatch",
        }
    }
}

/// A source-bearing field nested anywhere in an event.
#[derive(Clone, Copy, Debug)]
pub enum EventLocation<'a> {
    Source(&'a SourceLocation),
    Geometry(&'a GeometryLocation),
}

/// Mutable counterpart of [`EventLocation`].
#[derive(Debug)]
pub enum EventLocationMut<'a> {
    Source(&'a mut SourceLocation),
    Geometry(&'a mut GeometryLocation),
}

/// Identity half used to align two event streams independently of payloads.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EventAlignmentKey<'a> {
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
    DiagnosticLifecycle {
        transition: &'static str,
        diagnostic: Option<&'a str>,
    },
    Effect {
        kind: EffectKind,
        channel: &'a str,
    },
    Geometry {
        transition: &'static str,
        location: Option<(&'a str, u32)>,
    },
}

/// A structural stream boundary suitable for long-range resynchronization.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EventAnchorKey<'a> {
    Input {
        transition: InputTransition,
        reason: InputReason,
        name: &'a str,
    },
    Line {
        source: &'a str,
        line: u32,
    },
}

impl Event {
    /// Borrows this event through the exhaustive schema view.
    #[must_use]
    pub const fn view(&self) -> EventView<'_> {
        match self {
            Self::Command(value) => EventView::Command(value),
            Self::Input(value) => EventView::Input(value),
            Self::Recovery(value) => EventView::Recovery(value),
            Self::ScannerStatus(value) => EventView::ScannerStatus(value),
            Self::Macro(value) => EventView::Macro(value),
            Self::Condition(value) => EventView::Condition(value),
            Self::Scanner(value) => EventView::Scanner(value),
            Self::TokenList(value) => EventView::TokenList(value),
            Self::Alignment(value) => EventView::Alignment(value),
            Self::Mutation(value) => EventView::Mutation(value),
            Self::Diagnostic(value) => EventView::Diagnostic(value),
            Self::DiagnosticLifecycle(value) => EventView::DiagnosticLifecycle(value),
            Self::Effect(value) => EventView::Effect(value),
            Self::Geometry(value) => EventView::Geometry(value),
        }
    }

    /// Mutably borrows this event through the exhaustive schema view.
    #[must_use]
    pub fn view_mut(&mut self) -> EventViewMut<'_> {
        match self {
            Self::Command(value) => EventViewMut::Command(value),
            Self::Input(value) => EventViewMut::Input(value),
            Self::Recovery(value) => EventViewMut::Recovery(value),
            Self::ScannerStatus(value) => EventViewMut::ScannerStatus(value),
            Self::Macro(value) => EventViewMut::Macro(value),
            Self::Condition(value) => EventViewMut::Condition(value),
            Self::Scanner(value) => EventViewMut::Scanner(value),
            Self::TokenList(value) => EventViewMut::TokenList(value),
            Self::Alignment(value) => EventViewMut::Alignment(value),
            Self::Mutation(value) => EventViewMut::Mutation(value),
            Self::Diagnostic(value) => EventViewMut::Diagnostic(value),
            Self::DiagnosticLifecycle(value) => EventViewMut::DiagnosticLifecycle(value),
            Self::Effect(value) => EventViewMut::Effect(value),
            Self::Geometry(value) => EventViewMut::Geometry(value),
        }
    }

    /// Returns a clone with every nested source position erased.
    #[must_use]
    pub fn without_locations(&self) -> Self {
        let mut event = self.clone();
        event.view_mut().erase_locations();
        event
    }

    /// Bounded, Unicode-safe rendering compatible with the historical report.
    #[must_use]
    pub fn concise(&self, max_chars: usize) -> ConciseEvent<'_> {
        ConciseEvent {
            event: self,
            max_chars,
        }
    }
}

impl<'a> EventView<'a> {
    #[must_use]
    pub const fn class(self) -> EventClass {
        match self {
            Self::Command(_) => EventClass::Command,
            Self::Input(_) => EventClass::Input,
            Self::Recovery(_) => EventClass::Recovery,
            Self::ScannerStatus(_) => EventClass::ScannerStatus,
            Self::Macro(_) => EventClass::Macro,
            Self::Condition(_) => EventClass::Condition,
            Self::Scanner(_) => EventClass::Scanner,
            Self::TokenList(_) => EventClass::TokenList,
            Self::Alignment(_) => EventClass::Alignment,
            Self::Mutation(_) => EventClass::Mutation,
            Self::Diagnostic(_) => EventClass::Diagnostic,
            Self::DiagnosticLifecycle(_) => EventClass::DiagnosticLifecycle,
            Self::Effect(_) => EventClass::Effect,
            Self::Geometry(_) => EventClass::Geometry,
        }
    }

    #[must_use]
    pub fn alignment_key(self) -> EventAlignmentKey<'a> {
        match self {
            Self::Command(event) => EventAlignmentKey::Command {
                delivery: event.delivery,
                command: &event.command.command,
                control_sequence: event.command.control_sequence.as_deref(),
                location: event
                    .command
                    .location
                    .as_ref()
                    .map(|location| (location.source.as_str(), location.line, location.byte)),
            },
            Self::Input(event) => EventAlignmentKey::Input {
                transition: event.transition,
                reason: event.reason,
                name: &event.name,
            },
            Self::Recovery(event) => EventAlignmentKey::Recovery { kind: event.kind },
            Self::ScannerStatus(_) => EventAlignmentKey::ScannerStatus,
            Self::Macro(MacroEvent::Argument { parameter, .. }) => {
                EventAlignmentKey::MacroArgument {
                    parameter: *parameter,
                }
            }
            Self::Macro(MacroEvent::Activation {
                control_sequence,
                argument_count,
            }) => EventAlignmentKey::MacroActivation {
                control_sequence,
                argument_count: *argument_count,
            },
            Self::Condition(event) => EventAlignmentKey::Condition {
                transition: event.transition,
                condition: &event.condition,
            },
            Self::Scanner(event) => EventAlignmentKey::Scanner {
                scanner: &event.scanner,
            },
            Self::TokenList(event) => EventAlignmentKey::TokenList {
                transition: event.transition,
                purpose: &event.purpose,
            },
            Self::Alignment(event) => EventAlignmentKey::Alignment {
                transition: event.transition,
            },
            Self::Mutation(event) => EventAlignmentKey::Mutation {
                target: &event.target,
            },
            Self::Diagnostic(event) => EventAlignmentKey::Diagnostic {
                severity: event.severity,
                diagnostic: &event.diagnostic,
            },
            Self::DiagnosticLifecycle(event) => match event {
                DiagnosticLifecycleEvent::Report { diagnostic, .. } => {
                    EventAlignmentKey::DiagnosticLifecycle {
                        transition: "report",
                        diagnostic: Some(diagnostic),
                    }
                }
                DiagnosticLifecycleEvent::Outcome { .. } => {
                    EventAlignmentKey::DiagnosticLifecycle {
                        transition: "outcome",
                        diagnostic: None,
                    }
                }
            },
            Self::Effect(event) => EventAlignmentKey::Effect {
                kind: event.kind,
                channel: &event.channel,
            },
            Self::Geometry(event) => {
                let (transition, location) = match event {
                    GeometryEvent::Hpack { location, .. } => ("hpack", location),
                    GeometryEvent::Vpack { location, .. } => ("vpack", location),
                    GeometryEvent::Shipout { location, .. } => ("shipout", location),
                };
                EventAlignmentKey::Geometry {
                    transition,
                    location: location
                        .as_ref()
                        .map(|location| (location.source.as_str(), location.line)),
                }
            }
        }
    }

    #[must_use]
    pub fn anchor_key(self) -> Option<EventAnchorKey<'a>> {
        match self {
            Self::Input(event) => Some(EventAnchorKey::Input {
                transition: event.transition,
                reason: event.reason,
                name: &event.name,
            }),
            Self::Command(event) => {
                event
                    .command
                    .location
                    .as_ref()
                    .map(|location| EventAnchorKey::Line {
                        source: &location.source,
                        line: location.line,
                    })
            }
            Self::Recovery(_)
            | Self::ScannerStatus(_)
            | Self::Macro(_)
            | Self::Condition(_)
            | Self::Scanner(_)
            | Self::TokenList(_)
            | Self::Alignment(_)
            | Self::Mutation(_)
            | Self::Diagnostic(_)
            | Self::DiagnosticLifecycle(_)
            | Self::Effect(_)
            | Self::Geometry(_) => None,
        }
    }

    pub fn visit_locations(self, visitor: &mut impl FnMut(EventLocation<'_>)) {
        match self {
            Self::Command(event) => {
                if let Some(location) = &event.command.location {
                    visitor(EventLocation::Source(location));
                }
                visit_value_locations(&event.command.operand, visitor);
            }
            Self::Recovery(event) => visit_token_locations(&event.tokens, visitor),
            Self::Macro(MacroEvent::Argument { tokens, .. })
            | Self::TokenList(TokenListEvent { tokens, .. }) => {
                visit_token_locations(tokens, visitor);
            }
            Self::Scanner(event) => visit_value_locations(&event.result, visitor),
            Self::Mutation(event) => {
                visit_value_locations(&event.key, visitor);
                visit_value_locations(&event.value, visitor);
            }
            Self::Diagnostic(event) => {
                for value in &event.arguments {
                    visit_value_locations(value, visitor);
                }
            }
            Self::DiagnosticLifecycle(DiagnosticLifecycleEvent::Report {
                arguments,
                location,
                ..
            }) => {
                if let Some(location) = location {
                    visitor(EventLocation::Source(location));
                }
                for value in arguments {
                    visit_value_locations(value, visitor);
                }
            }
            Self::DiagnosticLifecycle(DiagnosticLifecycleEvent::Outcome { .. }) => {}
            Self::Effect(event) => visit_value_locations(&event.value, visitor),
            Self::Geometry(event) => {
                if let Some(location) = geometry_location(event) {
                    visitor(EventLocation::Geometry(location));
                }
            }
            Self::Input(_)
            | Self::ScannerStatus(_)
            | Self::Macro(MacroEvent::Activation { .. })
            | Self::Condition(_)
            | Self::Alignment(_) => {}
        }
    }
}

impl EventViewMut<'_> {
    /// Visits every source-bearing field nested in the mutable event payload.
    pub fn visit_locations(&mut self, visitor: &mut impl for<'a> FnMut(EventLocationMut<'a>)) {
        match self {
            Self::Command(event) => {
                if let Some(location) = &mut event.command.location {
                    visitor(EventLocationMut::Source(location));
                }
                visit_value_locations_mut(&mut event.command.operand, visitor);
            }
            Self::Recovery(event) => visit_token_locations_mut(&mut event.tokens, visitor),
            Self::Macro(MacroEvent::Argument { tokens, .. })
            | Self::TokenList(TokenListEvent { tokens, .. }) => {
                visit_token_locations_mut(tokens, visitor);
            }
            Self::Scanner(event) => visit_value_locations_mut(&mut event.result, visitor),
            Self::Mutation(event) => {
                visit_value_locations_mut(&mut event.key, visitor);
                visit_value_locations_mut(&mut event.value, visitor);
            }
            Self::Diagnostic(event) => {
                for value in &mut event.arguments {
                    visit_value_locations_mut(value, visitor);
                }
            }
            Self::DiagnosticLifecycle(DiagnosticLifecycleEvent::Report {
                arguments,
                location,
                ..
            }) => {
                if let Some(location) = location {
                    visitor(EventLocationMut::Source(location));
                }
                for value in arguments {
                    visit_value_locations_mut(value, visitor);
                }
            }
            Self::DiagnosticLifecycle(DiagnosticLifecycleEvent::Outcome { .. }) => {}
            Self::Effect(event) => visit_value_locations_mut(&mut event.value, visitor),
            Self::Geometry(event) => {
                if let Some(location) = geometry_location_mut(event) {
                    visitor(EventLocationMut::Geometry(location));
                }
            }
            Self::Input(_)
            | Self::ScannerStatus(_)
            | Self::Macro(MacroEvent::Activation { .. })
            | Self::Condition(_)
            | Self::Alignment(_) => {}
        }
    }

    /// Applies canonical textual normalization to all reachable fields.
    pub(crate) fn normalize(&mut self) {
        match self {
            Self::Command(event) => normalize_command(&mut event.command),
            Self::Input(event) => normalize_string(&mut event.name),
            Self::Recovery(event) => event.tokens.iter_mut().for_each(normalize_token),
            Self::ScannerStatus(_) => {}
            Self::Macro(MacroEvent::Argument { tokens, .. }) => {
                tokens.iter_mut().for_each(normalize_token)
            }
            Self::Macro(MacroEvent::Activation { .. }) => {}
            Self::Condition(event) => {
                normalize_string(&mut event.condition);
                normalize_string(&mut event.limit);
                if let Some(branch) = &mut event.branch {
                    normalize_string(branch);
                }
            }
            Self::Scanner(event) => {
                normalize_string(&mut event.scanner);
                normalize_value(&mut event.result);
            }
            Self::TokenList(event) => {
                normalize_string(&mut event.purpose);
                event.tokens.iter_mut().for_each(normalize_token);
            }
            Self::Alignment(event) => {
                if let Some(template) = &mut event.template {
                    normalize_string(template);
                }
            }
            Self::Mutation(event) => {
                normalize_value(&mut event.key);
                normalize_value(&mut event.value);
                normalize_string(&mut event.scope);
            }
            Self::Diagnostic(event) => {
                normalize_string(&mut event.diagnostic);
                event.arguments.iter_mut().for_each(normalize_value);
            }
            Self::DiagnosticLifecycle(DiagnosticLifecycleEvent::Report {
                diagnostic,
                arguments,
                location,
                ..
            }) => {
                normalize_string(diagnostic);
                arguments.iter_mut().for_each(normalize_value);
                if let Some(location) = location {
                    normalize_string(&mut location.source);
                }
            }
            Self::DiagnosticLifecycle(DiagnosticLifecycleEvent::Outcome { .. }) => {}
            Self::Effect(event) => {
                normalize_string(&mut event.channel);
                normalize_value(&mut event.value);
            }
            Self::Geometry(event) => {
                if let Some(location) = geometry_location_mut(event) {
                    normalize_string(&mut location.source);
                }
            }
        }
    }

    /// Removes all source and geometry positions while retaining every payload.
    pub fn erase_locations(&mut self) {
        match self {
            Self::Command(event) => {
                event.command.location = None;
                erase_value_locations(&mut event.command.operand);
            }
            Self::Recovery(event) => erase_token_locations(&mut event.tokens),
            Self::Macro(MacroEvent::Argument { tokens, .. })
            | Self::TokenList(TokenListEvent { tokens, .. }) => erase_token_locations(tokens),
            Self::Scanner(event) => erase_value_locations(&mut event.result),
            Self::Mutation(event) => {
                erase_value_locations(&mut event.key);
                erase_value_locations(&mut event.value);
            }
            Self::Diagnostic(event) => event.arguments.iter_mut().for_each(erase_value_locations),
            Self::DiagnosticLifecycle(DiagnosticLifecycleEvent::Report {
                arguments,
                location,
                ..
            }) => {
                arguments.iter_mut().for_each(erase_value_locations);
                *location = None;
            }
            Self::DiagnosticLifecycle(DiagnosticLifecycleEvent::Outcome { .. }) => {}
            Self::Effect(event) => erase_value_locations(&mut event.value),
            Self::Geometry(event) => match event {
                GeometryEvent::Hpack { location, .. }
                | GeometryEvent::Vpack { location, .. }
                | GeometryEvent::Shipout { location, .. } => *location = None,
            },
            Self::Input(_)
            | Self::ScannerStatus(_)
            | Self::Macro(MacroEvent::Activation { .. })
            | Self::Condition(_)
            | Self::Alignment(_) => {}
        }
    }
}

/// Display adapter for bounded event diagnostics.
pub struct ConciseEvent<'a> {
    event: &'a Event,
    max_chars: usize,
}

impl fmt::Display for ConciseEvent<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let rendered = format!("{:?}", self.event);
        if rendered.chars().count() <= self.max_chars {
            formatter.write_str(&rendered)
        } else {
            for character in rendered.chars().take(self.max_chars) {
                formatter.write_str(character.encode_utf8(&mut [0; 4]))?;
            }
            formatter.write_str("…")
        }
    }
}

fn geometry_location(event: &GeometryEvent) -> Option<&GeometryLocation> {
    match event {
        GeometryEvent::Hpack { location, .. }
        | GeometryEvent::Vpack { location, .. }
        | GeometryEvent::Shipout { location, .. } => location.as_ref(),
    }
}

fn geometry_location_mut(event: &mut GeometryEvent) -> Option<&mut GeometryLocation> {
    match event {
        GeometryEvent::Hpack { location, .. }
        | GeometryEvent::Vpack { location, .. }
        | GeometryEvent::Shipout { location, .. } => location.as_mut(),
    }
}

fn visit_value_locations<'a>(
    value: &'a CanonicalValue,
    visitor: &mut impl FnMut(EventLocation<'a>),
) {
    match value {
        CanonicalValue::Token(token) => visit_token_location(token, visitor),
        CanonicalValue::Tokens(tokens) => visit_token_locations(tokens, visitor),
        CanonicalValue::None
        | CanonicalValue::Bool(_)
        | CanonicalValue::Integer(_)
        | CanonicalValue::Character(_)
        | CanonicalValue::Scaled(_)
        | CanonicalValue::Glue { .. }
        | CanonicalValue::Name(_)
        | CanonicalValue::Bytes(_) => {}
    }
}

fn visit_token_locations<'a>(
    tokens: &'a [OracleToken],
    visitor: &mut impl FnMut(EventLocation<'a>),
) {
    for token in tokens {
        visit_token_location(token, visitor);
    }
}

fn visit_token_location<'a>(token: &'a OracleToken, visitor: &mut impl FnMut(EventLocation<'a>)) {
    if let Some(location) = &token.location {
        visitor(EventLocation::Source(location));
    }
}

fn visit_value_locations_mut(
    value: &mut CanonicalValue,
    visitor: &mut impl for<'a> FnMut(EventLocationMut<'a>),
) {
    match value {
        CanonicalValue::Token(token) => visit_token_location_mut(token, visitor),
        CanonicalValue::Tokens(tokens) => visit_token_locations_mut(tokens, visitor),
        CanonicalValue::None
        | CanonicalValue::Bool(_)
        | CanonicalValue::Integer(_)
        | CanonicalValue::Character(_)
        | CanonicalValue::Scaled(_)
        | CanonicalValue::Glue { .. }
        | CanonicalValue::Name(_)
        | CanonicalValue::Bytes(_) => {}
    }
}

fn visit_token_locations_mut(
    tokens: &mut [OracleToken],
    visitor: &mut impl for<'a> FnMut(EventLocationMut<'a>),
) {
    for token in tokens {
        visit_token_location_mut(token, visitor);
    }
}

fn visit_token_location_mut(
    token: &mut OracleToken,
    visitor: &mut impl for<'a> FnMut(EventLocationMut<'a>),
) {
    if let Some(location) = &mut token.location {
        visitor(EventLocationMut::Source(location));
    }
}

fn normalize_command(command: &mut CanonicalCommand) {
    normalize_string(&mut command.command);
    normalize_value(&mut command.operand);
    if let Some(location) = &mut command.location {
        normalize_string(&mut location.source);
    }
}

fn normalize_token(token: &mut OracleToken) {
    normalize_string(&mut token.catcode);
    if let Some(location) = &mut token.location {
        normalize_string(&mut location.source);
    }
}

fn normalize_value(value: &mut CanonicalValue) {
    match value {
        CanonicalValue::Token(token) => normalize_token(token),
        CanonicalValue::Tokens(tokens) => tokens.iter_mut().for_each(normalize_token),
        CanonicalValue::Name(name) => normalize_string(name),
        CanonicalValue::Glue {
            stretch_order,
            shrink_order,
            ..
        } => {
            normalize_string(stretch_order);
            normalize_string(shrink_order);
        }
        CanonicalValue::None
        | CanonicalValue::Bool(_)
        | CanonicalValue::Integer(_)
        | CanonicalValue::Character(_)
        | CanonicalValue::Scaled(_)
        | CanonicalValue::Bytes(_) => {}
    }
}

fn normalize_string(value: &mut String) {
    if value.contains('\r') {
        *value = value.replace("\r\n", "\n").replace('\r', "\n");
    }
}

fn erase_value_locations(value: &mut CanonicalValue) {
    match value {
        CanonicalValue::Token(token) => token.location = None,
        CanonicalValue::Tokens(tokens) => erase_token_locations(tokens),
        CanonicalValue::None
        | CanonicalValue::Bool(_)
        | CanonicalValue::Integer(_)
        | CanonicalValue::Character(_)
        | CanonicalValue::Scaled(_)
        | CanonicalValue::Glue { .. }
        | CanonicalValue::Name(_)
        | CanonicalValue::Bytes(_) => {}
    }
}

fn erase_token_locations(tokens: &mut [OracleToken]) {
    for token in tokens {
        token.location = None;
    }
}

#[cfg(test)]
mod tests;
