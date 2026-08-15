use super::{NodeListRef, NodeListWeakIndex};
use crate::glue::{GlueSpec, Order};
use crate::math::{
    FractionThickness, MathChoice, MathField, MathFraction, MathListNode, MathNoad, NoadClass,
    NoadKind,
};
use crate::node::{
    AdjustNode, BoxLr, BoxNode, BoxNodeFields, DiscKind, GlueKind, LeaderPayload, Node, Sign,
    UnsetKind, UnsetNode, UnsetNodeFields,
};
use crate::node_arena::{NodeSemanticId, SidecarNeeds};
use crate::provenance::OriginRef;
use crate::scaled::{GlueSetRatio, Scaled};
use crate::stores::Stores;
use crate::token::OriginId;

#[cfg(feature = "profiling")]
fn node_append_delta(
    before: crate::measurement::NodeAppendMeasurement,
) -> crate::measurement::NodeAppendMeasurement {
    let after = crate::measurement::node_append_measurement();
    crate::measurement::NodeAppendMeasurement {
        calls: after.calls - before.calls,
        words: after.words - before.words,
        sidecar_rows: core::array::from_fn(|index| {
            after.sidecar_rows[index] - before.sidecar_rows[index]
        }),
        capacity_growth_events: after.capacity_growth_events - before.capacity_growth_events,
        capacity_growth_by_column: core::array::from_fn(|index| {
            after.capacity_growth_by_column[index] - before.capacity_growth_by_column[index]
        }),
        compact_copy_calls: after.compact_copy_calls - before.compact_copy_calls,
        compact_copy_words: after.compact_copy_words - before.compact_copy_words,
        compact_copy_growth_by_column: core::array::from_fn(|index| {
            after.compact_copy_growth_by_column[index] - before.compact_copy_growth_by_column[index]
        }),
        retained_payload_bytes_grown: after.retained_payload_bytes_grown
            - before.retained_payload_bytes_grown,
    }
}

fn freeze(stores: &mut Stores, nodes: impl IntoIterator<Item = Node>) -> NodeListRef {
    let mut builder = stores.node_list_builder();
    for node in nodes {
        builder.push(node);
    }
    stores.freeze_node_list_ref(builder)
}

fn testing_ref(node: Node, semantic_id: NodeSemanticId) -> NodeListRef {
    let mut needs = SidecarNeeds::default();
    needs.preflight_and_count(&node);
    NodeListRef::freeze_builder(vec![node], Vec::new(), semantic_id, needs)
}

#[test]
fn canonical_empty_has_explicit_shared_ownership() {
    let first = NodeListRef::empty();
    let second = NodeListRef::empty();

    assert!(first.is_empty());
    assert_eq!(first.semantic_fingerprint(), second.semantic_fingerprint());
    assert!(first.shares_payload(&second));
    assert!(
        first.strong_count() >= 3,
        "static root plus two callers own empty"
    );
}

#[test]
fn builder_freeze_owns_and_resolves_child_spans_without_survivor_owner() {
    let mut stores = Stores::new();
    let child = freeze(&mut stores, [Node::Penalty(17)]);
    let mut parent_builder = stores.node_list_builder();
    parent_builder.push(Node::Adjust(AdjustNode::ordinary(child.clone())));
    #[cfg(feature = "profiling")]
    let before = crate::measurement::node_append_measurement();

    let parent = stores.freeze_node_list_ref(parent_builder);
    #[cfg(feature = "profiling")]
    let appended = node_append_delta(before);
    let Node::Adjust(adjust) = parent.get(0).expect("parent node") else {
        panic!("expected adjustment")
    };
    let resolved = adjust.content;

    assert_eq!(resolved.nodes().to_vec(), [Node::Penalty(17)]);
    assert_eq!(
        parent
            .child_nodes(resolved.id())
            .expect("borrowed child")
            .to_vec(),
        [Node::Penalty(17)]
    );
    assert!(
        !resolved.shares_payload(&child),
        "freeze is one self-contained graph"
    );
    assert!(resolved.shares_payload(&parent));
    #[cfg(feature = "profiling")]
    {
        assert_eq!((appended.calls, appended.words), (2, 2));
        assert_eq!(
            (appended.compact_copy_calls, appended.compact_copy_words),
            (1, 1),
            "nested freeze must copy only the preexisting child graph"
        );
    }
}

#[test]
fn every_child_sidecar_resolves_from_the_frozen_owner() {
    let mut stores = Stores::new();
    let mut builder = stores.node_list_builder();
    let empty = NodeListRef::empty();
    let glue = stores.intern_glue_in_domain(GlueSpec::ZERO, None);
    let mut box_node = BoxNode::new(BoxNodeFields {
        width: Scaled::from_raw(1),
        height: Scaled::from_raw(2),
        depth: Scaled::from_raw(3),
        shift: Scaled::from_raw(4),
        box_lr: BoxLr::Normal,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children: empty.clone(),
    });
    box_node.diagnostic_children = Some(empty.clone());
    let unset = UnsetNode::new(UnsetNodeFields {
        kind: UnsetKind::HBox,
        width: Scaled::from_raw(5),
        height: Scaled::from_raw(6),
        depth: Scaled::from_raw(7),
        span_count: 1,
        stretch: Scaled::from_raw(0),
        stretch_order: Order::Normal,
        shrink: Scaled::from_raw(0),
        shrink_order: Order::Normal,
        children: empty.clone(),
    });
    let noad = MathNoad {
        kind: NoadKind::Normal(NoadClass::Ord),
        nucleus: MathField::SubBox(empty.clone()),
        subscript: MathField::Empty,
        superscript: MathField::Empty,
    };

    for node in [
        Node::HList(box_node.clone()),
        Node::Unset(unset),
        Node::Glue {
            spec: glue.clone(),
            kind: GlueKind::Leaders,
            leader: Some(LeaderPayload::HList(box_node)),
        },
        Node::Disc {
            kind: DiscKind::Discretionary,
            pre: empty.clone(),
            post: empty.clone(),
            replace: empty.clone(),
            physical_replace_count: 0,
        },
        Node::Ins {
            class: 0,
            size: Scaled::from_raw(0),
            split_top_skip: glue,
            split_max_depth: Scaled::from_raw(0),
            floating_penalty: 0,
            content: empty.clone(),
        },
        Node::MathNoad(noad),
        Node::FractionNoad(MathFraction {
            numerator: empty.clone(),
            denominator: empty.clone(),
            thickness: FractionThickness::Default,
            left_delimiter: None,
            right_delimiter: None,
        }),
        Node::MathChoice(MathChoice {
            display: empty.clone(),
            text: empty.clone(),
            script: empty.clone(),
            script_script: empty.clone(),
        }),
        Node::MathList(MathListNode {
            display: false,
            content: empty.clone(),
        }),
        Node::Adjust(AdjustNode::ordinary(empty)),
    ] {
        builder.push(node);
    }

    let root = stores.freeze_node_list_ref(builder);
    let physical_children = root
        .nodes()
        .iter()
        .flat_map(|node| node.physical_children().collect::<Vec<_>>())
        .collect::<Vec<_>>();

    assert_eq!(physical_children.len(), 18);
    assert!(physical_children.iter().all(|&child| {
        root.child_nodes(child)
            .is_some_and(crate::node_arena::NodeList::is_empty)
    }));
}

#[test]
fn candidate_collision_compares_exact_semantic_projection() {
    let mut index = NodeListWeakIndex::new();
    let first = testing_ref(Node::Penalty(1), NodeSemanticId::testing_collision(7, 1));
    let equal = testing_ref(Node::Penalty(1), NodeSemanticId::testing_collision(7, 1));
    let collision = testing_ref(Node::Penalty(2), NodeSemanticId::testing_collision(7, 1));

    let first = index.intern(first);
    let equal = index.intern(equal);
    let collision = index.intern(collision);

    assert!(first.shares_payload(&equal));
    assert!(!first.shares_payload(&collision));
    assert_eq!(first.nodes().to_vec(), [Node::Penalty(1)]);
    assert_eq!(collision.nodes().to_vec(), [Node::Penalty(2)]);
}

#[test]
fn exact_candidate_guard_preserves_diagnostic_glyph_provenance() {
    let mut index = NodeListWeakIndex::new();
    let semantic_id = NodeSemanticId::testing_collision(8, 1);
    let first_origin = OriginRef::direct(OriginId::from_raw(1));
    let second_origin = OriginRef::direct(OriginId::from_raw(2));
    let first = testing_ref(
        Node::Char {
            font: crate::font::NULL_FONT,
            ch: 'a',
            origin: first_origin.clone(),
        },
        semantic_id,
    );
    let same_semantics = testing_ref(
        Node::Char {
            font: crate::font::NULL_FONT,
            ch: 'a',
            origin: second_origin.clone(),
        },
        semantic_id,
    );

    let first = index.intern(first);
    let same_semantics = index.intern(same_semantics);
    assert!(!first.shares_payload(&same_semantics));

    let first_ligature = testing_ref(
        Node::Lig {
            font: crate::font::NULL_FONT,
            ch: 'b',
            orig: vec!['a'],
            origins: vec![first_origin],
            left_hit: false,
            right_hit: true,
        },
        semantic_id,
    );
    let same_ligature_semantics = testing_ref(
        Node::Lig {
            font: crate::font::NULL_FONT,
            ch: 'b',
            orig: vec!['a'],
            origins: vec![second_origin],
            left_hit: false,
            right_hit: true,
        },
        semantic_id,
    );
    let first_ligature = index.intern(first_ligature);
    let same_ligature_semantics = index.intern(same_ligature_semantics);
    assert!(!first_ligature.shares_payload(&same_ligature_semantics));
}

#[test]
fn clones_share_exact_data_and_final_drop_rejects_stale_projection() {
    let root = testing_ref(Node::Penalty(31), NodeSemanticId::testing(31));
    let stale = root.downgrade();
    let clone = root.clone();

    assert!(root.shares_payload(&clone));
    assert_eq!(root.strong_count(), 2);
    drop(root);
    assert!(stale.upgrade().is_some());
    drop(clone);
    assert!(stale.upgrade().is_none());
}

#[test]
fn weak_metadata_plateaus_across_bounded_live_replacements() {
    #[cfg(feature = "profiling")]
    let before = crate::measurement::node_append_measurement();
    let mut index = NodeListWeakIndex::new();
    for value in 0..10_000_u64 {
        let root = testing_ref(Node::Penalty(value as i32), NodeSemanticId::testing(value));
        drop(index.intern(root));
    }
    let retained = index.intern(testing_ref(
        Node::Penalty(i32::MAX),
        NodeSemanticId::testing(u64::MAX),
    ));
    let (entries, capacity) = index.shape();

    assert_eq!(retained.nodes().to_vec(), [Node::Penalty(i32::MAX)]);
    assert!(entries <= 1, "dead weak entries did not plateau: {entries}");
    assert!(
        capacity <= 64,
        "weak metadata capacity did not plateau: {capacity}"
    );
    #[cfg(feature = "profiling")]
    {
        let appended = node_append_delta(before);
        assert_eq!((appended.calls, appended.words), (10_001, 10_001));
        assert_eq!(
            (appended.compact_copy_calls, appended.compact_copy_words),
            (0, 0),
            "flat bounded-live roots must never be materialized twice"
        );
    }
}

#[test]
fn all_live_roots_grow_by_exact_payload_bytes_and_owner_count() {
    const ROOTS: usize = 2_048;
    #[cfg(feature = "profiling")]
    let before = crate::measurement::node_append_measurement();
    let roots = (0..ROOTS)
        .map(|value| {
            testing_ref(
                Node::Penalty(value as i32),
                NodeSemanticId::testing(value as u64),
            )
        })
        .collect::<Vec<_>>();
    let bytes_per_root = roots[0].logical_payload_bytes();
    let retained_per_root = roots[0].retained_payload_bytes();

    assert_eq!(roots.len(), ROOTS);
    assert_eq!(
        roots
            .iter()
            .map(NodeListRef::logical_payload_bytes)
            .sum::<usize>(),
        ROOTS * bytes_per_root
    );
    assert_eq!(
        roots
            .iter()
            .map(NodeListRef::retained_payload_bytes)
            .sum::<usize>(),
        ROOTS * retained_per_root
    );
    assert!(roots.iter().all(|root| root.strong_count() == 1));
    #[cfg(feature = "profiling")]
    {
        let appended = node_append_delta(before);
        assert_eq!(
            (appended.calls, appended.words),
            (ROOTS as u64, ROOTS as u64)
        );
        assert_eq!(
            (appended.compact_copy_calls, appended.compact_copy_words),
            (0, 0),
            "flat all-live roots must never be materialized twice"
        );
    }
}

#[test]
fn semantic_identity_ignores_payload_coordinates_and_allocation_order() {
    fn nested(stores: &mut Stores, filler: bool) -> NodeListRef {
        if filler {
            let _ = freeze(stores, [Node::Penalty(999)]);
        }
        let child = freeze(stores, [Node::Penalty(41)]);
        let mut builder = stores.node_list_builder();
        builder.push(Node::Adjust(AdjustNode::ordinary(child)));
        stores.freeze_node_list_ref(builder)
    }

    let first = nested(&mut Stores::new(), false);
    let shifted = nested(&mut Stores::new(), true);

    assert_ne!(first.id(), shifted.id());
    assert_eq!(first.semantic_id(), shifted.semantic_id());
    assert!(first.exact_semantic_eq(&shifted));
    assert_eq!(
        first.logical_payload_bytes(),
        shifted.logical_payload_bytes()
    );
}
