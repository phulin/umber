use super::*;
use crate::{CanonicalValue, EffectEvent, EffectKind, GeometryEvent, Normalizer, SchemaVersion};

#[test]
fn json_lines_uses_the_validated_pinned_header() {
    for schema in [
        SchemaVersion::V1,
        SchemaVersion::V2,
        SchemaVersion::V3,
        SchemaVersion::V4,
    ] {
        let oracle = format!(
            "{{\"schema\":{},\"manifest\":\"{}\"}}\n",
            schema.number(),
            "1".repeat(64)
        );
        assert_eq!(
            canonical_bundle_json_lines(&[], oracle.as_bytes()).expect("empty stream"),
            oracle.as_bytes()
        );
    }
    let stale = b"{\"schema\":5,\"manifest\":\"1111111111111111111111111111111111111111111111111111111111111111\"}\n";
    assert!(canonical_bundle_json_lines(&[], stale).is_err());
}

#[test]
fn geometry_obeys_the_pinned_schema() {
    let header = |schema| {
        format!(
            "{{\"schema\":{schema},\"manifest\":\"{}\"}}\n",
            "1".repeat(64)
        )
    };
    let event = NormalizedEvent {
        sequence: 0,
        semantic: Event::Geometry(GeometryEvent::Hpack {
            width_sp: 1,
            height_sp: 2,
            depth_sp: 3,
            location: None,
        }),
    };
    assert!(
        canonical_bundle_json_lines(std::slice::from_ref(&event), header(1).as_bytes()).is_err()
    );
    assert!(
        canonical_bundle_json_lines(std::slice::from_ref(&event), header(2).as_bytes()).is_ok()
    );
    assert!(canonical_bundle_json_lines(&[event], header(3).as_bytes()).is_err());
}

#[test]
fn codec_round_trips_and_rejects_stream_confusion() {
    let mut semantic = Normalizer::new();
    let mut geometry = Normalizer::new();
    let bundle = OracleBundle {
        semantic: vec![semantic.normalize(Event::Effect(EffectEvent {
            kind: EffectKind::Terminate,
            channel: "engine".into(),
            value: CanonicalValue::None,
        }))],
        geometry: vec![geometry.normalize(Event::Geometry(GeometryEvent::Hpack {
            width_sp: 1,
            height_sp: 2,
            depth_sp: 3,
            location: None,
        }))],
    };
    let encoded = encode_oracle_bundle(&bundle).expect("encode");
    assert_eq!(decode_oracle_bundle(&encoded).expect("decode"), bundle);
    let mut gapped = bundle.clone();
    gapped.semantic[0].sequence = 1;
    assert!(encode_oracle_bundle(&gapped).is_err());
    let mut confused = bundle;
    confused.semantic.push(confused.geometry.remove(0));
    assert!(encode_oracle_bundle(&confused).is_err());
    let mut trailing = encoded;
    trailing.push(0);
    assert!(decode_oracle_bundle(&trailing).is_err());
}

#[test]
fn decoder_rejects_resource_bounds_before_event_decode() {
    let mut impossible_count = Vec::from(ORACLE_BUNDLE_MAGIC);
    impossible_count.extend_from_slice(&ORACLE_BUNDLE_SCHEMA.to_le_bytes());
    impossible_count.extend_from_slice(&(MAX_BUNDLE_EVENTS_PER_STREAM as u32).to_le_bytes());
    impossible_count.extend_from_slice(&0_u32.to_le_bytes());
    assert!(decode_oracle_bundle(&impossible_count).is_err());

    let framed = |json: &[u8]| {
        let mut bytes = Vec::from(ORACLE_BUNDLE_MAGIC);
        bytes.extend_from_slice(&ORACLE_BUNDLE_SCHEMA.to_le_bytes());
        bytes.extend_from_slice(&1_u32.to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        bytes.extend_from_slice(&(json.len() as u32).to_le_bytes());
        bytes.extend_from_slice(json);
        bytes
    };
    let oversized_string = format!("\"{}\"", "x".repeat(MAX_BUNDLE_STRING_BYTES + 1));
    assert!(decode_oracle_bundle(&framed(oversized_string.as_bytes())).is_err());
    let nested = format!(
        "{}0{}",
        "[".repeat(MAX_BUNDLE_NESTING_DEPTH + 1),
        "]".repeat(MAX_BUNDLE_NESTING_DEPTH + 1)
    );
    assert!(decode_oracle_bundle(&framed(nested.as_bytes())).is_err());
}
