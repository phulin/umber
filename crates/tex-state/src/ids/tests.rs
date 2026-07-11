use super::{ArenaRef, FontId, GlueId, NodeListId, OriginListId, SnapshotId, TokenListId};
use core::mem::size_of;

#[test]
fn placeholder_ids_preserve_raw_values_inside_the_crate() {
    assert_eq!(TokenListId::new(1).raw(), 1);
    assert_eq!(OriginListId::new(6).raw(), 6);
    assert_eq!(GlueId::new(2).raw(), 2);
    let nodes = NodeListId::testing_epoch(3, 4);
    assert_eq!(nodes.start(), 3);
    assert_eq!(nodes.len(), 4);
    assert_eq!(FontId::new(4).raw(), 4);
    assert_eq!(SnapshotId::new(5).raw(), 5);
}

#[test]
fn canonical_origin_list_id_is_empty() {
    assert_eq!(OriginListId::EMPTY.raw(), 0);
}

#[test]
fn generation_tagged_node_list_id_is_exactly_two_words() {
    assert_eq!(size_of::<NodeListId>(), 16);
}

#[test]
fn semantic_runtime_ids_are_exactly_two_words() {
    assert_eq!(size_of::<TokenListId>(), 16);
    assert_eq!(size_of::<super::MacroDefinitionId>(), 16);
    assert_eq!(size_of::<GlueId>(), 16);
    assert_eq!(size_of::<FontId>(), 16);
}

#[test]
fn semantic_id_serialization_emits_a_detached_slot_reference() {
    let mut store = crate::token_store::TokenStore::new();
    let live = store.intern(&[crate::token::Token::param(1)]);
    let bytes = bincode::serialize(&live).expect("semantic DTO slot serializes");
    let detached: TokenListId =
        bincode::deserialize(&bytes).expect("semantic DTO slot deserializes");
    assert_eq!(detached.raw(), live.raw());
    assert_ne!(detached, live);
    assert_eq!(store.resolve_stored(detached), Some(live));
}

#[test]
fn only_detached_node_list_references_are_serializable() {
    let detached = NodeListId::testing_epoch(3, 4);
    let bytes = bincode::serialize(&detached).expect("detached DTO reference serializes");
    let restored: NodeListId =
        bincode::deserialize(&bytes).expect("detached DTO reference deserializes");
    assert_eq!(restored.arena(), ArenaRef::Epoch);
    assert_eq!(restored.start(), 3);
    assert_eq!(restored.len(), 4);

    let mut arena = crate::node_arena::NodeArena::new();
    let live = arena.append(&[crate::node::Node::Penalty(1)]);
    assert!(bincode::serialize(&live).is_err());
}

#[test]
fn epoch_node_list_boundaries_round_trip() {
    for (start, len) in [
        (0, 0),
        (u32::MAX, 0),
        (0, (1 << 31) - 1),
        (u32::MAX - ((1 << 31) - 1), (1 << 31) - 1),
    ] {
        let id = NodeListId::testing_epoch(start, len);
        assert_eq!(id.arena(), ArenaRef::Epoch);
        assert_eq!(id.start(), start);
        assert_eq!(id.len(), len);
    }
}

#[test]
fn survivor_node_list_boundaries_round_trip() {
    for (root, start, len) in [(0, 0, 0), ((1 << 20) - 2, (1 << 21) - 1, (1 << 22) - 1)] {
        let id = NodeListId::testing_survivor(root, start, len);
        assert_eq!(
            id.arena(),
            ArenaRef::Survivor(super::SurvivorRootId::new(root))
        );
        assert_eq!(id.start(), start);
        assert_eq!(id.len(), len);
        assert_eq!(
            NodeListId::decode_box_word(NodeListId::encode_box_word(Some(id))),
            Some(id)
        );
    }
}

#[test]
fn box_word_uses_canonical_none_without_translating_survivor_ids() {
    let zero = NodeListId::testing_survivor(0, 0, 0);
    assert_eq!(
        NodeListId::decode_box_word(NodeListId::encode_box_word(Some(zero))),
        Some(zero)
    );
    assert_eq!(NodeListId::encode_box_word(None), u64::MAX);
    assert_eq!(NodeListId::decode_box_word(u64::MAX), None);
}

#[test]
#[should_panic(expected = "epoch node-list length exceeds encoding")]
fn epoch_length_above_capacity_is_rejected() {
    let _ = NodeListId::testing_epoch(0, 1 << 31);
}

#[test]
#[should_panic(expected = "epoch node-list span overflows storage index")]
fn epoch_span_overflow_is_rejected() {
    let _ = NodeListId::testing_epoch(u32::MAX, 1);
}

#[test]
#[should_panic(expected = "survivor root id exceeds encoding")]
fn reserved_survivor_root_is_rejected() {
    let _ = NodeListId::testing_survivor((1 << 20) - 1, 0, 0);
}

#[test]
#[should_panic(expected = "survivor span start exceeds encoding")]
fn survivor_start_above_capacity_is_rejected() {
    let _ = NodeListId::testing_survivor(0, 1 << 21, 0);
}

#[test]
#[should_panic(expected = "survivor span length exceeds encoding")]
fn survivor_length_above_capacity_is_rejected() {
    let _ = NodeListId::testing_survivor(0, 0, 1 << 22);
}

#[test]
#[should_panic(expected = "box word contains reserved survivor root id")]
fn box_word_rejects_non_null_encoding_with_reserved_root() {
    let reserved_root_word = (1_u64 << 63) | (((1_u64 << 20) - 1) << 43);
    let _ = NodeListId::decode_box_word(reserved_root_word);
}
