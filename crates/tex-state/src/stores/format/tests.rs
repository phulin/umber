use super::{
    FormatEnvEntry, FormatEnvValue, FormatListKey, FormatNode, StoreFormat, StoreFormatError,
};
use crate::cell::{BankTag, CellId};
use crate::env::banks::IntParam;
use crate::glue::GlueSpec;
use crate::glue::Order;
use crate::node::{BoxLr, BoxNode, BoxNodeFields, Node, Sign};
use crate::node_arena::NodeRef;
use crate::scaled::{GlueSetRatio, Scaled};
use crate::stores::Stores;
use crate::token::{Catcode, Token};
use std::panic::{AssertUnwindSafe, catch_unwind};

fn assert_invalid_without_unwind(format: StoreFormat) {
    let result = catch_unwind(AssertUnwindSafe(|| format.restore()));
    assert!(
        result.is_ok(),
        "malformed format must return an error, not unwind"
    );
    assert!(matches!(
        result.expect("checked above"),
        Err(StoreFormatError::Invalid(_))
    ));
}

#[test]
fn missing_node_dto_reference_fails_before_store_publication() {
    let mut stores = Stores::new();
    let list = stores.freeze_node_list(&[Node::Penalty(7)]);
    stores.set_box_reg(0, list);
    let mut format = StoreFormat::capture(&stores).expect("capture valid format");
    let root = format
        .node_lists
        .last_mut()
        .expect("stored box contributes a node list");
    root.nodes[0] = FormatNode::Adjust {
        content: FormatListKey {
            survivor_root: None,
            start: u32::MAX,
            len: 1,
        },
        pre: false,
    };

    assert!(matches!(
        format.restore(),
        Err(StoreFormatError::Invalid("node child precedes dependency"))
    ));
}

#[test]
fn raw_box_environment_value_fails_before_store_publication() {
    let mut stores = Stores::new();
    let list = stores.freeze_node_list(&[Node::Penalty(7)]);
    stores.set_box_reg(0, list);
    let mut format = StoreFormat::capture(&stores).expect("capture valid format");
    let box_entry = format
        .env
        .iter_mut()
        .find(|entry| matches!(entry.value, FormatEnvValue::Box(_)))
        .expect("stored box contributes env DTO");
    box_entry.value = FormatEnvValue::Raw(0);

    assert!(matches!(
        format.restore(),
        Err(StoreFormatError::Invalid("raw box environment value"))
    ));
}

#[test]
fn environment_dto_codec_preserves_full_30_bit_cell_indices() {
    for index in [1 << 26, (1 << 30) - 1] {
        let entry = FormatEnvEntry {
            cell: CellId::new_global(BankTag::Meaning, index).raw(),
            value: FormatEnvValue::Raw(17),
        };
        let bytes = bincode::serialize(&entry).expect("encode detached environment entry");
        let decoded: FormatEnvEntry =
            bincode::deserialize(&bytes).expect("decode detached environment entry");
        let cell = CellId::from_raw(decoded.cell).expect("valid detached cell key");

        assert_eq!(cell.bank(), BankTag::Meaning);
        assert_eq!(cell.index(), index);
        assert!(cell.is_global());
        assert!(matches!(decoded.value, FormatEnvValue::Raw(17)));
    }
}

#[test]
fn isolated_transitional_restore_instrumentation_observes_prohibited_load_work() {
    let mut stores = Stores::new();
    let child = stores.freeze_node_list(&[Node::Penalty(7)]);
    let root = stores.freeze_node_list(&[Node::Adjust(crate::node::AdjustNode::ordinary(child))]);
    stores.set_box_reg(0, root);
    stores.set_count(0, 17);

    let _ = super::testing_take_transitional_format_work();
    let bytes = stores
        .encode_format()
        .expect("encode transitional test DTO");
    let capture_work = super::testing_take_transitional_format_work();
    assert!(capture_work.graph_key_remaps > 0);

    Stores::decode_format(&bytes).expect("restore transitional test DTO");
    let restore_work = super::testing_take_transitional_format_work();
    assert!(restore_work.semantic_reseals > 0);
    assert!(restore_work.assignment_replays > 0);
}

#[test]
fn format_dump_resets_only_the_optional_etex_state_cell() {
    // e-TeX change [50.1307] clears every optional e-TeX state variable
    // before tex.web §1307 serializes `eqtb`. Ordinary neighboring e-TeX
    // integer parameters remain part of the format.
    let mut stores = Stores::new();
    stores.set_int_param(IntParam::TEX_XET_STATE, 1);
    stores.set_int_param(IntParam::SAVING_V_DISCARDS, 2);

    let format = StoreFormat::capture(&stores).expect("capture e-TeX format state");
    let restored = format.restore().expect("restore e-TeX format state");

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

    let restored = StoreFormat::capture(&stores)
        .expect("capture sparse register format")
        .restore()
        .expect("restore sparse register format");
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

    let restored = StoreFormat::capture(&stores)
        .expect("capture box_lr format")
        .restore()
        .expect("restore box_lr format");
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

    let format = StoreFormat::capture(&stores).expect("capture format namespace");
    let restored = format.restore().expect("restore format namespace");
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

    let restored = StoreFormat::capture(&stores)
        .expect("capture ordered namespace")
        .restore()
        .expect("restore ordered namespace");

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

    let restored = StoreFormat::capture(&stores)
        .expect("capture active-character namespace")
        .restore()
        .expect("restore active-character namespace");

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

    let restored = StoreFormat::capture(&stores)
        .expect("capture reverted namespace")
        .restore()
        .expect("restore reverted namespace");

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

    let restored = StoreFormat::capture(&stores)
        .expect("capture namespace")
        .restore()
        .expect("restore namespace");

    assert!(restored.symbol("never-interned").is_none());
}

#[test]
fn reserved_environment_cell_key_fails_before_store_publication() {
    let stores = Stores::new();
    let mut format = StoreFormat::capture(&stores).expect("capture valid format");
    format.env.push(FormatEnvEntry {
        cell: u64::MAX,
        value: FormatEnvValue::Raw(1),
    });

    assert!(matches!(
        format.restore(),
        Err(StoreFormatError::Invalid("unknown environment cell"))
    ));
}

#[test]
fn every_direct_reference_class_is_validated_without_unwind() {
    let stores = Stores::new();

    let mut token = StoreFormat::capture(&stores).expect("capture valid format");
    token
        .token_lists
        .push(vec![super::FormatToken::Cs(u32::MAX)]);
    assert_invalid_without_unwind(token);

    let mut macro_ref = StoreFormat::capture(&stores).expect("capture valid format");
    macro_ref.macros.push(super::FormatMacro {
        flags: 0,
        parameter_text: u32::MAX,
        replacement_text: 0,
    });
    assert_invalid_without_unwind(macro_ref);

    let mut register = StoreFormat::capture(&stores).expect("capture valid format");
    register.env.push(FormatEnvEntry {
        cell: CellId::new(BankTag::Toks, 32_768).raw(),
        value: FormatEnvValue::Raw(0),
    });
    assert_invalid_without_unwind(register);

    let mut content = StoreFormat::capture(&stores).expect("capture valid format");
    content.env.push(FormatEnvEntry {
        cell: CellId::new(BankTag::GlueParam, 0).raw(),
        value: FormatEnvValue::Raw(u64::from(u32::MAX)),
    });
    assert_invalid_without_unwind(content);

    let mut duplicate_code = StoreFormat::capture(&stores).expect("capture valid format");
    duplicate_code.code_tables.push(super::FormatCodeTables {
        code: 'x' as u32,
        catcode: 12,
        lccode: 0,
        uccode: 0,
        sfcode: 1000,
        mathcode: 0,
        delcode: -1,
    });
    duplicate_code.code_tables.push(super::FormatCodeTables {
        code: 'x' as u32,
        catcode: 12,
        lccode: 0,
        uccode: 0,
        sfcode: 1000,
        mathcode: 0,
        delcode: -1,
    });
    assert_invalid_without_unwind(duplicate_code);
}
