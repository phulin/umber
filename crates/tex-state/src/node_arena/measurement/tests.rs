use std::sync::Arc;

use super::{NodeStorageObservation, PeakNodeStorageRecorder};
use crate::ids::{FontId, GlueId, TokenListId};
use crate::node::{Node, PdfDestinationKind, PdfDestinationNode, Whatsit};
use crate::node_arena::{NodeArena, NodeStorage};
use crate::token::OriginId;
use crate::{PdfActionIdentifier, PdfColorStackAction};

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
    let mut color_payload = Vec::with_capacity(256);
    color_payload.extend_from_slice(b"color");
    let mut storage = NodeStorage {
        whatsits: Vec::with_capacity(4),
        ..NodeStorage::default()
    };
    storage.whatsits.push(Whatsit::Special { class, payload });
    storage.whatsits.push(Whatsit::PdfColorStack {
        id: 0,
        action: PdfColorStackAction::Push(color_payload),
    });
    storage
        .whatsits
        .push(Whatsit::PdfDestination(Box::new(PdfDestinationNode {
            identifier: PdfActionIdentifier::Number(1),
            structure: None,
            kind: PdfDestinationKind::Fit,
        })));
    storage.rebuild_nested_payload_measurement();

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
        (10, 384)
    );
    let boxes = measured
        .columns
        .iter()
        .find(|column| column.name == "peak.whatsits.owned_boxes")
        .expect("owned box column");
    assert_eq!(
        (boxes.logical_bytes, boxes.retained_payload_bytes),
        (
            core::mem::size_of::<PdfDestinationNode>(),
            core::mem::size_of::<PdfDestinationNode>()
        )
    );
}

#[test]
fn newer_whatsit_payloads_are_exhaustively_measured() {
    let mut color = Vec::with_capacity(128);
    color.extend_from_slice(b"color");
    let mut storage = NodeStorage::default();
    storage.whatsits.extend([
        Whatsit::PdfColorStack {
            id: 1,
            action: crate::PdfColorStackAction::Push(color),
        },
        Whatsit::PdfSavePos,
        Whatsit::PdfSnapRefPoint,
        Whatsit::PdfSnapY { glue: GlueId::ZERO },
        Whatsit::PdfSnapYComp { ratio: 1 },
        Whatsit::PdfDestination(Box::new(crate::node::PdfDestinationNode {
            identifier: crate::PdfActionIdentifier::Number(1),
            structure: None,
            kind: crate::node::PdfDestinationKind::Fit,
        })),
        Whatsit::PdfThread(Box::new(crate::node::PdfThreadNode {
            identifier: crate::PdfActionIdentifier::Number(2),
            dimensions: crate::PdfAnnotationDimensions::RUNNING,
            attributes: TokenListId::EMPTY,
            running: false,
        })),
    ]);
    storage.rebuild_nested_payload_measurement();

    let measured = observation(&storage);

    assert_column_sums(&measured);
    assert_eq!(
        storage.retained_payload_bytes() as u64,
        measured.retained_payload_bytes
    );
    assert_column_bytes(&measured, "peak.whatsits.owned_payloads", 5, 128);
    let boxed_bytes = core::mem::size_of::<crate::node::PdfDestinationNode>()
        + core::mem::size_of::<crate::node::PdfThreadNode>();
    assert_column_bytes(
        &measured,
        "peak.whatsits.owned_boxes",
        boxed_bytes,
        boxed_bytes,
    );
}

#[test]
fn owned_ligature_payloads_participate_in_totals_and_columns() {
    let mut arena = NodeArena::new();
    arena.append(&[Node::Lig {
        font: FontId::new(1),
        ch: 'f',
        orig: vec!['f', 'i'],
        origins: vec![OriginId::from_raw(1), OriginId::from_raw(2)],
    }]);

    let measured = NodeStorageObservation::from_columns(arena.memory_columns());

    assert_column_sums(&measured);
    let sources = measured
        .columns
        .iter()
        .find(|column| column.name == "epoch.ligatures.owned_sources")
        .expect("owned ligature-source column");
    assert_eq!(sources.logical_bytes, 2 * core::mem::size_of::<char>());
    let origins = measured
        .columns
        .iter()
        .find(|column| column.name == "epoch.ligatures.owned_origins")
        .expect("owned ligature-origin column");
    assert_eq!(origins.logical_bytes, 2 * core::mem::size_of::<OriginId>());
    assert_eq!(
        arena.measurement_payload_bytes(),
        measured.order_key(),
        "payload totals must equal the sum of canonical columns"
    );
}

#[test]
fn incremental_nested_payload_totals_follow_compact_copy_and_rollback() {
    let mut source = NodeStorage::default();
    source.append(&[
        Node::Lig {
            font: FontId::new(1),
            ch: 'f',
            orig: vec!['f', 'i'],
            origins: vec![OriginId::from_raw(1), OriginId::from_raw(2)],
        },
        Node::Whatsit(Whatsit::Special {
            class: "measurement".to_owned(),
            payload: vec![1, 2, 3, 4],
        }),
    ]);

    let mut destination = NodeStorage::default();
    let empty = destination.mark();
    let mut pending = Vec::new();
    destination.append_compact(source.view(0, 2), &mut pending);
    assert!(pending.is_empty());
    assert_eq!(
        destination.payload_bytes(),
        observation(&destination).order_key()
    );

    destination.truncate(empty);
    assert_eq!(
        destination.payload_bytes(),
        observation(&destination).order_key()
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
    storage.rebuild_nested_payload_measurement();
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

fn assert_column_bytes(
    observation: &NodeStorageObservation,
    name: &str,
    logical_bytes: usize,
    retained_payload_bytes: usize,
) {
    let column = observation
        .columns
        .iter()
        .find(|column| column.name == name)
        .expect("measurement column");
    assert_eq!(
        (column.logical_bytes, column.retained_payload_bytes),
        (logical_bytes, retained_payload_bytes)
    );
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
