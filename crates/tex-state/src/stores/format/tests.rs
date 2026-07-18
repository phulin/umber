use super::{
    FormatEnvEntry, FormatEnvValue, FormatListKey, FormatNode, StoreFormat, StoreFormatError,
};
use crate::cell::{BankTag, CellId};
use crate::node::Node;
use crate::stores::Stores;
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
    root.nodes[0] = FormatNode::Adjust(FormatListKey {
        survivor_root: None,
        start: u32::MAX,
        len: 1,
    });

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
    let root = stores.freeze_node_list(&[Node::Adjust(child)]);
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
