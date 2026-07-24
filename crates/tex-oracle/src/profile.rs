use crate::{CanonicalValue, EffectKind, Event, InputReason, InputTransition, ObservationStream};

/// Schema-v1 observer profiles with explicitly bounded stable event sets.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Tex82ObserverProfile {
    /// The complete focused command-fixture contract.
    FocusedCommand,
    /// The official full-TRIP contract: page commits and orderly termination.
    Trip,
}

impl Tex82ObserverProfile {
    /// Verifies that a decoded schema-v1 stream contains only this profile's
    /// stable observations and that their required ordering is intact.
    pub fn validate(self, stream: &ObservationStream) -> Result<(), String> {
        match self {
            Self::FocusedCommand => Ok(()),
            Self::Trip => validate_trip(stream),
        }
    }
}

fn validate_trip(stream: &ObservationStream) -> Result<(), String> {
    let events = &stream.events;
    if events.len() < 2 {
        return Err("TRIP observer stream must end with stop and terminate events".into());
    }
    let termination = &events[events.len() - 1].semantic;
    if !matches!(
        termination,
        Event::Effect(effect)
            if effect.kind == EffectKind::Terminate
                && effect.channel == "engine"
                && effect.value == CanonicalValue::None
    ) {
        return Err("TRIP observer stream must end with engine termination".into());
    }
    let stop = &events[events.len() - 2].semantic;
    if !matches!(
        stop,
        Event::Input(input)
            if input.transition == InputTransition::Stop
                && input.reason == InputReason::Source
                && input.name == "terminal"
    ) {
        return Err("TRIP observer termination must be preceded by terminal input stop".into());
    }

    let shipouts = events[..events.len() - 2]
        .iter()
        .filter_map(|event| match &event.semantic {
            Event::Effect(effect) if effect.kind == EffectKind::Shipout => Some(effect),
            _ => None,
        });
    for (index, effect) in shipouts.enumerate() {
        let expected_page = i64::try_from(index + 1)
            .map_err(|_| "TRIP observer page count exceeds schema integer range")?;
        if effect.channel != "dvi" || effect.value != CanonicalValue::Integer(expected_page) {
            return Err(format!(
                "TRIP observer shipout {index} must be DVI page {expected_page}"
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::{
        CanonicalValue, EffectEvent, EffectKind, Event, InputEvent, InputReason, InputTransition,
        Normalizer, ObservationHeader, ObservationStream, SCHEMA_VERSION,
    };

    use super::Tex82ObserverProfile;

    fn stream(events: Vec<Event>) -> ObservationStream {
        let mut normalizer = Normalizer::new();
        ObservationStream {
            header: ObservationHeader {
                schema: SCHEMA_VERSION,
                manifest: "0".repeat(64),
            },
            events: events
                .into_iter()
                .map(|event| normalizer.normalize(event))
                .collect(),
        }
    }

    fn terminate() -> Event {
        Event::Effect(EffectEvent {
            kind: EffectKind::Terminate,
            channel: "engine".into(),
            value: CanonicalValue::None,
        })
    }

    #[test]
    fn trip_profile_accepts_only_ordered_shipouts_and_termination() {
        let stream = stream(vec![
            Event::Effect(EffectEvent {
                kind: EffectKind::Shipout,
                channel: "dvi".into(),
                value: CanonicalValue::Integer(1),
            }),
            Event::Input(InputEvent {
                transition: InputTransition::Stop,
                reason: InputReason::Source,
                name: "terminal".into(),
            }),
            terminate(),
        ]);
        assert_eq!(Tex82ObserverProfile::Trip.validate(&stream), Ok(()));
    }

    #[test]
    fn trip_profile_ignores_nonstable_events_but_rejects_wrong_shipout_order() {
        let stream = stream(vec![
            Event::Effect(EffectEvent {
                kind: EffectKind::Shipout,
                channel: "dvi".into(),
                value: CanonicalValue::Integer(2),
            }),
            Event::Input(InputEvent {
                transition: InputTransition::Stop,
                reason: InputReason::Source,
                name: "terminal".into(),
            }),
            terminate(),
        ]);
        assert!(Tex82ObserverProfile::Trip.validate(&stream).is_err());
    }
}
