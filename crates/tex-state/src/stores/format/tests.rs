use super::{FrozenCoreSections, FrozenNodeSection, FrozenNonNodeSections};
use crate::env::banks::IntParam;
use crate::glue::GlueSpec;
use crate::glue::Order;
use crate::node::{BoxLr, BoxNode, BoxNodeFields, DiscKind, Node, Sign};
use crate::node_arena::NodeRef;
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
        let glue = stores.intern_glue(GlueSpec {
            width: Scaled::from_raw(value),
            ..GlueSpec::ZERO
        });
        stores.set_skip(index, glue);
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
