use super::{FormatEnvValue, FormatListKey, FormatNode, StoreFormat, StoreFormatError};
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
