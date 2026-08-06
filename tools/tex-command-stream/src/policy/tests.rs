use tex_oracle::{
    CanonicalValue, EffectEvent, EffectKind, Event, Normalizer, ObservationHeader, SchemaVersion,
};

use super::*;

fn stream(schema: SchemaVersion, events: impl IntoIterator<Item = Event>) -> Vec<u8> {
    let mut bytes = serde_json::to_vec(&ObservationHeader {
        schema: schema.number(),
        manifest: "a".repeat(64),
    })
    .expect("header");
    bytes.push(b'\n');
    let mut normalizer = Normalizer::new();
    for event in events {
        bytes.extend_from_slice(&serde_json::to_vec(&normalizer.normalize(event)).expect("event"));
        bytes.push(b'\n');
    }
    bytes
}

fn effect(value: i64) -> Event {
    Event::Effect(EffectEvent {
        kind: EffectKind::Shipout,
        channel: "dvi".into(),
        value: CanonicalValue::Integer(value),
    })
}

#[test]
fn strict_policy_rejects_malformed_input_before_reporting_accounting() {
    let valid = stream(SchemaVersion::V1, [effect(1)]);
    let error = StrictTripComparisonPolicy {
        channel: StrictTripChannel::Command,
        expected_initialization: None,
        actual_initialization: None,
    }
    .compare(Some(&valid), Some(b"not-json\n"))
    .expect_err("malformed actual stream");
    assert!(
        error
            .to_string()
            .contains("actual TRIP command-event stream"),
        "{error}"
    );
}

#[test]
fn strict_policy_returns_first_divergence_and_full_accounting_together() {
    let expected = stream(SchemaVersion::V1, [effect(1), effect(2)]);
    let actual = stream(SchemaVersion::V1, [effect(3), effect(4)]);
    let comparison = StrictTripComparisonPolicy {
        channel: StrictTripChannel::Command,
        expected_initialization: None,
        actual_initialization: None,
    }
    .compare(Some(&expected), Some(&actual))
    .expect("comparison");
    assert!(matches!(
        comparison.divergence,
        Some(StrictTripDivergence::Event { index: 0, .. })
    ));
    assert_eq!(
        comparison.accounting,
        StrictTripAccounting {
            expected_events: Some(2),
            actual_events: Some(2),
            projected_equivalent: Some(0),
            projected_divergences: Some(2),
        }
    );
}

#[test]
fn ordinary_policy_returns_grouped_and_budget_accounting() {
    let mut normalizer = Normalizer::new();
    let expected = (0..4)
        .map(|_| normalizer.normalize(effect(1)))
        .collect::<Vec<_>>();
    let actual = (0..4)
        .map(|_| ObservedEvent::new(effect(2), String::new()))
        .collect::<Vec<_>>();
    let comparison = OrdinaryComparisonPolicy {
        max_divergences: 2,
        alignment: AlignmentTuning::default(),
    }
    .compare("fixture", &expected, &actual);
    assert_eq!(comparison.accounting.ordered_divergences, 2);
    assert_eq!(comparison.accounting.root_sites, 1);
    assert!(comparison.accounting.budget_reached);
}

#[test]
fn strict_projection_walk_stays_bounded_at_one_million_events() {
    let event = Normalizer::new().normalize(effect(1));
    let mut projection = TripProjection::default();
    let mut equivalent = 0;
    for _ in 0..tex_oracle::MAX_BUNDLE_EVENTS_PER_STREAM {
        equivalent += usize::from(projection.events_match(&event, &event));
    }
    assert_eq!(equivalent, tex_oracle::MAX_BUNDLE_EVENTS_PER_STREAM);
    assert!(projection.matched_macros.is_empty());
    assert!(projection.explicit_group_macro_scopes.is_empty());
}
