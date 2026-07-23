use serde::{Deserialize, Serialize};

use crate::{
    CanonicalCommand, CanonicalValue, DiagnosticEvent, EffectEvent, Event, OracleToken,
    SourceLocation,
};

/// Event plus its deterministic zero-based position in the stream.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NormalizedEvent {
    pub sequence: u64,
    pub semantic: Event,
}

/// Stateless-value normalizer plus deterministic event numbering.
///
/// Canonicalization is intentionally narrow: it converts CRLF/CR in semantic
/// strings to LF. It does not hide semantic values, reorder events, rewrite
/// source names, or inspect host paths.
#[derive(Clone, Debug, Default)]
pub struct Normalizer {
    next_sequence: u64,
}

impl Normalizer {
    #[must_use]
    pub const fn new() -> Self {
        Self { next_sequence: 0 }
    }

    #[must_use]
    pub fn normalize(&mut self, mut event: Event) -> NormalizedEvent {
        normalize_event(&mut event);
        let sequence = self.next_sequence;
        self.next_sequence = self
            .next_sequence
            .checked_add(1)
            .expect("oracle event sequence overflowed");
        NormalizedEvent {
            sequence,
            semantic: event,
        }
    }
}

fn normalize_event(event: &mut Event) {
    match event {
        Event::Command(event) => normalize_command(&mut event.command),
        Event::Input(event) => normalize_string(&mut event.name),
        Event::Recovery(event) => event.tokens.iter_mut().for_each(normalize_token),
        Event::ScannerStatus(_) => {}
        Event::Macro(event) => match event {
            crate::MacroEvent::Argument { tokens, .. } => {
                tokens.iter_mut().for_each(normalize_token);
            }
            crate::MacroEvent::Activation {
                control_sequence, ..
            } => normalize_string(control_sequence),
        },
        Event::Condition(event) => {
            normalize_string(&mut event.condition);
            normalize_string(&mut event.limit);
            if let Some(branch) = &mut event.branch {
                normalize_string(branch);
            }
        }
        Event::Scanner(event) => {
            normalize_string(&mut event.scanner);
            normalize_value(&mut event.result);
        }
        Event::TokenList(event) => {
            normalize_string(&mut event.purpose);
            event.tokens.iter_mut().for_each(normalize_token);
        }
        Event::Alignment(event) => {
            if let Some(template) = &mut event.template {
                normalize_string(template);
            }
        }
        Event::Mutation(event) => {
            normalize_value(&mut event.key);
            normalize_value(&mut event.value);
            normalize_string(&mut event.scope);
        }
        Event::Diagnostic(event) => normalize_diagnostic(event),
        Event::Effect(event) => normalize_effect(event),
    }
}

fn normalize_command(command: &mut CanonicalCommand) {
    normalize_string(&mut command.command);
    normalize_value(&mut command.operand);
    if let Some(control_sequence) = &mut command.control_sequence {
        normalize_string(control_sequence);
    }
    if let Some(location) = &mut command.location {
        normalize_location(location);
    }
}

fn normalize_token(token: &mut OracleToken) {
    normalize_string(&mut token.catcode);
    if let Some(control_sequence) = &mut token.control_sequence {
        normalize_string(control_sequence);
    }
    if let Some(location) = &mut token.location {
        normalize_location(location);
    }
}

fn normalize_location(location: &mut SourceLocation) {
    normalize_string(&mut location.source);
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

fn normalize_diagnostic(event: &mut DiagnosticEvent) {
    normalize_string(&mut event.diagnostic);
    event.arguments.iter_mut().for_each(normalize_value);
}

fn normalize_effect(event: &mut EffectEvent) {
    normalize_string(&mut event.channel);
    normalize_value(&mut event.value);
}

fn normalize_string(value: &mut String) {
    if value.contains('\r') {
        *value = value.replace("\r\n", "\n").replace('\r', "\n");
    }
}
