use super::{FrozenCoreSections, FrozenNodeSection, FrozenNonNodeSections, MemoNodeBundle};
use crate::env::banks::IntParam;
use crate::glue::GlueSpec;
use crate::glue::Order;
use crate::node::{BoxLr, BoxNode, BoxNodeFields, DiscKind, GlueKind, LeaderPayload, Node, Sign};
use crate::node_arena::{NodeListRef, NodeRef};
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
fn source_and_loaded_children_preserve_nested_values() {
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
    assert_eq!(source_box.children.nodes().to_vec(), [Node::Penalty(17)]);

    set_box(&mut source, 0, source_parent);
    let mut loaded = frozen_round_trip(&source);
    let loaded_parent = loaded.box_reg_ref(0).expect("loaded parent root");
    let Node::HList(loaded_box) = loaded_parent.get(0).expect("loaded parent node") else {
        panic!("expected loaded hlist")
    };
    assert_eq!(loaded_box.children.nodes().to_vec(), [Node::Penalty(17)]);

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
    assert_eq!(dynamic_box.children, loaded_parent);
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
    target.set_int_param(IntParam::TRACING_STATS, 17);
    let before = target
        .encode_frozen_format()
        .expect("baseline destination format encodes");
    assert!(matches!(
        target.import_memo_node_list(&corrupted, 16, 16, 1024),
        Err(super::StoreFormatError::Invalid(
            "memo node semantic identity"
        ))
    ));
    assert_eq!(target.int_param(IntParam::TRACING_STATS), 17);

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
    assert_eq!(target.int_param(IntParam::TRACING_STATS), 17);
    let after = target
        .encode_frozen_format()
        .expect("unchanged destination format encodes");
    assert_eq!(
        [
            before.env.as_slice(),
            &before.names,
            &before.names_lookup,
            &before.token_lists,
            &before.macros,
            &before.glue,
            &before.fonts,
            &before.code_tables,
            &before.hyphenation,
            &before.nodes,
        ],
        [
            after.env.as_slice(),
            &after.names,
            &after.names_lookup,
            &after.token_lists,
            &after.macros,
            &after.glue,
            &after.fonts,
            &after.code_tables,
            &after.hyphenation,
            &after.nodes,
        ],
        "failed memo imports must leave every portable destination section unchanged"
    );
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
        .expect("semantic children remain readable");
    assert!(matches!(
        semantic_children.nodes().first(),
        Some(NodeRef::Penalty(1))
    ));
    let diagnostic = restored_box
        .diagnostic_children
        .expect("restored diagnostic child");
    let diagnostic = restored_root
        .resolve(diagnostic)
        .expect("diagnostic children remain readable");
    assert!(matches!(
        diagnostic.nodes().first(),
        Some(NodeRef::Penalty(2))
    ));
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
            "freshly frozen physical child remains readable"
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
            "installed physical child remains readable"
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
        .expect("semantic leader children remain readable");
    assert!(matches!(
        semantic_children.nodes().first(),
        Some(NodeRef::Penalty(3))
    ));
    let diagnostic = restored_box
        .diagnostic_children
        .expect("restored leader diagnostic child");
    let diagnostic = restored_root
        .resolve(diagnostic)
        .expect("diagnostic leader children remain readable");
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
        stores.set_skip(index, glue);
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
