use std::sync::Arc;

use super::{NodeStorageObservation, PeakNodeStorageRecorder};
use crate::node::{Node, Whatsit};
use crate::node_arena::{NodeArena, NodeStorage};

#[test]
fn divergent_logical_and_retained_maxima_keep_one_observation() {
    let logical_peak = storage_with_payload(1_024, 1_024);
    let retained_peak = storage_with_payload(1, 8_192);
    let logical_observation = observation(&logical_peak);
    let retained_observation = observation(&retained_peak);
    assert!(logical_observation.logical_bytes > retained_observation.logical_bytes);
    assert!(
        retained_observation.retained_payload_bytes > logical_observation.retained_payload_bytes
    );
    let recorder = PeakNodeStorageRecorder::default();

    observe_storage(&recorder, &logical_peak);
    observe_storage(&recorder, &retained_peak);

    let peak = recorder.snapshot().expect("an observation was recorded");
    assert_eq!(peak, logical_observation);
    assert_column_sums(&peak);
}

#[test]
fn owned_whatsit_payloads_participate_in_totals_and_columns() {
    let mut class = String::with_capacity(64);
    class.push_str("pdf");
    let mut payload = Vec::with_capacity(128);
    payload.extend_from_slice(b"bytes");
    let mut storage = NodeStorage {
        whatsits: Vec::with_capacity(3),
        ..NodeStorage::default()
    };
    storage.whatsits.push(Whatsit::Special { class, payload });

    let measured = observation(&storage);

    assert_column_sums(&measured);
    assert_eq!(
        storage.retained_payload_bytes() as u64,
        measured.retained_payload_bytes
    );
    let strings = measured
        .columns
        .iter()
        .find(|column| column.name == "peak.whatsits.owned_strings")
        .expect("owned string column");
    assert_eq!(
        (strings.logical_bytes, strings.retained_payload_bytes),
        (3, 64)
    );
    let payloads = measured
        .columns
        .iter()
        .find(|column| column.name == "peak.whatsits.owned_payloads")
        .expect("owned payload column");
    assert_eq!(
        (payloads.logical_bytes, payloads.retained_payload_bytes),
        (5, 128)
    );
}

#[test]
fn concurrent_updates_publish_only_complete_observations() {
    let storages = (0..16)
        .map(|index| storage_with_payload(index * 31 + 1, (16 - index) * 257))
        .collect::<Vec<_>>();
    let expected = storages
        .iter()
        .map(observation)
        .max_by_key(NodeStorageObservation::order_key)
        .expect("candidate observations");
    let recorder = Arc::new(PeakNodeStorageRecorder::default());

    std::thread::scope(|scope| {
        for storage in storages {
            let recorder = Arc::clone(&recorder);
            scope.spawn(move || {
                for _ in 0..64 {
                    observe_storage(&recorder, &storage);
                }
            });
        }
    });

    let peak = recorder.snapshot().expect("an observation was recorded");
    assert_eq!(peak, expected);
    assert_column_sums(&peak);
}

#[test]
fn epoch_identity_and_span_tables_are_coherent_live_and_peak_columns() {
    let mut arena = NodeArena::new();
    arena.append(&[Node::Penalty(1)]);
    let live = NodeStorageObservation::from_columns(arena.memory_columns());
    assert_column_sums(&live);
    assert_metadata_columns(&live, "epoch");

    let recorder = PeakNodeStorageRecorder::default();
    recorder.observe(arena.measurement_payload_bytes(), || {
        arena.measurement_columns("peak")
    });
    let peak = recorder.snapshot().expect("arena observation");
    assert_column_sums(&peak);
    assert_metadata_columns(&peak, "peak");
}

fn storage_with_payload(len: usize, capacity: usize) -> NodeStorage {
    let mut payload = Vec::with_capacity(capacity);
    payload.resize(len, 0);
    let mut storage = NodeStorage::default();
    storage.whatsits.push(Whatsit::Special {
        class: String::new(),
        payload,
    });
    storage
}

fn observation(storage: &NodeStorage) -> NodeStorageObservation {
    NodeStorageObservation::from_columns(storage.memory_columns("peak"))
}

fn observe_storage(recorder: &PeakNodeStorageRecorder, storage: &NodeStorage) {
    recorder.observe(storage.payload_bytes(), || storage.memory_columns("peak"));
}

fn assert_metadata_columns(observation: &NodeStorageObservation, prefix: &str) {
    let identities = observation
        .columns
        .iter()
        .find(|column| column.name == format!("{prefix}.identity_tags"))
        .expect("identity-tag column");
    let spans = observation
        .columns
        .iter()
        .find(|column| column.name == format!("{prefix}.spans"))
        .expect("span column");
    assert_eq!(identities.len, spans.len);
    assert_eq!(identities.element_bytes, 16);
    assert_eq!(spans.element_bytes, 8);
}

fn assert_column_sums(observation: &NodeStorageObservation) {
    assert_eq!(
        observation.logical_bytes,
        observation
            .columns
            .iter()
            .map(|column| column.logical_bytes as u64)
            .sum::<u64>()
    );
    assert_eq!(
        observation.retained_payload_bytes,
        observation
            .columns
            .iter()
            .map(|column| column.retained_payload_bytes as u64)
            .sum::<u64>()
    );
}
