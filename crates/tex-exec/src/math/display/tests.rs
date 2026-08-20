use tex_state::Universe;
use tex_state::glue::Order;
use tex_state::node::{BoxLr, BoxNode, BoxNodeFields, Direction, KernKind, Node, Sign};
use tex_state::node_arena::PageListId;
use tex_state::scaled::{GlueSetRatio, Scaled};

use super::{display_line_prototype, package_directed_display_line};

fn box_node(width: i32, height: i32, depth: i32, shift: i32, box_lr: BoxLr) -> BoxNode {
    let children = PageListId::empty();
    BoxNode::new(BoxNodeFields {
        width: Scaled::from_raw(width),
        height: Scaled::from_raw(height),
        depth: Scaled::from_raw(depth),
        shift: Scaled::from_raw(shift),
        box_lr,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children,
    })
}

#[test]
fn etex_display_prototype_replaces_its_list_without_repacking() {
    // Merged e-TeX §§1475 and 1478--1480: a display after a nonempty
    // paragraph copies the saved last-line prototype and replaces its list.
    // Only the no-prototype control calls hpack to create a new line box.
    let mut stores = Universe::new_with_plain_catcodes();
    stores.enable_geometry_observation();
    let last_line = box_node(100, 7, 3, 5, BoxLr::Normal);
    let prototype = display_line_prototype(&mut stores, last_line);
    let display = box_node(10, 8, 2, 0, BoxLr::DList);
    let before = stores.geometry_observation_len();

    let reused = package_directed_display_line(
        &mut stores,
        display,
        Some(prototype),
        Scaled::from_raw(20),
        Scaled::from_raw(10),
        Scaled::from_raw(100),
        1,
    );

    assert_eq!(stores.geometry_observation_len(), before);
    assert_eq!(
        (reused.width.raw(), reused.height.raw(), reused.depth.raw()),
        (100, 8, 2)
    );
    assert_eq!(reused.shift.raw(), 5);
    assert!(matches!(
        stores
            .page_node_list(reused.children)
            .expect("display children belong to the page arena")
            .nodes(),
        [
            Node::Direction(Direction::BeginM),
            Node::Kern { amount: left, kind: KernKind::Font },
            Node::HList(_),
            Node::Kern { amount: right, kind: KernKind::Font },
            Node::Direction(Direction::EndM),
        ] if left.raw() == 25 && right.raw() == 65
    ));

    let display = box_node(10, 8, 2, 0, BoxLr::DList);
    let packed = package_directed_display_line(
        &mut stores,
        display,
        None,
        Scaled::from_raw(20),
        Scaled::from_raw(10),
        Scaled::from_raw(100),
        1,
    );
    assert_eq!(stores.geometry_observation_len(), before + 1);
    assert_eq!(packed.shift.raw(), 10);
}
