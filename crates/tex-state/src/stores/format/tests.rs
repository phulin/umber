use super::{FrozenCoreSections, FrozenNodeSection, FrozenNonNodeSections};
use crate::cell::{BankTag, CellId};
use crate::env::banks::{IntParam, TokParam};
use crate::glue::GlueSpec;
use crate::glue::Order;
use crate::ids::MacroDefinitionId;
use crate::macro_store::MacroMeaning;
use crate::meaning::{Meaning, MeaningFlags};
use crate::node::{BoxLr, BoxNode, BoxNodeFields, DiscKind, GlueKind, LeaderPayload, Node, Sign};
use crate::node_arena::NodeRef;
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

#[test]
fn format_round_trip_preserves_diagnostic_disc_replacement_count() {
    let mut stores = Stores::new();
    let empty = stores.freeze_node_list(&[]);
    let root = stores.freeze_node_list(&[Node::Disc {
        kind: DiscKind::AutomaticHyphen,
        pre: empty,
        post: empty,
        replace: empty,
        physical_replace_count: 3,
    }]);
    stores.set_box_reg(0, root);

    let restored = frozen_round_trip(&stores);
    let restored_root = restored.box_reg(0).expect("restored box root");
    assert!(matches!(
        restored.nodes(restored_root).first(),
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
    let semantic_children = stores.freeze_node_list(&[Node::Penalty(1)]);
    let diagnostic_children = stores.freeze_node_list(&[Node::Penalty(2)]);
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
    stores.set_box_reg(0, root);

    let restored = frozen_round_trip(&stores);
    let restored_root = restored.box_reg(0).expect("restored box root");
    let Some(NodeRef::HList(restored_box)) = restored.nodes(restored_root).first() else {
        panic!("expected restored hlist")
    };
    assert!(matches!(
        restored.nodes(restored_box.children).first(),
        Some(NodeRef::Penalty(1))
    ));
    let diagnostic = restored_box
        .diagnostic_children
        .expect("restored diagnostic child");
    assert!(matches!(
        restored.nodes(diagnostic).first(),
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
        let semantic_children = stores.freeze_node_list(&[
            direct('A'),
            direct('/'),
            direct('B'),
            direct('B'),
            direct('C'),
            direct('A'),
        ]);
        let diagnostic_children = stores.freeze_node_list(&[
            direct('Z'),
            direct('Y'),
            direct('X'),
            direct('W'),
            direct('V'),
            direct('U'),
        ]);
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
        stores.set_box_reg(0, root);
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
fn recursive_box_copy_composes_with_live_projection_owners() {
    fn direct(ch: char) -> Node {
        Node::Char {
            font: crate::font::NULL_FONT,
            ch,
            origin: OriginRef::unknown(),
        }
    }

    fn box_node(children: crate::ids::NodeListId) -> BoxNode {
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
    let ligatures = stores.freeze_node_list(&[Node::Lig {
        font: crate::font::NULL_FONT,
        ch: 'A',
        orig: vec!['A'; 10],
        left_hit: false,
        right_hit: false,
        origins: vec![OriginRef::unknown(); 10],
    }]);
    let horizontal = stores.freeze_node_list(&[Node::HList(box_node(ligatures))]);
    let diagnostic = stores.freeze_node_list(&[direct('x'), direct('y'), direct('z')]);
    let mut root_box = box_node(horizontal);
    root_box.diagnostic_children = Some(diagnostic);
    let root = stores.freeze_node_list(&[Node::VList(root_box)]);
    stores.set_box_reg(254, root);
    let root = stores.box_reg(254).expect("promoted box root");

    let mut projection =
        super::main_memory_usage_without_scratch(&stores).expect("box graph projects");
    let baseline = projection.usage();
    assert_eq!(baseline.dynamic_extent, baseline.dynamic + 3);
    let copied = projection
        .usage_with_box_copy(root, 1)
        .expect("live box copy projects");
    // TeX82 §204 has four simultaneously live temporary heads on the path
    // root -> vlist -> hlist -> lig_ptr. The ten copied character cells are
    // retained in traversal order, while the backed-up scan terminator stays
    // live for the whole operation.
    assert_eq!(copied.variable, baseline.variable + 16);
    assert_eq!(copied.dynamic, baseline.dynamic + 1 + 10);
    assert_eq!(copied.dynamic_extent, baseline.dynamic_extent + 1 + 14);

    // A shared read or destructive `\box` does not invoke §204, so merely
    // consulting the cached projection changes no allocator coordinate.
    assert_eq!(projection.usage(), baseline);

    projection
        .update_box_root(&stores, Some(root), None, true)
        .expect("root removal projects");
    assert!(
        projection.usage_with_box_copy(root, 1).is_none(),
        "released owners cannot contribute to a later operation peak"
    );
}

#[test]
fn recursive_box_copy_peak_depends_on_copied_ligature_units() {
    fn projection_for(orig_len: usize) -> super::CopyNodeListProjection {
        let mut stores = Stores::new();
        let ligatures = stores.freeze_node_list(&[Node::Lig {
            font: crate::font::NULL_FONT,
            ch: 'A',
            orig: vec!['A'; orig_len],
            left_hit: false,
            right_hit: false,
            origins: vec![OriginRef::unknown(); orig_len],
        }]);
        let horizontal = stores.freeze_node_list(&[Node::HList(BoxNode::new(BoxNodeFields {
            width: Scaled::from_raw(0),
            height: Scaled::from_raw(0),
            depth: Scaled::from_raw(0),
            shift: Scaled::from_raw(0),
            box_lr: BoxLr::Normal,
            glue_set: GlueSetRatio::ZERO,
            glue_sign: Sign::Normal,
            glue_order: Order::Normal,
            children: ligatures,
        }))]);
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
        stores.set_box_reg(0, root);
        let root = stores.box_reg(0).expect("promoted box root");
        super::main_memory_usage_without_scratch(&stores)
            .expect("box graph projects")
            .box_copy_projections[&root]
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
    let semantic_children = stores.freeze_node_list(&[Node::Penalty(3)]);
    let diagnostic_children = stores.freeze_node_list(&[Node::Penalty(4)]);
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
    stores.set_box_reg(0, root);

    let restored = frozen_round_trip(&stores);
    let restored_root = restored.box_reg(0).expect("restored box root");
    let Some(NodeRef::Glue {
        leader: Some(LeaderPayload::HList(restored_box)),
        ..
    }) = restored.nodes(restored_root).first()
    else {
        panic!("expected restored hlist leader")
    };
    assert!(matches!(
        restored.nodes(restored_box.children).first(),
        Some(NodeRef::Penalty(3))
    ));
    let diagnostic = restored_box
        .diagnostic_children
        .expect("restored leader diagnostic child");
    assert!(matches!(
        restored.nodes(diagnostic).first(),
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
        stores.set_box_reg(index, list);
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
        let list = restored.box_reg(index).expect("restored sparse box");
        assert!(
            matches!(restored.nodes(list).first(), Some(NodeRef::Penalty(found)) if found == value)
        );
    }
}

#[test]
fn frozen_environment_and_macro_rows_install_exact_token_owners() {
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
    assert_eq!(stores.macros.watermark().definitions, 1);
    let encoded = stores.encode_frozen_format().expect("encode macro format");
    assert_eq!(
        u32::from_le_bytes(encoded.macros[4..8].try_into().expect("macro count field")),
        1
    );

    let mut restored = frozen_round_trip(&stores);
    assert_eq!(restored.macros.watermark().definitions, 1);
    for (cell, id) in [
        (CellId::new(BankTag::Toks, 300), restored.toks(300)),
        (
            CellId::new(BankTag::TokParam, u32::from(TokParam::EVERY_JOB.raw())),
            restored.tok_param(TokParam::EVERY_JOB),
        ),
    ] {
        let env_root = restored
            .env
            .token_root(cell)
            .expect("format Env token owner");
        let store_root = restored.tokens.owner(id).expect("loaded token owner");
        assert!(env_root.ptr_eq(&store_root));
        let base = restored
            .env
            .testing_format_base()
            .iter()
            .find(|entry| entry.cell == cell)
            .expect("token cell is installed in immutable format base");
        assert!(
            base.token_root
                .as_ref()
                .is_some_and(|root| root.ptr_eq(&store_root))
        );
    }

    let definition = restored
        .macros
        .resolve_stored(MacroDefinitionId::new(0))
        .expect("loaded macro definition");
    let (macro_parameter, macro_replacement) = restored.macros.testing_token_roots(definition);
    let loaded_meaning = restored.macros.get(definition);
    let loaded_parameter = loaded_meaning.parameter_text();
    let loaded_replacement = loaded_meaning.replacement_text();
    assert!(
        macro_parameter.ptr_eq(
            &restored
                .tokens
                .owner(loaded_parameter)
                .expect("parameter owner")
        )
    );
    assert!(
        macro_replacement.ptr_eq(
            &restored
                .tokens
                .owner(loaded_replacement)
                .expect("replacement owner")
        )
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
    let empty = stores.freeze_node_list(&[]);
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
            children: empty,
        }))]);
        stores.set_box_reg(register, list);
    }

    let restored = frozen_round_trip(&stores);
    for (register, expected) in [
        (10, BoxLr::Normal),
        (11, BoxLr::Reversed),
        (12, BoxLr::DList),
    ] {
        let list = restored.box_reg(register).expect("restored box register");
        let Some(NodeRef::HList(box_node)) = restored.nodes(list).first() else {
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
