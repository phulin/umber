use super::{FrozenCoreSections, FrozenNodeSection, FrozenNonNodeSections, MemoNodeBundle};
use crate::cell::{BankTag, CellId};
use crate::env::banks::{IntParam, TokParam};
use crate::glue::GlueSpec;
use crate::glue::Order;
use crate::macro_store::MacroMeaning;
use crate::meaning::{Meaning, MeaningFlags};
use crate::node::{BoxLr, BoxNode, BoxNodeFields, DiscKind, GlueKind, LeaderPayload, Node, Sign};
use crate::node_arena::{NodeListRef, NodeRef};
use crate::provenance::OriginRef;
use crate::scaled::{GlueSetRatio, Scaled};
use crate::stores::Stores;
use crate::token::{Catcode, Token};
fn frozen_round_trip(stores: &Stores) -> Stores {
    let encoded = stores.encode_frozen_format().expect("encode frozen format");
    Stores::decode_frozen_format(
        &encoded.env,
        FrozenCoreSections {
            names: &encoded.names,
            names_lookup: &encoded.names_lookup,
            token_lists: &encoded.token_lists,
            macros: &encoded.macros,
            glue: &encoded.glue,
            checksum: 0,
        },
        FrozenNonNodeSections {
            fonts: &encoded.fonts,
            code_tables: &encoded.code_tables,
            hyphenation: &encoded.hyphenation,
        },
        FrozenNodeSection {
            bytes: &encoded.nodes,
        },
    )
    .expect("decode frozen format")
}

fn freeze_ref(stores: &mut Stores, nodes: &[Node]) -> NodeListRef {
    stores.freeze_node_list(nodes)
}

fn set_box(stores: &mut Stores, index: u16, root: NodeListRef) {
    let _ = stores.write_box_reg_ref(index, Some(root), false);
}

fn nested_penalty_graph(stores: &mut Stores, filler: bool) -> NodeListRef {
    if filler {
        drop(stores.freeze_node_list(&[Node::Penalty(i32::MAX)]));
    }
    let child = stores.freeze_node_list(&[Node::Penalty(17)]);
    stores.freeze_node_list(&[Node::HList(BoxNode::new(BoxNodeFields {
        width: Scaled::from_raw(1),
        height: Scaled::from_raw(2),
        depth: Scaled::from_raw(3),
        shift: Scaled::from_raw(4),
        box_lr: BoxLr::Normal,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children: child,
    }))])
}

#[test]
fn source_and_loaded_children_keep_their_local_payload_boundary() {
    let mut source = Stores::new();
    let source_child = freeze_ref(&mut source, &[Node::Penalty(17)]);
    let source_parent = freeze_ref(
        &mut source,
        &[Node::HList(BoxNode::new(BoxNodeFields {
            width: Scaled::from_raw(1),
            height: Scaled::from_raw(2),
            depth: Scaled::from_raw(3),
            shift: Scaled::from_raw(4),
            box_lr: BoxLr::Normal,
            glue_set: GlueSetRatio::ZERO,
            glue_sign: Sign::Normal,
            glue_order: Order::Normal,
            children: source_child.clone(),
        }))],
    );
    let Node::HList(source_box) = source_parent.get(0).expect("source parent node") else {
        panic!("expected source hlist")
    };
    assert!(source_box.children.shares_payload(&source_child));
    assert!(!source_box.children.shares_payload(&source_parent));

    set_box(&mut source, 0, source_parent);
    let mut loaded = frozen_round_trip(&source);
    let loaded_parent = loaded.box_reg_ref(0).expect("loaded parent root");
    let Node::HList(loaded_box) = loaded_parent.get(0).expect("loaded parent node") else {
        panic!("expected loaded hlist")
    };
    assert!(
        loaded_box.children.shares_payload(&loaded_parent),
        "the frozen loader installs its validated self-contained payload"
    );

    let dynamic_parent = freeze_ref(
        &mut loaded,
        &[Node::HList(BoxNode::new(BoxNodeFields {
            width: Scaled::from_raw(5),
            height: Scaled::from_raw(6),
            depth: Scaled::from_raw(7),
            shift: Scaled::from_raw(8),
            box_lr: BoxLr::Normal,
            glue_set: GlueSetRatio::ZERO,
            glue_sign: Sign::Normal,
            glue_order: Order::Normal,
            children: loaded_parent.clone(),
        }))],
    );
    let Node::HList(dynamic_box) = dynamic_parent.get(0).expect("dynamic parent node") else {
        panic!("expected dynamic hlist")
    };
    assert!(dynamic_box.children.shares_payload(&loaded_parent));
    assert!(!dynamic_box.children.shares_payload(&dynamic_parent));
}

#[test]
fn memo_node_keys_are_dense_and_allocation_independent() {
    let mut first = Stores::new();
    let first_root = nested_penalty_graph(&mut first, false);
    let first_bytes = first
        .encode_memo_node_list_ref(&first_root)
        .expect("first memo graph encodes");

    let mut shifted = Stores::new();
    let shifted_root = nested_penalty_graph(&mut shifted, true);
    assert_ne!(first_root.id(), shifted_root.id());
    let shifted_bytes = shifted
        .encode_memo_node_list_ref(&shifted_root)
        .expect("shifted memo graph encodes");

    assert_eq!(first_bytes, shifted_bytes);
    let bundle: MemoNodeBundle = bincode::deserialize(&first_bytes).expect("memo bundle decodes");
    assert_eq!(bundle.root.payload_root, None);
    assert_eq!(bundle.root.start as usize, bundle.node_lists.len() - 1);
    assert!(bundle.node_lists.iter().enumerate().all(|(index, list)| {
        list.key.payload_root.is_none()
            && list.key.start as usize == index
            && list.key.len as usize == list.nodes.len()
    }));
}

#[test]
fn memo_graph_is_fully_validated_before_destination_mutation() {
    let mut source = Stores::new();
    let root = nested_penalty_graph(&mut source, false);
    let encoded = source
        .encode_memo_node_list_ref(&root)
        .expect("memo graph encodes");
    let mut bundle: MemoNodeBundle = bincode::deserialize(&encoded).expect("memo bundle decodes");
    bundle
        .node_lists
        .last_mut()
        .expect("memo graph has a root")
        .semantic_id ^= 1;
    let corrupted = bincode::serialize(&bundle).expect("corrupt memo bundle encodes");

    let mut target = Stores::new();
    let before = target.testing_ownership_census();
    assert!(matches!(
        target.import_memo_node_list(&corrupted, 16, 16, 1024),
        Err(super::StoreFormatError::Invalid(
            "memo node semantic identity"
        ))
    ));
    assert_eq!(target.testing_ownership_census(), before);

    bundle
        .node_lists
        .last_mut()
        .expect("memo graph has a root")
        .key
        .payload_root = Some(0);
    let noncanonical = bincode::serialize(&bundle).expect("noncanonical memo bundle encodes");
    assert!(matches!(
        target.import_memo_node_list(&noncanonical, 16, 16, 1024),
        Err(super::StoreFormatError::Invalid(
            "memo root is not canonical"
        ))
    ));
    assert_eq!(target.testing_ownership_census(), before);
}

#[test]
fn format_round_trip_preserves_diagnostic_disc_replacement_count() {
    let mut stores = Stores::new();
    let empty = NodeListRef::empty();
    let root = stores.freeze_node_list(&[Node::Disc {
        kind: DiscKind::AutomaticHyphen,
        pre: empty.clone(),
        post: empty.clone(),
        replace: empty,
        physical_replace_count: 3,
    }]);
    set_box(&mut stores, 0, root);

    let restored = frozen_round_trip(&stores);
    let restored_root = restored.box_reg_ref(0).expect("restored box root");
    assert!(matches!(
        restored_root.nodes().first(),
        Some(NodeRef::Disc {
            physical_replace_count: 3,
            ..
        })
    ));
}

#[test]
fn format_round_trip_preserves_physical_diagnostic_box_children() {
    // TeX82 §§135 and 1307 make a box's list pointer part of the dumped
    // memory graph. Umber's diagnostic list is a second physical pointer:
    // format capture must retain it without changing the semantic child.
    let mut stores = Stores::new();
    let semantic_children = freeze_ref(&mut stores, &[Node::Penalty(1)]);
    let diagnostic_children = freeze_ref(&mut stores, &[Node::Penalty(2)]);
    let mut box_node = BoxNode::new(BoxNodeFields {
        width: Scaled::from_raw(0),
        height: Scaled::from_raw(0),
        depth: Scaled::from_raw(0),
        shift: Scaled::from_raw(0),
        box_lr: BoxLr::Normal,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children: semantic_children,
    });
    box_node.diagnostic_children = Some(diagnostic_children);
    let root = stores.freeze_node_list(&[Node::HList(box_node)]);
    set_box(&mut stores, 0, root);

    let restored = frozen_round_trip(&stores);
    let restored_root = restored.box_reg_ref(0).expect("restored box root");
    let Some(NodeRef::HList(restored_box)) = restored_root.nodes().first() else {
        panic!("expected restored hlist")
    };
    let semantic_children = restored_root
        .resolve(restored_box.children)
        .expect("semantic children belong to restored owner");
    assert!(matches!(
        semantic_children.nodes().first(),
        Some(NodeRef::Penalty(1))
    ));
    let diagnostic = restored_box
        .diagnostic_children
        .expect("restored diagnostic child");
    let diagnostic = restored_root
        .resolve(diagnostic)
        .expect("diagnostic children belong to restored owner");
    assert!(matches!(
        diagnostic.nodes().first(),
        Some(NodeRef::Penalty(2))
    ));
}

#[test]
fn allocator_overlap_changes_only_detached_extent_not_format_bytes() {
    fn stores_with_overlap(overlap: u32) -> Stores {
        let mut stores = Stores::new();
        let direct = |ch| Node::Char {
            font: crate::font::NULL_FONT,
            ch,
            origin: OriginRef::unknown(),
        };
        let semantic_children = freeze_ref(
            &mut stores,
            &[
                direct('A'),
                direct('/'),
                direct('B'),
                direct('B'),
                direct('C'),
                direct('A'),
            ],
        );
        let diagnostic_children = freeze_ref(
            &mut stores,
            &[
                direct('Z'),
                direct('Y'),
                direct('X'),
                direct('W'),
                direct('V'),
                direct('U'),
            ],
        );
        let mut box_node = BoxNode::new(BoxNodeFields {
            width: Scaled::from_raw(0),
            height: Scaled::from_raw(0),
            depth: Scaled::from_raw(0),
            shift: Scaled::from_raw(0),
            box_lr: BoxLr::Normal,
            glue_set: GlueSetRatio::ZERO,
            glue_sign: Sign::Normal,
            glue_order: Order::Normal,
            children: semantic_children,
        });
        box_node.diagnostic_children = Some(diagnostic_children);
        box_node.allocator_high_cell_overlap = overlap;
        let root = stores.freeze_node_list(&[Node::HList(box_node)]);
        set_box(&mut stores, 0, root);
        stores
    }

    let without_overlap = stores_with_overlap(0);
    let with_overlap = stores_with_overlap(6);
    let without_usage = super::main_memory_usage_without_scratch(&without_overlap)
        .expect("plain allocation projects")
        .usage();
    let with_usage = super::main_memory_usage_without_scratch(&with_overlap)
        .expect("overlap allocation projects")
        .usage();
    assert_eq!(without_usage.dynamic, with_usage.dynamic);
    assert_eq!(without_usage.dynamic_extent, with_usage.dynamic_extent + 6);

    let without_format = without_overlap
        .encode_frozen_format()
        .expect("plain format encodes");
    let with_format = with_overlap
        .encode_frozen_format()
        .expect("overlap format encodes");
    assert_eq!(without_format.nodes, with_format.nodes);
}

#[test]
fn detached_extent_is_independent_of_equal_content_host_sharing() {
    fn stores_with_diagnostic(shared_content: Option<bool>) -> Stores {
        let mut stores = Stores::new();
        let direct = |ch| Node::Char {
            font: crate::font::NULL_FONT,
            ch,
            origin: OriginRef::unknown(),
        };
        let semantic_children = freeze_ref(&mut stores, &[direct('A'), direct('B'), direct('C')]);
        let diagnostic_children = if shared_content == Some(true) {
            let equal = freeze_ref(&mut stores, &[direct('A'), direct('B'), direct('C')]);
            assert!(semantic_children.shares_payload(&equal));
            equal
        } else if shared_content == Some(false) {
            freeze_ref(&mut stores, &[direct('X'), direct('Y'), direct('Z')])
        } else {
            NodeListRef::empty()
        };
        let mut box_node = BoxNode::new(BoxNodeFields {
            width: Scaled::from_raw(0),
            height: Scaled::from_raw(0),
            depth: Scaled::from_raw(0),
            shift: Scaled::from_raw(0),
            box_lr: BoxLr::Normal,
            glue_set: GlueSetRatio::ZERO,
            glue_sign: Sign::Normal,
            glue_order: Order::Normal,
            children: semantic_children,
        });
        box_node.diagnostic_children = Some(diagnostic_children);
        let root = stores.freeze_node_list(&[Node::HList(box_node)]);
        set_box(&mut stores, 0, root);
        stores
    }

    let distinct = stores_with_diagnostic(Some(false));
    let shared = stores_with_diagnostic(Some(true));
    let empty = stores_with_diagnostic(None);
    let distinct_usage = super::main_memory_usage_without_scratch(&distinct)
        .expect("distinct diagnostic allocation projects")
        .usage();
    let shared_usage = super::main_memory_usage_without_scratch(&shared)
        .expect("shared diagnostic allocation projects")
        .usage();
    let empty_usage = super::main_memory_usage_without_scratch(&empty)
        .expect("empty diagnostic allocation projects")
        .usage();
    assert_eq!(distinct_usage.dynamic, shared_usage.dynamic);
    assert_eq!(distinct_usage.dynamic, empty_usage.dynamic);
    assert_eq!(distinct_usage.dynamic_extent, distinct_usage.dynamic + 3);
    assert_eq!(shared_usage.dynamic_extent, distinct_usage.dynamic_extent);
    assert_eq!(empty_usage.dynamic_extent, empty_usage.dynamic);

    for (source, expected) in [
        (&distinct, distinct_usage),
        (&shared, shared_usage),
        (&empty, empty_usage),
    ] {
        let loaded = frozen_round_trip(source);
        let loaded_usage = super::main_memory_usage_without_scratch(&loaded)
            .expect("loaded diagnostic allocation projects")
            .usage();
        assert_eq!(loaded_usage, expected);
    }
}

#[test]
fn recursive_box_copy_composes_with_live_projection_owners() {
    fn direct(ch: char) -> Node {
        Node::Char {
            font: crate::font::NULL_FONT,
            ch,
            origin: OriginRef::unknown(),
        }
    }

    fn box_node(children: NodeListRef) -> BoxNode {
        BoxNode::new(BoxNodeFields {
            width: Scaled::from_raw(0),
            height: Scaled::from_raw(0),
            depth: Scaled::from_raw(0),
            shift: Scaled::from_raw(0),
            box_lr: BoxLr::Normal,
            glue_set: GlueSetRatio::ZERO,
            glue_sign: Sign::Normal,
            glue_order: Order::Normal,
            children,
        })
    }

    let mut stores = Stores::new();
    let ligatures = freeze_ref(
        &mut stores,
        &[Node::Lig {
            font: crate::font::NULL_FONT,
            ch: 'A',
            orig: vec!['A'; 10],
            left_hit: false,
            right_hit: false,
            origins: vec![OriginRef::unknown(); 10],
        }],
    );
    let horizontal = freeze_ref(&mut stores, &[Node::HList(box_node(ligatures))]);
    let diagnostic = freeze_ref(&mut stores, &[direct('x'), direct('y'), direct('z')]);
    let mut root_box = box_node(horizontal);
    root_box.diagnostic_children = Some(diagnostic);
    let root = stores.freeze_node_list(&[Node::VList(root_box)]);
    set_box(&mut stores, 254, root);
    let root = stores.box_reg_ref(254).expect("owned box root");

    let mut projection =
        super::main_memory_usage_without_scratch(&stores).expect("box graph projects");
    let baseline = projection.usage();
    assert_eq!(baseline.dynamic_extent, baseline.dynamic + 3);
    let copied = projection
        .usage_with_box_copy(root.id(), 1)
        .expect("live box copy projects");
    // TeX82 §204 has four simultaneously live temporary heads on the path
    // root -> vlist -> hlist -> lig_ptr. The ten copied character cells are
    // retained in traversal order, while the backed-up scan terminator stays
    // live for the whole operation.
    assert_eq!(copied.variable, baseline.variable + 16);
    assert_eq!(copied.dynamic, baseline.dynamic + 1 + 10);
    assert_eq!(copied.dynamic_extent, baseline.dynamic_extent + 1 + 14);

    let loaded = frozen_round_trip(&stores);
    let loaded_root = loaded.box_reg_ref(254).expect("loaded box root");
    let loaded_projection =
        super::main_memory_usage_without_scratch(&loaded).expect("loaded box graph projects");
    let loaded_baseline = loaded_projection.usage();
    let loaded_copy = loaded_projection
        .usage_with_box_copy(loaded_root.id(), 1)
        .expect("loaded live box copy projects");
    assert_eq!(loaded_baseline, baseline);
    assert_eq!(loaded_copy, copied);

    // A shared read or destructive `\box` does not invoke §204, so merely
    // consulting the cached projection changes no allocator coordinate.
    assert_eq!(projection.usage(), baseline);

    assert!(
        !projection
            .update_box_root(&stores, Some(root.id()), None, true)
            .expect("root removal is classified"),
        "direct ownership requires the caller to discard its borrowed projection"
    );
}

#[test]
fn transient_projection_charges_a_shared_child_as_a_new_tex_edge() {
    fn usage_with_adjust(
        stores: &Stores,
        projection: &super::MainMemoryProjection,
        content: NodeListRef,
    ) -> super::MainMemoryUsage {
        projection
            .usage_with_extra_nodes(
                stores,
                &[Node::Adjust(crate::node::AdjustNode::ordinary(content))],
            )
            .expect("transient adjustment projects")
    }

    let mut source = Stores::new();
    let live_child = freeze_ref(&mut source, &[Node::Penalty(17)]);
    set_box(&mut source, 0, live_child.clone());
    let distinct_child = freeze_ref(&mut source, &[Node::Penalty(18)]);
    let source_projection =
        super::main_memory_usage_without_scratch(&source).expect("source roots project");
    let source_baseline = source_projection.usage();
    let source_shared = usage_with_adjust(&source, &source_projection, live_child);
    let source_distinct = usage_with_adjust(&source, &source_projection, distinct_child);
    let source_empty = usage_with_adjust(&source, &source_projection, NodeListRef::empty());

    // TeX82 §§125--130/1334 charge the two-word adjustment and its two-word
    // penalty child. Host payload sharing cannot erase that physical edge.
    assert_eq!(source_shared, source_distinct);
    assert_eq!(source_shared.variable, source_baseline.variable + 4);
    assert_eq!(source_shared.dynamic, source_baseline.dynamic);
    assert_eq!(source_empty.variable, source_baseline.variable + 2);

    let mut loaded = frozen_round_trip(&source);
    let loaded_child = loaded.box_reg_ref(0).expect("loaded child root");
    let loaded_distinct = freeze_ref(&mut loaded, &[Node::Penalty(18)]);
    let loaded_projection =
        super::main_memory_usage_without_scratch(&loaded).expect("loaded roots project");
    let loaded_baseline = loaded_projection.usage();
    let loaded_shared = usage_with_adjust(&loaded, &loaded_projection, loaded_child);
    let loaded_distinct = usage_with_adjust(&loaded, &loaded_projection, loaded_distinct);

    assert_eq!(loaded_baseline, source_baseline);
    assert_eq!(loaded_shared, loaded_distinct);
    assert_eq!(loaded_shared, source_shared);
}

#[test]
fn shared_payload_box_roots_keep_distinct_physical_word_owners() {
    let mut stores = Stores::new();
    let root = stores.freeze_node_list(&[Node::Char {
        font: crate::font::NULL_FONT,
        ch: 'x',
        origin: OriginRef::unknown(),
    }]);
    set_box(&mut stores, 0, root.clone());
    let single =
        super::main_memory_usage_without_scratch(&stores).expect("single box root projects");

    set_box(&mut stores, 1, root);
    let aliased =
        super::main_memory_usage_without_scratch(&stores).expect("aliased box roots project");

    let (single, aliased) = (single.usage(), aliased.usage());
    assert_eq!(aliased.variable, single.variable);
    assert_eq!(aliased.dynamic, single.dynamic + 1);
    assert_eq!(aliased.dynamic_extent, single.dynamic_extent + 1);
}

#[test]
fn recursive_box_copy_peak_depends_on_copied_ligature_units() {
    fn projection_for(orig_len: usize) -> super::CopyNodeListProjection {
        let mut stores = Stores::new();
        let ligatures = freeze_ref(
            &mut stores,
            &[Node::Lig {
                font: crate::font::NULL_FONT,
                ch: 'A',
                orig: vec!['A'; orig_len],
                left_hit: false,
                right_hit: false,
                origins: vec![OriginRef::unknown(); orig_len],
            }],
        );
        let horizontal = freeze_ref(
            &mut stores,
            &[Node::HList(BoxNode::new(BoxNodeFields {
                width: Scaled::from_raw(0),
                height: Scaled::from_raw(0),
                depth: Scaled::from_raw(0),
                shift: Scaled::from_raw(0),
                box_lr: BoxLr::Normal,
                glue_set: GlueSetRatio::ZERO,
                glue_sign: Sign::Normal,
                glue_order: Order::Normal,
                children: ligatures,
            }))],
        );
        let root = stores.freeze_node_list(&[Node::VList(BoxNode::new(BoxNodeFields {
            width: Scaled::from_raw(0),
            height: Scaled::from_raw(0),
            depth: Scaled::from_raw(0),
            shift: Scaled::from_raw(0),
            box_lr: BoxLr::Normal,
            glue_set: GlueSetRatio::ZERO,
            glue_sign: Sign::Normal,
            glue_order: Order::Normal,
            children: horizontal,
        }))]);
        set_box(&mut stores, 0, root);
        let root = stores.box_reg_ref(0).expect("owned box root");
        super::main_memory_usage_without_scratch(&stores)
            .expect("box graph projects")
            .box_copy_projections[&root.id()]
    }

    let nine = projection_for(9);
    let ten = projection_for(10);
    assert_eq!(nine.high_words, 9);
    assert_eq!(ten.high_words, 10);
    assert_eq!(nine.high_peak, 13);
    assert_eq!(ten.high_peak, 14);
    assert_eq!(nine.high_peak + 1, ten.high_peak);
}

#[test]
fn format_round_trip_preserves_physical_diagnostic_leader_children() {
    let mut stores = Stores::new();
    let semantic_children = freeze_ref(&mut stores, &[Node::Penalty(3)]);
    let diagnostic_children = freeze_ref(&mut stores, &[Node::Penalty(4)]);
    let mut leader_box = BoxNode::new(BoxNodeFields {
        width: Scaled::from_raw(0),
        height: Scaled::from_raw(0),
        depth: Scaled::from_raw(0),
        shift: Scaled::from_raw(0),
        box_lr: BoxLr::Normal,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children: semantic_children,
    });
    leader_box.diagnostic_children = Some(diagnostic_children);
    let glue = stores.intern_glue_in_domain(GlueSpec::ZERO, None);
    let root = stores.freeze_node_list(&[Node::Glue {
        spec: glue,
        kind: GlueKind::Leaders,
        leader: Some(LeaderPayload::HList(leader_box)),
    }]);
    for child in root
        .nodes()
        .iter()
        .flat_map(|node| node.physical_children().collect::<Vec<_>>())
    {
        assert!(
            root.resolve(child).is_some(),
            "freshly frozen physical child must belong to its owner"
        );
    }
    set_box(&mut stores, 0, root);
    let installed = stores.box_reg_ref(0).expect("installed box root");
    for child in installed
        .nodes()
        .iter()
        .flat_map(|node| node.physical_children().collect::<Vec<_>>())
    {
        assert!(
            installed.resolve(child).is_some(),
            "installed physical child must belong to its owner"
        );
    }

    let restored = frozen_round_trip(&stores);
    let restored_root = restored.box_reg_ref(0).expect("restored box root");
    let Some(NodeRef::Glue {
        leader: Some(LeaderPayload::HList(restored_box)),
        ..
    }) = restored_root.nodes().first()
    else {
        panic!("expected restored hlist leader")
    };
    let semantic_children = restored_root
        .resolve(restored_box.children)
        .expect("semantic leader children belong to restored owner");
    assert!(matches!(
        semantic_children.nodes().first(),
        Some(NodeRef::Penalty(3))
    ));
    let diagnostic = restored_box
        .diagnostic_children
        .expect("restored leader diagnostic child");
    let diagnostic = restored_root
        .resolve(diagnostic)
        .expect("diagnostic leader children belong to restored owner");
    assert!(matches!(
        diagnostic.nodes().first(),
        Some(NodeRef::Penalty(4))
    ));
}

#[test]
fn format_dump_resets_only_the_optional_etex_state_cell() {
    // e-TeX change [50.1307] clears every optional e-TeX state variable
    // before tex.web §1307 serializes `eqtb`. Ordinary neighboring e-TeX
    // integer parameters remain part of the format.
    let mut stores = Stores::new();
    stores.set_int_param(IntParam::TEX_XET_STATE, 1);
    stores.set_int_param(IntParam::SAVING_V_DISCARDS, 2);

    let restored = frozen_round_trip(&stores);

    assert_eq!(restored.int_param(IntParam::TEX_XET_STATE), 0);
    assert_eq!(restored.int_param(IntParam::SAVING_V_DISCARDS), 2);
}

#[test]
fn format_round_trip_preserves_every_extended_register_family_at_boundaries() {
    let mut stores = Stores::new();
    for (index, value) in [(255, 1), (256, 2), (32_767, 3)] {
        stores.set_count(index, value);
        stores.set_dimen(index, Scaled::from_raw(value));
        let glue = stores.intern_glue_in_domain(
            GlueSpec {
                width: Scaled::from_raw(value),
                ..GlueSpec::ZERO
            },
            None,
        );
        stores.set_skip(index, &glue);
        stores.set_muskip(index, glue);
        let token_list = stores.intern_token_list(&[Token::Char {
            ch: char::from_digit(value as u32, 10).expect("single digit"),
            cat: Catcode::Other,
        }]);
        stores.set_toks(index, token_list);
        let list = stores.freeze_node_list(&[Node::Penalty(value)]);
        set_box(&mut stores, index, list);
    }

    let restored = frozen_round_trip(&stores);
    for (index, value) in [(255, 1), (256, 2), (32_767, 3)] {
        assert_eq!(restored.count(index), value);
        assert_eq!(restored.dimen(index), Scaled::from_raw(value));
        assert_eq!(
            restored.glue(restored.skip(index)).width,
            Scaled::from_raw(value)
        );
        assert_eq!(
            restored.glue(restored.muskip(index)).width,
            Scaled::from_raw(value)
        );
        assert_eq!(
            restored.tokens(restored.toks(index)),
            &[Token::Char {
                ch: char::from_digit(value as u32, 10).expect("single digit"),
                cat: Catcode::Other,
            }]
        );
        let list = restored.box_reg_ref(index).expect("restored sparse box");
        assert!(matches!(list.nodes().first(), Some(NodeRef::Penalty(found)) if found == value));
    }
}

#[test]
fn frozen_environment_and_macro_rows_install_exact_region_coordinates() {
    let mut stores = Stores::new();
    let register = stores.intern_token_list(&[Token::Char {
        ch: 'R',
        cat: Catcode::Other,
    }]);
    let parameter = stores.intern_token_list(&[Token::param(1)]);
    let replacement = stores.intern_token_list(&[Token::Char {
        ch: 'M',
        cat: Catcode::Other,
    }]);
    stores.set_toks_global(300, register);
    stores.set_tok_param_option_global(TokParam::EVERY_JOB, Some(replacement));
    let definition = stores.intern_macro(MacroMeaning::new(
        MeaningFlags::from_bits(0),
        parameter,
        replacement,
    ));
    let macro_symbol = stores.intern("owned-macro");
    stores.set_meaning_global(
        macro_symbol,
        Meaning::Macro {
            flags: MeaningFlags::from_bits(0),
            definition: definition.id(),
        },
    );
    assert_eq!(definition.raw(), 0);
    assert_eq!(stores.runtime_values.macro_len(), 1);
    let encoded = stores.encode_frozen_format().expect("encode macro format");
    assert_eq!(
        u32::from_le_bytes(encoded.macros[4..8].try_into().expect("macro count field")),
        1
    );

    let mut restored = frozen_round_trip(&stores);
    assert_eq!(restored.runtime_values.macro_len(), 1);
    for (cell, id) in [
        (CellId::new(BankTag::Toks, 300), restored.toks(300)),
        (
            CellId::new(BankTag::TokParam, u32::from(TokParam::EVERY_JOB.raw())),
            restored.tok_param(TokParam::EVERY_JOB),
        ),
    ] {
        let env_coordinate = restored
            .env
            .token_root(cell)
            .expect("format Env token coordinate");
        assert_eq!(env_coordinate.id(), id);
        assert!(restored.runtime_values.contains_token(id));
        let base = restored
            .env
            .testing_format_base()
            .iter()
            .find(|entry| entry.cell == cell)
            .expect("token cell is installed in immutable format base");
        assert!(base.token_root.is_some_and(|root| root.id() == id));
    }

    let definition = restored
        .runtime_values
        .macro_id_at(0)
        .expect("loaded macro definition");
    let loaded_meaning = restored.macro_definition(definition);
    let loaded_parameter = loaded_meaning.parameter_text();
    let loaded_replacement = loaded_meaning.replacement_text();
    assert_eq!(
        restored.tokens(loaded_parameter).tokens(),
        &[Token::param(1)]
    );
    assert_eq!(
        restored.tokens(loaded_replacement).tokens(),
        &[Token::Char {
            ch: 'M',
            cat: Catcode::Other,
        }]
    );

    let overlay = restored.intern_token_list(&[Token::Char {
        ch: 'O',
        cat: Catcode::Other,
    }]);
    restored.enter_group();
    restored.set_toks(300, overlay);
    assert_eq!(restored.toks(300), overlay);
    let _ = restored.leave_group();
    assert_eq!(
        restored.tokens(restored.toks(300)).tokens(),
        stores.tokens(register).tokens()
    );
}

#[test]
fn format_round_trip_preserves_all_box_lr_states() {
    let mut stores = Stores::new();
    let empty = NodeListRef::empty();
    for (register, box_lr) in [
        (10, BoxLr::Normal),
        (11, BoxLr::Reversed),
        (12, BoxLr::DList),
    ] {
        let list = stores.freeze_node_list(&[Node::HList(BoxNode::new(BoxNodeFields {
            width: Scaled::from_raw(0),
            height: Scaled::from_raw(0),
            depth: Scaled::from_raw(0),
            shift: Scaled::from_raw(0),
            box_lr,
            glue_set: GlueSetRatio::ZERO,
            glue_sign: Sign::Normal,
            glue_order: Order::Normal,
            children: empty.clone(),
        }))]);
        set_box(&mut stores, register, list);
    }

    let restored = frozen_round_trip(&stores);
    for (register, expected) in [
        (10, BoxLr::Normal),
        (11, BoxLr::Reversed),
        (12, BoxLr::DList),
    ] {
        let list = restored
            .box_reg_ref(register)
            .expect("restored box register");
        let Some(NodeRef::HList(box_node)) = list.nodes().first() else {
            panic!("expected restored hlist")
        };
        assert_eq!(box_node.box_lr, expected);
        assert!(box_node.diagnostic_children.is_none());
    }
}

#[test]
fn format_preserves_known_undefined_control_sequence_names() {
    // TeX82 §256 keeps every name entered by `id_lookup`, independently of
    // its current meaning, and §1309 dumps the complete occupied hash table.
    let mut stores = Stores::new();
    let known = stores.intern("known-but-undefined");
    assert_eq!(stores.meaning(known), crate::meaning::Meaning::Undefined);

    let restored = frozen_round_trip(&stores);
    let restored_name = restored
        .symbol("known-but-undefined")
        .expect("undefined name remains known after format load");

    assert_eq!(
        restored.meaning(restored_name),
        crate::meaning::Meaning::Undefined
    );
}

#[test]
fn format_preserves_multiple_undefined_names_in_interner_order() {
    let mut stores = Stores::new();
    let first = stores.intern("first-undefined");
    let middle = stores.intern("middle-defined");
    stores.set_meaning(middle, crate::meaning::Meaning::Relax);
    let last = stores.intern("last-undefined");

    let restored = frozen_round_trip(&stores);

    for (name, raw) in [
        ("first-undefined", first.raw()),
        ("middle-defined", middle.raw()),
        ("last-undefined", last.raw()),
    ] {
        assert_eq!(
            restored.symbol(name).expect("name survives").raw(),
            raw,
            "format round trip preserves the occupied hash-table order"
        );
    }
}

#[test]
fn format_preserves_undefined_active_character_names() {
    let mut stores = Stores::new();
    let active = stores.intern_active_character('~');
    assert_eq!(stores.meaning(active), crate::meaning::Meaning::Undefined);

    let restored = frozen_round_trip(&stores);

    let restored_active = restored
        .active_character_symbol('~')
        .expect("undefined active character remains known");
    assert_eq!(
        restored.meaning(restored_active),
        crate::meaning::Meaning::Undefined
    );
}

#[test]
fn format_preserves_names_reverted_to_undefined() {
    let mut stores = Stores::new();
    let name = stores.intern("reverted-to-undefined");
    stores.set_meaning(name, crate::meaning::Meaning::Relax);
    stores.set_meaning(name, crate::meaning::Meaning::Undefined);

    let restored = frozen_round_trip(&stores);

    let restored_name = restored
        .symbol("reverted-to-undefined")
        .expect("reverted name remains known");
    assert_eq!(
        restored.meaning(restored_name),
        crate::meaning::Meaning::Undefined
    );
}

#[test]
fn format_does_not_invent_absent_control_sequence_names() {
    let stores = Stores::new();
    assert!(stores.symbol("never-interned").is_none());

    let restored = frozen_round_trip(&stores);

    assert!(restored.symbol("never-interned").is_none());
}
