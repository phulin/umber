use super::{
    FormatEnvEntry, FormatEnvValue, FormatListKey, FormatNode, StoreFormat, StoreFormatError,
};
use crate::cell::{BankTag, CellId};
use crate::node::Node;
use crate::stores::Stores;

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
