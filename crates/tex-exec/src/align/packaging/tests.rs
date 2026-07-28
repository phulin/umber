use super::*;
use tex_state::glue::{GlueSpec, Order};
use tex_state::node::{GlueKind, UnsetKind};
use tex_state::scaled::Scaled;

fn sp(raw: i32) -> Scaled {
    Scaled::from_raw(raw * Scaled::UNITY)
}

#[test]
fn package_unset_cell_records_natural_extent_and_glue_orders() {
    let mut stores = Universe::new_with_plain_catcodes();
    let fil = stores.intern_glue(GlueSpec {
        width: sp(3),
        stretch: sp(7),
        stretch_order: Order::Fil,
        shrink: sp(4),
        shrink_order: Order::Fill,
    });
    let fill = stores.intern_glue(GlueSpec {
        width: sp(2),
        stretch: sp(9),
        stretch_order: Order::Fill,
        shrink: sp(6),
        shrink_order: Order::Fil,
    });
    let children = stores.freeze_node_list(&[
        Node::Rule {
            width: Some(sp(5)),
            height: Some(sp(2)),
            depth: Some(sp(1)),
        },
        Node::Glue {
            spec: fil,
            kind: GlueKind::Normal,
            leader: None,
        },
        Node::Glue {
            spec: fill,
            kind: GlueKind::Normal,
            leader: None,
        },
    ]);

    for (alignment, kind) in [
        (AlignmentKind::HAlign, UnsetKind::HBox),
        (AlignmentKind::VAlign, UnsetKind::VBox),
    ] {
        let expected = tex_typeset::measure_unset(&stores, children, kind);
        let Node::Unset(cell) = make_unset_node(&stores, children, kind, 3)
            .expect("a three-column span is far inside TeX82 \u{a7}110's max_quarterword")
        else {
            panic!("alignment cell must remain unset until fin_align");
        };

        assert_eq!(cell.kind, cell_unset_kind(alignment));
        assert_eq!(cell.span_count, 3);
        assert_eq!(cell.width, expected.width);
        assert_eq!(cell.height, expected.height);
        assert_eq!(cell.depth, expected.depth);
        assert_eq!(cell.stretch, expected.stretch);
        assert_eq!(cell.stretch_order, expected.stretch_order);
        assert_eq!(cell.shrink, expected.shrink);
        assert_eq!(cell.shrink_order, expected.shrink_order);
        assert_eq!(cell.stretch_order, Order::Fill);
        assert_eq!(cell.stretch, sp(9));
        assert_eq!(cell.shrink_order, Order::Fill);
        assert_eq!(cell.shrink, sp(4));
    }
}
